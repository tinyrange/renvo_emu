use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use std::sync::{Arc, Mutex};

const POWER_CONTROL: u64 = 0x00;
const SLEEP_DURATION: u64 = 0x04;
const SLEEP_ELAPSED: u64 = 0x08;
const RF_SLEEP_COMMAND: u32 = 0x47;
const RF_SLEEP_ACKNOWLEDGED: u32 = 1 << 15;
const WAKE_REQUEST: u32 = 1 << 4;
const WAKE_COMPLETE_REQUEST: u32 = 1 << 3;
// The ROM acknowledges the wake-start interrupt 22 architectural ticks after
// asserting WAKE_COMPLETE. Publish the hardware completion after that ISR has
// cleared its prior causes so the distinct wake-end edge cannot be coalesced.
const WAKE_COMPLETION_DELAY_TICKS: u64 = 64;
// Genuine ESP-IDF divides the 40 MHz main XTAL down to the controller's
// 1 MHz low-power clock. Renvo's S3 simulation timebase is 16 MHz.
const SIM_TICKS_PER_LP_CYCLE: u64 = 16;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Esp32S3BleSleepPhase {
    #[default]
    Awake,
    DurationProgrammed,
    Sleeping,
    WakeStarted,
    WakeCompletionPending,
}

struct Esp32S3BleLpClockState {
    registers: Vec<u32>,
    sleep_started_at: Option<u64>,
    wake_requested: bool,
    wake_completion_due: Option<u64>,
    phase: Esp32S3BleSleepPhase,
}

/// Machine-side view of the S3 BLE low-power wake request.
#[derive(Clone)]
pub struct Esp32S3BleLpClockHandle {
    state: Arc<Mutex<Esp32S3BleLpClockState>>,
}

impl Esp32S3BleLpClockHandle {
    /// Consumes one hardware wake request asserted by controller firmware.
    pub fn take_wake_request(&self) -> bool {
        let mut state = self
            .state
            .lock()
            .expect("ESP32-S3 BLE LP clock lock poisoned");
        let requested = state.wake_requested;
        state.wake_requested = false;
        if requested {
            // WAKE is a command strobe consumed by the low-power sequencer.
            state.registers[POWER_CONTROL as usize / 4] &= !WAKE_REQUEST;
        }
        requested
    }

    /// Consumes the controller's second-stage wake-completion request.
    pub fn take_wake_completion_request(&self, at: SimTime) -> bool {
        let mut state = self
            .state
            .lock()
            .expect("ESP32-S3 BLE LP clock lock poisoned");
        let requested = state
            .wake_completion_due
            .is_some_and(|due| at.ticks() >= due);
        if requested {
            state.wake_completion_due = None;
            // The wake-complete transition consumes all sleep-sequencer
            // command and status bits before the next independent sleep.
            state.registers[POWER_CONTROL as usize / 4] &=
                !(RF_SLEEP_COMMAND | RF_SLEEP_ACKNOWLEDGED | WAKE_REQUEST | WAKE_COMPLETE_REQUEST);
            state.sleep_started_at = None;
            state.phase = Esp32S3BleSleepPhase::Awake;
        }
        requested
    }
}

/// ESP32-S3 BLE controller low-power clock and sleep-state register page.
///
/// Genuine mode-1 controller firmware programs this page at `0x6004_2000`;
/// it is distinct from the public SAR ADC page ending immediately below it.
pub struct Esp32S3BleLpClock {
    name: String,
    state: Arc<Mutex<Esp32S3BleLpClockState>>,
}

