//! Deterministic memory maps and MMIO routing.

use remu_core::{AccessKind, AccessWidth, Bus, BusFault, BusFaultKind, ResetKind, SimTime};
use serde::Serialize;
use std::cell::RefCell;
use std::collections::BTreeSet;
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
    /// Operation type.
    pub kind: AccessKind,
    /// First byte address.
    pub address: u64,
    /// Transfer width.
    pub width: AccessWidth,
    /// Read result or written value.
    pub value: u64,
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

/// Deterministic, non-overlapping address space.
pub struct AddressSpace {
    endianness: Endianness,
    regions: Vec<Region>,
    record_accesses: bool,
    access_log: Vec<BusAccessRecord>,
    access_observer: Option<SharedBusAccessObserver>,
    watchpoints: BTreeSet<u64>,
    watchpoint_hit: Option<BusAccessRecord>,
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
            record_accesses: false,
            access_log: Vec::new(),
            access_observer: None,
            watchpoints: BTreeSet::new(),
            watchpoint_hit: None,
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

    /// Adds a byte address that stops the owning machine on a completed data access.
    pub fn add_watchpoint(&mut self, address: u64) {
        self.watchpoints.insert(address);
    }

    /// Removes every configured data watchpoint and pending hit.
    pub fn clear_watchpoints(&mut self) {
        self.watchpoints.clear();
        self.watchpoint_hit = None;
    }

    /// Clears a pending hit while preserving configured watchpoints.
    pub fn clear_watchpoint_hit(&mut self) {
        self.watchpoint_hit = None;
    }

    /// Takes the first completed watched access since the previous clear/take.
    pub fn take_watchpoint_hit(&mut self) -> Option<BusAccessRecord> {
        self.watchpoint_hit.take()
    }

    fn record_watchpoint_hit(&mut self, record: &BusAccessRecord) {
        if record.kind == AccessKind::Execute || self.watchpoint_hit.is_some() {
            return;
        }
        let end = record
            .address
            .saturating_add(u64::from(record.width.bytes()));
        if self.watchpoints.range(record.address..end).next().is_some() {
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
        self.map_device_with_permissions(name, start, size, Permissions::RW, device)
    }

    /// Maps an MMIO device with explicit access permissions.
    ///
    /// Most peripherals are read/write only and should use [`Self::map_device`].
    /// A few memory-backed devices, such as executable FRAM or ROM windows,
    /// also need to service instruction fetches and can opt into `RX`/`RWX`.
    pub fn map_device_with_permissions(
        &mut self,
        name: impl Into<String>,
        start: u64,
        size: usize,
        permissions: Permissions,
        device: Box<dyn Device>,
    ) -> Result<(), MapError> {
        self.insert_region(
            name.into(),
            start,
            size,
            permissions,
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
        let Some(region) = self
            .regions
            .iter_mut()
            .find(|region| address >= region.start && address < region.end)
        else {
            return Err(BusFault::new(
                BusFaultKind::Unmapped,
                access,
                address,
                width,
                "no mapped region",
            ));
        };
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
        Ok(region)
    }
}

impl Bus for AddressSpace {
    fn read(
        &mut self,
        address: u64,
        width: AccessWidth,
        kind: AccessKind,
        at: SimTime,
    ) -> Result<u64, BusFault> {
        let endianness = self.endianness;
        let region = self.region_for(address, width, kind)?;
        let relative = address - region.start;
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
        let region_name = region.name.clone();
        let record = BusAccessRecord {
            at,
            kind,
            address,
            width,
            value,
            region: region_name,
        };
        self.record_completed_access(record);
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
        let region = self.region_for(address, width, AccessKind::Write)?;
        let relative = address - region.start;
        let masked = value & width.value_mask();
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
                }
            }
            Backing::Device(device) => {
                device.write(relative, width, masked, at).map_err(|error| {
                    BusFault::new(
                        BusFaultKind::Device,
                        AccessKind::Write,
                        address,
                        width,
                        format!("{}: {error}", device.name()),
                    )
                })?;
            }
        }
        let region_name = region.name.clone();
        let record = BusAccessRecord {
            at,
            kind: AccessKind::Write,
            address,
            width,
            value: masked,
            region: region_name,
        };
        self.record_completed_access(record);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct CollectingObserver(Rc<RefCell<Vec<BusAccessRecord>>>);

    impl BusAccessObserver for CollectingObserver {
        fn observe(&mut self, record: &BusAccessRecord) {
            self.0.borrow_mut().push(record.clone());
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
}
