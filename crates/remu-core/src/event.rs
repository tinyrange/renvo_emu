use crate::{SimDuration, SimTime, TimeError};
use core::cmp::Ordering;
use std::collections::{BTreeSet, BinaryHeap};
use thiserror::Error;

/// Stable identifier assigned to a scheduled event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EventId(u64);

impl EventId {
    /// Returns the stable integer representation.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// An event removed from the deterministic queue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScheduledEvent<T> {
    /// Scheduled simulation timestamp.
    pub at: SimTime,
    /// Stable event identifier.
    pub id: EventId,
    /// User event payload.
    pub payload: T,
    sequence: u64,
}

impl<T> PartialOrd for ScheduledEvent<T>
where
    T: Eq,
{
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Ord for ScheduledEvent<T>
where
    T: Eq,
{
    fn cmp(&self, other: &Self) -> Ordering {
        // BinaryHeap is a max-heap. Reverse both keys so the earliest event and
        // then the earliest insertion sequence are popped first.
        other
            .at
            .cmp(&self.at)
            .then_with(|| other.sequence.cmp(&self.sequence))
    }
}

/// Event queue scheduling error.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum QueueError {
    /// Simulation timestamp arithmetic overflowed.
    #[error(transparent)]
    Time(#[from] TimeError),
    /// No more stable event identifiers can be allocated.
    #[error("event sequence exhausted")]
    SequenceExhausted,
}

/// A deterministic priority queue ordered by timestamp and insertion sequence.
#[derive(Debug)]
pub struct EventQueue<T>
where
    T: Eq,
{
    heap: BinaryHeap<ScheduledEvent<T>>,
    active: BTreeSet<EventId>,
    cancelled: BTreeSet<EventId>,
    next_sequence: u64,
}

impl<T> Default for EventQueue<T>
where
    T: Eq,
{
    fn default() -> Self {
        Self {
            heap: BinaryHeap::new(),
            active: BTreeSet::new(),
            cancelled: BTreeSet::new(),
            next_sequence: 0,
        }
    }
}

impl<T> EventQueue<T>
where
    T: Eq,
{
    /// Creates an empty event queue.
    pub fn new() -> Self {
        Self::default()
    }

    /// Schedules a payload at an absolute simulation timestamp.
    pub fn schedule_at(&mut self, at: SimTime, payload: T) -> Result<EventId, QueueError> {
        let sequence = self.next_sequence;
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or(QueueError::SequenceExhausted)?;
        let id = EventId(sequence);
        self.heap.push(ScheduledEvent {
            at,
            id,
            payload,
            sequence,
        });
        self.active.insert(id);
        Ok(id)
    }

    /// Schedules a payload relative to `now`.
    pub fn schedule_after(
        &mut self,
        now: SimTime,
        delay: SimDuration,
        payload: T,
    ) -> Result<EventId, QueueError> {
        self.schedule_at(now.checked_add(delay)?, payload)
    }

    /// Marks an event as cancelled.
    ///
    /// Returns false when the event is unknown, already cancelled, or already
    /// popped. Cancelling an ID owned by another queue is harmless.
    pub fn cancel(&mut self, id: EventId) -> bool {
        if !self.active.remove(&id) {
            return false;
        }
        self.cancelled.insert(id)
    }

    /// Removes and returns the next non-cancelled event.
    pub fn pop(&mut self) -> Option<ScheduledEvent<T>> {
        while let Some(event) = self.heap.pop() {
            if self.cancelled.remove(&event.id) {
                continue;
            }
            self.active.remove(&event.id);
            return Some(event);
        }
        None
    }

    /// Returns the timestamp of the next non-cancelled event.
    pub fn next_time(&mut self) -> Option<SimTime> {
        while let Some(event) = self.heap.peek() {
            if self.cancelled.contains(&event.id) {
                let id = event.id;
                self.heap.pop();
                self.cancelled.remove(&id);
            } else {
                return Some(event.at);
            }
        }
        None
    }

    /// Returns true if no non-cancelled events remain.
    pub fn is_empty(&mut self) -> bool {
        self.next_time().is_none()
    }

    /// Number of stored entries, including lazily cancelled entries.
    pub fn stored_len(&self) -> usize {
        self.heap.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_time_events_preserve_insertion_order() {
        let mut queue = EventQueue::new();
        queue.schedule_at(SimTime::from_ticks(7), "first").unwrap();
        queue.schedule_at(SimTime::from_ticks(7), "second").unwrap();
        queue
            .schedule_at(SimTime::from_ticks(6), "earlier")
            .unwrap();

        assert_eq!(queue.pop().unwrap().payload, "earlier");
        assert_eq!(queue.pop().unwrap().payload, "first");
        assert_eq!(queue.pop().unwrap().payload, "second");
    }

    #[test]
    fn cancellation_is_lazy_and_deterministic() {
        let mut queue = EventQueue::new();
        let cancelled = queue
            .schedule_at(SimTime::from_ticks(1), "cancelled")
            .unwrap();
        queue.schedule_at(SimTime::from_ticks(2), "kept").unwrap();
        assert!(queue.cancel(cancelled));
        assert_eq!(queue.next_time(), Some(SimTime::from_ticks(2)));
        assert_eq!(queue.pop().unwrap().payload, "kept");
        assert!(queue.is_empty());
    }

    #[test]
    fn cancelling_an_unknown_id_does_not_cancel_a_future_event() {
        let mut queue = EventQueue::new();
        let mut other_queue = EventQueue::new();
        let foreign_id = other_queue
            .schedule_at(SimTime::from_ticks(1), "foreign")
            .unwrap();

        assert!(!queue.cancel(foreign_id));
        queue.schedule_at(SimTime::from_ticks(1), "kept").unwrap();
        assert_eq!(queue.pop().unwrap().payload, "kept");
    }
}