impl Esp32S3BleLpClock {
    /// Creates a reset BLE low-power clock page.
    pub fn new(name: impl Into<String>) -> (Self, Esp32S3BleLpClockHandle) {
        let state = Arc::new(Mutex::new(Esp32S3BleLpClockState {
            registers: vec![0; 0x1000 / 4],
            sleep_started_at: None,
            wake_requested: false,
            wake_completion_due: None,
            phase: Esp32S3BleSleepPhase::Awake,
        }));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Esp32S3BleLpClockHandle { state },
        )
    }

    fn index(&self, offset: u64, width: AccessWidth) -> Result<usize, DeviceError> {
        if width != AccessWidth::Word || !offset.is_multiple_of(4) || offset >= 0x1000 {
            return Err(DeviceError::new(format!(
                "{} requires an aligned word access within its native page",
                self.name
            )));
        }
        Ok(offset as usize / 4)
    }
}

impl Device for Esp32S3BleLpClock {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let index = self.index(offset, width)?;
        let state = self
            .state
            .lock()
            .expect("ESP32-S3 BLE LP clock lock poisoned");
        if offset == SLEEP_ELAPSED {
            let elapsed = state
                .sleep_started_at
                .map_or(0, |started| _at.ticks().saturating_sub(started))
                / SIM_TICKS_PER_LP_CYCLE;
            return Ok(elapsed & u64::from(u32::MAX));
        }
        Ok(u64::from(state.registers[index]))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        let index = self.index(offset, width)?;
        let value = u32::try_from(value)
            .map_err(|_| DeviceError::new("ESP32-S3 BLE LP clock rejects wide writes"))?;
        if offset == SLEEP_ELAPSED {
            return Err(DeviceError::new(
                "ESP32-S3 BLE low-power elapsed counter is read-only",
            ));
        }
        let mut state = self
            .state
            .lock()
            .expect("ESP32-S3 BLE LP clock lock poisoned");
        let prior = state.registers[index];

        if offset == SLEEP_DURATION {
            match state.phase {
                Esp32S3BleSleepPhase::Awake | Esp32S3BleSleepPhase::DurationProgrammed => {
                    state.phase = Esp32S3BleSleepPhase::DurationProgrammed;
                }
                phase => {
                    return Err(DeviceError::new(format!(
                        "illegal radio state [scheduler-state]: ESP32-S3 BLE sleep duration programmed while {phase:?}"
                    )));
                }
            }
        }

        let rf_sleep_started = offset == POWER_CONTROL
            && value & RF_SLEEP_COMMAND == RF_SLEEP_COMMAND
            && prior & RF_SLEEP_COMMAND != RF_SLEEP_COMMAND;
        if rf_sleep_started {
            if state.phase != Esp32S3BleSleepPhase::DurationProgrammed || value & (1 << 31) == 0 {
                return Err(DeviceError::new(format!(
                    "illegal radio state [scheduler-state]: ESP32-S3 RF sleep requested while {:?}",
                    state.phase
                )));
            }
            state.phase = Esp32S3BleSleepPhase::Sleeping;
        }

        let wake_started =
            offset == POWER_CONTROL && value & WAKE_REQUEST != 0 && prior & WAKE_REQUEST == 0;
        if wake_started {
            if state.phase != Esp32S3BleSleepPhase::Sleeping || prior & RF_SLEEP_ACKNOWLEDGED == 0 {
                return Err(DeviceError::new(format!(
                    "illegal radio state [scheduler-state]: ESP32-S3 BLE wake requested while {:?}",
                    state.phase
                )));
            }
            state.phase = Esp32S3BleSleepPhase::WakeStarted;
        }

        let wake_completion_started = offset == POWER_CONTROL
            && value & WAKE_COMPLETE_REQUEST != 0
            && prior & WAKE_COMPLETE_REQUEST == 0;
        if wake_completion_started {
            if state.phase != Esp32S3BleSleepPhase::WakeStarted {
                return Err(DeviceError::new(format!(
                    "illegal radio state [scheduler-state]: ESP32-S3 BLE wake completion requested while {:?}",
                    state.phase
                )));
            }
            state.phase = Esp32S3BleSleepPhase::WakeCompletionPending;
        }

