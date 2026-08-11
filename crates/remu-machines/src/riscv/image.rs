use super::*;

impl RiscVMachine {
    /// Validates an esptool application image against the ESP32-C6 second-stage
    /// bootloader's shared I/D-MMU handoff and its corresponding direct ELF.
    pub fn validate_esp32c6_boot_image(
        elf: &FirmwareImage,
        application: &EspExecutableImage,
        partition_offset: u32,
    ) -> Result<(), MachineError> {
        const CHIP_ID: u16 = 13;
        const APP_DESC_MAGIC: u32 = 0xabcd_5432;
        const MMU_PAGE_MASK: u32 = 0xffff;

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
                .checked_add(segment.flash_offset)
                .ok_or_else(|| {
                    MachineError::Esp32c6BootLayout(format!(
                        "{role} segment physical flash address overflows"
                    ))
                })?;
            if physical & MMU_PAGE_MASK != segment.address & MMU_PAGE_MASK {
                return Err(MachineError::Esp32c6BootLayout(format!(
                    "{role} segment physical offset {physical:#010x} and virtual address {:#010x} have different 64 KiB page offsets",
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

    /// Loads a parsed direct-mode ELF and sets its entry point.
    pub fn load_firmware(&mut self, image: &FirmwareImage) -> Result<(), MachineError> {
        if image.architecture != FirmwareArchitecture::RiscV32 {
            return Err(MachineError::Architecture {
                target: self.target,
                actual: image.architecture,
            });
        }
        for segment in &image.segments {
            let data = if self.target == TargetId::Esp32c6 && segment.writable {
                segment
                    .data
                    .get(..segment.initialized_size)
                    .ok_or_else(|| MachineError::Load {
                        address: segment.address,
                        message: format!(
                            "ELF initialized size {} exceeds memory size {}",
                            segment.initialized_size,
                            segment.data.len()
                        ),
                    })?
            } else {
                segment.data.as_slice()
            };
            self.bus
                .load(segment.address, data)
                .map_err(|error| MachineError::Load {
                    address: segment.address,
                    message: error.to_string(),
                })?;
        }
        let entry =
            u32::try_from(image.entry).map_err(|_| MachineError::EntryRange(image.entry))?;
        self.cpu.set_pc(entry)?;
        if self.target == TargetId::Esp32c6 {
            self.esp_direct_firmware = Some(image.clone());
        }
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
        self.esp_application = Some(image.clone());
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
