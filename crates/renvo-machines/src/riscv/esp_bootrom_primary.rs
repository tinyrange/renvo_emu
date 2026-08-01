use super::*;

impl RiscVMachine {
    pub(super) fn service_esp32c6_bootrom_primary(&mut self, pc: u32) -> Result<bool, String> {
        match pc {
            ESP_ROM_FLASH_START_STUB => {
                self.complete_host_call(0)?;
                Ok(true)
            }
            ESP_ROM_FLASH_END_STUB => {
                let result = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())?;
                self.complete_host_call(result)?;
                Ok(true)
            }
            ESP_ROM_FLASH_CHIP_CHECK_STUB => {
                let inout = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let chip = self
                    .bus
                    .read(
                        u64::from(inout),
                        renvo_core::AccessWidth::Word,
                        renvo_core::AccessKind::Read,
                        self.now,
                    )
                    .map_err(|error| error.to_string())? as u32;
                let chip = if chip == 0 {
                    self.write_guest_words(inout, &[ESP_ROM_DEFAULT_FLASH])?;
                    ESP_ROM_DEFAULT_FLASH
                } else {
                    chip
                };
                let driver = self
                    .bus
                    .read(
                        u64::from(chip + 4),
                        renvo_core::AccessWidth::Word,
                        renvo_core::AccessKind::Read,
                        self.now,
                    )
                    .map_err(|error| error.to_string())? as u32;
                if driver == 0 {
                    self.write_guest_words(
                        chip,
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
                    )?;
                }
                self.complete_host_call(0)?;
                Ok(true)
            }
            ESP_ROM_FLASH_DETECT_SIZE_STUB => {
                let output = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())?;
                self.write_guest_words(output, &[4 * 1024 * 1024])?;
                self.complete_host_call(0)?;
                Ok(true)
            }
            ESP_ROM_FLASH_OK_STUB => {
                self.complete_host_call(0)?;
                Ok(true)
            }
            0x4000_0afc => {
                self.complete_host_call(ESP_ROM_COEX_VERSION)?;
                Ok(true)
            }
            // ESP32-C6 mask-ROM rtc_get_reset_reason /
            // esp_rom_get_reset_reason. A cold functional boot reports
            // POWERON_RESET for CPU0.
            0x4000_0018 => {
                self.complete_host_call(1)?;
                Ok(true)
            }
            // ets_printf decodes the guest's RISC-V varargs and emits to
            // the functional ROM console.
            0x4000_0028 => {
                let written = self.service_esp_printf()?;
                self.complete_host_call(written)?;
                Ok(true)
            }
            // ets_delay_us / esp_rom_delay_us. Functional timing is
            // instruction ordered rather than wall-clock accurate, so
            // the delay is a deterministic ordering point.
            0x4000_0040 => {
                self.complete_host_call(0)?;
                Ok(true)
            }
            // ets_get_cpu_frequency and ets_update_cpu_frequency expose
            // the ROM's ticks-per-microsecond state.
            0x4000_0044 => {
                self.complete_host_call(self.esp_cpu_frequency_mhz)?;
                Ok(true)
            }
            0x4000_0048 => {
                self.esp_cpu_frequency_mhz = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                self.complete_host_call(0)?;
                Ok(true)
            }
            // gpio_pad_select_gpio / esp_rom_gpio_pad_select_gpio.
            // The ROM helper selects the ordinary digital IO mux for
            // one pad; the register-level GPIO model applies the
            // direction and output state written by the IDF driver.
            0x4000_0700 => {
                self.complete_host_call(0)?;
                Ok(true)
            }
            // uart_tx_wait_idle: the functional UART drains immediately.
            0x4000_0078 => {
                self.complete_host_call(0)?;
                Ok(true)
            }
            0x4000_01e4 => {
                self.esp_flash_guard = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                self.complete_host_call(0)?;
                Ok(true)
            }
            0x4000_01e8 => {
                self.complete_host_call(self.esp_flash_guard)?;
                Ok(true)
            }
            // Install the OS hooks used by ROM flash mmap services. The
            // direct mapped-image path does not need to call them while
            // boot remains single-threaded.
            0x4000_0204 | 0x4000_0208 => {
                self.complete_host_call(0)?;
                Ok(true)
            }
            0x4000_020c => {
                let source = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let length = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())? as usize;
                let output = self
                    .cpu
                    .register(RiscVRegister::A3)
                    .map_err(|error| error.to_string())?;
                let handle = self
                    .cpu
                    .register(RiscVRegister::A4)
                    .map_err(|error| error.to_string())?;
                let start = source as usize;
                let requested_end = start
                    .checked_add(length)
                    .filter(|end| *end <= self.esp_flash.len());
                if let Some(requested_end) = requested_end {
                    let page_start = start & !0xffff;
                    let page_end = requested_end
                        .saturating_add(0xffff)
                        .min(self.esp_flash.len())
                        & !0xffff;
                    let page_end = page_end.max(requested_end);
                    let mapped = ESP_FUNCTIONAL_MMAP_BASE.wrapping_add(source);
                    self.bus
                        .load(
                            u64::from(ESP_FUNCTIONAL_MMAP_BASE.wrapping_add(page_start as u32)),
                            &self.esp_flash[page_start..page_end],
                        )
                        .map_err(|error| error.to_string())?;
                    self.write_guest_words(output, &[mapped])?;
                    self.write_guest_words(handle, &[source / 0x1_0000 + 1])?;
                    self.complete_host_call(0)?;
                } else {
                    self.complete_host_call(0x102)?;
                }
                Ok(true)
            }
            0x4000_0214 | 0x4000_0218 | 0x4000_021c => {
                self.complete_host_call(0)?;
                Ok(true)
            }
            0x4000_0220 => {
                self.complete_host_call(128)?;
                Ok(true)
            }
            0x4000_0224 => {
                let cached = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let physical = cached
                    .checked_sub(ESP_FUNCTIONAL_MMAP_BASE)
                    .filter(|offset| (*offset as usize) < self.esp_flash.len())
                    .unwrap_or(u32::MAX);
                self.complete_host_call(physical)?;
                Ok(true)
            }
            0x4000_0228 => {
                let physical = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let cached = if (physical as usize) < self.esp_flash.len() {
                    ESP_FUNCTIONAL_MMAP_BASE.wrapping_add(physical)
                } else {
                    0
                };
                self.complete_host_call(cached)?;
                Ok(true)
            }
            0x4000_022c => {
                self.complete_host_call(1)?;
                Ok(true)
            }
            0x4000_0230 | 0x4000_0270 => {
                let output = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())?;
                self.write_guest_words(output, &[0x0016_40c8])?;
                self.complete_host_call(0)?;
                Ok(true)
            }
            0x4000_0234 => {
                let output = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())?;
                self.write_guest_words(output, &[4 * 1024 * 1024])?;
                self.complete_host_call(0)?;
                Ok(true)
            }
            0x4000_0238 => {
                self.esp_flash.fill(0xff);
                self.complete_host_call(0)?;
                Ok(true)
            }
            0x4000_023c => {
                let address = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())? as usize;
                let length = self
                    .cpu
                    .register(RiscVRegister::A2)
                    .map_err(|error| error.to_string())? as usize;
                let end = address
                    .checked_add(length)
                    .filter(|end| *end <= self.esp_flash.len())
                    .ok_or_else(|| {
                        format!(
                            "ESP flash erase {address:#x}..{:#x} exceeds image",
                            address.saturating_add(length)
                        )
                    })?;
                self.esp_flash[address..end].fill(0xff);
                self.complete_host_call(0)?;
                Ok(true)
            }
            0x4000_0254 | 0x4000_0260 => {
                let output = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())?;
                let address = self
                    .cpu
                    .register(RiscVRegister::A2)
                    .map_err(|error| error.to_string())? as usize;
                let length = self
                    .cpu
                    .register(RiscVRegister::A3)
                    .map_err(|error| error.to_string())? as usize;
                let end = address
                    .checked_add(length)
                    .filter(|end| *end <= self.esp_flash.len())
                    .ok_or_else(|| {
                        format!(
                            "ESP flash read {address:#x}..{:#x} exceeds image",
                            address.saturating_add(length)
                        )
                    })?;
                let bytes = self.esp_flash[address..end].to_vec();
                self.write_guest_bytes(output, &bytes)?;
                self.complete_host_call(0)?;
                Ok(true)
            }
            0x4000_0258 | 0x4000_025c => {
                let input = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())?;
                let address = self
                    .cpu
                    .register(RiscVRegister::A2)
                    .map_err(|error| error.to_string())? as usize;
                let length = self
                    .cpu
                    .register(RiscVRegister::A3)
                    .map_err(|error| error.to_string())? as usize;
                let end = address
                    .checked_add(length)
                    .filter(|end| *end <= self.esp_flash.len())
                    .ok_or_else(|| {
                        format!(
                            "ESP flash write {address:#x}..{:#x} exceeds image",
                            address.saturating_add(length)
                        )
                    })?;
                let bytes = self.read_guest_bytes(input, length)?;
                for (current, requested) in self.esp_flash[address..end]
                    .iter_mut()
                    .zip(bytes.into_iter())
                {
                    *current &= requested;
                }
                self.bus
                    .load(
                        u64::from(ESP_FUNCTIONAL_MMAP_BASE.wrapping_add(address as u32)),
                        &self.esp_flash[address..end],
                    )
                    .map_err(|error| error.to_string())?;
                self.complete_host_call(0)?;
                Ok(true)
            }
            // intr_matrix_set / esp_rom_route_intr_matrix. ESP32-C6 is
            // single-core; retain the source-to-CPU-line association for
            // deterministic interrupt delivery as peripheral models are
            // activated.
            0x4000_0730 => {
                let source = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())?;
                let cpu_interrupt = self
                    .cpu
                    .register(RiscVRegister::A2)
                    .map_err(|error| error.to_string())?;
                self.esp_interrupt_routes.insert(source, cpu_interrupt);
                self.complete_host_call(0)?;
                Ok(true)
            }
            0x4000_0718 => {
                let interrupt = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let priority = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())?;
                self.esp_interrupt_priorities.insert(interrupt, priority);
                if interrupt < 32 {
                    self.bus
                        .write(
                            u64::from(0x2000_1010_u32 + interrupt * 4),
                            renvo_core::AccessWidth::Word,
                            u64::from(priority),
                            self.now,
                        )
                        .map_err(|error| error.to_string())?;
                }
                self.complete_host_call(0)?;
                Ok(true)
            }
            0x4000_071c => {
                self.esp_interrupt_threshold = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                self.bus
                    .write(
                        0x2000_1090,
                        renvo_core::AccessWidth::Word,
                        u64::from(self.esp_interrupt_threshold),
                        self.now,
                    )
                    .map_err(|error| error.to_string())?;
                self.complete_host_call(0)?;
                Ok(true)
            }
            0x4000_0720 => {
                let interrupt_mask = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let enabled = self
                    .bus
                    .read(
                        0x2000_1000,
                        renvo_core::AccessWidth::Word,
                        renvo_core::AccessKind::Read,
                        self.now,
                    )
                    .map_err(|error| error.to_string())? as u32;
                self.bus
                    .write(
                        0x2000_1000,
                        renvo_core::AccessWidth::Word,
                        u64::from(enabled | interrupt_mask),
                        self.now,
                    )
                    .map_err(|error| error.to_string())?;
                for interrupt in 0..32 {
                    if interrupt_mask & (1 << interrupt) != 0 {
                        self.esp_enabled_interrupts.insert(interrupt);
                        self.cpu
                            .set_machine_interrupt_enabled(interrupt as u16, true)
                            .map_err(|error| error.to_string())?;
                    }
                }
                self.complete_host_call(0)?;
                Ok(true)
            }
            0x4000_0724 => {
                let interrupt_mask = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let enabled = self
                    .bus
                    .read(
                        0x2000_1000,
                        renvo_core::AccessWidth::Word,
                        renvo_core::AccessKind::Read,
                        self.now,
                    )
                    .map_err(|error| error.to_string())? as u32;
                self.bus
                    .write(
                        0x2000_1000,
                        renvo_core::AccessWidth::Word,
                        u64::from(enabled & !interrupt_mask),
                        self.now,
                    )
                    .map_err(|error| error.to_string())?;
                for interrupt in 0..32 {
                    if interrupt_mask & (1 << interrupt) != 0 {
                        self.esp_enabled_interrupts.remove(&interrupt);
                        self.cpu
                            .set_machine_interrupt_enabled(interrupt as u16, false)
                            .map_err(|error| error.to_string())?;
                        self.cpu
                            .set_interrupt(interrupt as u16, false)
                            .map_err(|error| error.to_string())?;
                    }
                }
                self.complete_host_call(0)?;
                Ok(true)
            }
            // Trigger type and handler-vector installation are retained
            // by ESP-IDF's own tables.
            0x4000_0728 | 0x4000_072c => {
                self.complete_host_call(0)?;
                Ok(true)
            }
            // Mask-ROM MD5 API. Keep the accumulated message in the host
            // model, while also clearing the guest context to preserve
            // the API's observable initialization behavior.
            0x4000_074c => {
                let context = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                self.esp_md5_contexts.insert(context, Vec::new());
                self.write_guest_bytes(context, &[0; 88])?;
                self.complete_host_call(0)?;
                Ok(true)
            }
            0x4000_0750 => {
                let context = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let input = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())?;
                let length = self
                    .cpu
                    .register(RiscVRegister::A2)
                    .map_err(|error| error.to_string())? as usize;
                let bytes = self.read_guest_bytes(input, length)?;
                self.esp_md5_contexts
                    .entry(context)
                    .or_default()
                    .extend_from_slice(&bytes);
                self.complete_host_call(0)?;
                Ok(true)
            }
            0x4000_0754 => {
                let digest_address = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let context = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())?;
                let message = self.esp_md5_contexts.remove(&context).unwrap_or_default();
                let digest = Md5::digest(message);
                self.write_guest_bytes(digest_address, digest.as_slice())?;
                self.complete_host_call(0)?;
                Ok(true)
            }
            // Newlib routines exported by the ESP32-C6 mask ROM.
            0x4000_04a8 => {
                let destination = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let byte = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())? as u8;
                let length = self
                    .cpu
                    .register(RiscVRegister::A2)
                    .map_err(|error| error.to_string())?;
                for offset in 0..length {
                    self.bus
                        .write(
                            u64::from(destination.wrapping_add(offset)),
                            renvo_core::AccessWidth::Byte,
                            u64::from(byte),
                            self.now,
                        )
                        .map_err(|error| error.to_string())?;
                }
                self.complete_host_call(destination)?;
                Ok(true)
            }
            0x4000_04ac | 0x4000_04b0 => {
                let destination = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let source = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())?;
                let length = self
                    .cpu
                    .register(RiscVRegister::A2)
                    .map_err(|error| error.to_string())?;
                let mut bytes = Vec::with_capacity(length as usize);
                for offset in 0..length {
                    bytes.push(
                        self.bus
                            .read(
                                u64::from(source.wrapping_add(offset)),
                                renvo_core::AccessWidth::Byte,
                                renvo_core::AccessKind::Read,
                                self.now,
                            )
                            .map_err(|error| error.to_string())? as u8,
                    );
                }
                for (offset, byte) in bytes.into_iter().enumerate() {
                    self.bus
                        .write(
                            u64::from(destination.wrapping_add(offset as u32)),
                            renvo_core::AccessWidth::Byte,
                            u64::from(byte),
                            self.now,
                        )
                        .map_err(|error| error.to_string())?;
                }
                self.complete_host_call(destination)?;
                Ok(true)
            }
            0x4000_04b4 => {
                let left = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let right = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())?;
                let length = self
                    .cpu
                    .register(RiscVRegister::A2)
                    .map_err(|error| error.to_string())?;
                let mut result = 0_i32;
                for offset in 0..length {
                    let left_byte =
                        self.bus
                            .read(
                                u64::from(left.wrapping_add(offset)),
                                renvo_core::AccessWidth::Byte,
                                renvo_core::AccessKind::Read,
                                self.now,
                            )
                            .map_err(|error| error.to_string())? as u8;
                    let right_byte =
                        self.bus
                            .read(
                                u64::from(right.wrapping_add(offset)),
                                renvo_core::AccessWidth::Byte,
                                renvo_core::AccessKind::Read,
                                self.now,
                            )
                            .map_err(|error| error.to_string())? as u8;
                    if left_byte != right_byte {
                        result = i32::from(left_byte) - i32::from(right_byte);
                        break;
                    }
                }
                self.complete_host_call(result as u32)?;
                Ok(true)
            }
            0x4000_04b8 => {
                let destination = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let source = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())?;
                let mut offset = 0_u32;
                loop {
                    let byte = self
                        .bus
                        .read(
                            u64::from(source.wrapping_add(offset)),
                            renvo_core::AccessWidth::Byte,
                            renvo_core::AccessKind::Read,
                            self.now,
                        )
                        .map_err(|error| error.to_string())? as u8;
                    self.bus
                        .write(
                            u64::from(destination.wrapping_add(offset)),
                            renvo_core::AccessWidth::Byte,
                            u64::from(byte),
                            self.now,
                        )
                        .map_err(|error| error.to_string())?;
                    offset = offset.wrapping_add(1);
                    if byte == 0 {
                        break;
                    }
                }
                self.complete_host_call(destination)?;
                Ok(true)
            }
            0x4000_04bc => {
                let destination = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let source = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())?;
                let length = self
                    .cpu
                    .register(RiscVRegister::A2)
                    .map_err(|error| error.to_string())?;
                let mut terminated = false;
                for offset in 0..length {
                    let byte = if terminated {
                        0
                    } else {
                        let byte = self
                            .bus
                            .read(
                                u64::from(source.wrapping_add(offset)),
                                renvo_core::AccessWidth::Byte,
                                renvo_core::AccessKind::Read,
                                self.now,
                            )
                            .map_err(|error| error.to_string())?
                            as u8;
                        terminated = byte == 0;
                        byte
                    };
                    self.bus
                        .write(
                            u64::from(destination.wrapping_add(offset)),
                            renvo_core::AccessWidth::Byte,
                            u64::from(byte),
                            self.now,
                        )
                        .map_err(|error| error.to_string())?;
                }
                self.complete_host_call(destination)?;
                Ok(true)
            }
            0x4000_04c0 => {
                let left = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let right = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())?;
                let mut offset = 0_u32;
                let result = loop {
                    let left_byte =
                        self.bus
                            .read(
                                u64::from(left.wrapping_add(offset)),
                                renvo_core::AccessWidth::Byte,
                                renvo_core::AccessKind::Read,
                                self.now,
                            )
                            .map_err(|error| error.to_string())? as u8;
                    let right_byte =
                        self.bus
                            .read(
                                u64::from(right.wrapping_add(offset)),
                                renvo_core::AccessWidth::Byte,
                                renvo_core::AccessKind::Read,
                                self.now,
                            )
                            .map_err(|error| error.to_string())? as u8;
                    if left_byte != right_byte || left_byte == 0 {
                        break (i32::from(left_byte) - i32::from(right_byte)) as u32;
                    }
                    offset = offset.wrapping_add(1);
                };
                self.complete_host_call(result)?;
                Ok(true)
            }
            0x4000_04c4 => {
                let left = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let right = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())?;
                let length = self
                    .cpu
                    .register(RiscVRegister::A2)
                    .map_err(|error| error.to_string())?;
                let mut result = 0;
                for offset in 0..length {
                    let left_byte =
                        self.bus
                            .read(
                                u64::from(left.wrapping_add(offset)),
                                renvo_core::AccessWidth::Byte,
                                renvo_core::AccessKind::Read,
                                self.now,
                            )
                            .map_err(|error| error.to_string())? as u8;
                    let right_byte =
                        self.bus
                            .read(
                                u64::from(right.wrapping_add(offset)),
                                renvo_core::AccessWidth::Byte,
                                renvo_core::AccessKind::Read,
                                self.now,
                            )
                            .map_err(|error| error.to_string())? as u8;
                    if left_byte != right_byte || left_byte == 0 {
                        result = (i32::from(left_byte) - i32::from(right_byte)) as u32;
                        break;
                    }
                }
                self.complete_host_call(result)?;
                Ok(true)
            }
            0x4000_04c8 => {
                let source = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let mut length = 0_u32;
                while self
                    .bus
                    .read(
                        u64::from(source.wrapping_add(length)),
                        renvo_core::AccessWidth::Byte,
                        renvo_core::AccessKind::Read,
                        self.now,
                    )
                    .map_err(|error| error.to_string())?
                    != 0
                {
                    length = length.wrapping_add(1);
                }
                self.complete_host_call(length)?;
                Ok(true)
            }
            0x4000_051c => {
                let source = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let needle = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())? as u8;
                let length = self
                    .cpu
                    .register(RiscVRegister::A2)
                    .map_err(|error| error.to_string())?;
                let mut result = 0;
                for offset in 0..length {
                    let byte = self
                        .bus
                        .read(
                            u64::from(source + offset),
                            renvo_core::AccessWidth::Byte,
                            renvo_core::AccessKind::Read,
                            self.now,
                        )
                        .map_err(|error| error.to_string())? as u8;
                    if byte == needle {
                        result = source + offset;
                        break;
                    }
                }
                self.complete_host_call(result)?;
                Ok(true)
            }
            0x4000_052c => {
                let destination = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let source = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())?;
                let destination_length =
                    self.read_guest_c_string(destination, 1024 * 1024)?.len() as u32;
                let source_bytes = self.read_guest_c_string(source, 1024 * 1024)?;
                self.write_guest_bytes(
                    destination.wrapping_add(destination_length),
                    &source_bytes,
                )?;
                self.write_guest_bytes(
                    destination
                        .wrapping_add(destination_length)
                        .wrapping_add(source_bytes.len() as u32),
                    &[0],
                )?;
                self.complete_host_call(destination)?;
                Ok(true)
            }
            0x4000_0534 | 0x4000_055c => {
                let source = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let needle = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())? as u8;
                let bytes = self.read_guest_c_string(source, 1024 * 1024)?;
                let position = if self.cpu.pc() == 0x4000_0534 {
                    bytes.iter().position(|byte| *byte == needle)
                } else {
                    bytes.iter().rposition(|byte| *byte == needle)
                };
                let result = if needle == 0 {
                    source.wrapping_add(bytes.len() as u32)
                } else {
                    position
                        .map(|offset| source.wrapping_add(offset as u32))
                        .unwrap_or(0)
                };
                self.complete_host_call(result)?;
                Ok(true)
            }
            0x4000_0538 | 0x4000_0564 => {
                let source = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let set_address = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())?;
                let bytes = self.read_guest_c_string(source, 1024 * 1024)?;
                let set = self.read_guest_c_string(set_address, 1024 * 1024)?;
                let count = if self.cpu.pc() == 0x4000_0564 {
                    bytes.iter().take_while(|byte| set.contains(byte)).count()
                } else {
                    bytes.iter().take_while(|byte| !set.contains(byte)).count()
                };
                self.complete_host_call(count as u32)?;
                Ok(true)
            }
            0x4000_0544 => {
                let destination = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let source = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())?;
                let capacity = self
                    .cpu
                    .register(RiscVRegister::A2)
                    .map_err(|error| error.to_string())? as usize;
                let bytes = self.read_guest_c_string(source, 1024 * 1024)?;
                if capacity != 0 {
                    let copied = bytes.len().min(capacity - 1);
                    self.write_guest_bytes(destination, &bytes[..copied])?;
                    self.write_guest_bytes(destination.wrapping_add(copied as u32), &[0])?;
                }
                self.complete_host_call(bytes.len() as u32)?;
                Ok(true)
            }
            0x4000_0558 => {
                let source = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let maximum = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())? as usize;
                let mut length = 0;
                while length < maximum
                    && self
                        .bus
                        .read(
                            u64::from(source.wrapping_add(length as u32)),
                            renvo_core::AccessWidth::Byte,
                            renvo_core::AccessKind::Read,
                            self.now,
                        )
                        .map_err(|error| error.to_string())?
                        != 0
                {
                    length += 1;
                }
                self.complete_host_call(length as u32)?;
                Ok(true)
            }
            // qsort used by ESP-IDF's early heap-region preparation. Its
            // records are `(start, end)` word pairs and the comparator
            // orders the signed start address.
            0x4000_0588 => {
                let base = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let count = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())?;
                let size = self
                    .cpu
                    .register(RiscVRegister::A2)
                    .map_err(|error| error.to_string())?;
                if size != 8 {
                    return Err(format!(
                        "functional ESP qsort currently requires 8-byte region records, got {size}"
                    ));
                }
                let mut records = Vec::with_capacity(count as usize);
                for index in 0..count {
                    let address = base.wrapping_add(index * size);
                    let start = self
                        .bus
                        .read(
                            u64::from(address),
                            renvo_core::AccessWidth::Word,
                            renvo_core::AccessKind::Read,
                            self.now,
                        )
                        .map_err(|error| error.to_string())? as u32;
                    let end = self
                        .bus
                        .read(
                            u64::from(address + 4),
                            renvo_core::AccessWidth::Word,
                            renvo_core::AccessKind::Read,
                            self.now,
                        )
                        .map_err(|error| error.to_string())? as u32;
                    records.push((start, end));
                }
                records.sort_by_key(|(start, _)| *start as i32);
                for (index, (start, end)) in records.into_iter().enumerate() {
                    let address = base.wrapping_add(index as u32 * size);
                    for (offset, value) in [(0, start), (4, end)] {
                        self.bus
                            .write(
                                u64::from(address + offset),
                                renvo_core::AccessWidth::Word,
                                u64::from(value),
                                self.now,
                            )
                            .map_err(|error| error.to_string())?;
                    }
                }
                self.complete_host_call(0)?;
                Ok(true)
            }
            0x4000_0578 | 0x4000_0580 => {
                let value = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())? as i32;
                self.complete_host_call(value.wrapping_abs() as u32)?;
                Ok(true)
            }
            0x4000_057c | 0x4000_0584 => {
                let numerator = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())? as i32;
                let denominator = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())? as i32;
                let (quotient, remainder) = if denominator == 0 {
                    (-1, numerator)
                } else if numerator == i32::MIN && denominator == -1 {
                    (i32::MIN, 0)
                } else {
                    (numerator / denominator, numerator % denominator)
                };
                self.complete_host_call_u64(
                    u64::from(quotient as u32) | (u64::from(remainder as u32) << 32),
                )?;
                Ok(true)
            }
            // utoa / itoa
            0x4000_0598 | 0x4000_059c => {
                let raw = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let destination = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())?;
                let radix = self
                    .cpu
                    .register(RiscVRegister::A2)
                    .map_err(|error| error.to_string())?;
                let signed = self.cpu.pc() == 0x4000_059c && radix == 10 && (raw as i32) < 0;
                let mut value = if signed {
                    (raw as i32).unsigned_abs()
                } else {
                    raw
                };
                let mut rendered = Vec::new();
                if (2..=36).contains(&radix) {
                    loop {
                        let digit = (value % radix) as u8;
                        rendered.push(if digit < 10 {
                            b'0' + digit
                        } else {
                            b'a' + digit - 10
                        });
                        value /= radix;
                        if value == 0 {
                            break;
                        }
                    }
                    if signed {
                        rendered.push(b'-');
                    }
                    rendered.reverse();
                }
                rendered.push(0);
                self.write_guest_bytes(destination, &rendered)?;
                self.complete_host_call(destination)?;
                Ok(true)
            }
            // Mbed TLS SHA-224/SHA-256 backed by deterministic host
            // hashing, equivalent to the C6 accelerator at functional
            // fidelity.
            0x420f_57da => {
                let context = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                self.esp_sha256_contexts
                    .insert(context, EspFunctionalSha256::default());
                self.complete_host_call(0)?;
                Ok(true)
            }
            0x420f_57e8 => {
                let context = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                self.esp_sha256_contexts.remove(&context);
                self.complete_host_call(0)?;
                Ok(true)
            }
            0x420f_57fc => {
                let destination = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let source = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())?;
                let state = self
                    .esp_sha256_contexts
                    .get(&source)
                    .cloned()
                    .unwrap_or_default();
                self.esp_sha256_contexts.insert(destination, state);
                self.complete_host_call(0)?;
                Ok(true)
            }
            0x420f_5812 => {
                let context = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let sha224 = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())?
                    != 0;
                self.esp_sha256_contexts.insert(
                    context,
                    EspFunctionalSha256 {
                        sha224,
                        input: Vec::new(),
                    },
                );
                self.complete_host_call(0)?;
                Ok(true)
            }
            0x420f_5840 => {
                let context = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let input = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())?;
                let length = self
                    .cpu
                    .register(RiscVRegister::A2)
                    .map_err(|error| error.to_string())? as usize;
                let bytes = self.read_guest_bytes(input, length).map_err(|error| {
                    format!(
                        "mbedtls_sha256_update(ctx={context:#010x}, input={input:#010x}, length={length:#x}): {error}"
                    )
                })?;
                self.esp_sha256_contexts
                    .entry(context)
                    .or_default()
                    .input
                    .extend_from_slice(&bytes);
                self.complete_host_call(0)?;
                Ok(true)
            }
            0x420f_5966 => {
                let context = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let output = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())?;
                let state = self
                    .esp_sha256_contexts
                    .get(&context)
                    .cloned()
                    .unwrap_or_default();
                let digest = if state.sha224 {
                    Sha224::digest(&state.input).to_vec()
                } else {
                    Sha256::digest(&state.input).to_vec()
                };
                self.write_guest_bytes(output, &digest)?;
                self.complete_host_call(0)?;
                Ok(true)
            }
            // ESP-IDF has already installed device-backed FILE hooks at
            // this point. There is no additional host-side stream buffer
            // to prepare in the functional model.
            _ => Ok(false),
        }
    }
}
