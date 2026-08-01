use std::collections::BTreeMap;

#[derive(Debug)]
pub(super) struct EspFunctionalHeap {
    pub(super) free: BTreeMap<u32, u32>,
    pub(super) allocations: BTreeMap<u32, u32>,
    pub(super) minimum_free: u32,
}

impl EspFunctionalHeap {
    pub(super) fn new(start: u32, size: u32) -> Option<Self> {
        const METADATA_RESERVE: u32 = 64;
        if size <= METADATA_RESERVE {
            return None;
        }
        let first = start.checked_add(METADATA_RESERVE)?;
        let available = size - METADATA_RESERVE;
        Some(Self {
            free: BTreeMap::from([(first, available)]),
            allocations: BTreeMap::new(),
            minimum_free: available,
        })
    }

    pub(super) fn free_bytes(&self) -> u32 {
        self.free.values().copied().sum()
    }

    pub(super) fn allocate(&mut self, size: u32, alignment: u32, offset: u32) -> Option<u32> {
        let size = size.max(1).checked_add(3)? & !3;
        let alignment = alignment.max(4);
        if !alignment.is_power_of_two() {
            return None;
        }
        let selected = self.free.iter().find_map(|(&start, &length)| {
            let adjusted = start.checked_add(offset)?;
            let aligned = adjusted.checked_add(alignment - 1)? & !(alignment - 1);
            let allocation = aligned.checked_sub(offset)?;
            let end = allocation.checked_add(size)?;
            (allocation >= start && end <= start.checked_add(length)?)
                .then_some((start, length, allocation, end))
        })?;
        let (free_start, free_length, allocation, allocation_end) = selected;
        self.free.remove(&free_start);
        if allocation > free_start {
            self.free.insert(free_start, allocation - free_start);
        }
        let free_end = free_start + free_length;
        if allocation_end < free_end {
            self.free.insert(allocation_end, free_end - allocation_end);
        }
        self.allocations.insert(allocation, size);
        self.minimum_free = self.minimum_free.min(self.free_bytes());
        Some(allocation)
    }

    pub(super) fn release(&mut self, pointer: u32) -> bool {
        let Some(size) = self.allocations.remove(&pointer) else {
            return false;
        };
        self.free.insert(pointer, size);
        let ranges = std::mem::take(&mut self.free);
        for (start, length) in ranges {
            if let Some((&previous_start, &previous_length)) = self.free.last_key_value()
                && previous_start + previous_length == start
            {
                self.free.insert(previous_start, previous_length + length);
                continue;
            }
            self.free.insert(start, length);
        }
        true
    }
}
