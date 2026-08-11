use aes::Aes128;
use aes::cipher::{Array, BlockCipherEncrypt, KeyInit};
use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

const PHY_TX_DC_CALIBRATION_CONTROL: u64 = 0x4c;
const PHY_TX_DC_CALIBRATION_START: u32 = 1 << 1;
const PHY_TX_DC_CALIBRATION_DONE: u32 = 1 << 24;
const PHY_PACKET_DETECTOR_CONTROL: u64 = 0x50;
const PHY_PACKET_DETECTOR_RESTART: u32 = 1 << 1;
const PHY_PACKET_DETECTOR_STATE_MASK: u32 = 7 << 24;
const PHY_PACKET_DETECTOR_IDLE: u32 = 7 << 24;
const FE_IQ_ESTIMATE_CONTROL: u64 = 0x144;
const FE_IQ_ESTIMATE_START: u32 = 1 << 1;
const FE_IQ_ESTIMATE_STATUS: u64 = 0x174;
const FE_IQ_ESTIMATE_DONE: u32 = 1 << 16;
const WIFI_MAC_RESET_CONTROL: u64 = 0x0d14;
const WIFI_MAC_RESET_START: u32 = 1 << 1;
const WIFI_MAC_RESET_READY: u32 = 1 << 0;
const WIFI_MAC_INTERRUPT_MASK: u64 = 0x0c34;
const WIFI_MAC_INTERRUPT_EVENT: u64 = 0x0c3c;
const WIFI_MAC_INTERRUPT_CLEAR: u64 = 0x0c40;
const WIFI_MAC_EVENT_TX_DONE: u32 = 1 << 7;
const WIFI_MAC_EVENT_RX_DONE: u32 = 1 << 14;
const WIFI_MAC_RX_BASE: u64 = 0x0088;
const WIFI_MAC_RX_NEXT: u64 = 0x008c;
const WIFI_MAC_RX_LAST: u64 = 0x0090;
const WIFI_MAC_RX_ADDRESS_HIGH: u64 = 0x0c64;
const WIFI_MAC_INTERFACE_ADDRESS_LOW: u64 = 0x040;
const WIFI_MAC_INTERFACE_ADDRESS_HIGH: u64 = 0x044;
const WIFI_MAC_INTERFACE_ADDRESS_STRIDE: u64 = 8;
const WIFI_MAC_INTERFACE_ADDRESS_COUNT: usize = 4;
const WIFI_MAC_TX_QUEUE_CONTROL_HIGH: u64 = 0x0d08;
const WIFI_MAC_TX_QUEUE_CONTROL_LOW: u64 = 0x0cd0;
const WIFI_MAC_TX_QUEUE_ENABLE: u32 = 3 << 30;
const WIFI_MAC_TX_QUEUE_STATE_CLEAR: u64 = 0x0cac;
const WIFI_MAC_TX_QUEUE_STATE: u64 = 0x0cb0;
const WIFI_MAC_CURRENT_TIME: u64 = 0x2000;
const WIFI_MAC_TSF_LATCH_CONTROL: u64 = 0x200c;
const WIFI_MAC_TSF_HIGH: u64 = 0x2018;
const WIFI_MAC_TSF_LOW: u64 = 0x201c;
const WIFI_MAC_RANDOM_DATA: u64 = 0x207c;
const WIFI_MAC_RANDOM_SEED: u32 = 0x32f3_0001;
const BLE_TIME_LATCH: u64 = 0x01c;
const BLE_FINE_TIME: u64 = 0x020;
const BLE_TIME_LATCH_REQUEST: u32 = 1 << 31;
// ESP32-S3's SYSTIMER runs at 16 MHz in the vendor runtime. RWBLE's fine
// counter has 625 half-microsecond positions per 312.5-us half-slot, so one
// fine position spans eight simulation/SYSTIMER ticks.
const BLE_FINE_POSITION_TICKS: u64 = 8;
const BLE_FINE_POSITIONS_PER_HALF_SLOT: u64 = 625;
const BLE_HALF_SLOT_TICKS: u64 = BLE_FINE_POSITION_TICKS * BLE_FINE_POSITIONS_PER_HALF_SLOT;
const BLE_CORE_CONTROL: u64 = 0x000;
const BLE_CORE_SOFT_RESET: u32 = 1 << 31;
const BLE_CORE_SW_INTERRUPT_REQUEST: u32 = 1 << 27;
const BLE_CORE_VERSION: u64 = 0x004;
const BLE_CORE_VERSION_ESP32S3: u32 = 0x0900_1b00;
const BLE_INTERRUPT_STATUS: u64 = 0x010;
const BLE_INTERRUPT_CLEAR: u64 = 0x018;
const BLE_INTERRUPT_ENABLE: u64 = 0x00c;
const BLE_TIMER_HALF_SLOT: u64 = 0x0ec;
const BLE_TIMER_FINE: u64 = 0x0f0;
const BLE_TIMER_INTERRUPT: u32 = 1 << 11;
const BLE_SOFTWARE_INTERRUPT: u32 = 1 << 12;
const BLE_CRYPT_INTERRUPT: u32 = 1 << 7;
const BLE_CRYPT_START: u64 = 0x0b0;
const BLE_CRYPT_START_REQUEST: u32 = 1;
const BLE_CRYPT_KEY_BASE: u64 = 0x0b4;
const BLE_CRYPT_INPUT_RESULT_OFFSET: u64 = 0x0c4;
const BLE_ECO_INTERRUPT_DIAGNOSTIC: u64 = 0x2d8;
const BLE_ECO_ACTIVE_STATE: u32 = 1 << 5;
const BLE_ECO_STATUS_MASK: u32 = 0x001f_ffff;
const BLE_ECO_STATUS_SHIFT: u32 = 10;
const BLE_ECO_STATUS_FIELD: u32 = BLE_ECO_STATUS_MASK << BLE_ECO_STATUS_SHIFT;
const BLE_SCHEDULER_KICK: u64 = 0x100;
const BLE_SCHEDULER_START: u32 = 1 << 31;
const BLE_EM_MAPPING_BANK0_FIRST: u64 = 0x204;
const BLE_EM_MAPPING_BANK0_COUNT: usize = 48;
const BLE_EM_MAPPING_BANK1_FIRST: u64 = 0x2e0;
const BLE_EM_MAPPING_BANK1_COUNT: usize = 8;
const BLE_EM_MAPPING_COUNT: usize = BLE_EM_MAPPING_BANK0_COUNT + BLE_EM_MAPPING_BANK1_COUNT;
const BLE_EM_MAPPING_STRIDE: u64 = 4;
const BLE_EM_MAPPING_VALID_LOW: u64 = 0x2c4;
const BLE_EM_MAPPING_VALID_HIGH: u64 = 0x2c8;
const BLE_EM_MAPPING_VALID_TOP: u64 = 0x300;
const BLE_EM_CPU_ADDRESS_MASK: u32 = 0x0003_ffff;
const BLE_EM_OFFSET_SHIFT: u32 = 18;
// r_lld_update_rxbuf reads the hardware-owned receive head from RWBLE offset
// 0x024. Offset 0x2d0 is the firmware-written receive-buffer update command,
// not the descriptor that hardware is currently filling.
const BLE_RX_BUFFER_CURRENT: u64 = 0x024;
const BLE_RX_BUFFER_RING_BASE: u32 = 0x1000;

