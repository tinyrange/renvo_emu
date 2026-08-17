//! Deterministic memory maps and MMIO routing.

use remu_core::{AccessKind, AccessWidth, Bus, BusFault, BusFaultKind, ResetKind, SimTime};
use serde::Serialize;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::rc::Rc;
use thiserror::Error;

/// Byte ordering used by a mapped address space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Endianness {
    /// Least-significant byte at the lowest address.
    #[default]
    Little,
    /// Most-significant byte at the lowest address.
    Big,
}

/// Access permissions assigned to a region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Permissions {
    /// Data reads are permitted.
    pub read: bool,
    /// Data writes are permitted.
    pub write: bool,
    /// Instruction fetches are permitted.
    pub execute: bool,
}

impl Permissions {
    /// Read, write, and execute.
    pub const RWX: Self = Self {
        read: true,
        write: true,
        execute: true,
    };

    /// Read and write, but no execute.
    pub const RW: Self = Self {
        read: true,
        write: true,
        execute: false,
    };

    /// Read and execute, but no write.
    pub const RX: Self = Self {
        read: true,
        write: false,
        execute: true,
    };

    /// Read-only data.
    pub const RO: Self = Self {
        read: true,
        write: false,
        execute: false,
    };

    fn permits(self, access: AccessKind) -> bool {
        match access {
            AccessKind::Execute => self.execute,
            AccessKind::Read => self.read,
            AccessKind::Write => self.write,
        }
    }
}

/// Error produced while constructing an address space.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum MapError {
    /// A mapped region must contain at least one byte.
    #[error("region {name:?} has zero size")]
    Empty {
        /// Region name.
        name: String,
    },
    /// End address cannot be represented.
    #[error("region {name:?} address range overflows")]
    AddressOverflow {
        /// Region name.
        name: String,
    },
    /// Two mappings overlap.
    #[error(
        "region {name:?} at {start:#x}..{end:#x} overlaps {existing:?} at {existing_start:#x}..{existing_end:#x}"
    )]
    Overlap {
        /// New region name.
        name: String,
        /// New region start.
        start: u64,
        /// New region end, exclusive.
        end: u64,
        /// Existing region name.
        existing: String,
        /// Existing region start.
        existing_start: u64,
        /// Existing region end, exclusive.
        existing_end: u64,
    },
    /// Alias exceeds its shared backing store.
    #[error("region {name:?} exceeds shared memory backing")]
    BackingRange {
        /// Region name.
        name: String,
    },
}

/// Error returned by a memory-mapped device.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct DeviceError {
    /// Human-readable device diagnostic.
    pub message: String,
}

impl DeviceError {
    /// Creates a device error.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Memory-mapped peripheral contract.
pub trait Device {
    /// Stable diagnostic name.
    fn name(&self) -> &str;

    /// Reads a register at an offset relative to the mapping base.
    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError>;

    /// Writes a register at an offset relative to the mapping base.
    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError>;

    /// Returns a side-effect-free register value for trace correlation.
    ///
    /// Devices opt in only when their internal model can safely expose the
    /// value. The address space never substitutes a call to [`Device::read`],
    /// because reads may acknowledge or otherwise mutate hardware state.
    fn trace_value(&self, _offset: u64, _width: AccessWidth, _at: SimTime) -> Option<u64> {
        None
    }

    /// Applies a device reset.
    fn reset(&mut self, _kind: ResetKind) {}
}

/// Shareable memory bytes used to create aliases.
#[derive(Clone, Debug)]
pub struct SharedMemory {
    bytes: Rc<RefCell<Vec<u8>>>,
}

impl SharedMemory {
    /// Allocates zero-filled memory.
    pub fn zeroed(size: usize) -> Self {
        Self {
            bytes: Rc::new(RefCell::new(vec![0; size])),
        }
    }

    /// Allocates memory initialized from bytes.
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self {
            bytes: Rc::new(RefCell::new(bytes)),
        }
    }

    /// Returns the backing size in bytes.
    pub fn len(&self) -> usize {
        self.bytes.borrow().len()
    }

    /// Returns true when the backing contains no bytes.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Copies bytes from the backing store.
    pub fn to_vec(&self) -> Vec<u8> {
        self.bytes.borrow().clone()
    }

    /// Copies a checked byte range from the backing store.
    pub fn read_range(&self, offset: usize, length: usize) -> Option<Vec<u8>> {
        let bytes = self.bytes.borrow();
        let end = offset.checked_add(length)?;
        bytes.get(offset..end).map(<[u8]>::to_vec)
    }

    /// Replaces a checked byte range in the backing store.
    pub fn write_range(&self, offset: usize, value: &[u8]) -> bool {
        let mut bytes = self.bytes.borrow_mut();
        let Some(end) = offset.checked_add(value.len()) else {
            return false;
        };
        let Some(destination) = bytes.get_mut(offset..end) else {
            return false;
        };
        destination.copy_from_slice(value);
        true
    }

    /// Reads one little-endian 32-bit word.
    pub fn read_u32(&self, offset: usize) -> Option<u32> {
        self.read_range(offset, 4)
            .map(|bytes| u32::from_le_bytes(bytes.try_into().expect("range has four bytes")))
    }

    /// Writes one little-endian 32-bit word.
    pub fn write_u32(&self, offset: usize, value: u32) -> bool {
        self.write_range(offset, &value.to_le_bytes())
    }
}

enum Backing {
    Memory {
        storage: SharedMemory,
        storage_offset: usize,
        ignore_writes: bool,
    },
    Device(Box<dyn Device>),
}