        state.registers[index] = value;
        // The genuine S3 ROM's r_rf_sleep() asserts bits 0, 1, 2, and 6,
        // after which rw_schedule() waits for the hardware-owned bit 15.
        // Acknowledgement is architecturally visible by the first subsequent
        // register read; analogue transition latency is outside this model.
        if rf_sleep_started {
            state.registers[index] |= RF_SLEEP_ACKNOWLEDGED;
            state.sleep_started_at = Some(_at.ticks());
        }
        if wake_started {
            state.wake_requested = true;
        }
        if wake_completion_started {
            state.wake_completion_due =
                Some(_at.ticks().saturating_add(WAKE_COMPLETION_DELAY_TICKS));
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self
            .state
            .lock()
            .expect("ESP32-S3 BLE LP clock lock poisoned");
        state.registers.fill(0);
        state.sleep_started_at = None;
        state.wake_requested = false;
        state.wake_completion_due = None;
        state.phase = Esp32S3BleSleepPhase::Awake;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retains_controller_programmed_clock_and_timing_words() {
        let (mut clock, _) = Esp32S3BleLpClock::new("ble-lp-clock");
        clock
            .write(0x0c, AccessWidth::Word, 0x1234_5678, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            clock.read(0x0c, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0x1234_5678
        );
        assert!(clock.read(2, AccessWidth::HalfWord, SimTime::ZERO).is_err());
    }

    #[test]
    fn acknowledges_the_genuine_rom_rf_sleep_command() {
        let (mut clock, handle) = Esp32S3BleLpClock::new("ble-lp-clock");
        clock
            .write(SLEEP_DURATION, AccessWidth::Word, 20, SimTime::ZERO)
            .unwrap();
        clock
            .write(
                POWER_CONTROL,
                AccessWidth::Word,
                u64::from(0x8000_0000_u32),
                SimTime::ZERO,
            )
            .unwrap();
        clock
            .write(
                POWER_CONTROL,
                AccessWidth::Word,
                u64::from(0x8000_0000 | RF_SLEEP_COMMAND),
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            clock
                .read(POWER_CONTROL, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            u64::from(0x8000_0000 | RF_SLEEP_ACKNOWLEDGED | RF_SLEEP_COMMAND)
        );
        assert_eq!(
            clock
                .read(SLEEP_ELAPSED, AccessWidth::Word, SimTime::from_ticks(160))
                .unwrap(),
            10
        );
        clock
            .write(
                POWER_CONTROL,
                AccessWidth::Word,
                u64::from(0x8000_0000 | RF_SLEEP_ACKNOWLEDGED | WAKE_REQUEST),
                SimTime::from_ticks(160),
            )
            .unwrap();
        assert!(handle.take_wake_request());
        assert!(!handle.take_wake_request());
        assert_eq!(
            clock
                .read(POWER_CONTROL, AccessWidth::Word, SimTime::from_ticks(160))
                .unwrap()
                & u64::from(WAKE_REQUEST),
            0
        );
        clock
            .write(
                POWER_CONTROL,
                AccessWidth::Word,
                u64::from(0x8000_0000 | RF_SLEEP_ACKNOWLEDGED | WAKE_COMPLETE_REQUEST),
                SimTime::from_ticks(160),
            )
            .unwrap();
        assert!(!handle.take_wake_completion_request(SimTime::from_ticks(223)));
        assert!(handle.take_wake_completion_request(SimTime::from_ticks(224)));
        assert!(!handle.take_wake_completion_request(SimTime::from_ticks(224)));
        assert_eq!(
            clock
                .read(POWER_CONTROL, AccessWidth::Word, SimTime::from_ticks(160))
                .unwrap(),
            0x8000_0000
        );
        assert!(
            clock
                .write(
                    SLEEP_ELAPSED,
                    AccessWidth::Word,
                    1,
                    SimTime::from_ticks(160)
                )
                .is_err()
        );
    }

    #[test]
    fn rejects_impossible_sleep_and_wake_ordering() {
        let (mut clock, _) = Esp32S3BleLpClock::new("ble-lp-clock");
        let sleep_error = clock
            .write(
                POWER_CONTROL,
                AccessWidth::Word,
                u64::from(0x8000_0000 | RF_SLEEP_COMMAND),
                SimTime::ZERO,
            )
            .unwrap_err();
        assert!(sleep_error.to_string().contains("illegal radio state"));

        clock
            .write(SLEEP_DURATION, AccessWidth::Word, 20, SimTime::ZERO)
            .unwrap();
        let wake_error = clock
            .write(
                POWER_CONTROL,
                AccessWidth::Word,
                u64::from(WAKE_REQUEST),
                SimTime::ZERO,
            )
            .unwrap_err();
        assert!(wake_error.to_string().contains("illegal radio state"));
    }

    #[test]
    fn stress_replays_sleep_wake_and_reset_cancellation_without_stale_edges() {
        fn run() -> Vec<(u64, bool)> {
            let (mut clock, handle) = Esp32S3BleLpClock::new("ble-lp-clock");
            let mut evidence = Vec::new();

            for cycle in 0..256_u64 {
                let start = SimTime::from_ticks(cycle * 512);
                clock
                    .write(SLEEP_DURATION, AccessWidth::Word, 20 + cycle % 5, start)
                    .unwrap();
                clock
                    .write(
                        POWER_CONTROL,
                        AccessWidth::Word,
                        u64::from(0x8000_0000_u32),
                        start,
                    )
                    .unwrap();
                clock
                    .write(
                        POWER_CONTROL,
                        AccessWidth::Word,
                        u64::from(0x8000_0000 | RF_SLEEP_COMMAND),
                        start,
                    )
                    .unwrap();

                if cycle % 11 == 0 {
                    clock.reset(ResetKind::Software);
                    assert!(!handle.take_wake_request());
                    assert!(
                        !handle.take_wake_completion_request(SimTime::from_ticks(
                            start.ticks() + 256,
                        ))
                    );
                    evidence.push((cycle, true));
                    continue;
                }

                let wake = SimTime::from_ticks(start.ticks() + 160);
                assert_eq!(
                    clock.read(SLEEP_ELAPSED, AccessWidth::Word, wake).unwrap(),
                    10
                );
                clock
                    .write(
                        POWER_CONTROL,
                        AccessWidth::Word,
                        u64::from(0x8000_0000 | RF_SLEEP_ACKNOWLEDGED | WAKE_REQUEST),
                        wake,
                    )
                    .unwrap();
                assert!(handle.take_wake_request());
                assert!(!handle.take_wake_request());

                let duplicate = clock
                    .write(
                        POWER_CONTROL,
                        AccessWidth::Word,
                        u64::from(0x8000_0000 | RF_SLEEP_ACKNOWLEDGED | WAKE_REQUEST),
                        wake,
                    )
                    .unwrap_err();
                assert!(duplicate.to_string().contains("illegal radio state"));

                clock
                    .write(
                        POWER_CONTROL,
                        AccessWidth::Word,
                        u64::from(0x8000_0000 | RF_SLEEP_ACKNOWLEDGED | WAKE_COMPLETE_REQUEST),
                        wake,
                    )
                    .unwrap();
                let completion = SimTime::from_ticks(wake.ticks() + WAKE_COMPLETION_DELAY_TICKS);
                assert!(
                    !handle
                        .take_wake_completion_request(SimTime::from_ticks(completion.ticks() - 1,))
                );
                assert!(handle.take_wake_completion_request(completion));
                assert!(!handle.take_wake_completion_request(completion));
                evidence.push((cycle, false));
            }

            evidence
        }

        let first = run();
        assert_eq!(first, run());
        assert_eq!(first.len(), 256);
        assert_eq!(first.iter().filter(|(_, reset)| *reset).count(), 24);
    }
}
