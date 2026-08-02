//! Shared deterministic run-control mechanics for machine implementations.

use crate::{PinStimulus, SignalStop, matching_signal_stop};
use remu_core::{RunLimits, RunStats, SimTime, StopReason};
use remu_devices::SignalHub;
use remu_trace::{TraceDigest, TraceError, TraceSink};

/// Architecture-neutral state for ordered stimuli, limits, and tracing.
///
/// CPU stepping and device polling remain owned by each target machine. This
/// controller only owns the mechanics that must have identical semantics
/// across architectures.
pub(crate) struct RunControl {
    limits: RunLimits,
    stimuli: Vec<PinStimulus>,
    next_stimulus: usize,
    /// Canonical signal digest for this run.
    pub(crate) digest: TraceDigest,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SignalEdge;
    use remu_devices::SignalHub;
    use remu_signals::{Logic, SignalValue};

    #[test]
    fn equal_timestamp_stimuli_keep_insertion_order() {
        let stimuli = [
            PinStimulus {
                at: SimTime::from_ticks(4),
                pin: 7,
                value: Logic::One,
            },
            PinStimulus {
                at: SimTime::from_ticks(4),
                pin: 8,
                value: Logic::Zero,
            },
        ];
        let mut control = RunControl::new(RunLimits::default(), &stimuli);
        let mut stats = RunStats::default();
        let mut applied = Vec::new();
        control
            .apply_stimuli(SimTime::from_ticks(4), &mut stats, |stimulus| {
                applied.push(stimulus.pin);
                Ok::<_, ()>(())
            })
            .unwrap();
        assert_eq!(applied, [7, 8]);
        assert_eq!(stats.events, 2);
    }

    #[test]
    fn instruction_and_time_budgets_share_stop_semantics() {
        let control = RunControl::new(
            RunLimits {
                instructions: Some(3),
                deadline: Some(SimTime::from_ticks(10)),
            },
            &[],
        );
        let mut stats = RunStats {
            instructions: 2,
            time: SimTime::from_ticks(10),
            events: 0,
        };
        assert_eq!(
            control.limit_reason(stats.time, &stats),
            Some(StopReason::TimeLimit)
        );
        stats.time = SimTime::from_ticks(9);
        stats.instructions = 3;
        assert_eq!(
            control.limit_reason(stats.time, &stats),
            Some(StopReason::InstructionLimit)
        );
    }

    #[test]
    fn future_stimuli_wait_until_their_timestamp() {
        let stimuli = [
            PinStimulus {
                at: SimTime::from_ticks(4),
                pin: 4,
                value: Logic::One,
            },
            PinStimulus {
                at: SimTime::from_ticks(8),
                pin: 8,
                value: Logic::Zero,
            },
        ];
        let mut control = RunControl::new(RunLimits::default(), &stimuli);
        let mut stats = RunStats::default();
        let mut applied = Vec::new();
        control
            .apply_stimuli(SimTime::from_ticks(4), &mut stats, |stimulus| {
                applied.push(stimulus.pin);
                Ok::<_, ()>(())
            })
            .unwrap();
        assert_eq!(applied, [4]);
        assert_eq!(stats.events, 1);
        control
            .apply_stimuli(SimTime::from_ticks(7), &mut stats, |stimulus| {
                applied.push(stimulus.pin);
                Ok::<_, ()>(())
            })
            .unwrap();
        assert_eq!(applied, [4]);
        control
            .apply_stimuli(SimTime::from_ticks(8), &mut stats, |stimulus| {
                applied.push(stimulus.pin);
                Ok::<_, ()>(())
            })
            .unwrap();
        assert_eq!(applied, [4, 8]);
        assert_eq!(stats.events, 2);
    }

