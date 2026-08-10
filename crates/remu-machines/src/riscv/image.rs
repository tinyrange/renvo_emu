use super::*;

impl RiscVMachine {
    /// Loads real ESP32-C6 mask-ROM sections for low-level execution.
    ///
    /// Symbol metadata is ignored. Runtime behavior is determined exclusively
    /// by guest instructions, memory, devices, interrupts, and simulated time.
    pub fn load_boot_rom(&mut self, image: &FirmwareImage) -> Result<(), MachineError> {
        if self.target != TargetId::Esp32c6 {
            return Err(MachineError::UnsupportedTarget(self.target));
        }
        if image.architecture != FirmwareArchitecture::RiscV32 {
            return Err(MachineError::Architecture {
                target: self.target,
                actual: image.architecture,
            });
        }
        for segment in &image.segments {
            let end = segment
                .address
                .checked_add(segment.data.len() as u64)
                .ok_or_else(|| MachineError::Load {
                    address: segment.address,
                    message: "ROM section address overflow".to_owned(),
                })?;
            // The published ELF also carries host-side unwind metadata in the
            // application's 0x4200_0000 XIP window. It is not silicon ROM and
            // must never overwrite guest firmware loaded for direct execution.
            let instruction_rom = segment.address >= 0x4000_0000 && end <= 0x4005_0000;
            // Addressed sections also describe the mask ROM's retained SRAM
            // data, stack and BSS image. Direct application handoff does not
            // execute reset_rom(), so materialize that post-reset state here.
            let reserved_dram = segment.address >= 0x4086_ad08 && end <= 0x4088_0000;
            if !instruction_rom && !reserved_dram {
                continue;
            }
            let data = if reserved_dram {
                segment.data.as_slice()
            } else {
                segment
                    .data
                    .get(..segment.initialized_size)
                    .ok_or_else(|| MachineError::Load {
                        address: segment.address,
                        message: format!(
                            "initialized ROM bytes ({}) exceed segment data ({})",
                            segment.initialized_size,
                            segment.data.len()
                        ),
                    })?
            };
            if data.is_empty() {
                continue;
            }
            self.bus
                .load(segment.address, data)
                .map_err(|error| MachineError::Load {
                    address: segment.address,
                    message: error.to_string(),
                })?;
        }
        self.boot_rom_loaded = true;
        Ok(())
    }

    /// Validates an esptool application image against the ESP32-C6 second-stage
    /// bootloader's shared I/D-MMU handoff and its corresponding direct ELF.
    pub fn validate_esp32c6_boot_image(
        elf: &FirmwareImage,
        application: &EspExecutableImage,
        partition_offset: u32,
    ) -> Result<(), MachineError> {
        const CHIP_ID: u16 = 13;
        const APP_DESC_MAGIC: u32 = 0xabcd_5432;

        if elf.architecture != FirmwareArchitecture::RiscV32 {
            return Err(MachineError::Esp32c6BootLayout(format!(
                "ELF architecture is {:?}, expected RISC-V",
                elf.architecture
            )));
        }
        if application.header.chip_id != CHIP_ID {
            return Err(MachineError::Esp32c6BootLayout(format!(
                "image chip ID is {}, expected {CHIP_ID}",
                application.header.chip_id
            )));
        }
        if u64::from(application.header.entry) != elf.entry {
            return Err(MachineError::Esp32c6BootLayout(format!(
                "image entry {:#010x} differs from ELF entry {:#010x}",
                application.header.entry, elf.entry
            )));
        }

        let mapped = application
            .segments
            .iter()
            .filter(|segment| is_esp32c6_flash_mapped(segment.address))
            .collect::<Vec<_>>();
        if mapped.len() != 2 {
            return Err(MachineError::Esp32c6BootLayout(format!(
                "shared I/D-MMU handoff requires exactly two mapped segments (descriptor then text), found {}",
                mapped.len()
            )));
        }
        let descriptor = mapped[0];
        let text = mapped[1];
        let descriptor_magic = descriptor
            .data
            .get(..4)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u32::from_le_bytes);
        if descriptor.data.len() < 256 || descriptor_magic != Some(APP_DESC_MAGIC) {
            return Err(MachineError::Esp32c6BootLayout(
                "first mapped segment does not begin with a complete esp_app_desc_t".to_owned(),
            ));
        }
        let page_size = Self::esp32c6_image_mmu_page_size(application)?;
        let mmu_page_mask = page_size - 1;
        let entry_segment = application
            .segments
            .iter()
            .find(|segment| {
                let Ok(length) = u32::try_from(segment.data.len()) else {
                    return false;
                };
                segment.address <= application.header.entry
                    && application.header.entry < segment.address.saturating_add(length)
            })
            .ok_or_else(|| {
                MachineError::Esp32c6BootLayout(format!(
                    "entry {:#010x} is not contained in an application segment",
                    application.header.entry
                ))
            })?;
        if is_esp32c6_flash_mapped(application.header.entry) && !std::ptr::eq(entry_segment, text) {
            return Err(MachineError::Esp32c6BootLayout(format!(
                "mapped entry {:#010x} is not in the second mapped text segment",
                application.header.entry
            )));
        }