#[derive(Default)]
struct Esp32S3BleExchangeMemoryState {
    registers: Vec<u32>,
    pending_schedule_kicks: VecDeque<u32>,
    pending_crypt_commands: VecDeque<Esp32S3BleCryptCommand>,
    pending_radio_completions: VecDeque<(u64, u32)>,
    timer_due: Option<u64>,
}

/// One native scheduler command strobed by the ESP32-S3 BLE link layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Esp32S3BleScheduleKick {
    /// Native scheduler-control word, including the hardware start strobe.
    pub control: u32,
}

/// One native RWBLE AES-128 ECB transaction submitted by the S3 controller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Esp32S3BleCryptCommand {
    /// AES-128 key captured from RWBLE crypt key registers.
    pub key: [u8; 16],
    /// Firmware-programmed exchange-memory offset containing the input block.
    /// Native RWBLE writes the result to the immediately following 16 bytes.
    pub input_offset: u32,
}

impl Esp32S3BleCryptCommand {
    /// Encrypts one firmware-provided block with the captured AES-128 key.
    pub fn encrypt_block(self, input: [u8; 16]) -> [u8; 16] {
        let cipher = Aes128::new_from_slice(&self.key).expect("AES-128 key has fixed length");
        let mut block = Array::from(input);
        cipher.encrypt_block(&mut block);
        block.into()
    }
}

/// One firmware-programmed exchange-memory aperture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Esp32S3BleEmMapping {
    /// Exchange-memory byte offset represented by the mapping selector.
    pub em_offset: u32,
    /// CPU-visible byte address backing that exchange-memory aperture.
    pub cpu_address: u32,
}

/// Scheduler-facing view of the ESP32-S3 BLE exchange-memory registers.
#[derive(Clone)]
pub struct Esp32S3BleExchangeMemoryHandle {
    state: Arc<Mutex<Esp32S3BleExchangeMemoryState>>,
}

impl Esp32S3BleExchangeMemoryHandle {
    /// Advances the native RWBLE half-slot timer and raises its interrupt at the deadline.
    pub fn advance_to(&self, now: SimTime) {
        let mut state = self
            .state
            .lock()
            .expect("ESP32-S3 BLE exchange-memory lock poisoned");
        if state.timer_due.is_some_and(|due| now.ticks() >= due) {
            state.registers[BLE_INTERRUPT_STATUS as usize / 4] |= BLE_TIMER_INTERRUPT;
            state.timer_due = None;
        }
        while state
            .pending_radio_completions
            .front()
            .is_some_and(|(due, _)| now.ticks() >= *due)
        {
            if let Some((_, causes)) = state.pending_radio_completions.pop_front() {
                state.registers[BLE_INTERRUPT_STATUS as usize / 4] |= causes;
            }
        }
    }

    /// Removes the oldest native scheduler kick submitted by firmware.
    pub fn take_schedule_kick(&self) -> Option<Esp32S3BleScheduleKick> {
        self.state
            .lock()
            .expect("ESP32-S3 BLE exchange-memory lock poisoned")
            .pending_schedule_kicks
            .pop_front()
            .map(|control| Esp32S3BleScheduleKick { control })
    }

    /// Removes the oldest native RWBLE crypt transaction submitted by firmware.
    pub fn take_crypt_command(&self) -> Option<Esp32S3BleCryptCommand> {
        self.state
            .lock()
            .expect("ESP32-S3 BLE exchange-memory lock poisoned")
            .pending_crypt_commands
            .pop_front()
    }

    /// Publishes native AES completion through RWBLE crypt interrupt cause 7.
    pub fn complete_crypt(&self) {
        self.state
            .lock()
            .expect("ESP32-S3 BLE exchange-memory lock poisoned")
            .registers[BLE_INTERRUPT_STATUS as usize / 4] |= BLE_CRYPT_INTERRUPT;
    }

    /// Returns the controller-owned current receive descriptor offset.
    pub fn rx_buffer_current(&self) -> u16 {
        self.state
            .lock()
            .expect("ESP32-S3 BLE exchange-memory lock poisoned")
            .registers[BLE_RX_BUFFER_CURRENT as usize / 4] as u16
    }

    /// Advances the controller-owned receive ring after a packet is committed.
    pub fn advance_rx_buffer(&self, next: u16) {
        self.state
            .lock()
            .expect("ESP32-S3 BLE exchange-memory lock poisoned")
            .registers[BLE_RX_BUFFER_CURRENT as usize / 4] = u32::from(next);
    }

    /// Returns the exchange-memory apertures programmed by the controller.
    pub fn em_mappings(&self) -> Vec<Esp32S3BleEmMapping> {
        let state = self
            .state
            .lock()
            .expect("ESP32-S3 BLE exchange-memory lock poisoned");
        (0..BLE_EM_MAPPING_COUNT)
            .filter_map(|mapping| {
                if !ble_em_mapping_is_valid(&state.registers, mapping) {
                    return None;
                }
                let offset = ble_em_mapping_register(mapping);
                let raw = state.registers[offset as usize / 4];
                Some(Esp32S3BleEmMapping {
                    // The revision-zero ROM's r_emi_get_mem_addr_by_offset
                    // extracts bits 31:18 and multiplies by four. Mappings may
                    // therefore begin at any four-byte exchange-memory offset,
                    // not just at a 1-KiB segment boundary.
                    em_offset: (raw >> BLE_EM_OFFSET_SHIFT) << 2,
                    // The ROM stores the low 18 bits of an S3 internal-DRAM
                    // address divided by four. The hardware
                    // supplies the fixed 0x3fc00000 DRAM aperture prefix.
                    cpu_address: 0x3fc0_0000 | ((raw & BLE_EM_CPU_ADDRESS_MASK) << 2),
                })
            })
            .collect()
    }

    /// Resolves one controller exchange-memory byte offset to its CPU-visible DRAM address.
    ///
    /// The S3 ROM programs a separate base for each variable-sized exchange-memory
    /// allocation. A mapping therefore remains active until the next programmed
    /// exchange-memory base, rather than being limited to the 1-KiB selector unit.
    pub fn resolve_em_address(&self, em_offset: u16) -> Option<u32> {
        let state = self
            .state
            .lock()
            .expect("ESP32-S3 BLE exchange-memory lock poisoned");
        let target = u32::from(em_offset);
        (0..BLE_EM_MAPPING_COUNT)
            .filter_map(|mapping| {
                if !ble_em_mapping_is_valid(&state.registers, mapping) {
                    return None;
                }
                let offset = ble_em_mapping_register(mapping);
                let raw = state.registers[offset as usize / 4];
                let base = (raw >> BLE_EM_OFFSET_SHIFT) << 2;
                (base <= target).then_some((base, raw))
            })
            .max_by_key(|(base, _)| *base)
            .map(|(base, raw)| {
                let cpu_base = 0x3fc0_0000 | ((raw & BLE_EM_CPU_ADDRESS_MASK) << 2);
                cpu_base.wrapping_add(target - base)
            })
    }

    /// Schedules native RWBLE interrupt causes for completion of an RF operation.
    pub fn schedule_radio_completion(&self, due: SimTime, causes: u32) {
        let mut state = self
            .state
            .lock()
            .expect("ESP32-S3 BLE exchange-memory lock poisoned");
        let insertion = state
            .pending_radio_completions
            .iter()
            .position(|(existing, _)| *existing > due.ticks())
            .unwrap_or(state.pending_radio_completions.len());
        state
            .pending_radio_completions
            .insert(insertion, (due.ticks(), causes));
    }

