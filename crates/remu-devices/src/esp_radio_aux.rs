/// ESP32-C6 analog-register I²C master and its internal byte registers.
///
/// ESP-IDF accesses calibration and regulator state by writing packed
/// slave/address/data commands to the two master control words. Commands
/// complete synchronously in the functional model.
pub struct EspAnalogI2c {
    name: String,
    registers: Vec<u32>,
    analog: BTreeMap<(u8, u8), u8>,
}

impl EspAnalogI2c {
    /// Creates a reset analog I²C master.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            registers: vec![0; 0x1000 / 4],
            analog: BTreeMap::new(),
        }
    }
}

impl Device for EspAnalogI2c {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP analog I2C requires aligned word access",
            ));
        }
        let index = usize::try_from(offset / 4).expect("analog-I2C offset fits");
        self.registers
            .get(index)
            .copied()
            .map(u64::from)
            .ok_or_else(|| DeviceError::new(format!("{} read at {offset:#x}", self.name)))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP analog I2C requires aligned word access",
            ));
        }
        let index = usize::try_from(offset / 4).expect("analog-I2C offset fits");
        let command = value as u32;
        if index >= self.registers.len() {
            return Err(DeviceError::new(format!(
                "{} write at {offset:#x}",
                self.name
            )));
        }
        self.registers[index] = command;

        let slave = command as u8;
        if matches!(offset, 0x04 | 0x08) || offset == u64::from(slave) * 4 {
            let address = (command >> 8) as u8;
            if command & (1 << 24) != 0 {
                let data = (command >> 16) as u8;
                self.analog.insert((slave, address), data);
                // A completed BBPLL configuration makes the hardware
                // calibration-done status visible in I2C_MST_ANA_CONF0.
                // Functional time completes the calibration synchronously.
                if slave == 0x66 {
                    self.registers[0x18 / 4] |= 1 << 24;
                }
                // The C6 RFPLL charge-pump calibration starts through bit
                // five of slave 0x62 register 15. Register 14 reports its
                // completion in bit seven and the result in bits 4:0.
                if slave == 0x62 && address == 0x0f && data & (1 << 5) != 0 {
                    self.analog
                        .entry((slave, 0x0e))
                        .and_modify(|status| *status |= 1 << 7)
                        .or_insert(1 << 7);
                }
                // Releasing the ULP analog reset completes the deterministic
                // O-code and band-gap calibration.
                if slave == 0x61 && address == 0 && data & 1 != 0 {
                    self.analog
                        .entry((0x61, 3))
                        .and_modify(|value| *value |= 0x09)
                        .or_insert(0x09);
                }
            } else {
                let data = self.analog.get(&(slave, address)).copied().unwrap_or(0);
                self.registers[index] = (command & !(0xff << 16)) | (u32::from(data) << 16);
            }
            self.registers[index] &= !(1 << 25);
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.registers.fill(0);
        self.analog.clear();
    }
}

/// Functional ESP SPI-memory controller command window.
///
/// User commands complete synchronously. The facade currently exposes the
/// identification/status responses needed to discover a conventional 4 MiB
/// JEDEC flash; memory-mapped application bytes remain owned by the machine's
/// flash mapping.
pub struct EspSpiMem {
    name: String,
    registers: Vec<u32>,
    jedec_id: u32,
    write_enabled: bool,
    mmu_index: u8,
    mmu_items: [u32; 256],
    mmu_pending: Arc<Mutex<Vec<(usize, u32)>>>,
    mmu_dirty: Arc<AtomicBool>,
    flash_commands: Arc<Mutex<VecDeque<EspSpiFlashCommand>>>,
    flash_read_data: Arc<Mutex<Vec<u8>>>,
}

/// One transaction issued by the ESP SPI-memory user-command engine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EspSpiFlashCommand {
    /// Reads `length` bytes beginning at the physical byte `address`.
    Read {
        /// Physical flash byte address.
        address: u32,
        /// Number of bytes requested through the controller data window.
        length: usize,
    },
    /// Programs bytes using NOR one-to-zero semantics.
    Program {
        /// Physical flash byte address.
        address: u32,
        /// Bytes staged by firmware in W0..W15.
        data: Vec<u8>,
    },
    /// Erases the 4 KiB sector beginning at `address`.
    EraseSector {
        /// Aligned physical flash byte address.
        address: u32,
    },
}

/// Observation handle for ESP32-C6 indirect cache-MMU updates.
#[derive(Clone)]
pub struct EspSpiMemMmuHandle {
    pending: Arc<Mutex<Vec<(usize, u32)>>>,
    dirty: Arc<AtomicBool>,
    flash_commands: Arc<Mutex<VecDeque<EspSpiFlashCommand>>>,
    flash_read_data: Arc<Mutex<Vec<u8>>>,
}

