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

// The first word is the free-running modem timebase. The TSF latch, four
// target comparators, and power-event registers implement their native
// firmware-visible behavior; other words retain software-visible state while
// analog effects are handled by the functional radio model.
const C6_PHY_TSF_LATCH_CONTROL: u64 = 0x014;
const C6_PHY_TSF_LOW: u64 = 0x020;
const C6_PHY_TSF_HIGH: u64 = 0x024;
const C6_PHY_TSF_TIMER_CONTROL_BASE: u64 = 0x074;
const C6_PHY_TSF_TIMER_TARGET_BASE: u64 = 0x078;
const C6_PHY_TSF_TIMER_STRIDE: u64 = 8;
const C6_PHY_POWER_INTERRUPT_ENABLE: u64 = 0x0a8;
const C6_PHY_POWER_INTERRUPT_RAW: u64 = 0x0ac;
const C6_PHY_POWER_INTERRUPT_STATUS: u64 = 0x0b0;
const C6_PHY_POWER_INTERRUPT_CLEAR: u64 = 0x0b4;
const C6_PHY_TSF_TIMER_ENABLE: u32 = 1 << 31;
const C6_PHY_TSF_TIMER_WAKEUP_ENABLE: u32 = 1 << 30;
const C6_PHY_TICKS_PER_TSF_MICROSECOND: u64 = 16;

struct EspC6PhyRegistersState {
    registers: [u32; 1024],
    fired_tsf_timers: u8,
}

/// Host-side view of the native C6 PHY timer and interrupt state.
#[derive(Clone)]
pub struct EspC6PhyRegistersHandle {
    state: Arc<Mutex<EspC6PhyRegistersState>>,
}

impl EspC6PhyRegistersHandle {
    /// Advances the four native TSF comparators and returns newly fired timers.
    pub fn advance_to(&self, at: SimTime) -> u64 {
        advance_c6_phy_tsf_timers(
            &mut self
                .state
                .lock()
                .expect("ESP32-C6 PHY register lock poisoned"),
            at,
        )
    }

    /// Reports whether a masked PHY power/timer event is pending.
    pub fn interrupt_pending(&self) -> bool {
        let state = self
            .state
            .lock()
            .expect("ESP32-C6 PHY register lock poisoned");
        state.registers[C6_PHY_POWER_INTERRUPT_STATUS as usize / 4] != 0
    }

    /// Checks the timer ordering emitted by the pinned C6 vendor HAL.
    pub fn validate_tsf_timers(&self) -> Result<(), String> {
        let state = self
            .state
            .lock()
            .expect("ESP32-C6 PHY register lock poisoned");
        let interrupt_enable = state.registers[C6_PHY_POWER_INTERRUPT_ENABLE as usize / 4];
        for timer in 0..4 {
            let control_offset = C6_PHY_TSF_TIMER_CONTROL_BASE + timer * C6_PHY_TSF_TIMER_STRIDE;
            let control = state.registers[control_offset as usize / 4];
            let interrupt_bit = 0x80_u32 >> timer;
            if control & C6_PHY_TSF_TIMER_ENABLE != 0 && interrupt_enable & interrupt_bit == 0 {
                return Err(format!(
                    "native TSF timer {timer} is enabled before its firmware interrupt bit"
                ));
            }
            if control & C6_PHY_TSF_TIMER_WAKEUP_ENABLE != 0
                && control & C6_PHY_TSF_TIMER_ENABLE == 0
            {
                return Err(format!(
                    "native TSF timer {timer} requests wakeup while disabled"
                ));
            }
        }
        let raw = state.registers[C6_PHY_POWER_INTERRUPT_RAW as usize / 4];
        let status = state.registers[C6_PHY_POWER_INTERRUPT_STATUS as usize / 4];
        if status != raw & interrupt_enable {
            return Err(format!(
                "native PHY interrupt status {status:#010x} does not match raw {raw:#010x} and enable {interrupt_enable:#010x}"
            ));
        }
        Ok(())
    }
}

/// ESP32-C6 PHY-private register page with native TSF timer behavior.
pub struct EspC6PhyRegisters {
    name: String,
    state: Arc<Mutex<EspC6PhyRegistersState>>,
}

impl EspC6PhyRegisters {
    /// Creates a reset PHY register page.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            state: Arc::new(Mutex::new(EspC6PhyRegistersState {
                registers: [0; 1024],
                fired_tsf_timers: 0,
            })),
        }
    }

    /// Returns the machine-facing timer and interrupt handle.
    pub fn handle(&self) -> EspC6PhyRegistersHandle {
        EspC6PhyRegistersHandle {
            state: self.state.clone(),
        }
    }
}