struct Region {
    name: String,
    start: u64,
    end: u64,
    permissions: Permissions,
    backing: Backing,
}

impl fmt::Debug for Region {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self.backing {
            Backing::Memory { .. } => "memory",
            Backing::Device(_) => "device",
        };
        formatter
            .debug_struct("Region")
            .field("name", &self.name)
            .field("start", &format_args!("{:#x}", self.start))
            .field("end", &format_args!("{:#x}", self.end))
            .field("permissions", &self.permissions)
            .field("kind", &kind)
            .finish()
    }
}

/// One completed bus operation, optionally retained for diagnostics.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BusAccessRecord {
    /// Operation timestamp.
    pub at: SimTime,
    /// Program counter whose execution caused this operation, when known.
    ///
    /// This is observation-only context supplied by the owning machine. Bus
    /// and device behavior must never depend on it. Autonomous device, DMA,
    /// debugger, and host accesses deliberately omit it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pc: Option<u64>,
    /// Operation type.
    pub kind: AccessKind,
    /// First byte address.
    pub address: u64,
    /// Transfer width.
    pub width: AccessWidth,
    /// Read result or written value.
    pub value: u64,
    /// Safely observed value before a write, when the backing permits it.
    ///
    /// This is currently populated for direct memory and omitted for devices;
    /// the bus never performs an extra device read to obtain trace evidence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pre_value: Option<u64>,
    /// Safely observed value after a write, when the backing permits it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_value: Option<u64>,
    /// Mapped region name.
    pub region: String,
}

/// Receives completed bus operations without retaining them in the address space.
///
/// Observers are intended for streaming diagnostics such as CLI bus logs. The
/// existing in-memory access log remains available independently for debugger
/// and library callers.
pub trait BusAccessObserver {
    /// Observes one successfully completed operation in execution order.
    fn observe(&mut self, record: &BusAccessRecord);
}

/// Shareable observer handle used by machines with more than one access space.
pub type SharedBusAccessObserver = Rc<RefCell<dyn BusAccessObserver>>;

struct FanoutBusAccessObserver {
    observers: Vec<SharedBusAccessObserver>,
}

impl BusAccessObserver for FanoutBusAccessObserver {
    fn observe(&mut self, record: &BusAccessRecord) {
        for observer in &self.observers {
            observer.borrow_mut().observe(record);
        }
    }
}

/// Deterministic, non-overlapping address space.
pub struct AddressSpace {
    endianness: Endianness,
    regions: Vec<Region>,
    region_cache: [Option<usize>; 3],
    record_accesses: bool,
    access_log: Vec<BusAccessRecord>,
    access_observer: Option<SharedBusAccessObserver>,
    observation_pc: Option<u64>,
    watchpoints: BTreeSet<u64>,
    write_watchpoints: BTreeSet<u64>,
    masked_write_watchpoints: BTreeMap<u64, (u64, u64)>,
    watchpoint_hit: Option<BusAccessRecord>,
    last_device_access: Option<u64>,
}

impl fmt::Debug for AddressSpace {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AddressSpace")
            .field("endianness", &self.endianness)
            .field("regions", &self.regions)
            .field("record_accesses", &self.record_accesses)
            .field("access_log", &self.access_log)
            .field("has_access_observer", &self.access_observer.is_some())
            .field("watchpoints", &self.watchpoints)
            .field("write_watchpoints", &self.write_watchpoints)
            .field("masked_write_watchpoints", &self.masked_write_watchpoints)
            .field("watchpoint_hit", &self.watchpoint_hit)
            .finish()
    }
}

impl Default for AddressSpace {
    fn default() -> Self {
        Self::new(Endianness::Little)
    }
}

impl AddressSpace {
    /// Creates an empty address space.
    pub const fn new(endianness: Endianness) -> Self {
        Self {
            endianness,
            regions: Vec::new(),
            region_cache: [None; 3],
            record_accesses: false,
            access_log: Vec::new(),
            access_observer: None,
            observation_pc: None,
            watchpoints: BTreeSet::new(),
            write_watchpoints: BTreeSet::new(),
            masked_write_watchpoints: BTreeMap::new(),
            watchpoint_hit: None,
            last_device_access: None,
        }
    }

    /// Enables or disables completed-access recording.
    pub fn set_access_recording(&mut self, enabled: bool) {
        self.record_accesses = enabled;
    }

    /// Returns recorded bus operations.
    pub fn access_log(&self) -> &[BusAccessRecord] {
        &self.access_log
    }

    /// Clears recorded bus operations without disabling recording.
    pub fn clear_access_log(&mut self) {
        self.access_log.clear();
    }

    /// Installs or removes a streaming completed-access observer.
    pub fn set_access_observer(&mut self, observer: Option<SharedBusAccessObserver>) {
        self.access_observer = observer;
    }

    /// Sets observation-only PC context for subsequently completed accesses.
    ///
    /// Machines should set this immediately before one CPU or emulated-ROM
    /// action and clear it immediately afterwards. It is intentionally absent
    /// from device APIs so peripheral behavior cannot consume it.
    pub fn set_observation_pc(&mut self, pc: Option<u64>) {
        self.observation_pc = pc;
    }

    /// Adds a streaming observer while preserving any observer already installed.
    ///
    /// This is useful when an interactive debugger needs a bounded diagnostic
    /// view at the same time as a host frontend streams the complete log.
    pub fn add_access_observer(&mut self, observer: SharedBusAccessObserver) {
        self.access_observer = Some(match self.access_observer.take() {
            Some(previous) => Rc::new(RefCell::new(FanoutBusAccessObserver {
                observers: vec![previous, observer],
            })),
            None => observer,
        });
    }

