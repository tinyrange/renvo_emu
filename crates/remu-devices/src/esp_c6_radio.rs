use aes::Aes128;
use aes::cipher::{Array, BlockCipherEncrypt, KeyInit};
use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
include!("esp_c6_ble_modem.rs");
include!("esp_c6_ieee802154.rs");

const C6_BLE_ECB_START: u64 = 0x404;
const C6_BLE_ECB_LENGTH: u64 = 0x40c;
const C6_BLE_ECB_KEY_BASE: u64 = 0x410;
const C6_BLE_ECB_INPUT_ADDRESS: u64 = 0x420;
const C6_BLE_ECB_OUTPUT_ADDRESS: u64 = 0x424;
const C6_BLE_ECB_STATUS: u64 = 0x4c4;
const C6_BLE_CCM_START: u64 = 0x428;
const C6_BLE_CCM_RESET: u64 = 0x42c;
const C6_BLE_CCM_CONFIG: u64 = 0x430;
const C6_BLE_CCM_RESULT: u64 = 0x434;
const C6_BLE_CCM_INPUT_ADDRESS: u64 = 0x438;
const C6_BLE_CCM_OUTPUT_ADDRESS: u64 = 0x43c;
const C6_BLE_CCM_KEY_BASE: u64 = 0x440;
const C6_BLE_CCM_COUNTER_LOW: u64 = 0x450;
const C6_BLE_CCM_COUNTER_IV0: u64 = 0x454;
const C6_BLE_CCM_IV1: u64 = 0x458;
const C6_BLE_CCM_IV2: u64 = 0x45c;
const C6_BLE_CCM_AAD: u64 = 0x460;
const C6_BLE_CCM_STATUS: u64 = 0x4c0;
const C6_BLE_BASEBAND_RESET: u64 = 0xff0;
const C6_BLE_BASEBAND_TIMER_CURRENT: u64 = 0x924;
const C6_BLE_BASEBAND_SCHEDULER_KICK: u64 = 0x028;
const C6_BLE_BASEBAND_SCHEDULER_STOP: u64 = 0x02c;
const C6_BLE_BASEBAND_SCHEDULER_HEAD: u64 = 0x8fc;
const C6_BLE_BASEBAND_SCHEDULER_CURRENT: u64 = 0x900;
const C6_BLE_BASEBAND_SCHEDULER_NEXT: u64 = 0x904;
const C6_BLE_BASEBAND_CURRENT_TX_BUFFER: u64 = 0x960;
const C6_BLE_BASEBAND_CURRENT_RX_BUFFER: u64 = 0x964;
const C6_BLE_BASEBAND_INTERRUPT_ENABLE0: u64 = 0x304;
const C6_BLE_BASEBAND_INTERRUPT_CLEAR0: u64 = 0x308;
const C6_BLE_BASEBAND_INTERRUPT_RAW0: u64 = 0x30c;
const C6_BLE_BASEBAND_INTERRUPT_ENABLE1: u64 = 0x314;
const C6_BLE_BASEBAND_INTERRUPT_CLEAR1: u64 = 0x318;
const C6_BLE_BASEBAND_INTERRUPT_RAW1: u64 = 0x31c;
const C6_BLE_BASEBAND_EVENT_END: u32 = 1 << 21;
const C6_BLE_BASEBAND_EVENT_RX: u32 = 1 << 27;
const C6_BLE_BASEBAND_EVENT_SUCCESS: u32 = 1 << 28;
// The C6 controller's native scheduler uses a 1 MHz tick. Renvo's ESP
// simulation timebase is 16 MHz, matching the SYSTIMER clock exposed to ESP-IDF.
const C6_BLE_BASEBAND_TICKS_PER_SCHEDULER_TICK: u64 = 16;

/// One register-programmed AES-128 ECB DMA request from the C6 BLE controller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EspC6BleEcbCommand {
    /// Guest address of the sixteen-byte plaintext block.
    pub input_address: u32,
    /// Guest address where the sixteen-byte ciphertext block is written.
    pub output_address: u32,
    /// Programmed transfer length in bytes.
    pub length: u32,
    key: [u8; 16],
}

impl EspC6BleEcbCommand {
    /// Encrypts one block with the key captured when firmware strobed START.
    pub fn encrypt_block(&self, input: [u8; 16]) -> [u8; 16] {
        let cipher = Aes128::new_from_slice(&self.key).expect("AES-128 key has fixed length");
        let mut block = Array::from(input);
        cipher.encrypt_block(&mut block);
        block.into()
    }
}

/// One native AES-CCM data-channel operation submitted by the C6 controller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EspC6BleCcmCommand {
    /// Guest address of the input payload, excluding the two-byte LL header.
    pub input_address: u32,
    /// Guest address of the output payload, excluding the two-byte LL header.
    pub output_address: u32,
    /// Plaintext payload length. Decryption consumes four additional MIC bytes.
    pub payload_length: u8,
    /// Whether this is an authenticated decryption operation.
    pub decrypt: bool,
    /// AES-128 session key programmed by the controller.
    pub key: [u8; 16],
    /// 39-bit link-layer packet counter.
    pub packet_counter: u64,
    /// Link-layer direction bit from the native nonce register.
    pub peripheral_to_central: bool,
    /// Eight-byte connection IV from the native nonce registers.
    pub iv: [u8; 8],
    /// Data-channel header used as CCM associated data.
    pub aad_header: u8,
    /// Raw native configuration word for legality validation.
    pub config: u32,
}

struct EspC6BleControlState {
    registers: Vec<u32>,
    pending_ecb: VecDeque<EspC6BleEcbCommand>,
    pending_ccm: VecDeque<EspC6BleCcmCommand>,
}

/// Scheduler-facing handle for C6 BLE control and modem-security operations.
#[derive(Clone)]
pub struct EspC6BleControlHandle {
    state: Arc<Mutex<EspC6BleControlState>>,
}

impl EspC6BleControlHandle {
    /// Removes the oldest ECB DMA command submitted by guest firmware.
    pub fn take_ecb_command(&self) -> Option<EspC6BleEcbCommand> {
        self.state
            .lock()
            .expect("ESP32-C6 BLE control lock poisoned")
            .pending_ecb
            .pop_front()
    }

    /// Marks the current ECB DMA command complete and releases the guest poll.
    pub fn complete_ecb(&self) {
        self.state
            .lock()
            .expect("ESP32-C6 BLE control lock poisoned")
            .registers[C6_BLE_ECB_STATUS as usize / 4] = 1;
    }

    /// Removes the oldest CCM DMA operation submitted by guest firmware.
    pub fn take_ccm_command(&self) -> Option<EspC6BleCcmCommand> {
        self.state
            .lock()
            .expect("ESP32-C6 BLE control lock poisoned")
            .pending_ccm
            .pop_front()
    }

    /// Completes the current CCM operation with native success/failure status.
    pub fn complete_ccm(&self, authenticated: bool) {
        let mut state = self
            .state
            .lock()
            .expect("ESP32-C6 BLE control lock poisoned");
        state.registers[C6_BLE_CCM_RESULT as usize / 4] = u32::from(authenticated);
        state.registers[C6_BLE_CCM_STATUS as usize / 4] = 1;
    }
}

/// ESP32-C6 BLE controller register page with native modem-security ECB DMA.
pub struct EspC6BleControl {
    name: String,
    state: Arc<Mutex<EspC6BleControlState>>,
}

impl EspC6BleControl {
    /// Creates a reset BLE controller page and its scheduler handle.
    pub fn new(name: impl Into<String>) -> (Self, EspC6BleControlHandle) {
        let state = Arc::new(Mutex::new(EspC6BleControlState {
            registers: vec![0; 0x800 / 4],
            pending_ecb: VecDeque::new(),
            pending_ccm: VecDeque::new(),
        }));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            EspC6BleControlHandle { state },
        )
    }
}