fn c6_phy_tsf_time(at: SimTime) -> u64 {
    at.ticks() / C6_PHY_TICKS_PER_TSF_MICROSECOND
}

fn advance_c6_phy_tsf_timers(state: &mut EspC6PhyRegistersState, at: SimTime) -> u64 {
    let now = c6_phy_tsf_time(at) as u32;
    let mut fired = 0_u64;
    for timer in 0..4 {
        let timer_mask = 1_u8 << timer;
        if state.fired_tsf_timers & timer_mask != 0 {
            continue;
        }
        let control_offset = C6_PHY_TSF_TIMER_CONTROL_BASE + timer * C6_PHY_TSF_TIMER_STRIDE;
        let target_offset = C6_PHY_TSF_TIMER_TARGET_BASE + timer * C6_PHY_TSF_TIMER_STRIDE;
        let control = state.registers[control_offset as usize / 4];
        if control & C6_PHY_TSF_TIMER_ENABLE == 0 {
            continue;
        }
        let target = state.registers[target_offset as usize / 4];
        if now.wrapping_sub(target) >= 0x8000_0000 {
            continue;
        }
        state.fired_tsf_timers |= timer_mask;
        state.registers[C6_PHY_POWER_INTERRUPT_RAW as usize / 4] |= 0x80_u32 >> timer;
        fired = fired.saturating_add(1);
    }
    let raw = state.registers[C6_PHY_POWER_INTERRUPT_RAW as usize / 4];
    let enable = state.registers[C6_PHY_POWER_INTERRUPT_ENABLE as usize / 4];
    state.registers[C6_PHY_POWER_INTERRUPT_STATUS as usize / 4] = raw & enable;
    fired
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
        let mut state = self
            .state
            .lock()
            .expect("ESP32-C6 PHY register lock poisoned");
        advance_c6_phy_tsf_timers(&mut state, at);
        state
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
        at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || !offset.is_multiple_of(4) {
            return Err(DeviceError::new(
                "ESP32-C6 PHY registers require aligned word access",
            ));
        }
        let mut state = self
            .state
            .lock()
            .expect("ESP32-C6 PHY register lock poisoned");
        advance_c6_phy_tsf_timers(&mut state, at);
        if offset == C6_PHY_POWER_INTERRUPT_CLEAR {
            state.registers[C6_PHY_POWER_INTERRUPT_RAW as usize / 4] &= !(value as u32);
            let raw = state.registers[C6_PHY_POWER_INTERRUPT_RAW as usize / 4];
            let enable = state.registers[C6_PHY_POWER_INTERRUPT_ENABLE as usize / 4];
            state.registers[C6_PHY_POWER_INTERRUPT_STATUS as usize / 4] = raw & enable;
            return Ok(());
        }
        if offset == C6_PHY_POWER_INTERRUPT_RAW || offset == C6_PHY_POWER_INTERRUPT_STATUS {
            return Ok(());
        }
        let register = state
            .registers
            .get_mut(offset as usize / 4)
            .ok_or_else(|| DeviceError::new(format!("{} write outside native page", self.name)))?;
        *register = value as u32;
        if offset == C6_PHY_TSF_LATCH_CONTROL && value & 3 != 0 {
            let tsf = c6_phy_tsf_time(at);
            state.registers[C6_PHY_TSF_LOW as usize / 4] = tsf as u32;
            state.registers[C6_PHY_TSF_HIGH as usize / 4] = (tsf >> 32) as u32;
        }
        for timer in 0..4 {
            let timer_mask = 1_u8 << timer;
            let control_offset = C6_PHY_TSF_TIMER_CONTROL_BASE + timer * C6_PHY_TSF_TIMER_STRIDE;
            let target_offset = C6_PHY_TSF_TIMER_TARGET_BASE + timer * C6_PHY_TSF_TIMER_STRIDE;
            if offset == target_offset
                || offset == control_offset && value as u32 & C6_PHY_TSF_TIMER_ENABLE == 0
            {
                state.fired_tsf_timers &= !timer_mask;
            }
        }
        if offset == C6_PHY_POWER_INTERRUPT_ENABLE {
            let raw = state.registers[C6_PHY_POWER_INTERRUPT_RAW as usize / 4];
            state.registers[C6_PHY_POWER_INTERRUPT_STATUS as usize / 4] = raw & value as u32;
        }
        Ok(())
    }

    fn trace_value(&self, offset: u64, width: AccessWidth, at: SimTime) -> Option<u64> {
        if width != AccessWidth::Word || !offset.is_multiple_of(4) {
            return None;
        }
        if offset == 0 {
            return Some(at.ticks() & u64::from(u32::MAX));
        }
        self.state
            .lock()
            .expect("ESP32-C6 PHY register lock poisoned")
            .registers
            .get(offset as usize / 4)
            .copied()
            .map(u64::from)
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self
            .state
            .lock()
            .expect("ESP32-C6 PHY register lock poisoned");
        state.registers.fill(0);
        state.fired_tsf_timers = 0;
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
const C6_FREQUENCY_CODE_MASK: u32 = 0x3fff;
const C6_FREQUENCY_CHANNEL_BASE: u32 = 0x380;
const C6_FREQUENCY_CHANNEL_STRIDE: u32 = 0x280;
const C6_FREQUENCY_MODE_MASK: u32 = 0xffff_c000;
const C6_FREQUENCY_HT20_MODE: u32 = 0x4284_4000;
const C6_IQ_ESTIMATE_CONTROL: u64 = 0x474;
const C6_IQ_ESTIMATE_STATUS: u64 = 0x4a0;
const C6_IQ_ESTIMATE_START: u32 = 1 << 1;
const C6_IQ_ESTIMATE_DONE: u32 = 1 << 16;
const C6_TX_GAIN_FIRST: u64 = 0x8cc;
const C6_TX_GAIN_SECOND: u64 = 0x8d0;
const C6_TX_GAIN_FINAL: u64 = 0x8d4;
const C6_TX_GAIN_START_SENTINEL: u32 = 0xfe;
const C6_TX_GAIN_ENTRY_COUNT: u8 = 43;
const C6_VENDOR_TX_POWER_QDBM: i16 = 84;
const C6_VENDOR_GAIN_PROGRAM: [(u32, u32); 59] = [
    (0x1002_0301, 0xffff_f807),
    (0x1002_0301, 0xffff_f607),
    (0x9002_0301, 0xffff_f706),
    (0x1002_0301, 0xffff_f806),
    (0x1002_0301, 0xffff_f606),
    (0x9002_0301, 0xffff_f783),
    (0x9002_0301, 0xffff_fa82),
    (0x9002_0301, 0xffff_f882),
    (0x9002_0301, 0xffff_f682),
    (0x9002_0301, 0xffff_fb81),
    (0x9002_0301, 0xffff_f981),
    (0x9002_0301, 0xffff_f781),
    (0x1002_0301, 0xffff_fa81),
    (0x1002_0301, 0xffff_f881),
    (0x1002_0301, 0xffff_f681),
    (0x9002_0301, 0xffff_fb80),
    (0x9002_0301, 0xffff_f980),
    (0x9002_0301, 0xffff_f780),
    (0x9002_0301, 0xffff_f580),
    (0x1006_0100, 0xffff_fb80),
    (0x1006_0100, 0xffff_f980),
    (0x1006_0100, 0xffff_f780),
    (0x1002_0301, 0xffff_f980),
    (0x1002_0301, 0xffff_f780),
    (0x1002_0301, 0xffff_f580),
    (0x1002_0301, 0xffff_f380),
    (0x1002_0301, 0xffff_f180),
    (0x1002_0301, 0xffff_ef80),
    (0x1002_0301, 0xffff_ed80),
    (0x1002_0301, 0xffff_eb80),
    (0x1002_0301, 0xffff_e980),
    (0x1002_0301, 0xffff_e780),
    (0xe205_0080, 0x0000_00fe),
    (0xe207_0080, 0x0000_00fe),
    (0xe209_0080, 0x0000_00fe),
    (0xe20b_0080, 0x0000_00fe),
    (0xe301_0080, 0x0000_00fe),
    (0xe303_0080, 0x0000_00fe),
    (0xe305_0080, 0x0000_00fe),
    (0xe307_0080, 0x0000_00fe),
    (0xe309_0080, 0x0000_00fe),
    (0xe30b_0080, 0x0000_00fe),
    (0xe381_0080, 0x0000_00fe),
    (0xe383_0080, 0x0000_00fe),
    (0xe385_0080, 0x0000_00fe),
    (0xe387_0080, 0x0000_00fe),
    (0xe389_0080, 0x0000_00fe),
    (0xe38b_0080, 0x0000_00fe),
    (0xe3c1_0080, 0x0000_00fe),
    (0xe3c3_0080, 0x0000_00fe),
    (0xe3c5_0080, 0x0000_00fe),
    (0xe3c7_0080, 0x0000_00fe),
    (0xe3c9_0080, 0x0000_00fe),
    (0xe3cb_0080, 0x0000_00fe),
    (0xe3e1_0080, 0x0000_00fe),
    (0xe3e3_0080, 0x0000_00fe),
    (0xe3e5_0080, 0x0000_00fe),
    (0xe3e7_0080, 0x0000_00fe),
    (0xe3e9_0080, 0x0000_00fe),
];
const C6_FRONTEND_FORCE: u64 = 0x910;
const C6_FRONTEND_FORCE_MASK: u32 = 0x0f00;
const C6_FRONTEND_FORCED_OFF: u32 = 0x0200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EspC6PowerDetectorState {
    registers: [u32; 1024],
    channel: Option<u8>,
    bandwidth_khz: Option<u32>,
    pll_locked: bool,
    calibration_valid: bool,
    calibration_generation: u64,
    calibrated_generation: Option<u64>,
    power_qdbm: Option<i16>,
    frontend_released: Option<bool>,
    gain_phase: u8,
    gain_entries: u8,
    gain_sentinel_age: Option<u8>,
    gain_first_word: u32,
    gain_second_word: u32,
    vendor_gain_entries: u8,
    vendor_gain_matches: bool,
    generation: u64,
}

impl EspC6PowerDetectorState {
    fn new() -> Self {
        Self {
            registers: [0; 1024],
            channel: None,
            bandwidth_khz: None,
            pll_locked: false,
            calibration_valid: false,
            calibration_generation: 0,
            calibrated_generation: None,
            power_qdbm: None,
            frontend_released: None,
            gain_phase: 0,
            gain_entries: 0,
            gain_sentinel_age: None,
            gain_first_word: 0,
            gain_second_word: 0,
            vendor_gain_entries: 0,
            vendor_gain_matches: true,
            generation: 0,
        }
    }

    fn invalidate_rf(&mut self) {
        self.channel = None;
        self.bandwidth_khz = None;
        self.pll_locked = false;
        self.calibration_valid = false;
        self.calibration_generation = self.calibration_generation.wrapping_add(1);
        self.calibrated_generation = None;
        self.power_qdbm = None;
        self.frontend_released = None;
        self.gain_phase = 0;
        self.gain_entries = 0;
        self.gain_sentinel_age = None;
        self.gain_first_word = 0;
        self.gain_second_word = 0;
        self.vendor_gain_entries = 0;
        self.vendor_gain_matches = true;
        self.generation = self.generation.wrapping_add(1);
    }

    fn observe_rf_write(&mut self, offset: u64, value: u32) {
        if offset == C6_FREQUENCY_CONTROL
            && value & C6_FREQUENCY_MODE_MASK == C6_FREQUENCY_HT20_MODE
        {
            let code = value & C6_FREQUENCY_CODE_MASK;
            self.channel = code
                .checked_sub(C6_FREQUENCY_CHANNEL_BASE)
                .filter(|delta| delta % C6_FREQUENCY_CHANNEL_STRIDE == 0)
                .and_then(|delta| u8::try_from(delta / C6_FREQUENCY_CHANNEL_STRIDE).ok())
                .filter(|channel| (1..=14).contains(channel));
            self.pll_locked = true;
            self.bandwidth_khz = Some(20_000);
            self.calibration_generation = self.calibration_generation.wrapping_add(1);
            self.calibrated_generation = None;
            self.calibration_valid = false;
            self.power_qdbm = None;
            self.gain_phase = 0;
            self.gain_entries = 0;
            self.gain_sentinel_age = None;
            self.gain_first_word = 0;
            self.gain_second_word = 0;
            self.vendor_gain_entries = 0;
            self.vendor_gain_matches = true;
        }
        match offset {
            C6_TX_GAIN_FIRST => {
                self.gain_phase = 1;
                self.gain_first_word = value;
            }
            C6_TX_GAIN_SECOND if self.gain_phase == 1 => {
                self.gain_phase = 2;
                self.gain_second_word = value;
            }
            C6_TX_GAIN_SECOND => self.gain_phase = 0,
            C6_TX_GAIN_FINAL if self.gain_phase == 2 => {
                self.gain_phase = 0;
                let mut vendor_complete = false;
                if self.vendor_gain_matches {
                    let vendor_entry =
                        C6_VENDOR_GAIN_PROGRAM.get(usize::from(self.vendor_gain_entries));
                    self.vendor_gain_matches = vendor_entry.is_some_and(|expected| {
                        self.gain_first_word == 0x4020_0000
                            && *expected == (self.gain_second_word, value)
                    });
                    if self.vendor_gain_matches {
                        self.vendor_gain_entries = self.vendor_gain_entries.saturating_add(1);
                        vendor_complete =
                            usize::from(self.vendor_gain_entries) == C6_VENDOR_GAIN_PROGRAM.len();
                    }
                }
                if value == C6_TX_GAIN_START_SENTINEL {
                    self.gain_sentinel_age = Some(0);
                } else {
                    self.gain_sentinel_age = self
                        .gain_sentinel_age
                        .and_then(|age| age.checked_add(1))
                        .filter(|age| *age < C6_TX_GAIN_ENTRY_COUNT);
                }
                self.gain_entries = self.gain_entries.saturating_add(1);
                if self.gain_entries >= C6_TX_GAIN_ENTRY_COUNT {
                    self.gain_entries = C6_TX_GAIN_ENTRY_COUNT;
                    let decoded = (value as i32) / 128 + 133;
                    self.power_qdbm = self
                        .gain_sentinel_age
                        .and_then(|_| i16::try_from(decoded).ok())
                        .filter(|power| (8..=84).contains(power));
                    self.calibration_valid = self.power_qdbm.is_some();
                    self.calibrated_generation = self
                        .calibration_valid
                        .then_some(self.calibration_generation);
                }
                if vendor_complete {
                    self.gain_entries = C6_TX_GAIN_ENTRY_COUNT;
                    self.power_qdbm = Some(C6_VENDOR_TX_POWER_QDBM);
                    self.calibration_valid = true;
                    self.calibrated_generation = Some(self.calibration_generation);
                }
            }
            C6_TX_GAIN_FINAL => self.gain_phase = 0,
            _ => {}
        }

        if offset == C6_FRONTEND_FORCE {
            self.frontend_released = match value & C6_FRONTEND_FORCE_MASK {
                0 => Some(true),
                C6_FRONTEND_FORCED_OFF => Some(false),
                _ => None,
            };
        }
    }
}

/// Causal ESP32-C6 Wi-Fi RF configuration recovered from native register writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EspC6WifiRfSnapshot {
    /// Selected 2.4 GHz Wi-Fi channel, if a valid RFPLL strobe was observed.
    pub channel: Option<u8>,
    /// Selected RF bandwidth derived from the observed RFPLL mode encoding.
    pub bandwidth_khz: Option<u32>,
    /// Whether the last RFPLL channel strobe completed with a supported code.
    pub pll_locked: bool,
    /// Whether calibration completed since the current reset/invalidation.
    pub calibration_valid: bool,
    /// Generation established by the most recent RFPLL channel strobe.
    pub calibration_generation: u64,
    /// Generation for which a complete supported gain program was accepted.
    pub calibrated_generation: Option<u64>,
    /// Requested transmit-power ceiling in quarter-dBm, after a complete gain table.
    pub power_qdbm: Option<i16>,
    /// Whether the Wi-Fi frontend was explicitly released or forced off.
    pub frontend_released: Option<bool>,
    /// Number of entries accepted from the current 43-entry gain table.
    pub gain_entries: u8,
    /// Reset/invalidation generation for stale-state detection.
    pub generation: u64,
}