impl EspSpiMemMmuHandle {
    /// Drains MMU entries written since the preceding observation.
    pub fn drain_mappings(&self) -> Vec<(usize, u32)> {
        if !self.dirty.swap(false, Ordering::AcqRel) {
            return Vec::new();
        }
        let mut pending = self.pending.lock().expect("ESP SPI MMU lock poisoned");
        std::mem::take(&mut *pending)
    }

    /// Takes the next SPI-flash transaction issued by guest firmware.
    pub fn take_flash_command(&self) -> Option<EspSpiFlashCommand> {
        self.flash_commands
            .lock()
            .expect("ESP SPI flash-command lock poisoned")
            .pop_front()
    }

    /// Makes bytes returned by a flash-read transaction visible in W0..W15.
    pub fn complete_flash_read(&self, data: Vec<u8>) {
        *self
            .flash_read_data
            .lock()
            .expect("ESP SPI flash-response lock poisoned") = data;
    }
}

impl EspSpiMem {
    /// Creates a reset SPI-memory controller.
    pub fn new(name: impl Into<String>) -> Self {
        Self::new_observed(name).0
    }

    /// Creates a controller backed by a flash part with the supplied JEDEC ID.
    pub fn new_with_jedec_id(name: impl Into<String>, jedec_id: u32) -> Self {
        Self::new_observed_with_jedec_id(name, jedec_id).0
    }

    /// Creates the controller and a handle for observing indirect MMU writes.
    pub fn new_observed(name: impl Into<String>) -> (Self, EspSpiMemMmuHandle) {
        Self::new_observed_with_jedec_id(name, 0x0016_40c8)
    }

    /// Creates an observed controller with an explicit flash JEDEC ID.
    pub fn new_observed_with_jedec_id(
        name: impl Into<String>,
        jedec_id: u32,
    ) -> (Self, EspSpiMemMmuHandle) {
        let pending = Arc::new(Mutex::new(Vec::new()));
        let dirty = Arc::new(AtomicBool::new(false));
        let flash_commands = Arc::new(Mutex::new(VecDeque::new()));
        let flash_read_data = Arc::new(Mutex::new(Vec::new()));
        let mut registers = vec![0; 0x1000 / 4];
        // On an idle ESP32-C6 MSPI controller all AXI and synchronization
        // FIFOs report empty. The mask ROM waits for the aggregate bit before
        // changing flash-controller clocks.
        registers[0x170 / 4] = 0xfc00_0000;
        (
            Self {
                name: name.into(),
                registers,
                jedec_id,
                write_enabled: false,
                mmu_index: 0,
                mmu_items: [0; 256],
                mmu_pending: pending.clone(),
                mmu_dirty: dirty.clone(),
                flash_commands: flash_commands.clone(),
                flash_read_data: flash_read_data.clone(),
            },
            EspSpiMemMmuHandle {
                pending,
                dirty,
                flash_commands,
                flash_read_data,
            },
        )
    }

    fn execute_user_command(&mut self) {
        let command = self.registers[0x20 / 4] as u8;
        self.flash_read_data
            .lock()
            .expect("ESP SPI flash-response lock poisoned")
            .clear();
        let response = match command {
            // RDID. ESP's ROM helper consumes the bytes in this
            // little-endian word order.
            0x9f => self.jedec_id,
            // RDSR / RDSR2. Flash is idle; preserve WEL while applicable.
            0x05 => u32::from(self.write_enabled) << 1,
            0x35 => 0,
            // RDSFDP returns an unavailable signature for now, causing IDF
            // to use its JEDEC-ID fallback table deterministically.
            0x5a => 0,
            0x06 => {
                self.write_enabled = true;
                0
            }
            0x04 => {
                self.write_enabled = false;
                0
            }
            // READ4IO. IDF supplies the already-decoded 24-bit byte address
            // in ADDR and consumes the response through W0..W15.
            0xbb => {
                let length = usize::try_from((self.registers[0x28 / 4] & 0x3ff) / 8 + 1)
                    .expect("SPI read length fits usize");
                self.flash_commands
                    .lock()
                    .expect("ESP SPI flash-command lock poisoned")
                    .push_back(EspSpiFlashCommand::Read {
                        address: self.registers[0x04 / 4],
                        length,
                    });
                0
            }
            // Page program. The C6 controller exposes a 64-byte W0..W15
            // window and IDF chunks larger NVS writes accordingly.
            0x02 if self.write_enabled => {
                let length = usize::try_from((self.registers[0x24 / 4] & 0x3ff) / 8 + 1)
                    .expect("SPI program length fits usize")
                    .min(64);
                let mut data = Vec::with_capacity(length);
                for word in &self.registers[0x58 / 4..=0x94 / 4] {
                    data.extend_from_slice(&word.to_le_bytes());
                }
                data.truncate(length);
                self.flash_commands
                    .lock()
                    .expect("ESP SPI flash-command lock poisoned")
                    .push_back(EspSpiFlashCommand::Program {
                        address: self.registers[0x04 / 4],
                        data,
                    });
                self.write_enabled = false;
                0
            }
            // 4 KiB sector erase.
            0x20 if self.write_enabled => {
                self.flash_commands
                    .lock()
                    .expect("ESP SPI flash-command lock poisoned")
                    .push_back(EspSpiFlashCommand::EraseSector {
                        address: self.registers[0x04 / 4],
                    });
                self.write_enabled = false;
                0
            }
            _ => 0,
        };
        self.registers[0x58 / 4] = response;
        self.registers[0] &= !(1 << 18);
    }
}