impl Device for EspC6BleControl {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || !offset.is_multiple_of(4) {
            return Err(DeviceError::new(
                "ESP32-C6 BLE control requires aligned word access",
            ));
        }
        self.state
            .lock()
            .expect("ESP32-C6 BLE control lock poisoned")
            .registers
            .get(offset as usize / 4)
            .copied()
            .map(u64::from)
            .ok_or_else(|| DeviceError::new(format!("{} read outside native page", self.name)))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || !offset.is_multiple_of(4) {
            return Err(DeviceError::new(
                "ESP32-C6 BLE control requires aligned word access",
            ));
        }
        let mut state = self
            .state
            .lock()
            .expect("ESP32-C6 BLE control lock poisoned");
        let index = offset as usize / 4;
        let Some(register) = state.registers.get_mut(index) else {
            return Err(DeviceError::new(format!(
                "{} write outside native page",
                self.name
            )));
        };
        *register = value as u32;
        if offset == C6_BLE_ECB_START && value & 1 != 0 {
            let mut key = [0_u8; 16];
            for word in 0..4 {
                key[word * 4..word * 4 + 4].copy_from_slice(
                    &state.registers[C6_BLE_ECB_KEY_BASE as usize / 4 + word].to_le_bytes(),
                );
            }
            let command = EspC6BleEcbCommand {
                input_address: state.registers[C6_BLE_ECB_INPUT_ADDRESS as usize / 4],
                output_address: state.registers[C6_BLE_ECB_OUTPUT_ADDRESS as usize / 4],
                length: state.registers[C6_BLE_ECB_LENGTH as usize / 4],
                key,
            };
            state.registers[C6_BLE_ECB_STATUS as usize / 4] = 0;
            state.pending_ecb.push_back(command);
        }
        if offset == C6_BLE_CCM_START && value & 1 != 0 {
            let config = state.registers[C6_BLE_CCM_CONFIG as usize / 4];
            let mut key = [0_u8; 16];
            for word in 0..4 {
                key[word * 4..word * 4 + 4].copy_from_slice(
                    &state.registers[C6_BLE_CCM_KEY_BASE as usize / 4 + word].to_le_bytes(),
                );
            }
            let counter_iv0 = state.registers[C6_BLE_CCM_COUNTER_IV0 as usize / 4];
            let iv1 = state.registers[C6_BLE_CCM_IV1 as usize / 4].to_le_bytes();
            let mut iv = [0_u8; 8];
            iv[0] = (counter_iv0 >> 8) as u8;
            iv[1] = (counter_iv0 >> 16) as u8;
            iv[2] = (counter_iv0 >> 24) as u8;
            iv[3..7].copy_from_slice(&iv1);
            iv[7] = state.registers[C6_BLE_CCM_IV2 as usize / 4] as u8;
            let command = EspC6BleCcmCommand {
                input_address: state.registers[C6_BLE_CCM_INPUT_ADDRESS as usize / 4],
                output_address: state.registers[C6_BLE_CCM_OUTPUT_ADDRESS as usize / 4],
                payload_length: (config >> 12) as u8,
                decrypt: config & 1 != 0,
                key,
                packet_counter: u64::from(state.registers[C6_BLE_CCM_COUNTER_LOW as usize / 4])
                    | (u64::from(counter_iv0 & 0x7f) << 32),
                // Native bit 7 encodes the local peripheral role. Its link
                // direction follows that role for TX and is inverted for RX.
                peripheral_to_central: (counter_iv0 & (1 << 7) != 0) != (config & 1 != 0),
                iv,
                aad_header: state.registers[C6_BLE_CCM_AAD as usize / 4] as u8,
                config,
            };
            state.registers[C6_BLE_CCM_RESULT as usize / 4] = 0;
            state.registers[C6_BLE_CCM_STATUS as usize / 4] = 0;
            state.pending_ccm.push_back(command);
        }
        if offset == C6_BLE_CCM_RESET && value & 1 != 0 {
            state.pending_ccm.clear();
            state.registers[C6_BLE_CCM_RESULT as usize / 4] = 0;
            state.registers[C6_BLE_CCM_STATUS as usize / 4] = 0;
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self
            .state
            .lock()
            .expect("ESP32-C6 BLE control lock poisoned");
        state.registers.fill(0);
        state.pending_ecb.clear();
        state.pending_ccm.clear();
    }
}

/// ESP32-C6 BLE baseband register page and native 1 MHz scheduler timebase.
///
/// The Apache-licensed controller reads `0x600a1924` directly for all link-layer
/// scheduling decisions. It releases the baseband through the reset strobe at
/// `0x600a1ff0`; the hardware-owned counter then advances in microseconds.
/// Ordinary configuration words retain the values programmed by firmware.
struct EspC6BleBasebandState {
    registers: Vec<u32>,
    timer_epoch: Option<SimTime>,
    pending_schedules: VecDeque<u32>,
    pending_completions: VecDeque<(u64, u32, u32, u32)>,
    completed_schedules: VecDeque<u32>,
    acknowledged_schedules: VecDeque<u32>,
    retire_current_reads: u8,
    stop_requested: bool,
}

/// One native C6 link-layer schedule submitted through the baseband command strobe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EspC6BleSchedule {
    /// CPU-visible address of the firmware-owned schedule entry.
    pub address: u32,
}

/// Scheduler and interrupt view of the C6 BLE baseband.
#[derive(Clone)]
pub struct EspC6BleBasebandHandle {
    state: Arc<Mutex<EspC6BleBasebandState>>,
}

impl EspC6BleBasebandHandle {
    /// Returns the native reset-relative 1 MHz scheduler timestamp.
    pub fn scheduler_timestamp(&self, now: SimTime) -> u32 {
        let state = self
            .state
            .lock()
            .expect("ESP32-C6 BLE baseband lock poisoned");
        let Some(epoch) = state.timer_epoch else {
            return 0;
        };
        let elapsed = now.ticks().saturating_sub(epoch.ticks());
        (elapsed / C6_BLE_BASEBAND_TICKS_PER_SCHEDULER_TICK) as u32
    }

    /// Converts an interval in native scheduler ticks to simulation ticks.
    pub fn scheduler_interval_ticks(&self, ticks: u32) -> u64 {
        u64::from(ticks).saturating_mul(C6_BLE_BASEBAND_TICKS_PER_SCHEDULER_TICK)
    }

    /// Converts a future native 1 MHz scheduler timestamp to simulation ticks.
    pub fn scheduler_delay_ticks(&self, now: SimTime, target: u32) -> u64 {
        let state = self
            .state
            .lock()
            .expect("ESP32-C6 BLE baseband lock poisoned");
        let Some(epoch) = state.timer_epoch else {
            return 0;
        };
        let elapsed = now.ticks().saturating_sub(epoch.ticks());
        let current = (elapsed / C6_BLE_BASEBAND_TICKS_PER_SCHEDULER_TICK) as u32;
        let delta = target.wrapping_sub(current);
        if delta >= 0x8000_0000 {
            0
        } else {
            self.scheduler_interval_ticks(delta)
        }
    }

    /// Removes the oldest native link-layer schedule submitted by firmware.
    pub fn take_schedule(&self) -> Option<EspC6BleSchedule> {
        self.state
            .lock()
            .expect("ESP32-C6 BLE baseband lock poisoned")
            .pending_schedules
            .pop_front()
            .map(|address| EspC6BleSchedule { address })
    }

    /// Removes the oldest schedule whose hardware DMA lifecycle completed.
    pub fn take_completed_schedule(&self) -> Option<EspC6BleSchedule> {
        self.state
            .lock()
            .expect("ESP32-C6 BLE baseband lock poisoned")
            .completed_schedules
            .pop_front()
            .map(|address| EspC6BleSchedule { address })
    }

    /// Removes a schedule whose event-end cause firmware acknowledged.
    pub fn take_acknowledged_schedule(&self) -> Option<EspC6BleSchedule> {
        self.state
            .lock()
            .expect("ESP32-C6 BLE baseband lock poisoned")
            .acknowledged_schedules
            .pop_front()
            .map(|address| EspC6BleSchedule { address })
    }

    /// Reports and consumes a native scheduler stop barrier.
    pub fn take_stop_request(&self) -> bool {
        let mut state = self
            .state
            .lock()
            .expect("ESP32-C6 BLE baseband lock poisoned");
        std::mem::take(&mut state.stop_requested)
    }