        for (role, segment) in [("descriptor", descriptor), ("text", text)] {
            let physical = partition_offset
                .checked_add(segment.flash_offset - application.flash_offset)
                .ok_or_else(|| {
                    MachineError::Esp32c6BootLayout(format!(
                        "{role} segment physical flash address overflows"
                    ))
                })?;
            if physical & mmu_page_mask != segment.address & mmu_page_mask {
                return Err(MachineError::Esp32c6BootLayout(format!(
                    "{role} segment physical offset {physical:#010x} and virtual address {:#010x} have different {page_size} byte page offsets",
                    segment.address
                )));
            }
        }

        let Some(elf_text) = elf.segments.iter().find(|segment| {
            let Ok(length) = u64::try_from(segment.data.len()) else {
                return false;
            };
            segment.executable
                && segment.address <= elf.entry
                && elf.entry < segment.address.saturating_add(length)
        }) else {
            return Err(MachineError::Esp32c6BootLayout(
                "ELF entry is not contained in an executable load segment".to_owned(),
            ));
        };
        let elf_offset = usize::try_from(elf.entry - elf_text.address)
            .expect("validated ELF entry offset fits usize");
        let image_offset =
            usize::try_from(u64::from(application.header.entry - entry_segment.address))
                .expect("validated image entry offset fits usize");
        let compare_length = 64
            .min(elf_text.data.len().saturating_sub(elf_offset))
            .min(entry_segment.data.len().saturating_sub(image_offset));
        if compare_length == 0
            || elf_text.data[elf_offset..elf_offset + compare_length]
                != entry_segment.data[image_offset..image_offset + compare_length]
        {
            return Err(MachineError::Esp32c6BootLayout(
                "application text at the entry point does not match the ELF".to_owned(),
            ));
        }
        Ok(())
    }

    /// Returns the MMU page size recorded in an ESP-IDF C6 application descriptor.
    pub fn esp32c6_image_mmu_page_size(
        application: &EspExecutableImage,
    ) -> Result<u32, MachineError> {
        const APP_DESC_MMU_PAGE_SIZE_OFFSET: usize = 180;
        let descriptor = application
            .segments
            .iter()
            .find(|segment| is_esp32c6_flash_mapped(segment.address))
            .ok_or_else(|| {
                MachineError::Esp32c6BootLayout(
                    "application has no mapped descriptor segment".to_owned(),
                )
            })?;
        let exponent = *descriptor
            .data
            .get(APP_DESC_MMU_PAGE_SIZE_OFFSET)
            .ok_or_else(|| {
                MachineError::Esp32c6BootLayout(
                    "application descriptor omits the MMU page-size field".to_owned(),
                )
            })?;
        if !(13..=16).contains(&exponent) {
            return Err(MachineError::Esp32c6BootLayout(format!(
                "application descriptor has unsupported MMU page-size exponent {exponent}"
            )));
        }
        Ok(1_u32 << exponent)
    }

    /// Configures the C6 shared instruction/data MMU page size left by the bootloader.
    pub fn configure_esp32c6_mmu_page_size(&mut self, page_size: u32) -> Result<(), MachineError> {
        if self.target != TargetId::Esp32c6 {
            return Err(MachineError::UnsupportedTarget(self.target));
        }
        let code = match page_size {
            65_536 => 0,
            32_768 => 1,
            16_384 => 2,
            8_192 => 3,
            _ => {
                return Err(MachineError::Esp32c6BootLayout(format!(
                    "unsupported C6 MMU page size {page_size}"
                )));
            }
        };
        const SPI_MEM_MMU_POWER_CTRL: u64 = 0x6000_2384;
        let current = self
            .bus
            .read(
                SPI_MEM_MMU_POWER_CTRL,
                AccessWidth::Word,
                AccessKind::Read,
                self.now,
            )
            .map_err(MachineError::Bus)?;
        self.bus
            .write(
                SPI_MEM_MMU_POWER_CTRL,
                AccessWidth::Word,
                (current & !(3 << 3)) | (code << 3),
                self.now,
            )
            .map_err(MachineError::Bus)?;
        self.esp32c6_materialized_mmu.fill(u32::MAX);
        Ok(())
    }

    /// Recreates the second-stage bootloader's cache-MMU entries for the
    /// flash-mapped segments of a validated ESP32-C6 application image.
    pub fn configure_esp32c6_boot_mappings(
        &mut self,
        application: &EspExecutableImage,
        partition_offset: u32,
    ) -> Result<(), MachineError> {
        if self.target != TargetId::Esp32c6 {
            return Err(MachineError::UnsupportedTarget(self.target));
        }
        const SPI_MEM_MMU_ITEM_CONTENT: u64 = 0x6000_237c;
        const SPI_MEM_MMU_ITEM_INDEX: u64 = 0x6000_2380;
        const MMU_VALID: u32 = 1 << 9;
        let page_size = Self::esp32c6_image_mmu_page_size(application)?;
        let page_mask = page_size - 1;
        for segment in application
            .segments
            .iter()
            .filter(|segment| is_esp32c6_flash_mapped(segment.address))
        {
            let physical = partition_offset
                .checked_add(segment.flash_offset - application.flash_offset)
                .ok_or_else(|| {
                    MachineError::Esp32c6BootLayout(
                        "application segment physical flash address overflows".to_owned(),
                    )
                })?;
            let virtual_page = (segment.address - ESP32C6_CACHE_MMU_VADDR_BASE) / page_size;
            let physical_page = physical / page_size;
            let page_count = (u32::try_from(segment.data.len())
                .expect("ESP32-C6 segment length fits u32")
                .saturating_add(segment.address & page_mask)
                .saturating_add(page_mask))
                / page_size;
            for page in 0..page_count {
                let index = virtual_page + page;
                if index >= 256 || physical_page + page >= 512 {
                    return Err(MachineError::Esp32c6BootLayout(
                        "application cache-MMU mapping exceeds the hardware table".to_owned(),
                    ));
                }
                self.bus.write(
                    SPI_MEM_MMU_ITEM_INDEX,
                    AccessWidth::Word,
                    u64::from(index),
                    self.now,
                )?;
                self.bus.write(
                    SPI_MEM_MMU_ITEM_CONTENT,
                    AccessWidth::Word,
                    u64::from(MMU_VALID | (physical_page + page)),
                    self.now,
                )?;
            }
        }
        self.refresh_esp32c6_mmu_mappings()
    }

    /// Loads a parsed direct-mode ELF and sets its entry point.
    pub fn load_firmware(&mut self, image: &FirmwareImage) -> Result<(), MachineError> {
        if image.architecture != FirmwareArchitecture::RiscV32 {
            return Err(MachineError::Architecture {
                target: self.target,
                actual: image.architecture,
            });
        }
        let executable_ranges: Vec<(u64, u64)> = image
            .segments
            .iter()
            .filter(|segment| segment.executable)
            .map(|segment| {
                (
                    segment.address,
                    segment.address.saturating_add(segment.data.len() as u64),
                )
            })
            .collect();
        for segment in &image.segments {
            // A direct ESP32-C6 handoff bypasses the second-stage bootloader
            // work that clears ELF zero-fill tails. Materialize the complete
            // PT_LOAD memory image so .bss/.noinit globals cannot retain the
            // intentionally poisoned power-on RAM pattern.
            let data = segment.data.as_slice();
            let mut chunks = vec![(0_usize, data.len())];
            if self.target == TargetId::Esp32c6 && !segment.executable {
                // ESP-IDF cache-window ELFs can contain a writable rodata LOAD
                // segment whose leading flash-address dummy overlaps the text
                // LOAD segment. That dummy is a linker-placement device, not
                // initialized bytes, and must not erase executable flash.
                for (protected_start, protected_end) in &executable_ranges {
                    let segment_start = segment.address;
                    let overlap_start = protected_start.saturating_sub(segment_start) as usize;
                    let overlap_end = protected_end.saturating_sub(segment_start) as usize;
                    chunks = chunks
                        .into_iter()
                        .flat_map(|(start, end)| {
                            let mut parts = Vec::with_capacity(2);
                            if start < overlap_start.min(end) {
                                parts.push((start, overlap_start.min(end)));
                            }
                            if overlap_end.max(start) < end {
                                parts.push((overlap_end.max(start), end));
                            }
                            parts
                        })
                        .collect();
                }
            }
            for (start, end) in chunks {
                if start == end {
                    continue;
                }
                let address = segment.address.saturating_add(start as u64);
                self.bus
                    .load(address, &data[start..end])
                    .map_err(|error| MachineError::Load {
                        address,
                        message: error.to_string(),
                    })?;
            }
        }
        let entry =
            u32::try_from(image.entry).map_err(|_| MachineError::EntryRange(image.entry))?;
        if self.target == TargetId::Esp32c6 {
            // Direct ESP-IDF application ELFs enter after the mask-ROM and
            // second-stage bootloader handoff. Compiler probes commonly set
            // SP themselves, while an unmodified IDF entry immediately saves
            // registers to the ROM-provided application stack.
            self.initialize_esp32c6_direct_handoff(image)?;
        }
        self.cpu.set_pc(entry)?;
        Ok(())
    }

    fn initialize_esp32c6_direct_handoff(
        &mut self,
        image: &FirmwareImage,
    ) -> Result<(), MachineError> {
        // The mask ROM owns this reserved SRAM page. A real reset initializes
        // its Wi-Fi/coexistence callback pointers and bookkeeping before the
        // second-stage bootloader hands control to the application. Direct ELF
        // mode must reproduce that reset state explicitly because normal
        // application .bss does not cover ROM-owned data.
        const ROM_RESERVED_DATA: u64 = 0x4087_f000;
        self.bus
            .load(ROM_RESERVED_DATA, &[0; 0x1000])
            .map_err(|error| MachineError::Load {
                address: ROM_RESERVED_DATA,
                message: error.to_string(),
            })?;
        if let Some(rodata_start) = image
            .symbols
            .iter()
            .find(|symbol| symbol.name == "_rodata_start")
            .map(|symbol| symbol.address)
        {
            let entry =
                u32::try_from(image.entry).map_err(|_| MachineError::EntryRange(image.entry))?;
            let mut header = vec![0xe9, 5, 2, 0x10];
            header.extend_from_slice(&entry.to_le_bytes());
            header.extend_from_slice(&[0xee, 0, 0, 0]);
            header.extend_from_slice(&13_u16.to_le_bytes());
            header.push(0);
            header.extend_from_slice(&0_u16.to_le_bytes());
            header.extend_from_slice(&0x63_u16.to_le_bytes());
            header.extend_from_slice(&[0; 4]);
            header.push(1);
            let rodata_length = image
                .segments
                .iter()
                .filter(|segment| !segment.executable && segment.address <= rodata_start)
                .filter_map(|segment| {
                    segment
                        .address
                        .checked_add(segment.data.len() as u64)
                        .and_then(|end| end.checked_sub(rodata_start))
                })
                .max()
                .unwrap_or(0);
            header.extend_from_slice(
                &u32::try_from(rodata_start)
                    .map_err(|_| {
                        MachineError::BootBlock("ESP-IDF rodata address exceeds RV32".into())
                    })?
                    .to_le_bytes(),
            );
            header.extend_from_slice(
                &u32::try_from(rodata_length)
                    .map_err(|_| {
                        MachineError::BootBlock("ESP-IDF rodata length exceeds RV32".into())
                    })?
                    .to_le_bytes(),
            );
            let address = rodata_start
                .checked_sub(32)
                .ok_or_else(|| MachineError::BootBlock("invalid ESP-IDF rodata start".into()))?;
            self.bus
                .load(address, &header)
                .map_err(|error| MachineError::Load {
                    address,
                    message: error.to_string(),
                })?;
        }
        const ROM_FLASH_DATA: u64 = 0x4087_fb00;
        const ROM_FLASH_DATA_POINTER: u64 = 0x4087_ffec;
        let mut descriptor = Vec::with_capacity(28);
        for word in [
            0x0016_40c8_u32,
            4 * 1024 * 1024,
            64 * 1024,
            4 * 1024,
            256,
            0xffff,
            0,
        ] {
            descriptor.extend_from_slice(&word.to_le_bytes());
        }
        self.bus
            .load(ROM_FLASH_DATA, &descriptor)
            .map_err(|error| MachineError::Load {
                address: ROM_FLASH_DATA,
                message: error.to_string(),
            })?;
        self.bus
            .load(
                ROM_FLASH_DATA_POINTER,
                &(ROM_FLASH_DATA as u32).to_le_bytes(),
            )
            .map_err(|error| MachineError::Load {
                address: ROM_FLASH_DATA_POINTER,
                message: error.to_string(),
            })?;

        const ROM_LAYOUT: u64 = 0x4004_ff00;
        const ROM_LAYOUT_POINTER: u64 = 0x4004_fffc;
        const ROM_RESERVED_DRAM_START: u32 = 0x4087_e000;
        let mut layout = vec![0_u8; 30 * 4];
        for (index, word) in [
            ROM_RESERVED_DRAM_START,
            ROM_RESERVED_DRAM_START,
            0x4087_e600,
            0x4087_e610,
        ]
        .into_iter()
        .enumerate()
        {
            layout[index * 4..index * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        self.bus
            .load(ROM_LAYOUT, &layout)
            .map_err(|error| MachineError::Load {
                address: ROM_LAYOUT,
                message: error.to_string(),
            })?;
        self.bus
            .load(ROM_LAYOUT_POINTER, &(ROM_LAYOUT as u32).to_le_bytes())
            .map_err(|error| MachineError::Load {
                address: ROM_LAYOUT_POINTER,
                message: error.to_string(),
            })?;
        self.bus
            .load(u64::from(ESP_ROM_COEX_VERSION), b"remu-c6-functional\0")
            .map_err(|error| MachineError::Load {
                address: u64::from(ESP_ROM_COEX_VERSION),
                message: error.to_string(),
            })?;

        const ROM_FLASH_API: u64 = 0x4087_f800;
        const ROM_FLASH_NAME: u64 = 0x4087_f6e0;
        self.bus
            .load(ROM_FLASH_NAME, b"GD25Q32-functional\0")
            .map_err(|error| MachineError::Load {
                address: ROM_FLASH_NAME,
                message: error.to_string(),
            })?;
        self.write_guest_words(
            ROM_FLASH_API as u32,
            &[
                ESP_ROM_FLASH_START_STUB,
                ESP_ROM_FLASH_END_STUB,
                ESP_ROM_FLASH_CHIP_CHECK_STUB,
            ],
        )
        .map_err(MachineError::BootBlock)?;
        let mut driver = vec![0_u8; 128];
        driver[0..4].copy_from_slice(&(ROM_FLASH_NAME as u32).to_le_bytes());
        driver[16..20].copy_from_slice(&ESP_ROM_FLASH_DETECT_SIZE_STUB.to_le_bytes());
        driver[0x58..0x5c].copy_from_slice(&ESP_ROM_FLASH_OK_STUB.to_le_bytes());
        self.bus
            .load(u64::from(ESP_ROM_FLASH_DRIVER), &driver)
            .map_err(|error| MachineError::Load {
                address: u64::from(ESP_ROM_FLASH_DRIVER),
                message: error.to_string(),
            })?;
        self.write_guest_words(
            ESP_ROM_DEFAULT_FLASH,
            &[
                ESP_ROM_FLASH_HOST,
                ESP_ROM_FLASH_DRIVER,
                0,
                0,
                2,
                4 * 1024 * 1024,
                0x0016_40c8,
                0,
            ],
        )
        .map_err(MachineError::BootBlock)?;
        self.write_guest_words(0x4087_ffe4, &[ROM_FLASH_API as u32, ESP_ROM_DEFAULT_FLASH])
            .map_err(MachineError::BootBlock)?;
        const ROM_TLSF_TABLE: u64 = 0x4087_f600;
        self.bus
            .load(ROM_TLSF_TABLE, &[0; 20 * 4])
            .map_err(|error| MachineError::Load {
                address: ROM_TLSF_TABLE,
                message: error.to_string(),
            })?;
        self.write_guest_words(0x4087_ffd8, &[ROM_TLSF_TABLE as u32])
            .map_err(MachineError::BootBlock)?;
        // The second-stage loader enters with ROM-owned singleton pointers
        // reset. ESP-IDF uses a null coexistence adapter pointer to detect
        // and register its application-provided callback table.
        self.write_guest_words(0x4087_ffb4, &[0])
            .map_err(MachineError::BootBlock)?;
        self.cpu.set_register(RiscVRegister::Sp, 0x4087_e610)?;
        Ok(())
    }

    /// Loads an ESP32-C6 LP-core ELF into the retained 16 KiB LP SRAM.
    /// Execution begins when an enabled PMU wake source triggers the LP core.
    pub fn load_esp32c6_lp_firmware(&mut self, image: &FirmwareImage) -> Result<(), MachineError> {
        if self.target != TargetId::Esp32c6 || image.architecture != FirmwareArchitecture::RiscV32 {
            return Err(MachineError::Architecture {
                target: self.target,
                actual: image.architecture,
            });
        }
        for segment in &image.segments {
            let end = segment.address.saturating_add(segment.data.len() as u64);
            if segment.address < 0x5000_0000 || end > 0x5000_4000 {
                return Err(MachineError::Load {
                    address: segment.address,
                    message: "ESP32-C6 LP firmware must fit in 0x50000000..0x50004000".to_owned(),
                });
            }
            self.bus
                .load(segment.address, &segment.data)
                .map_err(|error| MachineError::Load {
                    address: segment.address,
                    message: error.to_string(),
                })?;
        }
        let entry =
            u32::try_from(image.entry).map_err(|_| MachineError::EntryRange(image.entry))?;
        self.cpu1.set_pc(entry)?;
        Ok(())
    }

    /// Retains the complete merged flash artifact for ROM flash and mmap APIs.
    pub fn set_esp_flash_image(&mut self, bytes: &[u8]) {
        self.esp_flash.clear();
        self.esp_flash.extend_from_slice(bytes);
        self.esp_flash.resize(4 * 1024 * 1024, 0xff);
        self.esp32c6_materialized_mmu.fill(u32::MAX);
        self.esp32c6_flash_dirty = false;
    }

    /// Returns the complete mutable SPI-flash state for persistence.
    pub fn esp_flash_image(&self) -> &[u8] {
        &self.esp_flash
    }

    pub(super) fn refresh_esp32c6_cache(
        &mut self,
        virtual_address: u32,
        size: u32,
    ) -> Result<(), MachineError> {
        if self.esp_flash.is_empty() {
            return Ok(());
        }
        let requested = virtual_address
            .checked_sub(ESP_FUNCTIONAL_MMAP_BASE)
            .filter(|offset| (*offset as usize) < self.esp_flash.len());
        let (start, end) = if let Some(start) = requested {
            let start = start as usize;
            let end = start
                .saturating_add(size.max(1) as usize)
                .min(self.esp_flash.len());
            (start, end)
        } else {
            (0, self.esp_flash.len())
        };
        self.bus
            .load(
                u64::from(ESP_FUNCTIONAL_MMAP_BASE) + start as u64,
                &self.esp_flash[start..end],
            )
            .map_err(|error| MachineError::Load {
                address: u64::from(ESP_FUNCTIONAL_MMAP_BASE) + start as u64,
                message: error.to_string(),
            })
    }

    /// Materializes valid entries from the C6 SPI-memory MMU's indirect table.
    pub(super) fn refresh_esp32c6_mmu_mappings(&mut self) -> Result<(), MachineError> {
        const SPI_MEM_MMU_POWER_CTRL: u64 = 0x6000_2384;
        const MMU_VALID: u32 = 1 << 9;
        const MMU_PAGE_MASK: u32 = 0x1ff;
        if self.esp_flash.is_empty() {
            return Ok(());
        }
        let mappings = self
            .esp_c6_spimem_mmu
            .as_ref()
            .map_or_else(Vec::new, EspSpiMemMmuHandle::drain_mappings);
        if mappings.is_empty() && !self.esp32c6_flash_dirty {
            return Ok(());
        }
        let page_code = (self.bus.read(
            SPI_MEM_MMU_POWER_CTRL,
            AccessWidth::Word,
            AccessKind::Read,
            self.now,
        )? >> 3)
            & 3;
        let page_size = 65_536_usize >> page_code;
        for (index, entry) in mappings {
            let materialized = &mut self.esp32c6_materialized_mmu[index];
            if *materialized == entry {
                continue;
            }
            *materialized = entry;
            if entry & MMU_VALID == 0 {
                continue;
            }
            let physical = usize::try_from(entry & MMU_PAGE_MASK)
                .expect("C6 MMU physical page fits usize")
                * page_size;
            let end = physical.saturating_add(page_size).min(self.esp_flash.len());
            if physical >= end {
                continue;
            }
            let virtual_address = u64::from(ESP32C6_CACHE_MMU_VADDR_BASE)
                + u64::try_from(index).expect("C6 MMU index fits u64") * page_size as u64;
            self.bus
                .load(virtual_address, &self.esp_flash[physical..end])
                .map_err(|error| MachineError::Load {
                    address: virtual_address,
                    message: error.to_string(),
                })?;
        }
        if self.esp32c6_flash_dirty {
            self.esp32c6_flash_dirty = false;
            for (index, entry) in self.esp32c6_materialized_mmu.into_iter().enumerate() {
                if entry & MMU_VALID == 0 {
                    continue;
                }
                let physical = usize::try_from(entry & MMU_PAGE_MASK)
                    .expect("C6 MMU physical page fits usize")
                    * page_size;
                let end = physical.saturating_add(page_size).min(self.esp_flash.len());
                if physical >= end {
                    continue;
                }
                let virtual_address = u64::from(ESP32C6_CACHE_MMU_VADDR_BASE)
                    + u64::try_from(index).expect("C6 MMU index fits u64") * page_size as u64;
                self.bus
                    .load(virtual_address, &self.esp_flash[physical..end])
                    .map_err(|error| MachineError::Load {
                        address: virtual_address,
                        message: error.to_string(),
                    })?;
            }
        }
        Ok(())
    }

    /// Performs the documented ESP ROM verified-image handoff to an ESP32-C6
    /// application. Mapped flash segments remain backed by the official image,
    /// while load segments are copied to their declared RAM addresses.
    pub fn load_esp_application(&mut self, image: &EspFlashImage) -> Result<(), MachineError> {
        if self.target != TargetId::Esp32c6 {
            return Err(MachineError::UnsupportedTarget(self.target));
        }
        for segment in &image.application.segments {
            self.bus
                .load(u64::from(segment.address), &segment.data)
                .map_err(|error| MachineError::Load {
                    address: u64::from(segment.address),
                    message: error.to_string(),
                })?;
            if (0x4200_0000..0x4400_0000).contains(&segment.address) {
                let mut segment_header = Vec::with_capacity(8);
                segment_header.extend_from_slice(&segment.address.to_le_bytes());
                segment_header.extend_from_slice(&(segment.data.len() as u32).to_le_bytes());
                self.bus
                    .load(u64::from(segment.address - 8), &segment_header)
                    .map_err(|error| MachineError::Load {
                        address: u64::from(segment.address - 8),
                        message: error.to_string(),
                    })?;
            }
        }
        // ESP-IDF deliberately reads the application header through the
        // bytes immediately preceding the first mapped DROM segment. Preserve
        // that cache-window relationship for the direct verified-image
        // handoff.
        if let Some(first) = image.application.segments.first() {
            let header = &image.application.header;
            let mut encoded = vec![
                0xe9,
                header.segment_count,
                header.flash_mode,
                header.flash_size_frequency,
            ];
            encoded.extend_from_slice(&header.entry.to_le_bytes());
            encoded.push(header.write_protect_pin);
            encoded.extend_from_slice(&header.drive_settings);
            encoded.extend_from_slice(&header.chip_id.to_le_bytes());
            encoded.push(header.minimum_revision_legacy);
            encoded.extend_from_slice(&header.minimum_revision.to_le_bytes());
            encoded.extend_from_slice(&header.maximum_revision.to_le_bytes());
            encoded.extend_from_slice(&[0; 4]);
            encoded.push(u8::from(header.hash_appended));
            self.bus
                .load(u64::from(first.address - 32), &encoded)
                .map_err(|error| MachineError::Load {
                    address: u64::from(first.address - 32),
                    message: error.to_string(),
                })?;
        }
        // The mask ROM leaves a small flash descriptor behind its fixed
        // compatibility pointer. The second-stage loader starts the
        // application on the ROM stack below the interface-data window.
        const ROM_FLASH_DATA: u64 = 0x4087_fb00;
        const ROM_FLASH_DATA_POINTER: u64 = 0x4087_ffec;
        let mut descriptor = Vec::with_capacity(28);
        for word in [
            0x0016_40c8_u32,
            4 * 1024 * 1024,
            64 * 1024,
            4 * 1024,
            256,
            0xffff,
            0,
        ] {
            descriptor.extend_from_slice(&word.to_le_bytes());
        }
        self.bus
            .load(ROM_FLASH_DATA, &descriptor)
            .map_err(|error| MachineError::Load {
                address: ROM_FLASH_DATA,
                message: error.to_string(),
            })?;
        self.bus
            .load(
                ROM_FLASH_DATA_POINTER,
                &(ROM_FLASH_DATA as u32).to_le_bytes(),
            )
            .map_err(|error| MachineError::Load {
                address: ROM_FLASH_DATA_POINTER,
                message: error.to_string(),
            })?;
        // The mask ROM publishes its retained-DRAM reservation through a
        // fixed pointer. ESP-IDF consumes this while constructing the heap
        // region table. Keep the layout record in an otherwise unused tail of
        // the mask-ROM data window.
        const ROM_LAYOUT: u64 = 0x4004_ff00;
        const ROM_LAYOUT_POINTER: u64 = 0x4004_fffc;
        const ROM_RESERVED_DRAM_START: u32 = 0x4087_e000;
        let mut layout = vec![0_u8; 30 * 4];
        for (index, word) in [
            ROM_RESERVED_DRAM_START,
            ROM_RESERVED_DRAM_START,
            0x4087_e600,
            0x4087_e610,
        ]
        .into_iter()
        .enumerate()
        {
            layout[index * 4..index * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        self.bus
            .load(ROM_LAYOUT, &layout)
            .map_err(|error| MachineError::Load {
                address: ROM_LAYOUT,
                message: error.to_string(),
            })?;
        self.bus
            .load(ROM_LAYOUT_POINTER, &(ROM_LAYOUT as u32).to_le_bytes())
            .map_err(|error| MachineError::Load {
                address: ROM_LAYOUT_POINTER,
                message: error.to_string(),
            })?;
        self.bus
            .load(u64::from(ESP_ROM_COEX_VERSION), b"remu-c6-functional\0")
            .map_err(|error| MachineError::Load {
                address: u64::from(ESP_ROM_COEX_VERSION),
                message: error.to_string(),
            })?;
        const ROM_FLASH_API: u64 = 0x4087_f800;
        const ROM_FLASH_NAME: u64 = 0x4087_f6e0;
        self.bus
            .load(ROM_FLASH_NAME, b"GD25Q32-functional\0")
            .map_err(|error| MachineError::Load {
                address: ROM_FLASH_NAME,
                message: error.to_string(),
            })?;
        self.write_guest_words(
            ROM_FLASH_API as u32,
            &[
                ESP_ROM_FLASH_START_STUB,
                ESP_ROM_FLASH_END_STUB,
                ESP_ROM_FLASH_CHIP_CHECK_STUB,
            ],
        )
        .map_err(MachineError::BootBlock)?;
        let mut driver = vec![0_u8; 128];
        driver[0..4].copy_from_slice(&(ROM_FLASH_NAME as u32).to_le_bytes());
        driver[16..20].copy_from_slice(&ESP_ROM_FLASH_DETECT_SIZE_STUB.to_le_bytes());
        driver[0x58..0x5c].copy_from_slice(&ESP_ROM_FLASH_OK_STUB.to_le_bytes());
        self.bus
            .load(u64::from(ESP_ROM_FLASH_DRIVER), &driver)
            .map_err(|error| MachineError::Load {
                address: u64::from(ESP_ROM_FLASH_DRIVER),
                message: error.to_string(),
            })?;
        let default_chip = [
            ESP_ROM_FLASH_HOST,
            ESP_ROM_FLASH_DRIVER,
            0,
            0,
            2,
            4 * 1024 * 1024,
            0x0016_40c8,
            0,
        ];
        self.write_guest_words(ESP_ROM_DEFAULT_FLASH, &default_chip)
            .map_err(MachineError::BootBlock)?;
        self.write_guest_words(0x4087_ffe4, &[ROM_FLASH_API as u32, ESP_ROM_DEFAULT_FLASH])
            .map_err(MachineError::BootBlock)?;
        const ROM_TLSF_TABLE: u64 = 0x4087_f600;
        self.bus
            .load(ROM_TLSF_TABLE, &[0; 20 * 4])
            .map_err(|error| MachineError::Load {
                address: ROM_TLSF_TABLE,
                message: error.to_string(),
            })?;
        self.write_guest_words(0x4087_ffd8, &[ROM_TLSF_TABLE as u32])
            .map_err(MachineError::BootBlock)?;
        self.cpu.set_register(RiscVRegister::Sp, 0x4087_e610)?;
        self.cpu.set_pc(image.application.header.entry)?;
        Ok(())
    }

    /// Loads raw instructions/data into an already mapped memory range.
    pub fn load_bytes(&mut self, address: u64, bytes: &[u8]) -> Result<(), MachineError> {
        self.bus
            .load(address, bytes)
            .map_err(|error| MachineError::Load {
                address,
                message: error.to_string(),
            })
    }

    /// Sets a direct-mode entry without parsing an ELF.
    pub fn set_entry(&mut self, entry: u32) -> Result<(), MachineError> {
        self.cpu.set_pc(entry)?;
        Ok(())
    }

    /// Queues bytes for delivery after the functional USB host enumerates CDC.
    pub fn queue_usb_input(&mut self, bytes: &[u8]) {
        if let Some(host) = &mut self.usb_host {
            host.queue_input(bytes);
        }
        if let Some(usb) = &self.esp_usb_serial_jtag {
            usb.queue_input(bytes);
        }
    }

    /// Selects whether the ESP USB Serial/JTAG host is attached.
    ///
    /// The host is connected by default. When connected, the peripheral
    /// asserts its SOF raw interrupt every fixed abstract USB frame period;
    /// disconnected mode is useful for testing non-blocking console paths.
    pub fn set_usb_host_connected(&mut self, connected: bool) {
        if let Some(usb) = &self.esp_usb_serial_jtag {
            usb.set_host_connected(connected, self.now);
        }
    }

    /// Stops a bounded run once all queued USB input returns to the raw-REPL prompt.
    pub fn stop_on_usb_input_complete(&mut self, enabled: bool) {
        self.stop_on_usb_input_complete = enabled;
    }

    /// Loads the official RP2350 RISC-V UF2 and performs its image-definition handoff.
    pub fn load_rp2350_riscv_uf2(&mut self, image: &Uf2Image) -> Result<(), MachineError> {
        const FAMILY: u32 = 0xe48b_ff5a;
        if self.target != TargetId::Rp2350 {
            return Err(MachineError::UnsupportedTarget(self.target));
        }
        let actual = image.family_id.unwrap_or_default();
        if actual != FAMILY {
            return Err(MachineError::Uf2Family {
                expected: FAMILY,
                actual,
            });
        }
        let materialized = image.materialize(0x1000_0000, 16 * 1024 * 1024, 0xff)?;
        for segment in &image.segments {
            self.bus
                .load(u64::from(segment.address), &segment.data)
                .map_err(|error| MachineError::Load {
                    address: u64::from(segment.address),
                    message: error.to_string(),
                })?;
        }
        let entry = materialized
            .get(0x20..0x24)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u32::from_le_bytes)
            .ok_or_else(|| MachineError::BootBlock("missing entry point at offset 0x20".into()))?;
        let stack = materialized
            .get(0x24..0x28)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u32::from_le_bytes)
            .ok_or_else(|| {
                MachineError::BootBlock("missing initial stack at offset 0x24".into())
            })?;
        if !(0x1000_0000..0x1100_0000).contains(&entry) || entry & 1 != 0 {
            return Err(MachineError::BootBlock(format!(
                "entry {entry:#010x} is not aligned XIP code"
            )));
        }
        if !(0x2000_0000..=0x2008_2000).contains(&stack) || stack & 3 != 0 {
            return Err(MachineError::BootBlock(format!(
                "stack {stack:#010x} is outside SRAM"
            )));
        }
        self.cpu.set_register(RiscVRegister::Sp, stack)?;
        self.cpu.set_pc(entry)?;
        Ok(())
    }

    /// Replaces the complete persistent RP2350 XIP flash backing before the
    /// official UF2 overlay is applied.
    pub fn set_rp_flash_image(&self, bytes: &[u8]) -> Result<(), MachineError> {
        if self.target != TargetId::Rp2350 {
            return Err(MachineError::UnsupportedTarget(self.target));
        }
        let storage = self
            .flash_storage
            .as_ref()
            .expect("RP2350 target has XIP flash storage");
        if bytes.len() != storage.len() {
            return Err(MachineError::BootBlock(format!(
                "persistent flash image is {} bytes; expected {}",
                bytes.len(),
                storage.len()
            )));
        }
        if !storage.write_range(0, bytes) {
            return Err(MachineError::BootBlock(
                "persistent flash backing rejected a full-image update".to_owned(),
            ));
        }
        Ok(())
    }

    /// Copies the complete mutable RP2350 XIP flash state for persistence.
    pub fn rp_flash_image(&self) -> Result<Vec<u8>, MachineError> {
        if self.target != TargetId::Rp2350 {
            return Err(MachineError::UnsupportedTarget(self.target));
        }
        Ok(self
            .flash_storage
            .as_ref()
            .expect("RP2350 target has XIP flash storage")
            .to_vec())
    }
}

fn is_esp32c6_flash_mapped(address: u32) -> bool {
    (0x4200_0000..0x4300_0000).contains(&address)
}