    /// Adds a byte address that stops the owning machine on a completed data access.
    pub fn add_watchpoint(&mut self, address: u64) {
        self.watchpoints.insert(address);
    }

    /// Adds a byte address that stops only on a completed overlapping write.
    pub fn add_write_watchpoint(&mut self, address: u64) {
        self.write_watchpoints.insert(address);
    }

    /// Stops on an overlapping write when `(value & mask) == expected`.
    pub fn add_masked_write_watchpoint(&mut self, address: u64, mask: u64, expected: u64) {
        self.masked_write_watchpoints
            .insert(address, (mask, expected & mask));
    }

    /// Removes every configured data watchpoint and pending hit.
    pub fn clear_watchpoints(&mut self) {
        self.watchpoints.clear();
        self.write_watchpoints.clear();
        self.masked_write_watchpoints.clear();
        self.watchpoint_hit = None;
    }

    /// Returns whether completed data accesses need watchpoint matching.
    pub fn has_watchpoints(&self) -> bool {
        !self.watchpoints.is_empty()
            || !self.write_watchpoints.is_empty()
            || !self.masked_write_watchpoints.is_empty()
    }

    /// Clears a pending hit while preserving configured watchpoints.
    pub fn clear_watchpoint_hit(&mut self) {
        self.watchpoint_hit = None;
    }

    /// Takes the first completed watched access since the previous clear/take.
    pub fn take_watchpoint_hit(&mut self) -> Option<BusAccessRecord> {
        self.watchpoint_hit.take()
    }

    /// Takes the address of the most recent completed device access.
    pub fn take_device_access(&mut self) -> Option<u64> {
        self.last_device_access.take()
    }

    fn record_watchpoint_hit(&mut self, record: &BusAccessRecord) {
        if record.kind == AccessKind::Execute || self.watchpoint_hit.is_some() {
            return;
        }
        let end = record
            .address
            .saturating_add(u64::from(record.width.bytes()));
        let any_hit = self.watchpoints.range(record.address..end).next().is_some();
        let write_hit = record.kind == AccessKind::Write
            && self
                .write_watchpoints
                .range(record.address..end)
                .next()
                .is_some();
        let masked_write_hit = record.kind == AccessKind::Write
            && self
                .masked_write_watchpoints
                .range(record.address..end)
                .any(|(_, (mask, expected))| record.value & mask == *expected);
        if any_hit || write_hit || masked_write_hit {
            self.watchpoint_hit = Some(record.clone());
        }
    }

    fn record_completed_access(&mut self, record: BusAccessRecord) {
        self.record_watchpoint_hit(&record);
        if self.record_accesses {
            self.access_log.push(record.clone());
        }
        if let Some(observer) = &self.access_observer {
            observer.borrow_mut().observe(&record);
        }
    }

    #[inline]
    fn monitors_accesses(&self) -> bool {
        self.record_accesses
            || self.access_observer.is_some()
            || !self.watchpoints.is_empty()
            || !self.write_watchpoints.is_empty()
            || !self.masked_write_watchpoints.is_empty()
    }

    /// Maps zero-filled RAM and returns its shareable backing.
    pub fn map_ram(
        &mut self,
        name: impl Into<String>,
        start: u64,
        size: usize,
        executable: bool,
    ) -> Result<SharedMemory, MapError> {
        let storage = SharedMemory::zeroed(size);
        let permissions = if executable {
            Permissions::RWX
        } else {
            Permissions::RW
        };
        self.map_shared(name, start, size, permissions, storage.clone(), 0)?;
        Ok(storage)
    }

    /// Maps initialized read/execute memory and returns its backing.
    pub fn map_rom(
        &mut self,
        name: impl Into<String>,
        start: u64,
        bytes: Vec<u8>,
    ) -> Result<SharedMemory, MapError> {
        let size = bytes.len();
        let storage = SharedMemory::from_bytes(bytes);
        self.map_shared(name, start, size, Permissions::RX, storage.clone(), 0)?;
        Ok(storage)
    }

    /// Maps initialized executable memory whose runtime writes complete without changing bytes.
    ///
    /// Some on-chip ROM interconnects acknowledge stores and discard them rather than raising a
    /// bus fault. Firmware must not rely on the stored value becoming observable.
    pub fn map_write_ignored_rom(
        &mut self,
        name: impl Into<String>,
        start: u64,
        bytes: Vec<u8>,
    ) -> Result<SharedMemory, MapError> {
        let size = bytes.len();
        let storage = SharedMemory::from_bytes(bytes);
        self.insert_region(
            name.into(),
            start,
            size,
            Permissions::RWX,
            Backing::Memory {
                storage: storage.clone(),
                storage_offset: 0,
                ignore_writes: true,
            },
        )?;
        Ok(storage)
    }

    /// Maps a window onto existing shared memory.
    pub fn map_shared(
        &mut self,
        name: impl Into<String>,
        start: u64,
        size: usize,
        permissions: Permissions,
        storage: SharedMemory,
        storage_offset: usize,
    ) -> Result<(), MapError> {
        let name = name.into();
        let backing_end = storage_offset
            .checked_add(size)
            .ok_or_else(|| MapError::BackingRange { name: name.clone() })?;
        if backing_end > storage.len() {
            return Err(MapError::BackingRange { name });
        }
        self.insert_region(
            name,
            start,
            size,
            permissions,
            Backing::Memory {
                storage,
                storage_offset,
                ignore_writes: false,
            },
        )
    }