    /// Schedules the normal event-end cause for one executed entry.
    pub fn schedule_event_end(&self, due: SimTime, schedule_address: u32, successor: Option<u32>) {
        self.schedule_completion(due, schedule_address, successor, C6_BLE_BASEBAND_EVENT_END);
    }

    /// Schedules event-end with the native successful-radio-operation cause.
    pub fn schedule_successful_event_end(
        &self,
        due: SimTime,
        schedule_address: u32,
        successor: Option<u32>,
    ) {
        self.schedule_completion(
            due,
            schedule_address,
            successor,
            C6_BLE_BASEBAND_EVENT_END | C6_BLE_BASEBAND_EVENT_SUCCESS,
        );
    }

    /// Schedules event-end with the native completed-reception cause.
    pub fn schedule_received_event_end(
        &self,
        due: SimTime,
        schedule_address: u32,
        successor: Option<u32>,
    ) {
        // RX is the terminal outcome of the same loaded schedule that already
        // carries a no-packet timeout. Hardware resolves those outcomes
        // atomically; exposing both END causes makes firmware recycle the
        // descriptor twice and can free a newly-created connection state.
        self.state
            .lock()
            .expect("ESP32-C6 BLE baseband lock poisoned")
            .pending_completions
            .retain(|(_, address, _, _)| *address != schedule_address);
        self.schedule_completion(
            due,
            schedule_address,
            successor,
            C6_BLE_BASEBAND_EVENT_END | C6_BLE_BASEBAND_EVENT_RX | C6_BLE_BASEBAND_EVENT_SUCCESS,
        );
    }

    fn schedule_completion(
        &self,
        due: SimTime,
        schedule_address: u32,
        successor: Option<u32>,
        causes: u32,
    ) {
        let mut state = self
            .state
            .lock()
            .expect("ESP32-C6 BLE baseband lock poisoned");
        let insertion = state
            .pending_completions
            .iter()
            .position(|(existing, _, _, _)| *existing > due.ticks())
            .unwrap_or(state.pending_completions.len());
        state.pending_completions.insert(
            insertion,
            (
                due.ticks(),
                schedule_address,
                successor.unwrap_or(0),
                causes,
            ),
        );
    }

    /// Publishes the descriptor immediately following a newly loaded head.
    pub fn set_loaded_schedule_successor(&self, schedule_address: u32, successor: Option<u32>) {
        let mut state = self
            .state
            .lock()
            .expect("ESP32-C6 BLE baseband lock poisoned");
        let current = state.registers[C6_BLE_BASEBAND_SCHEDULER_CURRENT as usize / 4];
        if current & 0x000f_ffff == schedule_address & 0x000f_ffff {
            state.registers[C6_BLE_BASEBAND_SCHEDULER_NEXT as usize / 4] =
                successor.unwrap_or(0) & 0x000f_ffff;
        }
    }

    /// Publishes the hardware-owned TX/RX buffer headers for a loaded schedule.
    ///
    /// Native controller code reads these cursors while an event is executing.
    /// The register representation points four bytes beyond the allocation
    /// header and retains only the internal-RAM offset.
    pub fn set_loaded_buffer_headers(
        &self,
        schedule_address: u32,
        tx_header: Option<u32>,
        rx_header: Option<u32>,
    ) {
        let mut state = self
            .state
            .lock()
            .expect("ESP32-C6 BLE baseband lock poisoned");
        let current = state.registers[C6_BLE_BASEBAND_SCHEDULER_CURRENT as usize / 4];
        if current & 0x000f_ffff != schedule_address & 0x000f_ffff {
            return;
        }
        let encode = |header: Option<u32>| {
            header
                .map(|address| address.wrapping_add(4) & 0x000f_ffff)
                .unwrap_or(0)
        };
        state.registers[C6_BLE_BASEBAND_CURRENT_TX_BUFFER as usize / 4] = encode(tx_header);
        state.registers[C6_BLE_BASEBAND_CURRENT_RX_BUFFER as usize / 4] = encode(rx_header);
    }

    /// Advances native completion state to the requested simulation timestamp.
    pub fn advance_to(&self, now: SimTime) {
        let mut state = self
            .state
            .lock()
            .expect("ESP32-C6 BLE baseband lock poisoned");
        while state
            .pending_completions
            .front()
            .is_some_and(|(due, _, _, _)| *due <= now.ticks())
        {
            let Some((_, address, successor, causes)) = state.pending_completions.pop_front()
            else {
                break;
            };
            // Bits 31 and 29 are the valid and executed flags consumed by
            // r_ble_lll_sched_update_cur_entry; the low 20 bits are the entry.
            state.registers[C6_BLE_BASEBAND_SCHEDULER_CURRENT as usize / 4] =
                0xa000_0000 | (address & 0x000f_ffff);
            state.registers[C6_BLE_BASEBAND_SCHEDULER_NEXT as usize / 4] = successor & 0x000f_ffff;
            state.registers[C6_BLE_BASEBAND_INTERRUPT_RAW0 as usize / 4] |= causes;
            state.completed_schedules.push_back(address);
        }
    }

    /// Whether an enabled baseband cause awaits firmware acknowledgement.
    pub fn interrupt_pending(&self) -> bool {
        let state = self
            .state
            .lock()
            .expect("ESP32-C6 BLE baseband lock poisoned");
        state.registers[C6_BLE_BASEBAND_INTERRUPT_RAW0 as usize / 4]
            & state.registers[C6_BLE_BASEBAND_INTERRUPT_ENABLE0 as usize / 4]
            != 0
            || state.registers[C6_BLE_BASEBAND_INTERRUPT_RAW1 as usize / 4]
                & state.registers[C6_BLE_BASEBAND_INTERRUPT_ENABLE1 as usize / 4]
                != 0
    }
}

/// ESP32-C6 BLE baseband register device used by the native controller.
pub struct EspC6BleBaseband {
    name: String,
    state: Arc<Mutex<EspC6BleBasebandState>>,
}

impl EspC6BleBaseband {
    /// Creates a reset C6 BLE baseband page and its scheduler handle.
    pub fn new(name: impl Into<String>) -> (Self, EspC6BleBasebandHandle) {
        let state = Arc::new(Mutex::new(EspC6BleBasebandState {
            registers: vec![0; 0x1000 / 4],
            timer_epoch: None,
            pending_schedules: VecDeque::new(),
            pending_completions: VecDeque::new(),
            completed_schedules: VecDeque::new(),
            acknowledged_schedules: VecDeque::new(),
            retire_current_reads: 0,
            stop_requested: false,
        }));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            EspC6BleBasebandHandle { state },
        )
    }

    fn scheduler_tick(state: &EspC6BleBasebandState, at: SimTime) -> u32 {
        let Some(epoch) = state.timer_epoch else {
            return 0;
        };
        let elapsed = at.ticks().saturating_sub(epoch.ticks());
        (elapsed / C6_BLE_BASEBAND_TICKS_PER_SCHEDULER_TICK) as u32
    }
}