    /// Cancels one exact RF completion when an earlier terminal outcome wins.
    pub fn cancel_radio_completion(&self, due: SimTime, causes: u32) -> bool {
        let mut state = self
            .state
            .lock()
            .expect("ESP32-S3 BLE exchange-memory lock poisoned");
        let Some(index) = state
            .pending_radio_completions
            .iter()
            .position(|pending| *pending == (due.ticks(), causes))
        else {
            return false;
        };
        state.pending_radio_completions.remove(index);
        true
    }

    /// Raises one or more native RWBLE interrupt causes.
    pub fn raise_interrupt(&self, causes: u32) {
        self.state
            .lock()
            .expect("ESP32-S3 BLE exchange-memory lock poisoned")
            .registers[BLE_INTERRUPT_STATUS as usize / 4] |= causes;
    }

    /// Whether any RWBLE interrupt cause is awaiting firmware acknowledgement.
    pub fn interrupt_pending(&self) -> bool {
        self.state
            .lock()
            .expect("ESP32-S3 BLE exchange-memory lock poisoned")
            .registers[BLE_INTERRUPT_STATUS as usize / 4]
            != 0
    }
}

fn ble_em_mapping_register(mapping: usize) -> u64 {
    debug_assert!(mapping < BLE_EM_MAPPING_COUNT);
    if mapping < BLE_EM_MAPPING_BANK0_COUNT {
        BLE_EM_MAPPING_BANK0_FIRST + mapping as u64 * BLE_EM_MAPPING_STRIDE
    } else {
        BLE_EM_MAPPING_BANK1_FIRST
            + (mapping - BLE_EM_MAPPING_BANK0_COUNT) as u64 * BLE_EM_MAPPING_STRIDE
    }
}

fn ble_em_mapping_is_valid(registers: &[u32], mapping: usize) -> bool {
    let (bitmap, bit) = if mapping < 32 {
        (BLE_EM_MAPPING_VALID_LOW, mapping)
    } else if mapping < 48 {
        (BLE_EM_MAPPING_VALID_HIGH, mapping - 32)
    } else {
        (BLE_EM_MAPPING_VALID_TOP, mapping - 48)
    };
    registers[bitmap as usize / 4] & (1_u32 << bit) != 0
}

/// ESP32-S3 BLE exchange-memory and native controller timer registers.
///
/// The revision-zero mask ROM's `r_rwip_time_get` routine establishes the
/// hardware protocol directly: it sets bit 31 at offset `0x01c`, polls until
/// hardware clears that bit, then consumes the latched coarse clock together
/// with the fine counter at offset `0x020`. Other words retain ordinary
/// exchange-memory mapping state programmed by the genuine controller.
pub struct Esp32S3BleExchangeMemoryRegisters {
    name: String,
    state: Arc<Mutex<Esp32S3BleExchangeMemoryState>>,
}

impl Esp32S3BleExchangeMemoryRegisters {
    /// Creates the native 8 KiB BLE exchange-memory register window.
    pub fn new(name: impl Into<String>) -> Self {
        let mut registers = vec![0; 0x2000 / 4];
        // The link-layer core verifies this read-only hardware revision word during
        // r_lld_core_init before it enables any scheduled BLE activity.
        registers[BLE_CORE_VERSION as usize / 4] = BLE_CORE_VERSION_ESP32S3;
        // Revision-zero ROM routines treat this hardware-owned pointer as an
        // exchange-memory offset into the ten-entry receive ring. Both
        // r_lld_update_rxbuf and lld_update_rxbuf_handler subtract 0x1000
        // before converting the pointer to a descriptor index.
        registers[BLE_RX_BUFFER_CURRENT as usize / 4] = BLE_RX_BUFFER_RING_BASE;
        Self {
            name: name.into(),
            state: Arc::new(Mutex::new(Esp32S3BleExchangeMemoryState {
                registers,
                pending_schedule_kicks: VecDeque::new(),
                pending_crypt_commands: VecDeque::new(),
                pending_radio_completions: VecDeque::new(),
                timer_due: None,
            })),
        }
    }

    /// Returns the scheduler and interrupt handle coupled to this register page.
    pub fn handle(&self) -> Esp32S3BleExchangeMemoryHandle {
        Esp32S3BleExchangeMemoryHandle {
            state: self.state.clone(),
        }
    }

    fn arm_timer(state: &mut Esp32S3BleExchangeMemoryState, at: SimTime) {
        const COARSE_MASK: u64 = 0x0fff_ffff;
        const TIMER_CYCLE_TICKS: u64 = (COARSE_MASK + 1) * BLE_HALF_SLOT_TICKS;
        let coarse = u64::from(state.registers[BLE_TIMER_HALF_SLOT as usize / 4]) & COARSE_MASK;
        let fine = u64::from(state.registers[BLE_TIMER_FINE as usize / 4])
            .min(BLE_FINE_POSITIONS_PER_HALF_SLOT - 1);
        let target_in_cycle = coarse * BLE_HALF_SLOT_TICKS
            + (BLE_FINE_POSITIONS_PER_HALF_SLOT - 1 - fine) * BLE_FINE_POSITION_TICKS;
        let now_in_cycle = at.ticks() % TIMER_CYCLE_TICKS;
        let delta = target_in_cycle
            .wrapping_add(TIMER_CYCLE_TICKS)
            .wrapping_sub(now_in_cycle)
            % TIMER_CYCLE_TICKS;
        state.timer_due = Some(at.ticks().saturating_add(delta));
    }
}