    /// Maps an MMIO device.
    pub fn map_device(
        &mut self,
        name: impl Into<String>,
        start: u64,
        size: usize,
        device: Box<dyn Device>,
    ) -> Result<(), MapError> {
        self.insert_region(
            name.into(),
            start,
            size,
            Permissions::RW,
            Backing::Device(device),
        )
    }

    fn insert_region(
        &mut self,
        name: String,
        start: u64,
        size: usize,
        permissions: Permissions,
        backing: Backing,
    ) -> Result<(), MapError> {
        if size == 0 {
            return Err(MapError::Empty { name });
        }
        let end = start
            .checked_add(
                u64::try_from(size)
                    .map_err(|_| MapError::AddressOverflow { name: name.clone() })?,
            )
            .ok_or_else(|| MapError::AddressOverflow { name: name.clone() })?;
        if let Some(existing) = self
            .regions
            .iter()
            .find(|existing| start < existing.end && end > existing.start)
        {
            return Err(MapError::Overlap {
                name,
                start,
                end,
                existing: existing.name.clone(),
                existing_start: existing.start,
                existing_end: existing.end,
            });
        }
        self.regions.push(Region {
            name,
            start,
            end,
            permissions,
            backing,
        });
        self.regions.sort_by_key(|region| region.start);
        self.region_cache = [None; 3];
        Ok(())
    }

    /// Copies firmware bytes into memory while bypassing runtime write permissions.
    pub fn load(&mut self, address: u64, data: &[u8]) -> Result<(), BusFault> {
        for (index, byte) in data.iter().copied().enumerate() {
            let current = address
                .checked_add(u64::try_from(index).map_err(|_| {
                    BusFault::new(
                        BusFaultKind::Boundary,
                        AccessKind::Write,
                        address,
                        AccessWidth::Byte,
                        "firmware load address overflow",
                    )
                })?)
                .ok_or_else(|| {
                    BusFault::new(
                        BusFaultKind::Boundary,
                        AccessKind::Write,
                        address,
                        AccessWidth::Byte,
                        "firmware load address overflow",
                    )
                })?;
            let Some(region) = self
                .regions
                .iter_mut()
                .find(|region| current >= region.start && current < region.end)
            else {
                return Err(BusFault::new(
                    BusFaultKind::Unmapped,
                    AccessKind::Write,
                    current,
                    AccessWidth::Byte,
                    "firmware load address is not mapped",
                ));
            };
            match &mut region.backing {
                Backing::Memory {
                    storage,
                    storage_offset,
                    ..
                } => {
                    let offset = usize::try_from(current - region.start)
                        .expect("mapped region offset fits usize");
                    storage.bytes.borrow_mut()[*storage_offset + offset] = byte;
                }
                Backing::Device(_) => {
                    return Err(BusFault::new(
                        BusFaultKind::Permission,
                        AccessKind::Write,
                        current,
                        AccessWidth::Byte,
                        "firmware loader cannot initialize an MMIO device",
                    ));
                }
            }
        }
        Ok(())
    }

    /// Applies reset to all mapped devices in address order.
    pub fn reset_devices(&mut self, kind: ResetKind) {
        for region in &mut self.regions {
            if let Backing::Device(device) = &mut region.backing {
                device.reset(kind);
            }
        }
    }

    /// Returns mapped region names and address ranges.
    pub fn region_map(&self) -> Vec<(&str, u64, u64, Permissions)> {
        self.regions
            .iter()
            .map(|region| {
                (
                    region.name.as_str(),
                    region.start,
                    region.end,
                    region.permissions,
                )
            })
            .collect()
    }

    fn region_for(
        &mut self,
        address: u64,
        width: AccessWidth,
        access: AccessKind,
    ) -> Result<&mut Region, BusFault> {
        let access_end = address
            .checked_add(u64::from(width.bytes()))
            .ok_or_else(|| {
                BusFault::new(
                    BusFaultKind::Boundary,
                    access,
                    address,
                    width,
                    "access address overflow",
                )
            })?;
        let cache_slot = match access {
            AccessKind::Execute => 0,
            AccessKind::Read => 1,
            AccessKind::Write => 2,
        };
        if let Some(index) = self.region_cache[cache_slot]
            && address >= self.regions[index].start
            && access_end <= self.regions[index].end
            && self.regions[index].permissions.permits(access)
        {
            return Ok(&mut self.regions[index]);
        }
        let Some(index) = self
            .regions
            .partition_point(|region| region.start <= address)
            .checked_sub(1)
            .filter(|index| address < self.regions[*index].end)
        else {
            return Err(BusFault::new(
                BusFaultKind::Unmapped,
                access,
                address,
                width,
                "no mapped region",
            ));
        };
        let region = &mut self.regions[index];
        if access_end > region.end {
            return Err(BusFault::new(
                BusFaultKind::Boundary,
                access,
                address,
                width,
                format!("access crosses region {:?}", region.name),
            ));
        }
        if !region.permissions.permits(access) {
            return Err(BusFault::new(
                BusFaultKind::Permission,
                access,
                address,
                width,
                format!("operation not permitted by region {:?}", region.name),
            ));
        }
        self.region_cache[cache_slot] = Some(index);
        Ok(region)
    }
}