impl Device for EspC6BleBaseband {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        let index = checked_word_index(&self.name, offset, width)?;
        let mut state = self
            .state
            .lock()
            .expect("ESP32-C6 BLE baseband lock poisoned");
        if offset == C6_BLE_BASEBAND_TIMER_CURRENT {
            return Ok(u64::from(Self::scheduler_tick(&state, at)));
        }
        let value =
            state.registers.get(index).copied().ok_or_else(|| {
                DeviceError::new(format!("{} read outside baseband page", self.name))
            })?;
        if offset == C6_BLE_BASEBAND_SCHEDULER_CURRENT && state.retire_current_reads != 0 {
            state.retire_current_reads -= 1;
            if state.retire_current_reads == 0 {
                let successor =
                    state.registers[C6_BLE_BASEBAND_SCHEDULER_NEXT as usize / 4] & 0x000f_ffff;
                state.registers[index] = if successor == 0 {
                    0
                } else {
                    0xa000_0000 | successor
                };
            }
        }
        Ok(u64::from(value))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        let index = checked_word_index(&self.name, offset, width)?;
        let mut state = self
            .state
            .lock()
            .expect("ESP32-C6 BLE baseband lock poisoned");
        let register = state.registers.get_mut(index).ok_or_else(|| {
            DeviceError::new(format!("{} write outside baseband page", self.name))
        })?;
        let previous = *register;
        if offset == C6_BLE_BASEBAND_INTERRUPT_CLEAR0 {
            if value as u32 & C6_BLE_BASEBAND_EVENT_END != 0
                && state.registers[C6_BLE_BASEBAND_INTERRUPT_RAW0 as usize / 4]
                    & C6_BLE_BASEBAND_EVENT_END
                    != 0
            {
                // Event acknowledgement leaves the executed entry visible to
                // the native ISR's validation read. The following scheduler
                // snapshot observes that hardware has advanced beyond the
                // completed tail (zero when the descriptor has no successor).
                state.retire_current_reads = 1;
                let current =
                    state.registers[C6_BLE_BASEBAND_SCHEDULER_CURRENT as usize / 4] & 0x000f_ffff;
                if current != 0 {
                    state
                        .acknowledged_schedules
                        .push_back(0x4080_0000 | current);
                }
            }
            state.registers[C6_BLE_BASEBAND_INTERRUPT_RAW0 as usize / 4] &= !(value as u32);
            return Ok(());
        }
        if offset == C6_BLE_BASEBAND_INTERRUPT_CLEAR1 {
            state.registers[C6_BLE_BASEBAND_INTERRUPT_RAW1 as usize / 4] &= !(value as u32);
            return Ok(());
        }
        *register = value as u32;
        if offset == C6_BLE_BASEBAND_RESET && previous & 1 == 0 && value & 1 != 0 {
            state.timer_epoch = Some(at);
        }
        if offset == C6_BLE_BASEBAND_SCHEDULER_KICK && value & 1 != 0 {
            let address =
                state.registers[C6_BLE_BASEBAND_SCHEDULER_HEAD as usize / 4] & 0x000f_ffff;
            if address != 0 {
                // CURRENT is valid and loaded as soon as KICK transfers the
                // scheduler head to hardware. Native scheduler snapshots only
                // accept entries carrying both status bits.
                state.registers[C6_BLE_BASEBAND_SCHEDULER_CURRENT as usize / 4] =
                    0xa000_0000 | address;
                state.registers[C6_BLE_BASEBAND_SCHEDULER_NEXT as usize / 4] = 0;
                state.registers[C6_BLE_BASEBAND_CURRENT_TX_BUFFER as usize / 4] = 0;
                state.registers[C6_BLE_BASEBAND_CURRENT_RX_BUFFER as usize / 4] = 0;
                state.retire_current_reads = 0;
                state.pending_schedules.push_back(0x4080_0000 | address);
            }
        }
        if offset == C6_BLE_BASEBAND_SCHEDULER_STOP && value & 1 != 0 {
            // Native PHY disable strobes STOP and polls CURRENT until hardware
            // releases its valid bit. Future loads and completions are
            // canceled at this barrier; otherwise a delayed completion can
            // mutate a descriptor after controller firmware has freed it.
            state.registers[C6_BLE_BASEBAND_SCHEDULER_CURRENT as usize / 4] &= 0x7fff_ffff;
            state.registers[C6_BLE_BASEBAND_SCHEDULER_NEXT as usize / 4] = 0;
            state.registers[C6_BLE_BASEBAND_CURRENT_TX_BUFFER as usize / 4] = 0;
            state.registers[C6_BLE_BASEBAND_CURRENT_RX_BUFFER as usize / 4] = 0;
            state.retire_current_reads = 0;
            state.pending_schedules.clear();
            state.pending_completions.clear();
            state.stop_requested = true;
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self
            .state
            .lock()
            .expect("ESP32-C6 BLE baseband lock poisoned");
        state.registers.fill(0);
        state.timer_epoch = None;
        state.pending_schedules.clear();
        state.pending_completions.clear();
        state.completed_schedules.clear();
        state.acknowledged_schedules.clear();
        state.retire_current_reads = 0;
        state.stop_requested = false;
    }
}

/// ESP32-C6 PHY-private register page used during calibration.
///
/// The first word is the free-running modem timebase. Remaining words retain
/// software-visible state while analog effects are handled by the functional
/// radio model.
pub struct EspC6PhyRegisters {
    name: String,
    registers: [u32; 1024],
}

impl EspC6PhyRegisters {
    /// Creates a reset PHY register page.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            registers: [0; 1024],
        }
    }
}

impl Device for EspC6PhyRegisters {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || !offset.is_multiple_of(4) {
            return Err(DeviceError::new(
                "ESP32-C6 PHY registers require aligned word access",
            ));
        }
        if offset == 0 {
            return Ok(at.ticks() & u64::from(u32::MAX));
        }
        self.registers
            .get(offset as usize / 4)
            .copied()
            .map(u64::from)
            .ok_or_else(|| DeviceError::new(format!("{} read outside native page", self.name)))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || !offset.is_multiple_of(4) {
            return Err(DeviceError::new(
                "ESP32-C6 PHY registers require aligned word access",
            ));
        }
        let register = self
            .registers
            .get_mut(offset as usize / 4)
            .ok_or_else(|| DeviceError::new(format!("{} write outside native page", self.name)))?;
        *register = value as u32;
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.registers.fill(0);
    }
}

const C6_POWER_DETECTOR_CONVERSION: u64 = 0x418;
const C6_POWER_DETECTOR_START: u32 = 1 << 0;
const C6_POWER_DETECTOR_DONE: u32 = 1 << 22;
const C6_POWER_DETECTOR_TONE_CONTROL: u64 = 0x810;
const C6_POWER_DETECTOR_TONE_STATUS: u64 = 0x814;
const C6_POWER_DETECTOR_TONE_IDLE: u32 = 7 << 14;
const C6_FREQUENCY_CONTROL: u64 = 0x0c0;
const C6_FREQUENCY_STATUS: u64 = 0x0cc;
const C6_FREQUENCY_CHANNEL_START: u32 = 1 << 14;
const C6_FREQUENCY_CHANNEL_DONE: u32 = 1 << 8;
const C6_IQ_ESTIMATE_CONTROL: u64 = 0x474;
const C6_IQ_ESTIMATE_STATUS: u64 = 0x4a0;
const C6_IQ_ESTIMATE_START: u32 = 1 << 1;
const C6_IQ_ESTIMATE_DONE: u32 = 1 << 16;

/// ESP32-C6 RF power-detector and calibration register page.
///
/// The RF calibration code starts a detector conversion with bit zero of the
/// conversion register and polls bit 22 for completion. Conversions complete
/// synchronously in simulation; the comparator outputs remain deterministic
/// zero until an analog RF environment is attached.
pub struct EspC6PowerDetector {
    name: String,
    registers: [u32; 1024],
}

impl EspC6PowerDetector {
    /// Creates a reset RF power-detector register page.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            registers: [0; 1024],
        }
    }
}

impl Device for EspC6PowerDetector {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let index = checked_word_index(&self.name, offset, width)?;
        self.registers
            .get(index)
            .copied()
            .map(u64::from)
            .ok_or_else(|| DeviceError::new(format!("{} read outside native page", self.name)))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        let index = checked_word_index(&self.name, offset, width)?;
        let value = u32::try_from(value)
            .map_err(|_| DeviceError::new("ESP32-C6 power detector rejects wide writes"))?;
        let register = self
            .registers
            .get_mut(index)
            .ok_or_else(|| DeviceError::new(format!("{} write outside native page", self.name)))?;
        *register = value & !C6_POWER_DETECTOR_DONE;
        if offset == C6_POWER_DETECTOR_CONVERSION && value & C6_POWER_DETECTOR_START != 0 {
            *register |= C6_POWER_DETECTOR_DONE;
        }
        if offset == C6_POWER_DETECTOR_TONE_CONTROL && value & C6_POWER_DETECTOR_START != 0 {
            let status = &mut self.registers[C6_POWER_DETECTOR_TONE_STATUS as usize / 4];
            *status = (*status & !(7 << 14)) | C6_POWER_DETECTOR_TONE_IDLE;
        }
        if offset == C6_FREQUENCY_CONTROL && value & C6_FREQUENCY_CHANNEL_START != 0 {
            self.registers[C6_FREQUENCY_STATUS as usize / 4] |= C6_FREQUENCY_CHANNEL_DONE;
        }
        if offset == C6_IQ_ESTIMATE_CONTROL && value & C6_IQ_ESTIMATE_START != 0 {
            self.registers[C6_IQ_ESTIMATE_STATUS as usize / 4] |= C6_IQ_ESTIMATE_DONE;
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.registers.fill(0);
    }
}