impl Device for Esp32S3BleExchangeMemoryRegisters {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        let state = self
            .state
            .lock()
            .expect("ESP32-S3 BLE exchange-memory lock poisoned");
        let index = checked_register_index(&self.name, state.registers.len(), offset, width)?;
        if offset == BLE_FINE_TIME {
            let phase = at.ticks() % BLE_HALF_SLOT_TICKS;
            return Ok(BLE_FINE_POSITIONS_PER_HALF_SLOT - 1 - phase / BLE_FINE_POSITION_TICKS);
        }
        if offset == BLE_ECO_INTERRUPT_DIAGNOSTIC {
            // The S3 revision-zero ROM's RWBLE ISR works around an early
            // controller erratum by consuming a live interrupt snapshot from
            // this diagnostic word rather than INTSTAT directly. Bits 10:30
            // mirror the low 21 interrupt causes and bits 5:9 report a
            // non-idle controller state while a cause is pending. The other
            // bits retain the control value programmed by firmware.
            let pending = state.registers[BLE_INTERRUPT_STATUS as usize / 4] & BLE_ECO_STATUS_MASK;
            let programmed = state.registers[index] & !(BLE_ECO_STATUS_FIELD | (0x1f << 5));
            let live = if pending == 0 {
                0
            } else {
                BLE_ECO_ACTIVE_STATE | (pending << BLE_ECO_STATUS_SHIFT)
            };
            return Ok(u64::from(programmed | live));
        }
        Ok(u64::from(state.registers[index]))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        let mut state = self
            .state
            .lock()
            .expect("ESP32-S3 BLE exchange-memory lock poisoned");
        let index = checked_register_index(&self.name, state.registers.len(), offset, width)?;
        let value = u32::try_from(value)
            .map_err(|_| DeviceError::new("ESP32-S3 BLE registers reject wide writes"))?;
        if offset == BLE_INTERRUPT_CLEAR {
            state.registers[BLE_INTERRUPT_STATUS as usize / 4] &= !value;
            state.registers[index] = value;
            return Ok(());
        }
        if offset == BLE_ECO_INTERRUPT_DIAGNOSTIC {
            // The rev0 ECO path writes the snapshot it just consumed back to
            // the diagnostic register. Hardware treats the mirrored cause
            // field as W1C; this is how scheduler-end causes are acknowledged
            // (ordinary TX/RX causes are also redundantly acknowledged through
            // INTACK). Active-state bits are live status and are not retained.
            let acknowledged = (value & BLE_ECO_STATUS_FIELD) >> BLE_ECO_STATUS_SHIFT;
            state.registers[BLE_INTERRUPT_STATUS as usize / 4] &= !acknowledged;
            state.registers[index] = value & !(BLE_ECO_STATUS_FIELD | (0x1f << 5));
            return Ok(());
        }
        if offset == BLE_CORE_VERSION {
            return Ok(());
        }
        state.registers[index] = if offset == BLE_CORE_CONTROL {
            // RESET and SWINT are command strobes. The controller consumes
            // both rather than retaining either bit in RWBLECNTL. SWINT
            // publishes cause bit 12, which the ROM acknowledges through
            // INTACK before it runs the software scheduler.
            if value & BLE_CORE_SOFT_RESET != 0 {
                state.registers[BLE_RX_BUFFER_CURRENT as usize / 4] = BLE_RX_BUFFER_RING_BASE;
            }
            if value & BLE_CORE_SW_INTERRUPT_REQUEST != 0 {
                state.registers[BLE_INTERRUPT_STATUS as usize / 4] |= BLE_SOFTWARE_INTERRUPT;
            }
            value & !(BLE_CORE_SOFT_RESET | BLE_CORE_SW_INTERRUPT_REQUEST)
        } else if offset == BLE_TIME_LATCH && value & BLE_TIME_LATCH_REQUEST != 0 {
            // Hardware acknowledges the request synchronously and publishes
            // the 28-bit native half-slot clock with the request bit clear.
            ((at.ticks() / BLE_HALF_SLOT_TICKS) as u32) & 0x0fff_ffff
        } else {
            value
        };
        if offset == BLE_INTERRUPT_ENABLE {
            if value & BLE_TIMER_INTERRUPT == 0 {
                state.timer_due = None;
            } else {
                Self::arm_timer(&mut state, at);
            }
        } else if matches!(offset, BLE_TIMER_HALF_SLOT | BLE_TIMER_FINE)
            && state.registers[BLE_INTERRUPT_ENABLE as usize / 4] & BLE_TIMER_INTERRUPT != 0
        {
            // Once enabled, RWBLE's comparator consumes subsequent target
            // writes directly; firmware does not need to toggle INTENABLE for
            // every scheduler deadline.
            Self::arm_timer(&mut state, at);
        }
        if offset == BLE_SCHEDULER_KICK && value & BLE_SCHEDULER_START != 0 {
            state.pending_schedule_kicks.push_back(value);
        }
        if offset == BLE_CRYPT_START && value & BLE_CRYPT_START_REQUEST != 0 {
            if !state.pending_crypt_commands.is_empty() {
                return Err(DeviceError::new(
                    "ESP32-S3 RWBLE crypt START overlaps an unfinished transaction",
                ));
            }
            let mut key = [0_u8; 16];
            for word in 0..4 {
                key[word * 4..word * 4 + 4].copy_from_slice(
                    &state.registers[BLE_CRYPT_KEY_BASE as usize / 4 + word].to_le_bytes(),
                );
            }
            let input_offset = state.registers[BLE_CRYPT_INPUT_RESULT_OFFSET as usize / 4];
            state
                .pending_crypt_commands
                .push_back(Esp32S3BleCryptCommand { key, input_offset });
            // START is a hardware-consumed command strobe.
            state.registers[index] &= !BLE_CRYPT_START_REQUEST;
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self
            .state
            .lock()
            .expect("ESP32-S3 BLE exchange-memory lock poisoned");
        state.registers.fill(0);
        state.registers[BLE_CORE_VERSION as usize / 4] = BLE_CORE_VERSION_ESP32S3;
        state.registers[BLE_RX_BUFFER_CURRENT as usize / 4] = BLE_RX_BUFFER_RING_BASE;
        state.pending_schedule_kicks.clear();
        state.pending_crypt_commands.clear();
        state.pending_radio_completions.clear();
        state.timer_due = None;
    }
}

struct Esp32S3WifiMacState {
    registers: Vec<u32>,
    random_state: u32,
    pending_tx: VecDeque<Esp32S3WifiTxDescriptor>,
    rx_descriptor: Option<u32>,
}

impl Esp32S3WifiMacState {
    fn reset(&mut self) {
        self.registers.fill(0);
        self.random_state = WIFI_MAC_RANDOM_SEED;
        self.pending_tx.clear();
        self.rx_descriptor = None;
    }

    fn next_random(&mut self) -> u32 {
        let mut value = self.random_state;
        value ^= value << 13;
        value ^= value >> 17;
        value ^= value << 5;
        self.random_state = value;
        value
    }
}

/// One native ESP32-S3 Wi-Fi transmit descriptor submitted by guest firmware.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Esp32S3WifiTxDescriptor {
    /// Native MAC queue index.
    pub queue: u8,
    /// Reconstructed DRAM address of the first DMA descriptor.
    pub address: u32,
}

/// One native ESP32-S3 Wi-Fi receive descriptor owned by the MAC DMA engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Esp32S3WifiRxDescriptor {
    /// Full DRAM address programmed through the native receive-base register.
    pub address: u32,
}

/// Host-side view of ESP32-S3 Wi-Fi MAC event state.
#[derive(Clone)]
pub struct Esp32S3WifiMacHandle {
    state: Arc<Mutex<Esp32S3WifiMacState>>,
}

impl Esp32S3WifiMacHandle {
    /// Whether an enabled Wi-Fi MAC event asserts interrupt-matrix source 0.
    pub fn interrupt_pending(&self) -> bool {
        let state = self.state.lock().expect("ESP32-S3 Wi-Fi MAC lock poisoned");
        let mask = state.registers[WIFI_MAC_INTERRUPT_MASK as usize / 4];
        let events = state.registers[WIFI_MAC_INTERRUPT_EVENT as usize / 4];
        mask & events != 0
    }

    /// Removes the oldest native DMA transmit submitted by firmware.
    pub fn take_tx_descriptor(&self) -> Option<Esp32S3WifiTxDescriptor> {
        self.state
            .lock()
            .expect("ESP32-S3 Wi-Fi MAC lock poisoned")
            .pending_tx
            .pop_front()
    }

    /// Returns the current firmware-provided receive descriptor, if armed.
    pub fn rx_descriptor(&self) -> Option<Esp32S3WifiRxDescriptor> {
        self.state
            .lock()
            .expect("ESP32-S3 Wi-Fi MAC lock poisoned")
            .rx_descriptor
            .map(|address| Esp32S3WifiRxDescriptor { address })
    }