impl EspC6WifiRfSnapshot {
    /// Returns the selected center frequency using the 2.4 GHz channel plan.
    pub fn center_khz(self) -> Option<u32> {
        let channel = self.channel?;
        match channel {
            1..=13 => Some(2_412_000 + u32::from(channel - 1) * 5_000),
            14 => Some(2_484_000),
            _ => None,
        }
    }

    /// Returns true only when all causal state needed for Wi-Fi airtime is present.
    pub fn airtime_ready(self) -> bool {
        self.center_khz().is_some()
            && self.bandwidth_khz == Some(20_000)
            && self.pll_locked
            && self.calibration_valid
            && self.calibrated_generation == Some(self.calibration_generation)
            && self.power_qdbm.is_some()
            && self.frontend_released == Some(true)
            && self.gain_entries == C6_TX_GAIN_ENTRY_COUNT
    }
}

/// Machine-facing view of the ESP32-C6 RF configuration.
#[derive(Clone, Debug)]
pub struct EspC6PowerDetectorHandle {
    state: Arc<Mutex<EspC6PowerDetectorState>>,
}

impl EspC6PowerDetectorHandle {
    /// Returns a coherent snapshot derived entirely from guest MMIO writes.
    pub fn wifi_rf_snapshot(&self) -> EspC6WifiRfSnapshot {
        let state = self
            .state
            .lock()
            .expect("ESP32-C6 power-detector lock poisoned");
        EspC6WifiRfSnapshot {
            channel: state.channel,
            bandwidth_khz: state.bandwidth_khz,
            pll_locked: state.pll_locked,
            calibration_valid: state.calibration_valid,
            calibration_generation: state.calibration_generation,
            calibrated_generation: state.calibrated_generation,
            power_qdbm: state.power_qdbm,
            frontend_released: state.frontend_released,
            gain_entries: state.gain_entries,
            generation: state.generation,
        }
    }