const C6_WIFI_MAC_RESET_CONTROL: u64 = 0xddc;
const C6_WIFI_MAC_RESET_START: u32 = 1 << 1;
const C6_WIFI_MAC_RESET_READY: u32 = 1 << 0;
const C6_WIFI_MAC_INTERRUPT_MASK: u64 = 0xc40;
const C6_WIFI_MAC_INTERRUPT_EVENT: u64 = 0xc48;
const C6_WIFI_MAC_INTERRUPT_CLEAR: u64 = 0xc4c;
const C6_WIFI_MAC_EVENT_TX_DONE: u32 = 1 << 7;
const C6_WIFI_MAC_EVENT_RX_DONE: u32 = 1 << 14;
const C6_WIFI_MAC_RX_BASE: u64 = 0x084;
const C6_WIFI_MAC_RX_NEXT: u64 = 0x088;
const C6_WIFI_MAC_RX_LAST: u64 = 0x08c;
const C6_WIFI_MAC_RX_ADDRESS_HIGH: u64 = 0xc70;
const C6_WIFI_MAC_INTERFACE_ADDRESS_LOW: u64 = 0x05c;
const C6_WIFI_MAC_INTERFACE_ADDRESS_HIGH: u64 = 0x060;
const C6_WIFI_MAC_INTERFACE_ADDRESS_STRIDE: u64 = 8;
const C6_WIFI_MAC_INTERFACE_ADDRESS_COUNT: usize = 4;
const C6_WIFI_MAC_INTERFACE_ADDRESS_VALID: u32 = 1 << 16;
const C6_WIFI_MAC_TX_QUEUE_STATE_CLEAR: u64 = 0xcb4;
const C6_WIFI_MAC_TX_QUEUE_STATE: u64 = 0xcb8;
const C6_WIFI_MAC_TX_QUEUE_CONTROL_HIGH: u64 = 0xd6c;
const C6_WIFI_MAC_TX_QUEUE_CONTROL_LOW: u64 = 0xd1c;
const C6_WIFI_MAC_TX_QUEUE_ENABLE: u32 = 3 << 30;
const C6_WIFI_MAC_TX_QUEUE_TIMEOUT_HIGH: u64 = 0xd68;
const C6_WIFI_MAC_TX_QUEUE_TIMEOUT_STRIDE: u64 = 0x10;
const C6_WIFI_MAC_TX_QUEUE_COMPLETION_HIGH: u64 = 0x14ec;
const C6_WIFI_MAC_TX_QUEUE_COMPLETION_STRIDE: u64 = 0x74;
const C6_WIFI_MAC_TX_QUEUE_COMPLETION_STATUS: u32 = 0xf << 12;
const C6_WIFI_MAC_RX_BA_CONTROL_HIGH: u64 = 0x290;
const C6_WIFI_MAC_RX_BA_MAC_HIGH_HIGH: u64 = 0x294;
const C6_WIFI_MAC_RX_BA_MAC_LOW_HIGH: u64 = 0x298;
const C6_WIFI_MAC_RX_BA_SEQUENCE_HIGH: u64 = 0x2a0;
const C6_WIFI_MAC_RX_BA_BITMAP_LOW_HIGH: u64 = 0x2a8;
const C6_WIFI_MAC_RX_BA_BITMAP_HIGH_HIGH: u64 = 0x2b0;
const C6_WIFI_MAC_RX_BA_STRIDE: u64 = 0x28;
const C6_WIFI_MAC_RX_BA_COUNT: usize = 8;
const C6_WIFI_MAC_RX_BA_VALID: u32 = 1 << 31;
const C6_WIFI_MAC_RX_BA_ACTIVE: u32 = 3 << 30;
const C6_WIFI_MAC_RX_BA_MODE: u32 = 5;
const C6_WIFI_MAC_CRYPTO_VALID: u64 = 0x814;
const C6_WIFI_MAC_CRYPTO_TABLE: u64 = 0x1800;
const C6_WIFI_MAC_CRYPTO_ENTRY_STRIDE: u64 = 0x28;
const C6_WIFI_MAC_CRYPTO_ENTRY_WORDS: usize = 10;
const C6_WIFI_MAC_CRYPTO_ENTRY_COUNT: usize = 32;

struct EspC6WifiMacState {
    registers: Vec<u32>,
    pending_tx: VecDeque<EspC6WifiTxDescriptor>,
    active_tx: u32,
    rx_descriptor: Option<u32>,
}

impl EspC6WifiMacState {
    fn reset(&mut self) {
        self.registers.fill(0);
        self.pending_tx.clear();
        self.active_tx = 0;
        self.rx_descriptor = None;
    }
}

/// One native ESP32-C6 Wi-Fi transmit descriptor submitted by guest firmware.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EspC6WifiTxDescriptor {
    /// Native MAC queue index.
    pub queue: u8,
    /// Reconstructed DRAM address of the first DMA descriptor.
    pub address: u32,
}

/// One native ESP32-C6 Wi-Fi receive descriptor owned by the MAC DMA engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EspC6WifiRxDescriptor {
    /// Full DRAM address programmed through the native receive-base register.
    pub address: u32,
}

/// Scheduler-facing view of ESP32-C6 Wi-Fi MAC interrupt state.
#[derive(Clone)]
pub struct EspC6WifiMacHandle {
    state: Arc<Mutex<EspC6WifiMacState>>,
}

impl EspC6WifiMacHandle {
    fn crypto_key_entry_from_state(
        state: &EspC6WifiMacState,
        slot: usize,
    ) -> Option<crate::EspWifiCryptoKeyEntry> {
        if slot >= C6_WIFI_MAC_CRYPTO_ENTRY_COUNT
            || state.registers[C6_WIFI_MAC_CRYPTO_VALID as usize / 4] & (1 << slot) == 0
        {
            return None;
        }
        let base =
            (C6_WIFI_MAC_CRYPTO_TABLE + slot as u64 * C6_WIFI_MAC_CRYPTO_ENTRY_STRIDE) as usize / 4;
        let words = state
            .registers
            .get(base..base + C6_WIFI_MAC_CRYPTO_ENTRY_WORDS)?;
        let mut key = [0_u8; 32];
        for (destination, word) in key.chunks_exact_mut(4).zip(&words[2..]) {
            destination.copy_from_slice(&word.to_le_bytes());
        }
        Some(crate::EspWifiCryptoKeyEntry {
            slot: slot as u8,
            match_low: words[0],
            control: words[1],
            key,
        })
    }

    fn rx_block_ack_slot(state: &EspC6WifiMacState, peer: &[u8; 6], tid: u8) -> Option<usize> {
        (0..C6_WIFI_MAC_RX_BA_COUNT).find(|slot| {
            let distance = *slot as u64 * C6_WIFI_MAC_RX_BA_STRIDE;
            let control = state.registers[(C6_WIFI_MAC_RX_BA_CONTROL_HIGH - distance) as usize / 4];
            let mac_low = state.registers[(C6_WIFI_MAC_RX_BA_MAC_LOW_HIGH - distance) as usize / 4];
            let mac_high =
                state.registers[(C6_WIFI_MAC_RX_BA_MAC_HIGH_HIGH - distance) as usize / 4];
            control & C6_WIFI_MAC_RX_BA_ACTIVE == C6_WIFI_MAC_RX_BA_ACTIVE
                && control & 0x0fff == C6_WIFI_MAC_RX_BA_MODE
                && (control >> 12) as u8 & 0xf == tid & 0xf
                && mac_low == u32::from_le_bytes(peer[..4].try_into().unwrap())
                && mac_high as u16 == u16::from_le_bytes(peer[4..].try_into().unwrap())
        })
    }