    /// Returns the firmware-programmed RX-interface match bitmap for a receiver address.
    pub fn rx_match_mask(&self, receiver: &[u8]) -> u8 {
        let Some(receiver) = receiver.get(..6) else {
            return 0;
        };
        let state = self.state.lock().expect("ESP32-S3 Wi-Fi MAC lock poisoned");
        let mut configured = 0_u8;
        let mut matches = 0_u8;
        for interface in 0..WIFI_MAC_INTERFACE_ADDRESS_COUNT {
            let offset = interface as u64 * WIFI_MAC_INTERFACE_ADDRESS_STRIDE;
            let low = state.registers[(WIFI_MAC_INTERFACE_ADDRESS_LOW + offset) as usize / 4];
            let high = state.registers[(WIFI_MAC_INTERFACE_ADDRESS_HIGH + offset) as usize / 4];
            // Unlike C6, S3 stores only the upper two address bytes here.
            // hal_mac_set_addr enables a separate companion group-address
            // slot; a nonzero individual address therefore identifies a
            // configured interface without a validity bit in this word.
            if low == 0 && high & 0xffff == 0 {
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

    /// Advances the native receive ring and raises the hardware RX event.
    pub fn complete_rx_descriptor(&self, address: u32, next: u32) {
        let mut state = self.state.lock().expect("ESP32-S3 Wi-Fi MAC lock poisoned");
        state.registers[WIFI_MAC_RX_NEXT as usize / 4] = next;
        state.registers[WIFI_MAC_RX_LAST as usize / 4] = address & 0x000f_ffff;
        state.registers[WIFI_MAC_RX_ADDRESS_HIGH as usize / 4] = address & 0xfff0_0000;
        state.rx_descriptor = (next != 0).then_some(next);
        state.registers[WIFI_MAC_INTERRUPT_EVENT as usize / 4] |= WIFI_MAC_EVENT_RX_DONE;
    }
}

/// ESP32-S3 Wi-Fi MAC register window used by ROM and vendor net80211 code.
///
/// The revision-zero ROM and Wi-Fi libraries access three contiguous 4 KiB
/// pages beginning at `0x60033000`. Fields retain normal read/modify/write
/// state here; command, timer, DMA and interrupt fields are promoted into
/// explicit behavior as their firmware protocols are established.
pub struct Esp32S3WifiMacRegisters {
    name: String,
    state: Arc<Mutex<Esp32S3WifiMacState>>,
}

impl Esp32S3WifiMacRegisters {
    /// Creates the native 12 KiB Wi-Fi MAC/WDEV window.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            state: Arc::new(Mutex::new(Esp32S3WifiMacState {
                registers: vec![0; 0x3000 / 4],
                random_state: WIFI_MAC_RANDOM_SEED,
                pending_tx: VecDeque::new(),
                rx_descriptor: None,
            })),
        }
    }

    /// Returns the event handle coupled to this register frontend.
    pub fn handle(&self) -> Esp32S3WifiMacHandle {
        Esp32S3WifiMacHandle {
            state: self.state.clone(),
        }
    }

    fn is_tx_queue_control(offset: u64) -> bool {
        (WIFI_MAC_TX_QUEUE_CONTROL_LOW..=WIFI_MAC_TX_QUEUE_CONTROL_HIGH).contains(&offset)
            && (WIFI_MAC_TX_QUEUE_CONTROL_HIGH - offset).is_multiple_of(8)
    }
}

impl Device for Esp32S3WifiMacRegisters {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let mut state = self.state.lock().expect("ESP32-S3 Wi-Fi MAC lock poisoned");
        let index = checked_register_index(&self.name, state.registers.len(), offset, width)?;
        if offset == WIFI_MAC_INTERRUPT_CLEAR {
            return Ok(0);
        }
        if offset == WIFI_MAC_CURRENT_TIME {
            return Ok(_at.ticks() & u64::from(u32::MAX));
        }
        if offset == WIFI_MAC_RANDOM_DATA {
            return Ok(u64::from(state.next_random()));
        }
        let value = state.registers[index];
        Ok(u64::from(value))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        let mut state = self.state.lock().expect("ESP32-S3 Wi-Fi MAC lock poisoned");
        let index = checked_register_index(&self.name, state.registers.len(), offset, width)?;
        let mut value = u32::try_from(value)
            .map_err(|_| DeviceError::new("ESP32-S3 Wi-Fi MAC rejects wide writes"))?;
        if offset == WIFI_MAC_INTERRUPT_CLEAR {
            state.registers[WIFI_MAC_INTERRUPT_EVENT as usize / 4] &= !value;
            return Ok(());
        }
        if offset == WIFI_MAC_TX_QUEUE_STATE_CLEAR {
            state.registers[WIFI_MAC_TX_QUEUE_STATE as usize / 4] &= !value;
            state.registers[index] = value;
            return Ok(());
        }
        if offset == WIFI_MAC_TSF_LATCH_CONTROL && value & 0x3 != 0 {
            let tsf = at.ticks();
            state.registers[WIFI_MAC_TSF_LOW as usize / 4] = tsf as u32;
            state.registers[WIFI_MAC_TSF_HIGH as usize / 4] = (tsf >> 32) as u32;
        }
        if offset == WIFI_MAC_RESET_CONTROL && value & WIFI_MAC_RESET_START != 0 {
            value |= WIFI_MAC_RESET_READY;
        }
        state.registers[index] = value;
        if offset == WIFI_MAC_RX_BASE {
            state.rx_descriptor = (value != 0).then_some(value);
            state.registers[WIFI_MAC_RX_NEXT as usize / 4] = value;
        }
        if Self::is_tx_queue_control(offset)
            && value & WIFI_MAC_TX_QUEUE_ENABLE == WIFI_MAC_TX_QUEUE_ENABLE
        {
            let queue = (WIFI_MAC_TX_QUEUE_CONTROL_HIGH - offset) / 8;
            if queue < 4 {
                state.registers[WIFI_MAC_TX_QUEUE_STATE as usize / 4] |= 1 << queue;
            }
            let descriptor = 0x3fc0_0000 | (value & 0x000f_ffff);
            if descriptor != 0x3fc0_0000 {
                state.pending_tx.push_back(Esp32S3WifiTxDescriptor {
                    queue: queue as u8,
                    address: descriptor,
                });
            }
            state.registers[WIFI_MAC_INTERRUPT_EVENT as usize / 4] |= WIFI_MAC_EVENT_TX_DONE;
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state
            .lock()
            .expect("ESP32-S3 Wi-Fi MAC lock poisoned")
            .reset();
    }
}

/// ESP32-S3 RF front-end register page.
///
/// RX-IQ calibration arms the estimator at offset `0x144` and polls the
/// hardware-owned completion bit at offset `0x174`. The estimator completes
/// synchronously with deterministic zero-valued accumulators.
pub struct Esp32S3FeRegisters {
    name: String,
    registers: [u32; 1024],
}

impl Esp32S3FeRegisters {
    /// Creates a reset RF front-end page.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            registers: [0; 1024],
        }
    }
}

impl Device for Esp32S3FeRegisters {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let index = checked_register_index(&self.name, self.registers.len(), offset, width)?;
        Ok(u64::from(self.registers[index]))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        let index = checked_register_index(&self.name, self.registers.len(), offset, width)?;
        let value = u32::try_from(value)
            .map_err(|_| DeviceError::new("ESP32-S3 FE registers reject wide writes"))?;
        self.registers[index] = value;
        if offset == FE_IQ_ESTIMATE_CONTROL && value & FE_IQ_ESTIMATE_START != 0 {
            self.registers[FE_IQ_ESTIMATE_STATUS as usize / 4] |= FE_IQ_ESTIMATE_DONE;
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.registers.fill(0);
    }
}

/// ESP32-S3 AGC register page used by the revision-zero mask ROM.
///
/// The page occupies the private gap immediately before the public NRX base.
/// ROM AGC initialization performs ordinary read/modify/write sequences here;
/// completion and measurement fields will be promoted to explicit behavior as
/// the genuine firmware reaches them.
pub struct Esp32S3AgcRegisters {
    name: String,
    registers: [u32; 0x0c00 / 4],
}

impl Esp32S3AgcRegisters {
    /// Creates a reset AGC register page.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            registers: [0; 0x0c00 / 4],
        }
    }
}