impl Bus for AddressSpace {
    fn fast_fetch32(&mut self, address: u64, _at: SimTime) -> Option<Result<u32, BusFault>> {
        if self.monitors_accesses() {
            return None;
        }
        let endianness = self.endianness;
        let access_end = address.checked_add(4)?;
        let index = if let Some(index) = self.region_cache[0]
            && address >= self.regions[index].start
            && access_end <= self.regions[index].end
            && self.regions[index].permissions.execute
        {
            index
        } else {
            let index = self
                .regions
                .partition_point(|region| region.start <= address)
                .checked_sub(1)
                .filter(|index| {
                    access_end <= self.regions[*index].end
                        && self.regions[*index].permissions.execute
                })?;
            self.region_cache[0] = Some(index);
            index
        };
        let region = &self.regions[index];
        let relative = usize::try_from(address - region.start).ok()?;
        let Backing::Memory {
            storage,
            storage_offset,
            ..
        } = &region.backing
        else {
            return None;
        };
        let start = *storage_offset + relative;
        let bytes = storage.bytes.borrow();
        let raw: [u8; 4] = bytes[start..start + 4].try_into().ok()?;
        Some(Ok(match endianness {
            Endianness::Little => u32::from_le_bytes(raw),
            Endianness::Big => u32::from_be_bytes(raw),
        }))
    }

    fn fast_read(&mut self, address: u64, width: AccessWidth) -> Option<u64> {
        if self.monitors_accesses() {
            return None;
        }
        let access_end = address.checked_add(u64::from(width.bytes()))?;
        let index = if let Some(index) = self.region_cache[1]
            && address >= self.regions[index].start
            && access_end <= self.regions[index].end
            && self.regions[index].permissions.read
        {
            index
        } else {
            let index = self
                .regions
                .partition_point(|region| region.start <= address)
                .checked_sub(1)
                .filter(|index| {
                    access_end <= self.regions[*index].end && self.regions[*index].permissions.read
                })?;
            self.region_cache[1] = Some(index);
            index
        };
        let region = &self.regions[index];
        let Backing::Memory {
            storage,
            storage_offset,
            ..
        } = &region.backing
        else {
            return None;
        };
        let start = *storage_offset + usize::try_from(address - region.start).ok()?;
        let bytes = storage.bytes.borrow();
        Some(match (self.endianness, width) {
            (_, AccessWidth::Byte) => u64::from(bytes[start]),
            (Endianness::Little, AccessWidth::HalfWord) => {
                u64::from(u16::from_le_bytes(bytes[start..start + 2].try_into().ok()?))
            }
            (Endianness::Big, AccessWidth::HalfWord) => {
                u64::from(u16::from_be_bytes(bytes[start..start + 2].try_into().ok()?))
            }
            (Endianness::Little, AccessWidth::Word) => {
                u64::from(u32::from_le_bytes(bytes[start..start + 4].try_into().ok()?))
            }
            (Endianness::Big, AccessWidth::Word) => {
                u64::from(u32::from_be_bytes(bytes[start..start + 4].try_into().ok()?))
            }
            (Endianness::Little, AccessWidth::DoubleWord) => {
                u64::from_le_bytes(bytes[start..start + 8].try_into().ok()?)
            }
            (Endianness::Big, AccessWidth::DoubleWord) => {
                u64::from_be_bytes(bytes[start..start + 8].try_into().ok()?)
            }
        })
    }

    fn fast_write(&mut self, address: u64, width: AccessWidth, value: u64) -> bool {
        if self.monitors_accesses() {
            return false;
        }
        let Some(access_end) = address.checked_add(u64::from(width.bytes())) else {
            return false;
        };
        let index = if let Some(index) = self.region_cache[2]
            && address >= self.regions[index].start
            && access_end <= self.regions[index].end
            && self.regions[index].permissions.write
        {
            index
        } else {
            let Some(index) = self
                .regions
                .partition_point(|region| region.start <= address)
                .checked_sub(1)
                .filter(|index| {
                    access_end <= self.regions[*index].end && self.regions[*index].permissions.write
                })
            else {
                return false;
            };
            self.region_cache[2] = Some(index);
            index
        };
        let region = &self.regions[index];
        let Backing::Memory {
            storage,
            storage_offset,
            ignore_writes,
        } = &region.backing
        else {
            return false;
        };
        if *ignore_writes {
            return true;
        }
        let Ok(relative) = usize::try_from(address - region.start) else {
            return false;
        };
        let start = *storage_offset + relative;
        let mut bytes = storage.bytes.borrow_mut();
        let encoded = match self.endianness {
            Endianness::Little => value.to_le_bytes(),
            Endianness::Big => value.to_be_bytes(),
        };
        let source = match self.endianness {
            Endianness::Little => &encoded[..usize::from(width.bytes())],
            Endianness::Big => &encoded[8 - usize::from(width.bytes())..],
        };
        bytes[start..start + source.len()].copy_from_slice(source);
        true
    }

    fn read(
        &mut self,
        address: u64,
        width: AccessWidth,
        kind: AccessKind,
        at: SimTime,
    ) -> Result<u64, BusFault> {
        let endianness = self.endianness;
        let monitored = self.monitors_accesses();
        let observation_pc = self.observation_pc;
        let region = self.region_for(address, width, kind)?;
        let relative = address - region.start;
        let device_access = matches!(region.backing, Backing::Device(_));
        let value = match &mut region.backing {
            Backing::Memory {
                storage,
                storage_offset,
                ..
            } => {
                let start =
                    *storage_offset + usize::try_from(relative).expect("mapped offset fits usize");
                let end = start + usize::from(width.bytes());
                let bytes = storage.bytes.borrow();
                let slice = &bytes[start..end];
                match endianness {
                    Endianness::Little => {
                        slice.iter().enumerate().fold(0_u64, |value, (i, byte)| {
                            value | (u64::from(*byte) << (i * 8))
                        })
                    }
                    Endianness::Big => slice
                        .iter()
                        .rev()
                        .enumerate()
                        .fold(0_u64, |value, (i, byte)| {
                            value | (u64::from(*byte) << (i * 8))
                        }),
                }
            }
            Backing::Device(device) => device.read(relative, width, at).map_err(|error| {
                BusFault::new(
                    BusFaultKind::Device,
                    kind,
                    address,
                    width,
                    format!("{}: {error}", device.name()),
                )
            })?,
        } & width.value_mask();
        if monitored {
            let record = BusAccessRecord {
                at,
                pc: observation_pc,
                kind,
                address,
                width,
                value,
                pre_value: None,
                post_value: None,
                region: region.name.clone(),
            };
            self.record_completed_access(record);
        }
        if device_access {
            self.last_device_access = Some(address);
        }
        Ok(value)
    }