    /// Invalidates all causal RF state after a Wi-Fi reset edge.
    pub fn invalidate_wifi_rf(&self) {
        self.state
            .lock()
            .expect("ESP32-C6 power-detector lock poisoned")
            .invalidate_rf();
    }
}

/// ESP32-C6 RF power-detector and calibration register page.
///
/// The RF calibration code starts a detector conversion with bit zero of the
/// conversion register and polls bit 22 for completion. Conversions complete
/// synchronously in simulation; the comparator outputs remain deterministic
/// zero until an analog RF environment is attached.
pub struct EspC6PowerDetector {
    name: String,
    state: Arc<Mutex<EspC6PowerDetectorState>>,
}

impl EspC6PowerDetector {
    /// Creates a reset RF power-detector register page.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            state: Arc::new(Mutex::new(EspC6PowerDetectorState::new())),
        }
    }

    /// Returns the machine-facing causal RF-state handle.
    pub fn handle(&self) -> EspC6PowerDetectorHandle {
        EspC6PowerDetectorHandle {
            state: self.state.clone(),
        }
    }
}

impl Device for EspC6PowerDetector {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let index = checked_word_index(&self.name, offset, width)?;
        self.state
            .lock()
            .expect("ESP32-C6 power-detector lock poisoned")
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
        let value = u32::try_from(value)
            .map_err(|_| DeviceError::new("ESP32-C6 power detector rejects wide writes"))?;
        let mut state = self
            .state
            .lock()
            .expect("ESP32-C6 power-detector lock poisoned");
        if index >= state.registers.len() {
            return Err(DeviceError::new(format!(
                "{} write outside native page",
                self.name
            )));
        }
        state.registers[index] = value & !C6_POWER_DETECTOR_DONE;
        if offset == C6_POWER_DETECTOR_CONVERSION && value & C6_POWER_DETECTOR_START != 0 {
            state.registers[index] |= C6_POWER_DETECTOR_DONE;
        }
        if offset == C6_POWER_DETECTOR_TONE_CONTROL && value & C6_POWER_DETECTOR_START != 0 {
            let status = &mut state.registers[C6_POWER_DETECTOR_TONE_STATUS as usize / 4];
            *status = (*status & !(7 << 14)) | C6_POWER_DETECTOR_TONE_IDLE;
        }
        if offset == C6_FREQUENCY_CONTROL && value & C6_FREQUENCY_CHANNEL_START != 0 {
            state.registers[C6_FREQUENCY_STATUS as usize / 4] |= C6_FREQUENCY_CHANNEL_DONE;
        }
        if offset == C6_IQ_ESTIMATE_CONTROL && value & C6_IQ_ESTIMATE_START != 0 {
            state.registers[C6_IQ_ESTIMATE_STATUS as usize / 4] |= C6_IQ_ESTIMATE_DONE;
        }
        state.observe_rf_write(offset, value);
        Ok(())
    }

    fn trace_value(&self, offset: u64, width: AccessWidth, _at: SimTime) -> Option<u64> {
        let state = self.state.lock().ok()?;
        (width == AccessWidth::Word && offset.is_multiple_of(4))
            .then(|| state.registers.get(offset as usize / 4).copied())
            .flatten()
            .map(u64::from)
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self
            .state
            .lock()
            .expect("ESP32-C6 power-detector lock poisoned");
        state.registers.fill(0);
        state.invalidate_rf();
    }
}

include!("esp_c6_wifi_mac.rs");
include!("esp_c6_modem.rs");

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