impl Device for Esp32S3AgcRegisters {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let index = checked_register_index(&self.name, self.registers.len(), offset, width)?;
        Ok(u64::from(self.registers[index]))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        let index = checked_register_index(&self.name, self.registers.len(), offset, width)?;
        self.registers[index] = u32::try_from(value)
            .map_err(|_| DeviceError::new("ESP32-S3 AGC registers reject wide writes"))?;
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.registers.fill(0);
    }
}

/// ESP32-S3 PHY-private register page used by ROM and vendor PHY calibration.
///
/// Most words are ordinary firmware-visible state. The packet-detector state
/// field is hardware-owned: ROM restarts the detector through bit 1 and waits
/// for bits 26:24 to return to the idle state. The deterministic RF model
/// completes that transition synchronously until an analog timing model is
/// attached.
pub struct Esp32S3PhyRegisters {
    name: String,
    registers: [u32; 1024],
}

impl Esp32S3PhyRegisters {
    /// Creates a PHY register page in its hardware reset state.
    pub fn new(name: impl Into<String>) -> Self {
        let mut registers = [0; 1024];
        registers[0x40 / 4] = 1 << 24;
        registers[PHY_PACKET_DETECTOR_CONTROL as usize / 4] = PHY_PACKET_DETECTOR_IDLE;
        Self {
            name: name.into(),
            registers,
        }
    }

    fn checked_index(&self, offset: u64, width: AccessWidth) -> Result<usize, DeviceError> {
        checked_register_index(&self.name, self.registers.len(), offset, width)
    }
}

fn checked_register_index(
    name: &str,
    register_count: usize,
    offset: u64,
    width: AccessWidth,
) -> Result<usize, DeviceError> {
    if width != AccessWidth::Word || !offset.is_multiple_of(4) {
        return Err(DeviceError::new(format!(
            "{name} requires aligned word access"
        )));
    }
    let index = usize::try_from(offset / 4)
        .map_err(|_| DeviceError::new(format!("{name} offset overflow")))?;
    if index >= register_count {
        return Err(DeviceError::new(format!(
            "{name} access outside native page"
        )));
    }
    Ok(index)
}

impl Device for Esp32S3PhyRegisters {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let index = self.checked_index(offset, width)?;
        Ok(u64::from(self.registers[index]))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        let index = self.checked_index(offset, width)?;
        let value = u32::try_from(value)
            .map_err(|_| DeviceError::new("ESP32-S3 PHY registers reject wide writes"))?;
        if offset == PHY_TX_DC_CALIBRATION_CONTROL {
            self.registers[index] = value & !PHY_TX_DC_CALIBRATION_DONE;
            if value & PHY_TX_DC_CALIBRATION_START != 0 {
                self.registers[index] |= PHY_TX_DC_CALIBRATION_DONE;
            }
        } else if offset == PHY_PACKET_DETECTOR_CONTROL {
            let software_bits = value & !PHY_PACKET_DETECTOR_STATE_MASK;
            self.registers[index] = software_bits | PHY_PACKET_DETECTOR_IDLE;
            if value & PHY_PACKET_DETECTOR_RESTART != 0 {
                self.registers[index] |= PHY_PACKET_DETECTOR_RESTART;
            }
        } else {
            self.registers[index] = value;
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self = Self::new(self.name.clone());
    }
}