    fn write(
        &mut self,
        address: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), BusFault> {
        let endianness = self.endianness;
        let monitored = self.monitors_accesses();
        let observation_pc = self.observation_pc;
        let region = self.region_for(address, width, AccessKind::Write)?;
        let relative = address - region.start;
        let device_access = matches!(region.backing, Backing::Device(_));
        let masked = value & width.value_mask();
        let mut safe_values = (None, None);
        match &mut region.backing {
            Backing::Memory {
                storage,
                storage_offset,
                ignore_writes,
            } => {
                if !*ignore_writes {
                    let start = *storage_offset
                        + usize::try_from(relative).expect("mapped offset fits usize");
                    let end = start + usize::from(width.bytes());
                    let mut bytes = storage.bytes.borrow_mut();
                    let previous = monitored.then(|| match endianness {
                        Endianness::Little => bytes[start..end]
                            .iter()
                            .enumerate()
                            .fold(0_u64, |value, (index, byte)| {
                                value | (u64::from(*byte) << (index * 8))
                            }),
                        Endianness::Big => bytes[start..end]
                            .iter()
                            .rev()
                            .enumerate()
                            .fold(0_u64, |value, (index, byte)| {
                                value | (u64::from(*byte) << (index * 8))
                            }),
                    });
                    match endianness {
                        Endianness::Little => {
                            for (index, byte) in bytes[start..end].iter_mut().enumerate() {
                                *byte = u8::try_from((masked >> (index * 8)) & 0xff)
                                    .expect("masked byte fits u8");
                            }
                        }
                        Endianness::Big => {
                            let length = end - start;
                            for (index, byte) in bytes[start..end].iter_mut().enumerate() {
                                *byte = u8::try_from((masked >> ((length - 1 - index) * 8)) & 0xff)
                                    .expect("masked byte fits u8");
                            }
                        }
                    }
                    safe_values = (previous, previous.map(|_| masked));
                } else if monitored {
                    let start = *storage_offset
                        + usize::try_from(relative).expect("mapped offset fits usize");
                    let end = start + usize::from(width.bytes());
                    let bytes = storage.bytes.borrow();
                    let previous = match endianness {
                        Endianness::Little => bytes[start..end]
                            .iter()
                            .enumerate()
                            .fold(0_u64, |value, (index, byte)| {
                                value | (u64::from(*byte) << (index * 8))
                            }),
                        Endianness::Big => bytes[start..end]
                            .iter()
                            .rev()
                            .enumerate()
                            .fold(0_u64, |value, (index, byte)| {
                                value | (u64::from(*byte) << (index * 8))
                            }),
                    };
                    safe_values = (Some(previous), Some(previous));
                }
            }
            Backing::Device(device) => {
                let pre_value = monitored
                    .then(|| device.trace_value(relative, width, at))
                    .flatten();
                device.write(relative, width, masked, at).map_err(|error| {
                    BusFault::new(
                        BusFaultKind::Device,
                        AccessKind::Write,
                        address,
                        width,
                        format!("{}: {error}", device.name()),
                    )
                })?;
                let post_value = monitored
                    .then(|| device.trace_value(relative, width, at))
                    .flatten();
                safe_values = (pre_value, post_value);
            }
        }
        if monitored {
            let record = BusAccessRecord {
                at,
                pc: observation_pc,
                kind: AccessKind::Write,
                address,
                width,
                value: masked,
                pre_value: safe_values.0,
                post_value: safe_values.1,
                region: region.name.clone(),
            };
            self.record_completed_access(record);
        }
        if device_access {
            self.last_device_access = Some(address);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct CollectingObserver(Rc<RefCell<Vec<BusAccessRecord>>>);

    impl BusAccessObserver for CollectingObserver {
        fn observe(&mut self, record: &BusAccessRecord) {
            self.0.borrow_mut().push(record.clone());
        }
    }

    struct TraceableRegister {
        value: u64,
        reads: Rc<Cell<u32>>,
    }

    impl Device for TraceableRegister {
        fn name(&self) -> &str {
            "traceable-register"
        }

        fn read(
            &mut self,
            _offset: u64,
            _width: AccessWidth,
            _at: SimTime,
        ) -> Result<u64, DeviceError> {
            self.reads.set(self.reads.get() + 1);
            Ok(self.value)
        }

        fn write(
            &mut self,
            _offset: u64,
            _width: AccessWidth,
            value: u64,
            _at: SimTime,
        ) -> Result<(), DeviceError> {
            self.value = value;
            Ok(())
        }

        fn trace_value(&self, _offset: u64, _width: AccessWidth, _at: SimTime) -> Option<u64> {
            Some(self.value)
        }
    }

    #[test]
    fn maps_and_accesses_little_endian_memory() {
        let mut bus = AddressSpace::default();
        bus.map_ram("ram", 0x2000_0000, 16, true).unwrap();
        bus.write(0x2000_0000, AccessWidth::Word, 0x4433_2211, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            bus.read(
                0x2000_0001,
                AccessWidth::HalfWord,
                AccessKind::Read,
                SimTime::ZERO
            )
            .unwrap(),
            0x3322
        );
    }

    #[test]
    fn fast_fetch_is_disabled_when_accesses_are_observable() {
        let mut bus = AddressSpace::default();
        bus.map_rom("rom", 0x1000, vec![0x11, 0x22, 0x33, 0x44])
            .unwrap();

        assert_eq!(
            bus.fast_fetch32(0x1000, SimTime::ZERO)
                .expect("unobserved memory fetch uses the fast path")
                .unwrap(),
            0x4433_2211
        );

        bus.set_access_recording(true);
        assert!(bus.fast_fetch32(0x1000, SimTime::ZERO).is_none());
        bus.set_access_recording(false);
        bus.add_watchpoint(0x1000);
        assert!(bus.fast_fetch32(0x1000, SimTime::ZERO).is_none());
    }

    #[test]
    fn fast_fetch_falls_back_at_a_region_boundary() {
        let mut bus = AddressSpace::default();
        bus.map_rom("rom", 0x1000, vec![0x11, 0x22, 0x33, 0x44])
            .unwrap();

        assert!(bus.fast_fetch32(0x1002, SimTime::ZERO).is_none());
        assert_eq!(
            bus.read(
                0x1002,
                AccessWidth::HalfWord,
                AccessKind::Execute,
                SimTime::ZERO
            )
            .unwrap(),
            0x4433
        );
    }

    #[test]
    fn fast_data_paths_preserve_width_endianness_and_observation() {
        let mut little = AddressSpace::default();
        little.map_ram("ram", 0x1000, 16, false).unwrap();
        assert!(little.fast_write(0x1001, AccessWidth::Word, 0x8877_6655));
        assert_eq!(
            little.fast_read(0x1001, AccessWidth::Word),
            Some(0x8877_6655)
        );
        assert_eq!(
            little.fast_read(0x1002, AccessWidth::HalfWord),
            Some(0x7766)
        );
        assert!(little.fast_read(0x100e, AccessWidth::Word).is_none());

        little.set_access_recording(true);
        assert!(little.fast_read(0x1001, AccessWidth::Word).is_none());
        assert!(!little.fast_write(0x1001, AccessWidth::Word, 0));

        let mut big = AddressSpace::new(Endianness::Big);
        big.map_ram("ram", 0x2000, 8, false).unwrap();
        assert!(big.fast_write(0x2000, AccessWidth::Word, 0x1122_3344));
        assert_eq!(big.fast_read(0x2001, AccessWidth::HalfWord), Some(0x2233));
    }

    #[test]
    fn rejects_overlap_and_cross_boundary_accesses() {
        let mut bus = AddressSpace::default();
        bus.map_ram("ram", 0x1000, 4, false).unwrap();
        assert!(matches!(
            bus.map_ram("overlap", 0x1003, 4, false),
            Err(MapError::Overlap { .. })
        ));
        let fault = bus
            .read(0x1002, AccessWidth::Word, AccessKind::Read, SimTime::ZERO)
            .unwrap_err();
        assert_eq!(fault.kind, BusFaultKind::Boundary);
    }

    #[test]
    fn write_ignored_rom_acknowledges_without_mutating() {
        let mut bus = AddressSpace::default();
        bus.map_write_ignored_rom("rom", 0, vec![0x11, 0x22, 0x33, 0x44])
            .unwrap();

        bus.write(0, AccessWidth::Word, 0xaabb_ccdd, SimTime::ZERO)
            .unwrap();

        assert_eq!(
            bus.read(0, AccessWidth::Word, AccessKind::Read, SimTime::ZERO)
                .unwrap(),
            0x4433_2211
        );
    }

    #[test]
    fn aliases_share_memory() {
        let mut bus = AddressSpace::default();
        let ram = bus.map_ram("ram", 0x1000, 8, false).unwrap();
        bus.map_shared("alias", 0x2000, 8, Permissions::RW, ram, 0)
            .unwrap();
        bus.write(0x1000, AccessWidth::Word, 42, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            bus.read(0x2000, AccessWidth::Word, AccessKind::Read, SimTime::ZERO)
                .unwrap(),
            42
        );
    }

    #[test]
    fn loader_can_initialize_rom() {
        let mut bus = AddressSpace::default();
        bus.map_rom("flash", 0, vec![0; 8]).unwrap();
        bus.load(2, &[0xaa, 0xbb]).unwrap();
        assert_eq!(
            bus.read(2, AccessWidth::HalfWord, AccessKind::Read, SimTime::ZERO)
                .unwrap(),
            0xbbaa
        );
        assert_eq!(
            bus.write(2, AccessWidth::Byte, 0, SimTime::ZERO)
                .unwrap_err()
                .kind,
            BusFaultKind::Permission
        );
    }

    #[test]
    fn watchpoints_report_completed_overlapping_data_accesses_only() {
        let mut bus = AddressSpace::default();
        bus.map_ram("ram", 0x1000, 16, true).unwrap();
        bus.add_watchpoint(0x1002);

        bus.read(
            0x1000,
            AccessWidth::Word,
            AccessKind::Execute,
            SimTime::ZERO,
        )
        .unwrap();
        assert!(bus.take_watchpoint_hit().is_none());

        bus.write(
            0x1000,
            AccessWidth::Word,
            0x4433_2211,
            SimTime::from_ticks(1),
        )
        .unwrap();
        let hit = bus.take_watchpoint_hit().unwrap();
        assert_eq!(hit.address, 0x1000);
        assert_eq!(hit.kind, AccessKind::Write);
        assert_eq!(hit.width, AccessWidth::Word);

        bus.clear_watchpoints();
        bus.read(
            0x1002,
            AccessWidth::Byte,
            AccessKind::Read,
            SimTime::from_ticks(2),
        )
        .unwrap();
        assert!(bus.take_watchpoint_hit().is_none());
    }

    #[test]
    fn write_watchpoints_ignore_reads_and_stop_on_overlapping_writes() {
        let mut bus = AddressSpace::default();
        bus.map_ram("ram", 0x1000, 16, true).unwrap();
        bus.add_write_watchpoint(0x1002);
        bus.read(0x1000, AccessWidth::Word, AccessKind::Read, SimTime::ZERO)
            .unwrap();
        assert!(bus.take_watchpoint_hit().is_none());
        bus.write(
            0x1000,
            AccessWidth::Word,
            0x4433_2211,
            SimTime::from_ticks(1),
        )
        .unwrap();
        assert_eq!(bus.take_watchpoint_hit().unwrap().kind, AccessKind::Write);
    }

    #[test]
    fn masked_write_watchpoints_require_the_value_predicate() {
        let mut bus = AddressSpace::default();
        bus.map_ram("ram", 0x1000, 16, true).unwrap();
        bus.add_masked_write_watchpoint(0x1000, 0xc000_0000, 0xc000_0000);
        bus.write(0x1000, AccessWidth::Word, 0x8000_1234, SimTime::ZERO)
            .unwrap();
        assert!(bus.take_watchpoint_hit().is_none());
        bus.write(
            0x1000,
            AccessWidth::Word,
            0xc000_1234,
            SimTime::from_ticks(1),
        )
        .unwrap();
        assert_eq!(bus.take_watchpoint_hit().unwrap().value, 0xc000_1234);
    }

    #[test]
    fn observer_streams_without_populating_the_in_memory_log() {
        let records = Rc::new(RefCell::new(Vec::new()));
        let observer: SharedBusAccessObserver =
            Rc::new(RefCell::new(CollectingObserver(records.clone())));
        let mut bus = AddressSpace::default();
        bus.map_ram("ram", 0x1000, 16, true).unwrap();
        bus.set_access_observer(Some(observer));

        bus.write(
            0x1000,
            AccessWidth::Word,
            0x4433_2211,
            SimTime::from_ticks(1),
        )
        .unwrap();
        bus.read(
            0x1000,
            AccessWidth::Word,
            AccessKind::Execute,
            SimTime::from_ticks(2),
        )
        .unwrap();

        assert!(bus.access_log().is_empty());
        let records = records.borrow();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].kind, AccessKind::Write);
        assert_eq!(records[1].kind, AccessKind::Execute);
    }

