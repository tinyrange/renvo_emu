const C6_BLE_MODEM_TIMER_INTERRUPT_RAW: u64 = 0x01c;
const C6_BLE_MODEM_TIMER_CURRENT: u64 = 0x044;
const C6_BLE_MODEM_TIMER_COMPARE: u64 = 0x058;
const C6_BLE_MODEM_RTC_INTERRUPT_ENABLE: u64 = 0x010;
const C6_BLE_MODEM_RTC_INTERRUPT_CLEAR: u64 = 0x014;
const C6_BLE_MODEM_RTC_TIMER0_PENDING: u64 = 0x024;
const C6_BLE_MODEM_RTC_INTERRUPT_STATUS: u64 = 0x034;
const C6_BLE_MODEM_RTC_COMPARE: u64 = 0x060;
const C6_BLE_MODEM_RTC_INTERRUPT_BIT: u32 = 1 << 18;
// Genuine ESP-IDF configures the BLE sleep clock to 100 kHz when it divides
// the main 40 MHz crystal. Renvo's ESP simulation timebase is 16 MHz.
const C6_BLE_MODEM_TICKS_PER_SLEEP_TICK: u64 = 160;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum EspC6BleRtcPhase {
    #[default]
    Idle,
    CompareProgrammed,
    Armed,
    Pending,
}

struct EspC6BleModemState {
    registers: Vec<u32>,
    sync_compare_armed: bool,
    rtc_compare_armed: bool,
    rtc_phase: EspC6BleRtcPhase,
}

impl EspC6BleModemState {
    fn timer_value(at: SimTime) -> u32 {
        (at.ticks() / C6_BLE_MODEM_TICKS_PER_SLEEP_TICK) as u32
    }

    fn materialize_timer_compares(&mut self, at: SimTime) {
        let current = Self::timer_value(at);
        let compare = self.registers[C6_BLE_MODEM_TIMER_COMPARE as usize / 4];
        if self.sync_compare_armed && current.wrapping_sub(compare) < (1_u32 << 31) {
            self.registers[C6_BLE_MODEM_TIMER_INTERRUPT_RAW as usize / 4] = 1;
            self.sync_compare_armed = false;
        }
        let rtc_compare = self.registers[C6_BLE_MODEM_RTC_COMPARE as usize / 4];
        if self.rtc_compare_armed && current.wrapping_sub(rtc_compare) < (1_u32 << 31) {
            self.registers[C6_BLE_MODEM_RTC_TIMER0_PENDING as usize / 4] = 1;
            self.registers[C6_BLE_MODEM_RTC_INTERRUPT_STATUS as usize / 4] = 1;
            self.rtc_compare_armed = false;
            self.rtc_phase = EspC6BleRtcPhase::Pending;
        }
    }
}

/// Host-side view of the C6 BLE low-power timer interrupt.
#[derive(Clone)]
pub struct EspC6BleModemHandle {
    state: Arc<Mutex<EspC6BleModemState>>,
}

impl EspC6BleModemHandle {
    /// Returns whether the firmware-programmed BLE wake compare has expired.
    pub fn interrupt_pending(&self, at: SimTime) -> bool {
        let mut state = self.state.lock().expect("C6 BLE modem lock poisoned");
        state.materialize_timer_compares(at);
        state.registers[C6_BLE_MODEM_RTC_INTERRUPT_STATUS as usize / 4] != 0
    }
}

/// ESP32-C6 BLE modem register page with its hardware-owned sleep timer.
///
/// The controller ROM samples the counter twice until it observes an edge,
/// then programs the compare register at offset `0x58`. The counter continues
/// while the BLE MAC is clock-gated because it uses the selected LP clock.
pub struct EspC6BleModem {
    name: String,
    state: Arc<Mutex<EspC6BleModemState>>,
}

impl EspC6BleModem {
    /// Creates a reset BLE modem page and interrupt handle.
    pub fn new(name: impl Into<String>) -> (Self, EspC6BleModemHandle) {
        let state = Arc::new(Mutex::new(EspC6BleModemState {
            registers: vec![0; 0x1000 / 4],
            sync_compare_armed: false,
            rtc_compare_armed: false,
            rtc_phase: EspC6BleRtcPhase::Idle,
        }));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            EspC6BleModemHandle { state },
        )
    }
}