    /// Rejects active RX BA encodings that the pinned firmware never creates.
    pub fn validate_block_ack_sessions(&self) -> Result<(), String> {
        let state = self.state.lock().expect("ESP32-C6 Wi-Fi MAC lock poisoned");
        for slot in 0..C6_WIFI_MAC_RX_BA_COUNT {
            let distance = slot as u64 * C6_WIFI_MAC_RX_BA_STRIDE;
            let control = state.registers[(C6_WIFI_MAC_RX_BA_CONTROL_HIGH - distance) as usize / 4];
            if control & C6_WIFI_MAC_RX_BA_VALID != 0
                && (control & C6_WIFI_MAC_RX_BA_ACTIVE != C6_WIFI_MAC_RX_BA_ACTIVE
                    || control & 0x0fff != C6_WIFI_MAC_RX_BA_MODE)
            {
                return Err(format!(
                    "RX block-ACK slot {slot} has impossible active control {control:#010x}"
                ));
            }
        }
        Ok(())
    }

    /// Returns one valid firmware-programmed native crypto-table entry.
    pub fn crypto_key_entry(&self, slot: u8) -> Option<crate::EspWifiCryptoKeyEntry> {
        let state = self.state.lock().expect("ESP32-C6 Wi-Fi MAC lock poisoned");
        Self::crypto_key_entry_from_state(&state, usize::from(slot))
    }

    /// Rejects valid crypto slots with a control class the pinned HAL cannot emit.
    pub fn validate_crypto_key_table(&self) -> Result<(), String> {
        let state = self.state.lock().expect("ESP32-C6 Wi-Fi MAC lock poisoned");
        let valid = state.registers[C6_WIFI_MAC_CRYPTO_VALID as usize / 4];
        for slot in 0..C6_WIFI_MAC_CRYPTO_ENTRY_COUNT {
            if valid & (1 << slot) == 0 {
                continue;
            }
            let entry = Self::crypto_key_entry_from_state(&state, slot)
                .expect("C6 crypto table fits its native register window");
            let control_class = (entry.control >> 21) & 7;
            if !matches!(control_class, 3 | 6 | 7) {
                return Err(format!(
                    "crypto key slot {slot} has impossible control class {control_class} in {:#010x}",
                    entry.control
                ));
            }
        }
        Ok(())
    }

    /// Whether an enabled native MAC event asserts interrupt source zero.
    pub fn interrupt_pending(&self) -> bool {
        let state = self.state.lock().expect("ESP32-C6 Wi-Fi MAC lock poisoned");
        let mask = state.registers[C6_WIFI_MAC_INTERRUPT_MASK as usize / 4];
        let events = state.registers[C6_WIFI_MAC_INTERRUPT_EVENT as usize / 4];
        mask & events != 0
    }

    /// Removes the oldest native DMA transmit submitted by firmware.
    pub fn take_tx_descriptor(&self) -> Option<EspC6WifiTxDescriptor> {
        self.state
            .lock()
            .expect("ESP32-C6 Wi-Fi MAC lock poisoned")
            .pending_tx
            .pop_front()
    }

    /// Returns the firmware-programmed twelve-bit ACK timeout for a queue.
    pub fn tx_ack_timeout(&self, queue: u8) -> u16 {
        let state = self.state.lock().expect("ESP32-C6 Wi-Fi MAC lock poisoned");
        let offset = C6_WIFI_MAC_TX_QUEUE_TIMEOUT_HIGH
            .saturating_sub(u64::from(queue) * C6_WIFI_MAC_TX_QUEUE_TIMEOUT_STRIDE);
        state
            .registers
            .get(offset as usize / 4)
            .copied()
            .unwrap_or_default() as u16
            & 0x0fff
    }

    /// Whether a queue has been kicked but has not yet received a completion.
    pub fn tx_active(&self, queue: u8) -> bool {
        let state = self.state.lock().expect("ESP32-C6 Wi-Fi MAC lock poisoned");
        state.active_tx & (1 << queue) != 0
    }

    /// Publishes one native completion record and raises the TX interrupt.
    pub fn complete_tx(&self, queue: u8, outcome: crate::EspWifiTxOutcome) -> bool {
        let mut state = self.state.lock().expect("ESP32-C6 Wi-Fi MAC lock poisoned");
        let bit = 1_u32 << queue;
        if state.active_tx & bit == 0 {
            return false;
        }
        let offset = C6_WIFI_MAC_TX_QUEUE_COMPLETION_HIGH
            .saturating_sub(u64::from(queue) * C6_WIFI_MAC_TX_QUEUE_COMPLETION_STRIDE);
        let Some(completion) = state.registers.get_mut(offset as usize / 4) else {
            return false;
        };
        *completion =
            (*completion & !C6_WIFI_MAC_TX_QUEUE_COMPLETION_STATUS) | (outcome.status() << 12);
        state.active_tx &= !bit;
        state.registers[C6_WIFI_MAC_TX_QUEUE_STATE as usize / 4] |= bit;
        state.registers[C6_WIFI_MAC_INTERRUPT_EVENT as usize / 4] |= C6_WIFI_MAC_EVENT_TX_DONE;
        true
    }

    /// Returns the current firmware-provided receive descriptor, if armed.
    pub fn rx_descriptor(&self) -> Option<EspC6WifiRxDescriptor> {
        self.state
            .lock()
            .expect("ESP32-C6 Wi-Fi MAC lock poisoned")
            .rx_descriptor
            .map(|address| EspC6WifiRxDescriptor { address })
    }

    /// Returns the native RX-interface match bitmap for an 802.11 receiver address.
    ///
    /// Vendor firmware programs one address-filter slot per virtual interface.
    /// Exact receiver addresses select their configured slot. Other group frames
    /// match every valid slot. A reset MAC has no valid slots, in which case
    /// freestanding firmware receives through the hardware-default slot zero.
    pub fn rx_match_mask(&self, receiver: &[u8]) -> u8 {
        let Some(receiver) = receiver.get(..6) else {
            return 0;
        };
        let state = self.state.lock().expect("ESP32-C6 Wi-Fi MAC lock poisoned");
        let mut configured = 0_u8;
        let mut matches = 0_u8;
        for interface in 0..C6_WIFI_MAC_INTERFACE_ADDRESS_COUNT {
            let offset = interface as u64 * C6_WIFI_MAC_INTERFACE_ADDRESS_STRIDE;
            let low = state.registers[(C6_WIFI_MAC_INTERFACE_ADDRESS_LOW + offset) as usize / 4];
            let high = state.registers[(C6_WIFI_MAC_INTERFACE_ADDRESS_HIGH + offset) as usize / 4];
            if high & C6_WIFI_MAC_INTERFACE_ADDRESS_VALID == 0 {
                continue;
            }
            let bit = 1_u8 << interface;
            configured |= bit;
            let address = [
                low as u8,
                (low >> 8) as u8,
                (low >> 16) as u8,
                (low >> 24) as u8,
                high as u8,
                (high >> 8) as u8,
            ];
            if receiver == address {
                matches |= bit;
            }
        }
        if configured == 0 {
            1
        } else if matches != 0 {
            matches
        } else if receiver[0] & 1 != 0 {
            configured
        } else {
            0
        }
    }

