use crate::{EventId, EventQueue, QueueError, ScheduledEvent, SimDuration, SimTime};
use thiserror::Error;

/// Error returned by the monotonic simulation scheduler.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum SchedulerError {
    /// Scheduling relative work failed because the timestamp or event sequence
    /// could not be represented.
    #[error(transparent)]
    Queue(#[from] QueueError),
    /// Simulation time cannot move backwards.
    #[error("scheduler time cannot move backwards from {current} to {requested}")]
    Rewind {
        /// Current scheduler timestamp.
        current: SimTime,
        /// Requested earlier timestamp.
        requested: SimTime,
    },
}

/// Deterministic event dispatcher built on [`EventQueue`].
///
/// A scheduler owns the simulation timestamp and only dispatches events whose
/// timestamp is at or before that time. Consumers can advance to the next
/// device wakeup without stepping every CPU instruction, then drain due work
/// in stable timestamp-plus-insertion order. Device-specific payloads remain
/// outside this crate.
#[derive(Debug)]
pub struct Scheduler<T>
where
    T: Eq,
{
    now: SimTime,
    queue: EventQueue<T>,
    dispatched: u64,
}

impl<T> Scheduler<T>
where
    T: Eq,
{
    /// Creates an empty scheduler at `start`.
    pub fn new(start: SimTime) -> Self {
        Self {
            now: start,
            queue: EventQueue::new(),
            dispatched: 0,
        }
    }

    /// Returns the current simulation timestamp.
    pub const fn now(&self) -> SimTime {
        self.now
    }

    /// Returns the number of events removed for dispatch.
    pub const fn dispatched(&self) -> u64 {
        self.dispatched
    }

    /// Returns the next non-cancelled event timestamp, if any.
    pub fn next_wakeup(&mut self) -> Option<SimTime> {
        self.queue.next_time()
    }

    /// Schedules a payload at an absolute timestamp.
    pub fn schedule_at(&mut self, at: SimTime, payload: T) -> Result<EventId, SchedulerError> {
        self.queue.schedule_at(at, payload).map_err(Into::into)
    }

    /// Schedules a payload relative to the current scheduler timestamp.
    pub fn schedule_after(
        &mut self,
        delay: SimDuration,
        payload: T,
    ) -> Result<EventId, SchedulerError> {
        self.queue
            .schedule_after(self.now, delay, payload)
            .map_err(Into::into)
    }

    /// Cancels a queued event. Unknown or already-dispatched IDs are harmless.
    pub fn cancel(&mut self, id: EventId) -> bool {
        self.queue.cancel(id)
    }

    /// Advances the scheduler timestamp without dispatching work.
    pub fn advance_to(&mut self, at: SimTime) -> Result<(), SchedulerError> {
        if at < self.now {
            return Err(SchedulerError::Rewind {
                current: self.now,
                requested: at,
            });
        }
        self.now = at;
        Ok(())
    }

    /// Advances to the next queued event, if one exists.
    pub fn advance_to_next(&mut self) -> Result<Option<SimTime>, SchedulerError> {
        let Some(at) = self.next_wakeup() else {
            return Ok(None);
        };
        self.advance_to(at)?;
        Ok(Some(at))
    }

    /// Removes one event that is due at the current timestamp.
    pub fn pop_due(&mut self) -> Option<ScheduledEvent<T>> {
        if self.next_wakeup().is_some_and(|at| at <= self.now) {
            let event = self
                .queue
                .pop()
                .expect("next_wakeup reports a queued event");
            self.dispatched = self.dispatched.saturating_add(1);
            Some(event)
        } else {
            None
        }
    }

    /// Dispatches every event due at the current timestamp.
    ///
    /// The callback receives the complete scheduled event, including its
    /// timestamp and stable ID. If the callback returns an error, later due
    /// events remain queued for a subsequent call.
    pub fn dispatch_due<F, E>(&mut self, mut callback: F) -> Result<u64, E>
    where
        F: FnMut(ScheduledEvent<T>) -> Result<(), E>,
    {
        let mut dispatched = 0;
        while let Some(event) = self.pop_due() {
            callback(event)?;
            dispatched += 1;
        }
        Ok(dispatched)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatches_due_events_in_stable_order() {
        let mut scheduler = Scheduler::new(SimTime::ZERO);
        scheduler
            .schedule_at(SimTime::from_ticks(5), "first")
            .unwrap();
        scheduler
            .schedule_at(SimTime::from_ticks(5), "second")
            .unwrap();
        scheduler
            .schedule_at(SimTime::from_ticks(7), "future")
            .unwrap();

        scheduler.advance_to(SimTime::from_ticks(5)).unwrap();
        let mut seen = Vec::new();
        assert_eq!(
            scheduler
                .dispatch_due(|event| {
                    seen.push(event.payload);
                    Ok::<(), ()>(())
                })
                .unwrap(),
            2
        );
        assert_eq!(seen, ["first", "second"]);
        assert_eq!(scheduler.next_wakeup(), Some(SimTime::from_ticks(7)));
        assert_eq!(scheduler.dispatched(), 2);
    }

    #[test]
    fn advances_to_next_event_and_respects_cancellation() {
        let mut scheduler = Scheduler::new(SimTime::from_ticks(10));
        let cancelled = scheduler
            .schedule_after(SimDuration::TICK, "cancelled")
            .unwrap();
        scheduler
            .schedule_after(SimDuration::from_ticks(3), "kept")
            .unwrap();
        assert!(scheduler.cancel(cancelled));

        assert_eq!(
            scheduler.advance_to_next().unwrap(),
            Some(SimTime::from_ticks(13))
        );
        assert_eq!(scheduler.pop_due().unwrap().payload, "kept");
        assert_eq!(scheduler.next_wakeup(), None);
    }

    #[test]
    fn rejects_time_rewind() {
        let mut scheduler = Scheduler::<u8>::new(SimTime::from_ticks(4));
        assert_eq!(
            scheduler.advance_to(SimTime::from_ticks(3)),
            Err(SchedulerError::Rewind {
                current: SimTime::from_ticks(4),
                requested: SimTime::from_ticks(3),
            })
        );
    }
}