impl Device for EspC6BleModem {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        let index = checked_word_index(&self.name, offset, width)?;
        let mut state = self.state.lock().expect("C6 BLE modem lock poisoned");
        state.materialize_timer_compares(at);
        if offset == C6_BLE_MODEM_TIMER_CURRENT {
            return Ok(u64::from(EspC6BleModemState::timer_value(at)));
        }
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
        if offset == C6_BLE_MODEM_TIMER_CURRENT {
            return Err(DeviceError::new(
                "ESP32-C6 BLE sleep timer counter is read-only",
            ));
        }
        let value = u32::try_from(value)
            .map_err(|_| DeviceError::new("ESP32-C6 BLE modem rejects wide writes"))?;
        let mut state = self.state.lock().expect("C6 BLE modem lock poisoned");
        state.materialize_timer_compares(_at);

        if offset == C6_BLE_MODEM_TIMER_COMPARE {
            let current = EspC6BleModemState::timer_value(_at);
            let distance = value.wrapping_sub(current);
            if distance == 0 || distance >= (1_u32 << 31) {
                return Err(DeviceError::new(format!(
                    "illegal radio state [monotonic-time]: ESP32-C6 BLE sync compare {value} is not after counter {current}"
                )));
            }
        }

        if offset == C6_BLE_MODEM_RTC_COMPARE {
            if !matches!(
                state.rtc_phase,
                EspC6BleRtcPhase::Idle | EspC6BleRtcPhase::CompareProgrammed
            ) {
                return Err(DeviceError::new(format!(
                    "illegal radio state [scheduler-state]: ESP32-C6 BLE RTC compare programmed while {:?}",
                    state.rtc_phase
                )));
            }
            let current = EspC6BleModemState::timer_value(_at);
            let distance = value.wrapping_sub(current);
            if distance == 0 || distance >= (1_u32 << 31) {
                return Err(DeviceError::new(format!(
                    "illegal radio state [monotonic-time]: ESP32-C6 BLE RTC compare {value} is not after counter {current}"
                )));
            }
            state.rtc_phase = EspC6BleRtcPhase::CompareProgrammed;
        } else if offset == C6_BLE_MODEM_RTC_INTERRUPT_ENABLE
            && value & C6_BLE_MODEM_RTC_INTERRUPT_BIT != 0
        {
            if state.rtc_phase != EspC6BleRtcPhase::CompareProgrammed {
                return Err(DeviceError::new(format!(
                    "illegal radio state [scheduler-state]: ESP32-C6 BLE RTC wake enabled while {:?}",
                    state.rtc_phase
                )));
            }
            state.rtc_phase = EspC6BleRtcPhase::Armed;
        } else if offset == C6_BLE_MODEM_RTC_INTERRUPT_CLEAR
            && value & C6_BLE_MODEM_RTC_INTERRUPT_BIT != 0
        {
            if !matches!(
                state.rtc_phase,
                EspC6BleRtcPhase::Idle | EspC6BleRtcPhase::Armed | EspC6BleRtcPhase::Pending
            ) {
                return Err(DeviceError::new(format!(
                    "illegal radio state [scheduler-state]: ESP32-C6 BLE RTC wake cleared while {:?}",
                    state.rtc_phase
                )));
            }
            state.rtc_phase = EspC6BleRtcPhase::Idle;
        }

        *state.registers.get_mut(index).ok_or_else(|| {
            DeviceError::new(format!("{} write outside native page", self.name))
        })? = value;
        if offset == C6_BLE_MODEM_TIMER_COMPARE {
            state.sync_compare_armed = true;
        } else if offset == C6_BLE_MODEM_RTC_INTERRUPT_ENABLE
            && value & C6_BLE_MODEM_RTC_INTERRUPT_BIT != 0
        {
            state.rtc_compare_armed = true;
        } else if offset == C6_BLE_MODEM_RTC_INTERRUPT_CLEAR
            && value & C6_BLE_MODEM_RTC_INTERRUPT_BIT != 0
        {
            state.registers[C6_BLE_MODEM_RTC_INTERRUPT_STATUS as usize / 4] = 0;
            state.registers[C6_BLE_MODEM_RTC_TIMER0_PENDING as usize / 4] = 0;
            state.rtc_compare_armed = false;
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.lock().expect("C6 BLE modem lock poisoned");
        state.registers.fill(0);
        state.sync_compare_armed = false;
        state.rtc_compare_armed = false;
        state.rtc_phase = EspC6BleRtcPhase::Idle;
    }
}