    /// Records a received QoS MPDU in the matching firmware-owned RX BA window.
    ///
    /// The register layout and descending slot stride are the native C6
    /// `hal_agreement_add_rx_ba` contract. Sequence arithmetic is modulo the
    /// twelve-bit 802.11 sequence space; frames older than the current window
    /// do not move it backwards.
    pub fn record_block_ack_mpdu(&self, peer: &[u8; 6], tid: u8, sequence: u16) -> bool {
        let mut state = self.state.lock().expect("ESP32-C6 Wi-Fi MAC lock poisoned");
        let Some(slot) = Self::rx_block_ack_slot(&state, peer, tid) else {
            return false;
        };
        let distance = slot as u64 * C6_WIFI_MAC_RX_BA_STRIDE;
        let sequence_index = (C6_WIFI_MAC_RX_BA_SEQUENCE_HIGH - distance) as usize / 4;
        let bitmap_low_index = (C6_WIFI_MAC_RX_BA_BITMAP_LOW_HIGH - distance) as usize / 4;
        let bitmap_high_index = (C6_WIFI_MAC_RX_BA_BITMAP_HIGH_HIGH - distance) as usize / 4;
        let mut origin = state.registers[sequence_index] as u16 & 0x0fff;
        let sequence = sequence & 0x0fff;
        let delta = sequence.wrapping_sub(origin) & 0x0fff;
        if delta >= 0x0800 {
            return true;
        }
        let mut bitmap = u64::from(state.registers[bitmap_low_index])
            | (u64::from(state.registers[bitmap_high_index]) << 32);
        if delta < 64 {
            bitmap |= 1_u64 << delta;
        } else {
            let shift = u32::from(delta - 63);
            bitmap = bitmap.checked_shr(shift).unwrap_or(0) | (1_u64 << 63);
            origin = origin.wrapping_add(shift as u16) & 0x0fff;
            state.registers[sequence_index] =
                (state.registers[sequence_index] & !0x0fff) | u32::from(origin);
        }
        state.registers[bitmap_low_index] = bitmap as u32;
        state.registers[bitmap_high_index] = (bitmap >> 32) as u32;
        true
    }

    /// Returns the matching compressed block-ACK bitmap at a requested origin.
    pub fn block_ack_bitmap(&self, peer: &[u8; 6], tid: u8, starting_sequence: u16) -> Option<u64> {
        let state = self.state.lock().expect("ESP32-C6 Wi-Fi MAC lock poisoned");
        let slot = Self::rx_block_ack_slot(&state, peer, tid)?;
        let distance = slot as u64 * C6_WIFI_MAC_RX_BA_STRIDE;
        let origin = state.registers[(C6_WIFI_MAC_RX_BA_SEQUENCE_HIGH - distance) as usize / 4]
            as u16
            & 0x0fff;
        let bitmap =
            u64::from(state.registers[(C6_WIFI_MAC_RX_BA_BITMAP_LOW_HIGH - distance) as usize / 4])
                | (u64::from(
                    state.registers[(C6_WIFI_MAC_RX_BA_BITMAP_HIGH_HIGH - distance) as usize / 4],
                ) << 32);
        let requested = starting_sequence & 0x0fff;
        let forward = requested.wrapping_sub(origin) & 0x0fff;
        if forward < 0x0800 {
            Some(bitmap.checked_shr(u32::from(forward)).unwrap_or(0))
        } else {
            let backward = origin.wrapping_sub(requested) & 0x0fff;
            Some(bitmap.checked_shl(u32::from(backward)).unwrap_or(0))
        }
    }

    /// Advances the native receive ring and raises the hardware RX event.
    pub fn complete_rx_descriptor(&self, address: u32, next: u32) {
        let mut state = self.state.lock().expect("ESP32-C6 Wi-Fi MAC lock poisoned");
        state.registers[C6_WIFI_MAC_RX_NEXT as usize / 4] = next;
        state.registers[C6_WIFI_MAC_RX_LAST as usize / 4] = address & 0x000f_ffff;
        state.registers[C6_WIFI_MAC_RX_ADDRESS_HIGH as usize / 4] = address & 0xfff0_0000;
        state.rx_descriptor = (next != 0).then_some(next);
        state.registers[C6_WIFI_MAC_INTERRUPT_EVENT as usize / 4] |= C6_WIFI_MAC_EVENT_RX_DONE;
    }
}

/// ESP32-C6 Wi-Fi MAC register page.
///
/// The MAC reset command acknowledges synchronously in simulation while all
/// other words retain firmware-visible read/modify/write state.
pub struct EspC6WifiMacRegisters {
    name: String,
    state: Arc<Mutex<EspC6WifiMacState>>,
}

impl EspC6WifiMacRegisters {
    /// Creates a reset Wi-Fi MAC register page.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            state: Arc::new(Mutex::new(EspC6WifiMacState {
                registers: vec![0; 0x3000 / 4],
                pending_tx: VecDeque::new(),
                active_tx: 0,
                rx_descriptor: None,
            })),
        }
    }

    /// Returns the interrupt handle coupled to this register frontend.
    pub fn handle(&self) -> EspC6WifiMacHandle {
        EspC6WifiMacHandle {
            state: self.state.clone(),
        }
    }

    fn tx_queue(offset: u64) -> Option<u8> {
        if !(C6_WIFI_MAC_TX_QUEUE_CONTROL_LOW..=C6_WIFI_MAC_TX_QUEUE_CONTROL_HIGH).contains(&offset)
        {
            return None;
        }
        let distance = C6_WIFI_MAC_TX_QUEUE_CONTROL_HIGH - offset;
        distance.is_multiple_of(16).then_some((distance / 16) as u8)
    }
}

impl Device for EspC6WifiMacRegisters {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let index = checked_word_index(&self.name, offset, width)?;
        let state = self.state.lock().expect("ESP32-C6 Wi-Fi MAC lock poisoned");
        state
            .registers
            .get(index)
            .copied()
            .map(u64::from)
            .ok_or_else(|| DeviceError::new(format!("{} read outside native page", self.name)))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        let index = checked_word_index(&self.name, offset, width)?;
        let mut state = self.state.lock().expect("ESP32-C6 Wi-Fi MAC lock poisoned");
        let mut value = u32::try_from(value)
            .map_err(|_| DeviceError::new("ESP32-C6 Wi-Fi MAC rejects wide writes"))?;
        if offset == C6_WIFI_MAC_INTERRUPT_CLEAR {
            state.registers[C6_WIFI_MAC_INTERRUPT_EVENT as usize / 4] &= !value;
            return Ok(());
        }
        if offset == C6_WIFI_MAC_TX_QUEUE_STATE_CLEAR {
            state.registers[C6_WIFI_MAC_TX_QUEUE_STATE as usize / 4] &= !value;
            state.registers[index] = value;
            return Ok(());
        }
        if offset == C6_WIFI_MAC_RESET_CONTROL && value & C6_WIFI_MAC_RESET_START != 0 {
            value |= C6_WIFI_MAC_RESET_READY;
        }
        *state.registers.get_mut(index).ok_or_else(|| {
            DeviceError::new(format!("{} write outside native page", self.name))
        })? = value;
        if offset == C6_WIFI_MAC_RX_BASE {
            state.rx_descriptor = (value != 0).then_some(value);
            state.registers[C6_WIFI_MAC_RX_NEXT as usize / 4] = value;
        }
        if let Some(queue) = Self::tx_queue(offset)
            && value & C6_WIFI_MAC_TX_QUEUE_ENABLE == C6_WIFI_MAC_TX_QUEUE_ENABLE
        {
            let bit = 1_u32 << queue;
            if state.active_tx & bit != 0
                || state.registers[C6_WIFI_MAC_TX_QUEUE_STATE as usize / 4] & bit != 0
            {
                return Err(DeviceError::new(format!(
                    "ESP32-C6 Wi-Fi queue {queue} was kicked before its previous completion was cleared"
                )));
            }
            let descriptor = 0x4080_0000 | (value & 0x000f_ffff);
            if descriptor != 0x4080_0000 {
                state.active_tx |= bit;
                state.pending_tx.push_back(EspC6WifiTxDescriptor {
                    queue,
                    address: descriptor,
                });
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state
            .lock()
            .expect("ESP32-C6 Wi-Fi MAC lock poisoned")
            .reset();
    }
}

/// ESP32-C6 modem clock/reset register bank.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EspC6ModemBank {
    /// High-performance modem system control at `0x600a9800`.
    Syscon,
    /// Low-power modem control at `0x600af000`.
    Lpcon,
}

#[derive(Clone, Debug)]
struct ModemState {
    syscon: [u32; 512],
    lpcon: [u32; 512],
    wifi_reset_generation: u64,
    ble_reset_generation: u64,
    ieee802154_reset_generation: u64,
    coexist_reset_generation: u64,
}