    #[test]
    fn observation_pc_is_correlated_without_leaking_into_later_activity() {
        let records = Rc::new(RefCell::new(Vec::new()));
        let observer: SharedBusAccessObserver =
            Rc::new(RefCell::new(CollectingObserver(records.clone())));
        let mut bus = AddressSpace::default();
        bus.map_ram("ram", 0x1000, 16, true).unwrap();
        bus.set_access_observer(Some(observer));

        bus.set_observation_pc(Some(0x4200_1234));
        bus.write(0x1000, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        bus.set_observation_pc(None);
        bus.write(0x1004, AccessWidth::Word, 2, SimTime::from_ticks(1))
            .unwrap();

        let records = records.borrow();
        assert_eq!(records[0].pc, Some(0x4200_1234));
        assert_eq!(records[0].pre_value, Some(0));
        assert_eq!(records[0].post_value, Some(1));
        assert_eq!(records[1].pc, None);
        assert_eq!(records[1].pre_value, Some(0));
        assert_eq!(records[1].post_value, Some(2));
    }

    #[test]
    fn device_pre_post_trace_uses_snapshot_hook_without_reading() {
        let records = Rc::new(RefCell::new(Vec::new()));
        let reads = Rc::new(Cell::new(0));
        let mut bus = AddressSpace::default();
        bus.map_device(
            "register",
            0x2000,
            4,
            Box::new(TraceableRegister {
                value: 0x11,
                reads: reads.clone(),
            }),
        )
        .unwrap();
        bus.set_access_observer(Some(Rc::new(RefCell::new(CollectingObserver(
            records.clone(),
        )))));

        bus.write(0x2000, AccessWidth::Word, 0x22, SimTime::ZERO)
            .unwrap();

        assert_eq!(reads.get(), 0);
        assert_eq!(records.borrow()[0].pre_value, Some(0x11));
        assert_eq!(records.borrow()[0].post_value, Some(0x22));
    }

    #[test]
    fn added_observer_preserves_the_existing_stream() {
        let first = Rc::new(RefCell::new(Vec::new()));
        let second = Rc::new(RefCell::new(Vec::new()));
        let mut bus = AddressSpace::default();
        bus.map_ram("ram", 0x1000, 16, true).unwrap();
        bus.set_access_observer(Some(Rc::new(RefCell::new(CollectingObserver(
            first.clone(),
        )))));
        bus.add_access_observer(Rc::new(RefCell::new(CollectingObserver(second.clone()))));

        bus.write(
            0x1000,
            AccessWidth::Word,
            0x4433_2211,
            SimTime::from_ticks(1),
        )
        .unwrap();

        assert_eq!(*first.borrow(), *second.borrow());
        assert_eq!(first.borrow().len(), 1);
    }
}