#[cfg(test)]
include!("esp32s3_radio_basic_tests.rs");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ble_half_slot_timer_raises_one_native_interrupt_at_programmed_fine_time() {
        let mut ble = Esp32S3BleExchangeMemoryRegisters::new("ble");
        let handle = ble.handle();
        let armed_at = SimTime::from_ticks(BLE_HALF_SLOT_TICKS * 10 + 100);
        ble.write(BLE_TIMER_HALF_SLOT, AccessWidth::Word, 12, armed_at)
            .unwrap();
        ble.write(BLE_TIMER_FINE, AccessWidth::Word, 624, armed_at)
            .unwrap();
        ble.write(
            BLE_INTERRUPT_CLEAR,
            AccessWidth::Word,
            u64::from(BLE_TIMER_INTERRUPT),
            armed_at,
        )
        .unwrap();
        ble.write(
            BLE_INTERRUPT_ENABLE,
            AccessWidth::Word,
            u64::from(BLE_TIMER_INTERRUPT),
            armed_at,
        )
        .unwrap();

        handle.advance_to(SimTime::from_ticks(BLE_HALF_SLOT_TICKS * 12 - 1));
        assert!(!handle.interrupt_pending());
        handle.advance_to(SimTime::from_ticks(BLE_HALF_SLOT_TICKS * 12));
        assert!(handle.interrupt_pending());
        ble.write(
            BLE_ECO_INTERRUPT_DIAGNOSTIC,
            AccessWidth::Word,
            0x8000_001e,
            SimTime::ZERO,
        )
        .unwrap();
        ble.write(
            BLE_TIMER_HALF_SLOT,
            AccessWidth::Word,
            14,
            SimTime::from_ticks(BLE_HALF_SLOT_TICKS * 12),
        )
        .unwrap();
        ble.write(
            BLE_TIMER_FINE,
            AccessWidth::Word,
            624,
            SimTime::from_ticks(BLE_HALF_SLOT_TICKS * 12),
        )
        .unwrap();
        assert_eq!(
            ble.read(
                BLE_ECO_INTERRUPT_DIAGNOSTIC,
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap(),
            u64::from(
                0x8000_001e | BLE_ECO_ACTIVE_STATE | (BLE_TIMER_INTERRUPT << BLE_ECO_STATUS_SHIFT)
            )
        );
        ble.write(
            BLE_INTERRUPT_CLEAR,
            AccessWidth::Word,
            u64::from(BLE_TIMER_INTERRUPT),
            SimTime::from_ticks(BLE_HALF_SLOT_TICKS * 12),
        )
        .unwrap();
        assert_eq!(
            ble.read(
                BLE_ECO_INTERRUPT_DIAGNOSTIC,
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap(),
            0x8000_001e
        );
        handle.advance_to(SimTime::from_ticks(BLE_HALF_SLOT_TICKS * 13));
        assert!(!handle.interrupt_pending());
        handle.advance_to(SimTime::from_ticks(BLE_HALF_SLOT_TICKS * 14));
        assert!(handle.interrupt_pending());
    }

    #[test]
    fn ble_exchange_memory_exposes_mapping_kick_and_w1c_interrupt_contract() {
        let mut ble = Esp32S3BleExchangeMemoryRegisters::new("ble");
        let handle = ble.handle();
        let cpu_address = 0x3fca_4000_u32;
        let em_offset = 0x0c34_u32;
        let encoded_mapping =
            ((em_offset >> 2) << BLE_EM_OFFSET_SHIFT) | ((cpu_address & 0x000f_ffff) >> 2);
        ble.write(
            BLE_EM_MAPPING_BANK0_FIRST,
            AccessWidth::Word,
            u64::from(encoded_mapping),
            SimTime::ZERO,
        )
        .unwrap();
        // A programmed base does not become visible to hardware until the ROM
        // marks its mapping slot allocated in the corresponding bitmap.
        assert_eq!(handle.resolve_em_address(em_offset as u16), None);
        ble.write(
            BLE_EM_MAPPING_VALID_LOW,
            AccessWidth::Word,
            1,
            SimTime::ZERO,
        )
        .unwrap();
        ble.write(
            BLE_SCHEDULER_KICK,
            AccessWidth::Word,
            u64::from(BLE_SCHEDULER_START | 0x42),
            SimTime::ZERO,
        )
        .unwrap();

        assert_eq!(
            handle.em_mappings(),
            [Esp32S3BleEmMapping {
                em_offset,
                cpu_address,
            }]
        );
        assert_eq!(handle.resolve_em_address(0x0c76), Some(cpu_address + 0x42));
        assert_eq!(handle.resolve_em_address(0x0800), None);
        assert_eq!(
            handle.take_schedule_kick(),
            Some(Esp32S3BleScheduleKick {
                control: BLE_SCHEDULER_START | 0x42,
            })
        );
        assert_eq!(handle.take_schedule_kick(), None);

        ble.write(
            BLE_EM_MAPPING_VALID_LOW,
            AccessWidth::Word,
            0,
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(handle.resolve_em_address(0x0c76), None);

        handle.raise_interrupt((1 << 2) | (1 << 6));
        assert!(handle.interrupt_pending());
        ble.write(
            BLE_INTERRUPT_CLEAR,
            AccessWidth::Word,
            1 << 2,
            SimTime::ZERO,
        )
        .unwrap();
        assert!(handle.interrupt_pending());
        assert_eq!(
            ble.read(BLE_INTERRUPT_STATUS, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            1 << 6
        );
        ble.write(
            BLE_INTERRUPT_CLEAR,
            AccessWidth::Word,
            1 << 6,
            SimTime::ZERO,
        )
        .unwrap();
        assert!(!handle.interrupt_pending());

        handle.schedule_radio_completion(SimTime::from_ticks(12), (1 << 1) | (1 << 5));
        handle.advance_to(SimTime::from_ticks(11));
        assert!(!handle.interrupt_pending());
        handle.advance_to(SimTime::from_ticks(12));
        assert!(handle.interrupt_pending());
        let diagnostic = ble
            .read(
                BLE_ECO_INTERRUPT_DIAGNOSTIC,
                AccessWidth::Word,
                SimTime::from_ticks(12),
            )
            .unwrap();
        ble.write(
            BLE_ECO_INTERRUPT_DIAGNOSTIC,
            AccessWidth::Word,
            diagnostic,
            SimTime::from_ticks(12),
        )
        .unwrap();
        assert!(!handle.interrupt_pending());
    }

    #[test]
    fn ble_crypt_start_captures_one_ecb_block_and_raises_native_interrupt() {
        let mut ble = Esp32S3BleExchangeMemoryRegisters::new("ble");
        let handle = ble.handle();
        let input = [
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
            0xee, 0xff,
        ];
        for (word, bytes) in input.chunks_exact(4).enumerate() {
            ble.write(
                BLE_CRYPT_KEY_BASE + word as u64 * 4,
                AccessWidth::Word,
                u64::from(u32::from_le_bytes(bytes.try_into().unwrap())),
                SimTime::ZERO,
            )
            .unwrap();
        }
        ble.write(
            BLE_CRYPT_INPUT_RESULT_OFFSET,
            AccessWidth::Word,
            0x0128,
            SimTime::ZERO,
        )
        .unwrap();
        ble.write(
            BLE_CRYPT_START,
            AccessWidth::Word,
            u64::from(BLE_CRYPT_START_REQUEST),
            SimTime::ZERO,
        )
        .unwrap();

        let command = handle.take_crypt_command().unwrap();
        assert_eq!(command.key, input);
        assert_eq!(command.input_offset, 0x0128);
        assert_eq!(
            ble.read(BLE_CRYPT_START, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            0
        );
        assert!(!handle.interrupt_pending());
        handle.complete_crypt();
        assert!(handle.interrupt_pending());
        ble.write(
            BLE_INTERRUPT_CLEAR,
            AccessWidth::Word,
            u64::from(BLE_CRYPT_INTERRUPT),
            SimTime::ZERO,
        )
        .unwrap();
        assert!(!handle.interrupt_pending());
    }

    #[test]
    fn packet_detector_is_idle_at_reset_and_after_rom_restart() {
        let mut phy = Esp32S3PhyRegisters::new("phy");
        let reset = phy
            .read(
                PHY_PACKET_DETECTOR_CONTROL,
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap() as u32;
        assert_eq!(
            reset & PHY_PACKET_DETECTOR_STATE_MASK,
            PHY_PACKET_DETECTOR_IDLE
        );

        phy.write(
            PHY_PACKET_DETECTOR_CONTROL,
            AccessWidth::Word,
            u64::from(PHY_PACKET_DETECTOR_RESTART),
            SimTime::ZERO,
        )
        .unwrap();
        let restarted = phy
            .read(
                PHY_PACKET_DETECTOR_CONTROL,
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap() as u32;
        assert_ne!(restarted & PHY_PACKET_DETECTOR_RESTART, 0);
        assert_eq!(
            restarted & PHY_PACKET_DETECTOR_STATE_MASK,
            PHY_PACKET_DETECTOR_IDLE
        );
    }

    #[test]
    fn packet_detector_status_is_hardware_owned() {
        let mut phy = Esp32S3PhyRegisters::new("phy");
        phy.write(
            PHY_PACKET_DETECTOR_CONTROL,
            AccessWidth::Word,
            u32::MAX as u64 & !PHY_PACKET_DETECTOR_STATE_MASK as u64,
            SimTime::ZERO,
        )
        .unwrap();
        let value = phy
            .read(
                PHY_PACKET_DETECTOR_CONTROL,
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap() as u32;
        assert_eq!(
            value & PHY_PACKET_DETECTOR_STATE_MASK,
            PHY_PACKET_DETECTOR_IDLE
        );
    }

    #[test]
    fn tx_dc_calibration_completes_after_command_edge() {
        let mut phy = Esp32S3PhyRegisters::new("phy");
        phy.write(
            PHY_TX_DC_CALIBRATION_CONTROL,
            AccessWidth::Word,
            0x0011_3cf1,
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            phy.read(
                PHY_TX_DC_CALIBRATION_CONTROL,
                AccessWidth::Word,
                SimTime::ZERO
            )
            .unwrap() as u32
                & PHY_TX_DC_CALIBRATION_DONE,
            0
        );

        phy.write(
            PHY_TX_DC_CALIBRATION_CONTROL,
            AccessWidth::Word,
            0x0011_3cf3,
            SimTime::ZERO,
        )
        .unwrap();
        let completed = phy
            .read(
                PHY_TX_DC_CALIBRATION_CONTROL,
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap() as u32;
        assert_ne!(completed & PHY_TX_DC_CALIBRATION_DONE, 0);
        assert_eq!(completed & 0xc000_0000, 0);
    }

    #[test]
    fn agc_page_retains_rom_initialization_words_and_resets() {
        let mut agc = Esp32S3AgcRegisters::new("agc");
        agc.write(0x13c, AccessWidth::Word, 0x0130_0000, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            agc.read(0x13c, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0x0130_0000
        );
        agc.reset(ResetKind::PowerOn);
        assert_eq!(
            agc.read(0x13c, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0
        );
    }

    #[test]
    fn rx_iq_estimator_sets_front_end_completion() {
        let mut fe = Esp32S3FeRegisters::new("fe");
        fe.write(
            FE_IQ_ESTIMATE_CONTROL,
            AccessWidth::Word,
            u64::from(FE_IQ_ESTIMATE_START),
            SimTime::ZERO,
        )
        .unwrap();
        assert_ne!(
            fe.read(FE_IQ_ESTIMATE_STATUS, AccessWidth::Word, SimTime::ZERO)
                .unwrap() as u32
                & FE_IQ_ESTIMATE_DONE,
            0
        );
    }

    #[test]
    fn wifi_mac_window_covers_all_three_native_pages() {
        let mut mac = Esp32S3WifiMacRegisters::new("wifi-mac");
        for (offset, value) in [(0x0400, 3), (0x1d04, 0x1234), (0x2d04, 0x5678)] {
            mac.write(offset, AccessWidth::Word, value, SimTime::ZERO)
                .unwrap();
            assert_eq!(
                mac.read(offset, AccessWidth::Word, SimTime::ZERO).unwrap(),
                value
            );
        }
        assert!(mac.read(0x3000, AccessWidth::Word, SimTime::ZERO).is_err());
    }

    #[test]
    fn wifi_mac_reset_command_acknowledges_ready() {
        let mut mac = Esp32S3WifiMacRegisters::new("wifi-mac");
        mac.write(
            WIFI_MAC_RESET_CONTROL,
            AccessWidth::Word,
            u64::from(WIFI_MAC_RESET_START),
            SimTime::ZERO,
        )
        .unwrap();
        let status = mac
            .read(WIFI_MAC_RESET_CONTROL, AccessWidth::Word, SimTime::ZERO)
            .unwrap() as u32;
        assert_eq!(
            status & (WIFI_MAC_RESET_START | WIFI_MAC_RESET_READY),
            WIFI_MAC_RESET_START | WIFI_MAC_RESET_READY
        );
    }

    #[test]
    fn wifi_mac_rx_match_uses_firmware_programmed_interface_address() {
        let mut mac = Esp32S3WifiMacRegisters::new("wifi-mac");
        let handle = mac.handle();
        assert_eq!(handle.rx_match_mask(&[0xff; 6]), 1);

        mac.write(
            WIFI_MAC_INTERFACE_ADDRESS_LOW + WIFI_MAC_INTERFACE_ADDRESS_STRIDE,
            AccessWidth::Word,
            0x2233_4455,
            SimTime::ZERO,
        )
        .unwrap();
        mac.write(
            WIFI_MAC_INTERFACE_ADDRESS_HIGH + WIFI_MAC_INTERFACE_ADDRESS_STRIDE,
            AccessWidth::Word,
            0x0111,
            SimTime::ZERO,
        )
        .unwrap();

        assert_eq!(
            handle.rx_match_mask(&[0x55, 0x44, 0x33, 0x22, 0x11, 0x01]),
            1 << 1
        );
        assert_eq!(
            handle.rx_match_mask(&[0x54, 0x44, 0x33, 0x22, 0x11, 0x02]),
            0
        );
        assert_eq!(handle.rx_match_mask(&[0xff; 6]), 1 << 1);
    }

    #[test]
    fn wifi_tx_queue_completion_asserts_and_clears_mac_interrupt() {
        let mut mac = Esp32S3WifiMacRegisters::new("wifi-mac");
        let handle = mac.handle();
        mac.write(
            WIFI_MAC_INTERRUPT_MASK,
            AccessWidth::Word,
            u64::from(WIFI_MAC_EVENT_TX_DONE),
            SimTime::ZERO,
        )
        .unwrap();
        mac.write(
            WIFI_MAC_TX_QUEUE_CONTROL_HIGH,
            AccessWidth::Word,
            u64::from(WIFI_MAC_TX_QUEUE_ENABLE | 0x5678),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            handle.take_tx_descriptor(),
            Some(Esp32S3WifiTxDescriptor {
                queue: 0,
                address: 0x3fc0_5678,
            })
        );
        assert!(handle.interrupt_pending());
        assert_eq!(
            mac.read(WIFI_MAC_TX_QUEUE_STATE, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            1
        );
        assert_ne!(
            mac.read(WIFI_MAC_INTERRUPT_EVENT, AccessWidth::Word, SimTime::ZERO)
                .unwrap() as u32
                & WIFI_MAC_EVENT_TX_DONE,
            0
        );

        mac.write(
            WIFI_MAC_INTERRUPT_CLEAR,
            AccessWidth::Word,
            u64::from(WIFI_MAC_EVENT_TX_DONE),
            SimTime::ZERO,
        )
        .unwrap();
        assert!(!handle.interrupt_pending());
        mac.write(
            WIFI_MAC_TX_QUEUE_STATE_CLEAR,
            AccessWidth::Word,
            1,
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            mac.read(WIFI_MAC_TX_QUEUE_STATE, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            0
        );
        assert_eq!(
            mac.read(WIFI_MAC_INTERRUPT_EVENT, AccessWidth::Word, SimTime::ZERO)
                .unwrap() as u32
                & WIFI_MAC_EVENT_TX_DONE,
            0
        );
    }

    #[test]
    fn wifi_rx_base_advances_native_ring_and_asserts_event() {
        let mut mac = Esp32S3WifiMacRegisters::new("wifi-mac");
        let handle = mac.handle();
        mac.write(
            WIFI_MAC_INTERRUPT_MASK,
            AccessWidth::Word,
            u64::from(WIFI_MAC_EVENT_RX_DONE),
            SimTime::ZERO,
        )
        .unwrap();
        mac.write(
            WIFI_MAC_RX_BASE,
            AccessWidth::Word,
            0x3fca_1000,
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            handle.rx_descriptor(),
            Some(Esp32S3WifiRxDescriptor {
                address: 0x3fca_1000
            })
        );
        handle.complete_rx_descriptor(0x3fca_1000, 0x3fca_100c);
        assert_eq!(
            handle.rx_descriptor(),
            Some(Esp32S3WifiRxDescriptor {
                address: 0x3fca_100c
            })
        );
        assert_eq!(
            mac.read(WIFI_MAC_RX_NEXT, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            0x3fca_100c
        );
        assert_eq!(
            mac.read(WIFI_MAC_RX_LAST, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            0x000a_1000
        );
        assert!(handle.interrupt_pending());
        mac.write(
            WIFI_MAC_INTERRUPT_CLEAR,
            AccessWidth::Word,
            u64::from(WIFI_MAC_EVENT_RX_DONE),
            SimTime::ZERO,
        )
        .unwrap();
        assert!(!handle.interrupt_pending());
    }

    #[test]
    fn wifi_wdev_latches_tsf_and_provides_replayable_entropy() {
        let mut mac = Esp32S3WifiMacRegisters::new("wifi-mac");
        assert_eq!(
            mac.read(
                WIFI_MAC_CURRENT_TIME,
                AccessWidth::Word,
                SimTime::from_ticks(0x1234_5678),
            )
            .unwrap(),
            0x1234_5678
        );
        mac.write(
            WIFI_MAC_TSF_LATCH_CONTROL,
            AccessWidth::Word,
            1,
            SimTime::from_ticks(0x1_2345_6789),
        )
        .unwrap();
        assert_eq!(
            mac.read(WIFI_MAC_TSF_LOW, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            0x2345_6789
        );
        assert_eq!(
            mac.read(WIFI_MAC_TSF_HIGH, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            1
        );

        let first = mac
            .read(WIFI_MAC_RANDOM_DATA, AccessWidth::Word, SimTime::ZERO)
            .unwrap();
        let second = mac
            .read(WIFI_MAC_RANDOM_DATA, AccessWidth::Word, SimTime::ZERO)
            .unwrap();
        assert_ne!(first, second);
        mac.reset(ResetKind::PowerOn);
        assert_eq!(
            mac.read(WIFI_MAC_RANDOM_DATA, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            first
        );
    }
}