impl ModemState {
    fn reset(&mut self) {
        self.syscon = [0; 512];
        self.syscon[1] = 1 << 21;
        self.syscon[8] = 4 << 3;
        self.syscon[9] = 35_676_928;
        self.lpcon = [0; 512];
        self.lpcon[10] = (1 << 0) | (1 << 2) | (1 << 4) | (4 << 15);
        self.lpcon[11] = 35_676_736;
        self.wifi_reset_generation = self.wifi_reset_generation.wrapping_add(1);
        self.ble_reset_generation = self.ble_reset_generation.wrapping_add(1);
        self.ieee802154_reset_generation = self.ieee802154_reset_generation.wrapping_add(1);
        self.coexist_reset_generation = self.coexist_reset_generation.wrapping_add(1);
    }
}

/// Host-side view of C6 radio clock, reset, and power eligibility.
#[derive(Clone, Debug)]
pub struct EspC6ModemHandle {
    state: Arc<Mutex<ModemState>>,
}

impl EspC6ModemHandle {
    /// Returns whether the Wi-Fi MAC/APB clock is enabled or forced on and not reset.
    pub fn wifi_ready(&self) -> bool {
        let state = self.state.lock().expect("C6 modem state lock poisoned");
        let clocks = state.syscon[5] | state.syscon[6];
        clocks & ((1 << 9) | (1 << 10)) == ((1 << 9) | (1 << 10))
            && state.syscon[4] & ((1 << 8) | (1 << 10)) == 0
    }

    /// Returns whether the Bluetooth MAC/baseband clocks are enabled or forced on and not reset.
    pub fn ble_ready(&self) -> bool {
        let state = self.state.lock().expect("C6 modem state lock poisoned");
        let clocks = state.syscon[5] | state.syscon[6];
        clocks & ((1 << 17) | (1 << 18)) == ((1 << 17) | (1 << 18))
            && state.syscon[4] & ((1 << 15) | (1 << 16) | (1 << 17) | (1 << 18)) == 0
    }

    /// Returns whether the IEEE 802.15.4 APB/MAC clocks are enabled and not reset.
    pub fn ieee802154_ready(&self) -> bool {
        let state = self.state.lock().expect("C6 modem state lock poisoned");
        let clocks = state.syscon[1] | state.syscon[2];
        clocks & ((1 << 23) | (1 << 24)) == ((1 << 23) | (1 << 24))
            && state.syscon[4] & (1 << 24) == 0
    }

    /// Returns whether the low-power coexistence clock is enabled and not reset.
    pub fn coexistence_ready(&self) -> bool {
        let state = self.state.lock().expect("C6 modem state lock poisoned");
        let clocks = state.lpcon[6] | state.lpcon[7];
        clocks & (1 << 1) != 0
    }

    /// Reset generations for Wi-Fi, BLE, 802.15.4, and coexistence respectively.
    pub fn reset_generations(&self) -> [u64; 4] {
        let state = self.state.lock().expect("C6 modem state lock poisoned");
        [
            state.wifi_reset_generation,
            state.ble_reset_generation,
            state.ieee802154_reset_generation,
            state.coexist_reset_generation,
        ]
    }
}

/// One mapped half of the shared ESP32-C6 modem control block.
pub struct EspC6ModemControl {
    name: String,
    bank: EspC6ModemBank,
    state: Arc<Mutex<ModemState>>,
}

impl EspC6ModemControl {
    /// Creates the paired MODEM_SYSCON and MODEM_LPCON devices.
    pub fn new_pair(
        syscon_name: impl Into<String>,
        lpcon_name: impl Into<String>,
    ) -> (Self, Self, EspC6ModemHandle) {
        let state = Arc::new(Mutex::new(ModemState {
            syscon: [0; 512],
            lpcon: [0; 512],
            wifi_reset_generation: 0,
            ble_reset_generation: 0,
            ieee802154_reset_generation: 0,
            coexist_reset_generation: 0,
        }));
        state.lock().expect("C6 modem state lock poisoned").reset();
        (
            Self {
                name: syscon_name.into(),
                bank: EspC6ModemBank::Syscon,
                state: state.clone(),
            },
            Self {
                name: lpcon_name.into(),
                bank: EspC6ModemBank::Lpcon,
                state: state.clone(),
            },
            EspC6ModemHandle { state },
        )
    }

    fn registers<'a>(&self, state: &'a ModemState) -> &'a [u32] {
        match self.bank {
            EspC6ModemBank::Syscon => &state.syscon,
            EspC6ModemBank::Lpcon => &state.lpcon,
        }
    }

    fn write_mask(&self, index: usize) -> u32 {
        const SYSCON: [u32; 10] = [
            0x0000_0001,
            0xffe0_0000,
            0xffc0_0000,
            0xffff_ff00,
            0xefc7_c500,
            0x00ff_ffff,
            0x00ff_ffff,
            0xffff_ffff,
            0x0000_00ff,
            0x0fff_ffff,
        ];
        const LPCON: [u32; 12] = [
            0x0000_0003,
            0x0000_ffff,
            0x0000_ffff,
            0x0000_ffff,
            0x0000_0001,
            0x0000_0003,
            0x0000_000f,
            0x0000_03ff,
            0xffff_0000,
            0x0000_000f,
            0x000f_ffff,
            0x0fff_ffff,
        ];
        match self.bank {
            EspC6ModemBank::Syscon => SYSCON.get(index).copied().unwrap_or(u32::MAX),
            EspC6ModemBank::Lpcon => LPCON.get(index).copied().unwrap_or(u32::MAX),
        }
    }

    fn record_reset_edges(&self, state: &mut ModemState, old: u32, new: u32) {
        let rising = !old & new;
        match self.bank {
            EspC6ModemBank::Syscon => {
                if rising & ((1 << 8) | (1 << 10) | (1 << 14)) != 0 {
                    state.wifi_reset_generation = state.wifi_reset_generation.wrapping_add(1);
                }
                if rising & ((1 << 15) | (1 << 16) | (1 << 17) | (1 << 18)) != 0 {
                    state.ble_reset_generation = state.ble_reset_generation.wrapping_add(1);
                }
                if rising & (1 << 24) != 0 {
                    state.ieee802154_reset_generation =
                        state.ieee802154_reset_generation.wrapping_add(1);
                }
            }
            EspC6ModemBank::Lpcon => {
                if new & (1 << 1) != 0 {
                    state.coexist_reset_generation = state.coexist_reset_generation.wrapping_add(1);
                }
            }
        }
    }
}

impl Device for EspC6ModemControl {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let index = checked_word_index(&self.name, offset, width)?;
        let state = self.state.lock().expect("C6 modem state lock poisoned");
        self.registers(&state)
            .get(index)
            .copied()
            .map(u64::from)
            .ok_or_else(|| DeviceError::new(format!("{} read at {offset:#x}", self.name)))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        let index = checked_word_index(&self.name, offset, width)?;
        let mut state = self.state.lock().expect("C6 modem state lock poisoned");
        if index >= self.registers(&state).len() {
            return Err(DeviceError::new(format!(
                "{} write at {offset:#x}",
                self.name
            )));
        }
        let old = self.registers(&state)[index];
        let mask = self.write_mask(index);
        let new = (old & !mask) | ((value as u32) & mask);
        if self.bank == EspC6ModemBank::Lpcon && index == 9 {
            self.record_reset_edges(&mut state, old, value as u32);
            return Ok(());
        }
        match self.bank {
            EspC6ModemBank::Syscon => state.syscon[index] = new,
            EspC6ModemBank::Lpcon => state.lpcon[index] = new,
        }
        if self.bank == EspC6ModemBank::Syscon && index == 4 {
            self.record_reset_edges(&mut state, old, new);
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state
            .lock()
            .expect("C6 modem state lock poisoned")
            .reset();
    }
}

fn checked_word_index(name: &str, offset: u64, width: AccessWidth) -> Result<usize, DeviceError> {
    if width != AccessWidth::Word || offset & 3 != 0 {
        return Err(DeviceError::new(format!(
            "{name} requires aligned word access"
        )));
    }
    usize::try_from(offset / 4).map_err(|_| DeviceError::new(format!("{name} offset overflow")))
}

#[cfg(test)]
include!("esp_c6_radio_tests.rs");