impl Device for EspSpiMem {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width == AccessWidth::DoubleWord || !width.is_aligned(offset) {
            return Err(DeviceError::new(
                "ESP SPI memory controller requires naturally aligned access",
            ));
        }
        let index = usize::try_from(offset / 4).expect("SPI-memory offset fits");
        if (0x58..=0x94).contains(&(offset & !3)) {
            let response = self
                .flash_read_data
                .lock()
                .expect("ESP SPI flash-response lock poisoned");
            if !response.is_empty() {
                let start = usize::try_from((offset & !3) - 0x58)
                    .expect("SPI response offset fits usize");
                let mut bytes = [0_u8; 4];
                for (index, byte) in bytes.iter_mut().enumerate() {
                    *byte = response.get(start + index).copied().unwrap_or(0);
                }
                let value = u32::from_le_bytes(bytes);
                let shift = (offset & 3) * 8;
                let mask = match width {
                    AccessWidth::Byte => 0xff,
                    AccessWidth::HalfWord => 0xffff,
                    AccessWidth::Word => u64::from(u32::MAX),
                    AccessWidth::DoubleWord => unreachable!("double-word access rejected"),
                };
                return Ok((u64::from(value) >> shift) & mask);
            }
        }
        let register = match offset & !3 {
            0x37c => Some(self.mmu_items[usize::from(self.mmu_index)]),
            0x380 => Some(u32::from(self.mmu_index)),
            _ => self.registers.get(index).copied(),
        };
        register
            .map(|value| {
                let shift = (offset & 3) * 8;
                let mask = match width {
                    AccessWidth::Byte => 0xff,
                    AccessWidth::HalfWord => 0xffff,
                    AccessWidth::Word => u64::from(u32::MAX),
                    AccessWidth::DoubleWord => unreachable!("double-word access rejected"),
                };
                (u64::from(value) >> shift) & mask
            })
            .ok_or_else(|| DeviceError::new(format!("{} read at {offset:#x}", self.name)))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width == AccessWidth::DoubleWord || !width.is_aligned(offset) {
            return Err(DeviceError::new(
                "ESP SPI memory controller requires naturally aligned access",
            ));
        }
        let index = usize::try_from(offset / 4).expect("SPI-memory offset fits");
        let register = self
            .registers
            .get_mut(index)
            .ok_or_else(|| DeviceError::new(format!("{} write at {offset:#x}", self.name)))?;
        let shift = ((offset & 3) * 8) as u32;
        let mask = match width {
            AccessWidth::Byte => 0xff_u32,
            AccessWidth::HalfWord => 0xffff,
            AccessWidth::Word => u32::MAX,
            AccessWidth::DoubleWord => unreachable!("double-word access rejected"),
        } << shift;
        *register = (*register & !mask) | (((value as u32) << shift) & mask);
        if offset & !3 == 0x380 {
            self.mmu_index = (*register & 0xff) as u8;
        } else if offset & !3 == 0x37c {
            let index = usize::from(self.mmu_index);
            self.mmu_items[index] = *register;
            self.mmu_pending
                .lock()
                .expect("ESP SPI MMU lock poisoned")
                .push((index, *register));
            self.mmu_dirty.store(true, Ordering::Release);
        }
        if offset & !3 == 0 {
            let command = *register;
            if command & (1 << 30) != 0 {
                self.write_enabled = true;
            }
            if command & (1 << 29) != 0 {
                self.write_enabled = false;
            }
            if command & (1 << 28) != 0 {
                self.registers[0x58 / 4] = self.jedec_id;
            }
            if command & (1 << 27) != 0 {
                self.registers[0x58 / 4] = u32::from(self.write_enabled) << 1;
            }
            if command & (1 << 18) != 0 {
                self.execute_user_command();
            }
            // Every operation trigger in CMD[31:17] is self-clearing after
            // the synchronous functional transaction completes.
            self.registers[0] &= 0x0001_ffff;
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.registers.fill(0);
        self.registers[0x170 / 4] = 0xfc00_0000;
        self.write_enabled = false;
        self.mmu_index = 0;
        self.mmu_items.fill(0);
        self.mmu_pending
            .lock()
            .expect("ESP SPI MMU lock poisoned")
            .clear();
        self.mmu_dirty.store(false, Ordering::Release);
        self.flash_commands
            .lock()
            .expect("ESP SPI flash-command lock poisoned")
            .clear();
        self.flash_read_data
            .lock()
            .expect("ESP SPI flash-response lock poisoned")
            .clear();
    }
}