    #[test]
    fn failed_stimulus_application_is_retryable_without_losing_the_event() {
        let stimulus = [PinStimulus {
            at: SimTime::ZERO,
            pin: 3,
            value: Logic::One,
        }];
        let mut control = RunControl::new(RunLimits::default(), &stimulus);
        let mut stats = RunStats::default();
        assert_eq!(
            control.apply_stimuli(SimTime::ZERO, &mut stats, |_stimulus| {
                Err::<(), _>("host input rejected")
            }),
            Err("host input rejected")
        );
        assert_eq!(stats.events, 0);
        let mut applied = Vec::new();
        control
            .apply_stimuli(SimTime::ZERO, &mut stats, |stimulus| {
                applied.push(stimulus.pin);
                Ok::<_, ()>(())
            })
            .unwrap();
        assert_eq!(applied, [3]);
        assert_eq!(stats.events, 1);
    }

    #[test]
    fn signal_recording_returns_the_first_matching_stop_and_drains_all_changes() {
        let signals = SignalHub::new();
        let signal = signals
            .declare(
                "board.led",
                SignalValue::repeat(Logic::Zero, 1).unwrap(),
                None,
            )
            .unwrap();
        let mut control = RunControl::new(RunLimits::default(), &[]);
        let mut trace: Option<&mut dyn TraceSink> = None;
        control.begin_trace(&signals, &mut trace).unwrap();
        signals
            .set(
                signal,
                SignalValue::repeat(Logic::One, 1).unwrap(),
                SimTime::from_ticks(1),
            )
            .unwrap();
        let stops = [SignalStop {
            signal,
            path: "board.led".to_owned(),
            edge: SignalEdge::Rising,
        }];
        assert_eq!(
            control
                .record_signals(&signals, &stops, &mut trace)
                .unwrap(),
            Some("board.led".to_owned())
        );
        assert!(signals.drain_changes().is_empty());
    }
}

impl RunControl {
    /// Creates a controller with timestamp-ordered, stable stimuli.
    pub(crate) fn new(limits: RunLimits, stimuli: &[PinStimulus]) -> Self {
        let mut stimuli = stimuli.to_vec();
        stimuli.sort_by_key(|stimulus| stimulus.at);
        Self {
            limits,
            stimuli,
            next_stimulus: 0,
            digest: TraceDigest::new(),
        }
    }

    /// Begins the canonical digest and optional trace sink.
    pub(crate) fn begin_trace(
        &mut self,
        signals: &SignalHub,
        trace: &mut Option<&mut dyn TraceSink>,
    ) -> Result<(), TraceError> {
        signals.with_registry(|registry| {
            self.digest.begin(registry);
            trace
                .as_deref_mut()
                .map_or(Ok(()), |sink| sink.begin(registry))
        })
    }

    /// Applies every stimulus due at `now` and counts it as a run event.
    pub(crate) fn apply_stimuli<E>(
        &mut self,
        now: SimTime,
        stats: &mut RunStats,
        mut set_pin: impl FnMut(PinStimulus) -> Result<(), E>,
    ) -> Result<(), E> {
        while self
            .stimuli
            .get(self.next_stimulus)
            .is_some_and(|stimulus| stimulus.at <= now)
        {
            let stimulus = self.stimuli[self.next_stimulus];
            set_pin(stimulus)?;
            stats.events = stats.events.saturating_add(1);
            self.next_stimulus += 1;
        }
        Ok(())
    }

    /// Returns the shared instruction/time stop, if its budget is exhausted.
    pub(crate) fn limit_reason(&self, now: SimTime, stats: &RunStats) -> Option<StopReason> {
        if self
            .limits
            .instructions
            .is_some_and(|limit| stats.instructions >= limit)
        {
            Some(StopReason::InstructionLimit)
        } else if self.limits.deadline.is_some_and(|deadline| now >= deadline) {
            Some(StopReason::TimeLimit)
        } else {
            None
        }
    }

    /// Streams pending signal changes and returns the first matching signal stop.
    pub(crate) fn record_signals(
        &mut self,
        signals: &SignalHub,
        stops: &[SignalStop],
        trace: &mut Option<&mut dyn TraceSink>,
    ) -> Result<Option<String>, TraceError> {
        let mut signal_stop = None;
        for change in signals.drain_changes() {
            signal_stop = signal_stop.or_else(|| matching_signal_stop(&change, stops));
            self.digest.change(&change);
            if let Some(sink) = trace.as_deref_mut() {
                sink.change(&change)?;
            }
        }
        Ok(signal_stop)
    }
}
