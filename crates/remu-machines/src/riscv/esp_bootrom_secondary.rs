use super::*;

impl RiscVMachine {
    pub(super) fn service_esp32c6_bootrom_secondary(&mut self, pc: u32) -> Result<bool, String> {
        match pc {
            0x4000_05b0 | 0x4000_05b4 | 0x4000_05b8 | 0x4000_05bc | 0x4000_05c0 | 0x4000_05c4
            | 0x4000_05d0 => {
                self.complete_host_call(0)?;
                Ok(true)
            }
            0x4000_05c8 | 0x4000_05cc => {
                let register = if self.cpu.pc() == 0x4000_05c8 {
                    RiscVRegister::A1
                } else {
                    RiscVRegister::A0
                };
                let character = self
                    .cpu
                    .register(register)
                    .map_err(|error| error.to_string())? as u8;
                self.uart.transmit(&[character]);
                self.complete_host_call(u32::from(character))?;
                Ok(true)
            }
            // The mapped application segments are already visible through
            // Renvo Emulator's deterministic flash view, so enabling the ROM
            // instruction cache is an ordering point.
            0x4000_0690 | 0x4000_0694 | 0x4000_0698 | 0x4000_069c | 0x4000_06a0 | 0x4000_06a4
            | 0x4000_06a8 => {
                self.refresh_esp32c6_cache(ESP_FUNCTIONAL_MMAP_BASE, u32::MAX)
                    .map_err(|error| error.to_string())?;
                self.complete_host_call(0)?;
                Ok(true)
            }
            // Cache maintenance and lock/preload controls complete
            // synchronously in the functional cache model.
            0x4000_0640 | 0x4000_0648 | 0x4000_064c => {
                self.refresh_esp32c6_mmu_mappings()
                    .map_err(|error| error.to_string())?;
                self.complete_host_call(0)?;
                Ok(true)
            }
            0x4000_0644 | 0x4000_0650 | 0x4000_0654 | 0x4000_0658 | 0x4000_065c | 0x4000_0660
            | 0x4000_0668 | 0x4000_066c | 0x4000_0670 | 0x4000_0674 | 0x4000_0678 | 0x4000_067c
            | 0x4000_0680 | 0x4000_0684 | 0x4000_0688 | 0x4000_068c => {
                self.complete_host_call(0)?;
                Ok(true)
            }
            0x4000_0664 => {
                self.complete_host_call(1)?;
                Ok(true)
            }
            // ESP-IDF's ROM-resident watchdog HAL. Renvo Emulator does not advance
            // a watchdog countdown in functional mode, but preserves
            // enable state per HAL context so driver probes remain
            // coherent.
            0x4000_0394 | 0x4000_0398 | 0x4000_039c | 0x4000_03a0 | 0x4000_03a4 | 0x4000_03b0
            | 0x4000_03b4 | 0x4000_03b8 => {
                self.complete_host_call(0)?;
                Ok(true)
            }
            0x4000_03a8 => {
                let context = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                self.esp_enabled_watchdogs.insert(context);
                self.complete_host_call(0)?;
                Ok(true)
            }
            0x4000_03ac => {
                let context = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                self.esp_enabled_watchdogs.remove(&context);
                self.complete_host_call(0)?;
                Ok(true)
            }
            0x4000_03bc => {
                let context = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let enabled = u32::from(self.esp_enabled_watchdogs.contains(&context));
                self.complete_host_call(enabled)?;
                Ok(true)
            }
            // Copy the two tick-rate conversion callbacks into the
            // caller-owned systimer HAL context.
            0x4000_03c8 => {
                let context = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let operations = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())?;
                let mut callbacks = [0_u32; 2];
                for (index, callback) in callbacks.iter_mut().enumerate() {
                    *callback = self
                        .bus
                        .read(
                            u64::from(operations + index as u32 * 4),
                            remu_core::AccessWidth::Word,
                            remu_core::AccessKind::Read,
                            self.now,
                        )
                        .map_err(|error| error.to_string())? as u32;
                }
                self.write_guest_words(context + 4, &callbacks)?;
                self.complete_host_call(0)?;
                Ok(true)
            }
            0x4000_03cc | 0x4000_03d0 => {
                let counter = self
                    .now
                    .ticks()
                    .wrapping_div(ESP32C6_CPU_TICKS_PER_SYSTIMER_TICK)
                    .wrapping_add(self.esp_systimer_offset);
                self.complete_host_call_u64(counter)?;
                Ok(true)
            }
            0x4000_03d4 | 0x4000_03d8 => {
                let alarm = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())? as usize;
                if alarm >= self.esp_systimer_alarms.len() {
                    return Err(format!("invalid ESP systimer alarm {alarm}"));
                }
                if self.cpu.pc() == 0x4000_03d4 {
                    let value = u64::from(
                        self.cpu
                            .register(RiscVRegister::A2)
                            .map_err(|error| error.to_string())?,
                    ) | (u64::from(
                        self.cpu
                            .register(RiscVRegister::A3)
                            .map_err(|error| error.to_string())?,
                    ) << 32);
                    self.esp_systimer_alarms[alarm] = value;
                    self.esp_systimer_next[alarm] = value;
                    let address = ESP32C6_SYSTIMER_TARGET_VALUE + alarm as u64 * 8;
                    self.bus
                        .write(address, AccessWidth::Word, value >> 32, self.now)
                        .map_err(|error| error.to_string())?;
                    self.bus
                        .write(
                            address + 4,
                            AccessWidth::Word,
                            value & u64::from(u32::MAX),
                            self.now,
                        )
                        .map_err(|error| error.to_string())?;
                } else {
                    // Unlike set_alarm_target(), the period argument is
                    // a 32-bit value in A2. Mirror it into TARGET_CONF:
                    // the inlined ISR reads this register directly.
                    let value = self
                        .cpu
                        .register(RiscVRegister::A2)
                        .map_err(|error| error.to_string())?
                        & ((1 << 26) - 1);
                    self.esp_systimer_periods[alarm] = u64::from(value);
                    let address = ESP32C6_SYSTIMER_TARGET_CONF + alarm as u64 * 4;
                    let current =
                        self.bus
                            .read(address, AccessWidth::Word, AccessKind::Read, self.now)
                            .map_err(|error| error.to_string())? as u32;
                    self.bus
                        .write(
                            address,
                            AccessWidth::Word,
                            u64::from((current & !((1 << 26) - 1)) | value),
                            self.now,
                        )
                        .map_err(|error| error.to_string())?;
                    if value != 0 {
                        self.esp_systimer_next[alarm] = self
                            .now
                            .ticks()
                            .wrapping_div(ESP32C6_CPU_TICKS_PER_SYSTIMER_TICK)
                            .wrapping_add(self.esp_systimer_offset)
                            .wrapping_add(u64::from(value));
                    }
                }
                self.complete_host_call(0)?;
                Ok(true)
            }
            0x4000_03dc => {
                let alarm = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())? as usize;
                let value = *self
                    .esp_systimer_alarms
                    .get(alarm)
                    .ok_or_else(|| format!("invalid ESP systimer alarm {alarm}"))?;
                self.complete_host_call_u64(value)?;
                Ok(true)
            }
            0x4000_03e8 => {
                let advance = u64::from(
                    self.cpu
                        .register(RiscVRegister::A2)
                        .map_err(|error| error.to_string())?,
                ) | (u64::from(
                    self.cpu
                        .register(RiscVRegister::A3)
                        .map_err(|error| error.to_string())?,
                ) << 32);
                self.esp_systimer_offset = self.esp_systimer_offset.wrapping_add(advance);
                self.complete_host_call(0)?;
                Ok(true)
            }
            0x4000_03e0 => {
                let alarm = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())? as usize;
                let enabled = self
                    .esp_systimer_interrupt_enabled
                    .get_mut(alarm)
                    .ok_or_else(|| format!("invalid ESP systimer alarm {alarm}"))?;
                *enabled = true;
                let interrupt_enable = self
                    .bus
                    .read(
                        ESP32C6_SYSTIMER_INT_ENA,
                        AccessWidth::Word,
                        AccessKind::Read,
                        self.now,
                    )
                    .map_err(|error| error.to_string())?
                    | (1_u64 << alarm);
                self.bus
                    .write(
                        ESP32C6_SYSTIMER_INT_ENA,
                        AccessWidth::Word,
                        interrupt_enable,
                        self.now,
                    )
                    .map_err(|error| error.to_string())?;
                if self.esp_systimer_next[alarm] == u64::MAX
                    && self.esp_systimer_periods[alarm] != 0
                {
                    self.esp_systimer_next[alarm] = self
                        .now
                        .ticks()
                        .wrapping_div(ESP32C6_CPU_TICKS_PER_SYSTIMER_TICK)
                        .wrapping_add(self.esp_systimer_offset)
                        .wrapping_add(self.esp_systimer_periods[alarm]);
                }
                self.complete_host_call(0)?;
                Ok(true)
            }
            // Counter enable mirrors the work-enable bit used by
            // direct low-level reads.
            0x4000_03ec => {
                let counter = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())?;
                if counter > 1 {
                    return Err(format!("invalid ESP systimer counter {counter}"));
                }
                let current = self
                    .bus
                    .read(
                        ESP32C6_SYSTIMER_BASE,
                        AccessWidth::Word,
                        AccessKind::Read,
                        self.now,
                    )
                    .map_err(|error| error.to_string())? as u32;
                self.bus
                    .write(
                        ESP32C6_SYSTIMER_BASE,
                        AccessWidth::Word,
                        u64::from(current | (1 << (30 - counter))),
                        self.now,
                    )
                    .map_err(|error| error.to_string())?;
                self.complete_host_call(0)?;
                Ok(true)
            }
            0x4000_03f0 | 0x4000_03f4 => {
                let alarm = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())? as usize;
                if alarm >= self.esp_systimer_alarms.len() {
                    return Err(format!("invalid ESP systimer alarm {alarm}"));
                }
                let value = self
                    .cpu
                    .register(RiscVRegister::A2)
                    .map_err(|error| error.to_string())?;
                let address = ESP32C6_SYSTIMER_TARGET_CONF + alarm as u64 * 4;
                let current = self
                    .bus
                    .read(address, AccessWidth::Word, AccessKind::Read, self.now)
                    .map_err(|error| error.to_string())? as u32;
                let updated = if self.cpu.pc() == 0x4000_03f0 {
                    (current & !(1 << 30)) | ((value & 1) << 30)
                } else {
                    (current & !(1 << 31)) | ((value & 1) << 31)
                };
                self.bus
                    .write(address, AccessWidth::Word, u64::from(updated), self.now)
                    .map_err(|error| error.to_string())?;
                self.complete_host_call(0)?;
                Ok(true)
            }
            // APB update and stall controls do not alter the
            // functional counter value.
            0x4000_03e4 | 0x4000_03f8 => {
                self.complete_host_call(0)?;
                Ok(true)
            }
            // ROM TLSF lock setup/entry. The functional machine executes
            // one guest thread at a time, making these ordering points.
            0x4000_0460 | 0x4000_0464 | 0x4000_0468 | 0x4000_046c => {
                self.complete_host_call(0)?;
                Ok(true)
            }
            0x4000_0458 => {
                let handle = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let pointer = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())?;
                let size = self
                    .esp_heaps
                    .get(&handle)
                    .and_then(|heap| heap.allocations.get(&pointer))
                    .copied()
                    .unwrap_or(0);
                self.complete_host_call(size)?;
                Ok(true)
            }
            0x4000_045c => {
                let start = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let size = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())?;
                let result = EspFunctionalHeap::new(start, size).map_or(0, |heap| {
                    self.esp_heaps.insert(start, heap);
                    start
                });
                self.complete_host_call(result)?;
                Ok(true)
            }
            0x4000_047c => {
                let handle = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let size = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())?;
                let result = self.esp_heap_allocate(handle, size, 4, 0)?;
                self.complete_host_call(result)?;
                Ok(true)
            }
            0x4000_0480 => {
                let handle = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let pointer = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())?;
                if pointer != 0 {
                    self.esp_heaps
                        .get_mut(&handle)
                        .ok_or_else(|| format!("unknown ESP heap handle {handle:#010x}"))?
                        .release(pointer);
                }
                self.complete_host_call(0)?;
                Ok(true)
            }
            0x4000_0484 => {
                let handle = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let pointer = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())?;
                let size = self
                    .cpu
                    .register(RiscVRegister::A2)
                    .map_err(|error| error.to_string())?;
                let result = self.esp_heap_reallocate(handle, pointer, size)?;
                self.complete_host_call(result)?;
                Ok(true)
            }
            0x4000_0488 | 0x4000_048c => {
                let handle = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let size = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())?;
                let alignment = self
                    .cpu
                    .register(RiscVRegister::A2)
                    .map_err(|error| error.to_string())?;
                let offset = if self.cpu.pc() == 0x4000_0488 {
                    self.cpu
                        .register(RiscVRegister::A3)
                        .map_err(|error| error.to_string())?
                } else {
                    0
                };
                let result = self.esp_heap_allocate(handle, size, alignment, offset)?;
                self.complete_host_call(result)?;
                Ok(true)
            }
            0x4000_0490 => {
                self.complete_host_call(1)?;
                Ok(true)
            }
            0x4000_0498 | 0x4000_049c => {
                let handle = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let heap = self
                    .esp_heaps
                    .get(&handle)
                    .ok_or_else(|| format!("unknown ESP heap handle {handle:#010x}"))?;
                let value = if self.cpu.pc() == 0x4000_0498 {
                    heap.free_bytes()
                } else {
                    heap.minimum_free
                };
                self.complete_host_call(value)?;
                Ok(true)
            }
            0x4000_04a0 => {
                let handle = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let info = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())?;
                let heap = self
                    .esp_heaps
                    .get(&handle)
                    .ok_or_else(|| format!("unknown ESP heap handle {handle:#010x}"))?;
                let free = heap.free_bytes();
                let allocated: u32 = heap.allocations.values().copied().sum();
                let largest = heap.free.values().copied().max().unwrap_or(0);
                let words = [
                    free,
                    allocated,
                    largest,
                    heap.minimum_free,
                    heap.allocations.len() as u32,
                    heap.free.len() as u32,
                    (heap.allocations.len() + heap.free.len()) as u32,
                ];
                self.write_guest_words(info, &words)?;
                self.complete_host_call(0)?;
                Ok(true)
            }
            // Newlib's ROM lock objects are unnecessary in the
            // single-threaded functional boot phase.
            0x4000_04a4 => {
                self.complete_host_call(0)?;
                Ok(true)
            }
            // ESP32-C6 ROM RVFP entry points. The C ABI passes soft-float
            // payloads through a0-a3, with 64-bit values split low word
            // first. Keeping these calls at the ROM boundary makes the
            // implementation deterministic while still executing the
            // unmodified vendor firmware around them.
            0x4000_08d0 | 0x4000_09f4 | 0x4000_0a64 | 0x4000_0a74 => {
                let left_bits = u64::from(
                    self.cpu
                        .register(RiscVRegister::A0)
                        .map_err(|error| error.to_string())?,
                ) | (u64::from(
                    self.cpu
                        .register(RiscVRegister::A1)
                        .map_err(|error| error.to_string())?,
                ) << 32);
                let right_bits = u64::from(
                    self.cpu
                        .register(RiscVRegister::A2)
                        .map_err(|error| error.to_string())?,
                ) | (u64::from(
                    self.cpu
                        .register(RiscVRegister::A3)
                        .map_err(|error| error.to_string())?,
                ) << 32);
                let left = f64::from_bits(left_bits);
                let right = f64::from_bits(right_bits);
                let result = match self.cpu.pc() {
                    0x4000_08d0 => left / right,
                    0x4000_09f4 => left + right,
                    0x4000_0a64 => left * right,
                    0x4000_0a74 => left - right,
                    _ => unreachable!(),
                };
                self.complete_host_call_u64(result.to_bits())?;
                Ok(true)
            }
            0x4000_08dc | 0x4000_09f8 | 0x4000_0a68 | 0x4000_0a78 => {
                let left = f32::from_bits(
                    self.cpu
                        .register(RiscVRegister::A0)
                        .map_err(|error| error.to_string())?,
                );
                let right = f32::from_bits(
                    self.cpu
                        .register(RiscVRegister::A1)
                        .map_err(|error| error.to_string())?,
                );
                let result = match self.cpu.pc() {
                    0x4000_08dc => left / right,
                    0x4000_09f8 => left + right,
                    0x4000_0a68 => left * right,
                    0x4000_0a78 => left - right,
                    _ => unreachable!(),
                };
                self.complete_host_call(result.to_bits())?;
                Ok(true)
            }
            0x4000_0988 => {
                let bits = u64::from(
                    self.cpu
                        .register(RiscVRegister::A0)
                        .map_err(|error| error.to_string())?,
                ) | (u64::from(
                    self.cpu
                        .register(RiscVRegister::A1)
                        .map_err(|error| error.to_string())?,
                ) << 32);
                self.complete_host_call_u64(bits ^ (1_u64 << 63))?;
                Ok(true)
            }
            0x4000_0990 => {
                let bits = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                self.complete_host_call(bits ^ (1_u32 << 31))?;
                Ok(true)
            }
            0x4000_09ac => {
                let bits = u64::from(
                    self.cpu
                        .register(RiscVRegister::A0)
                        .map_err(|error| error.to_string())?,
                ) | (u64::from(
                    self.cpu
                        .register(RiscVRegister::A1)
                        .map_err(|error| error.to_string())?,
                ) << 32);
                let exponent = self
                    .cpu
                    .register(RiscVRegister::A2)
                    .map_err(|error| error.to_string())? as i32;
                self.complete_host_call_u64(f64::from_bits(bits).powi(exponent).to_bits())?;
                Ok(true)
            }
            0x4000_09b0 => {
                let value = f32::from_bits(
                    self.cpu
                        .register(RiscVRegister::A0)
                        .map_err(|error| error.to_string())?,
                );
                let exponent = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())? as i32;
                self.complete_host_call(value.powi(exponent).to_bits())?;
                Ok(true)
            }
            0x4000_09e4 => {
                let left_bits = u64::from(
                    self.cpu
                        .register(RiscVRegister::A0)
                        .map_err(|error| error.to_string())?,
                ) | (u64::from(
                    self.cpu
                        .register(RiscVRegister::A1)
                        .map_err(|error| error.to_string())?,
                ) << 32);
                let right_bits = u64::from(
                    self.cpu
                        .register(RiscVRegister::A2)
                        .map_err(|error| error.to_string())?,
                ) | (u64::from(
                    self.cpu
                        .register(RiscVRegister::A3)
                        .map_err(|error| error.to_string())?,
                ) << 32);
                self.complete_host_call(u32::from(
                    f64::from_bits(left_bits).is_nan() || f64::from_bits(right_bits).is_nan(),
                ))?;
                Ok(true)
            }
            0x4000_09e8 => {
                let left = f32::from_bits(
                    self.cpu
                        .register(RiscVRegister::A0)
                        .map_err(|error| error.to_string())?,
                );
                let right = f32::from_bits(
                    self.cpu
                        .register(RiscVRegister::A1)
                        .map_err(|error| error.to_string())?,
                );
                self.complete_host_call(u32::from(left.is_nan() || right.is_nan()))?;
                Ok(true)
            }
            0x4000_09fc | 0x4000_0a6c => {
                let left_bits = u64::from(
                    self.cpu
                        .register(RiscVRegister::A0)
                        .map_err(|error| error.to_string())?,
                ) | (u64::from(
                    self.cpu
                        .register(RiscVRegister::A1)
                        .map_err(|error| error.to_string())?,
                ) << 32);
                let right_bits = u64::from(
                    self.cpu
                        .register(RiscVRegister::A2)
                        .map_err(|error| error.to_string())?,
                ) | (u64::from(
                    self.cpu
                        .register(RiscVRegister::A3)
                        .map_err(|error| error.to_string())?,
                ) << 32);
                let left = f64::from_bits(left_bits);
                let right = f64::from_bits(right_bits);
                self.complete_host_call(u32::from(left != right))?;
                Ok(true)
            }
            0x4000_0a00 | 0x4000_0a70 => {
                let left = f32::from_bits(
                    self.cpu
                        .register(RiscVRegister::A0)
                        .map_err(|error| error.to_string())?,
                );
                let right = f32::from_bits(
                    self.cpu
                        .register(RiscVRegister::A1)
                        .map_err(|error| error.to_string())?,
                );
                self.complete_host_call(u32::from(left != right))?;
                Ok(true)
            }
            0x4000_0a04 => {
                let value = f32::from_bits(
                    self.cpu
                        .register(RiscVRegister::A0)
                        .map_err(|error| error.to_string())?,
                );
                self.complete_host_call_u64((value as f64).to_bits())?;
                Ok(true)
            }
            0x4000_0a7c => {
                let bits = u64::from(
                    self.cpu
                        .register(RiscVRegister::A0)
                        .map_err(|error| error.to_string())?,
                ) | (u64::from(
                    self.cpu
                        .register(RiscVRegister::A1)
                        .map_err(|error| error.to_string())?,
                ) << 32);
                self.complete_host_call((f64::from_bits(bits) as f32).to_bits())?;
                Ok(true)
            }
            0x4000_0a08 | 0x4000_0a0c | 0x4000_0a18 => {
                let bits = u64::from(
                    self.cpu
                        .register(RiscVRegister::A0)
                        .map_err(|error| error.to_string())?,
                ) | (u64::from(
                    self.cpu
                        .register(RiscVRegister::A1)
                        .map_err(|error| error.to_string())?,
                ) << 32);
                let value = f64::from_bits(bits);
                match self.cpu.pc() {
                    0x4000_0a08 => self.complete_host_call_u64((value as i64) as u64)?,
                    0x4000_0a0c => self.complete_host_call((value as i32) as u32)?,
                    0x4000_0a18 => self.complete_host_call(value as u32)?,
                    _ => unreachable!(),
                }
                Ok(true)
            }
            0x4000_0a10 | 0x4000_0a14 | 0x4000_0a1c | 0x4000_0a20 => {
                let value = f32::from_bits(
                    self.cpu
                        .register(RiscVRegister::A0)
                        .map_err(|error| error.to_string())?,
                );
                match self.cpu.pc() {
                    0x4000_0a10 => self.complete_host_call_u64((value as i64) as u64)?,
                    0x4000_0a14 => self.complete_host_call((value as i32) as u32)?,
                    0x4000_0a1c => self.complete_host_call_u64(value as u64)?,
                    0x4000_0a20 => self.complete_host_call(value as u32)?,
                    _ => unreachable!(),
                }
                Ok(true)
            }
            0x4000_0a24 | 0x4000_0a28 | 0x4000_0a34 | 0x4000_0a38 => {
                let bits = u64::from(
                    self.cpu
                        .register(RiscVRegister::A0)
                        .map_err(|error| error.to_string())?,
                ) | (u64::from(
                    self.cpu
                        .register(RiscVRegister::A1)
                        .map_err(|error| error.to_string())?,
                ) << 32);
                match self.cpu.pc() {
                    0x4000_0a24 => self.complete_host_call_u64(((bits as i64) as f64).to_bits())?,
                    0x4000_0a28 => self.complete_host_call(((bits as i64) as f32).to_bits())?,
                    0x4000_0a34 => self.complete_host_call_u64((bits as f64).to_bits())?,
                    0x4000_0a38 => self.complete_host_call((bits as f32).to_bits())?,
                    _ => unreachable!(),
                }
                Ok(true)
            }
            0x4000_0a2c | 0x4000_0a30 | 0x4000_0a3c | 0x4000_0a40 => {
                let bits = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                match self.cpu.pc() {
                    0x4000_0a2c => self.complete_host_call_u64(((bits as i32) as f64).to_bits())?,
                    0x4000_0a30 => self.complete_host_call(((bits as i32) as f32).to_bits())?,
                    0x4000_0a3c => self.complete_host_call_u64((f64::from(bits)).to_bits())?,
                    0x4000_0a40 => self.complete_host_call((bits as f32).to_bits())?,
                    _ => unreachable!(),
                }
                Ok(true)
            }
            0x4000_0a44 | 0x4000_0a4c | 0x4000_0a54 | 0x4000_0a5c => {
                let left_bits = u64::from(
                    self.cpu
                        .register(RiscVRegister::A0)
                        .map_err(|error| error.to_string())?,
                ) | (u64::from(
                    self.cpu
                        .register(RiscVRegister::A1)
                        .map_err(|error| error.to_string())?,
                ) << 32);
                let right_bits = u64::from(
                    self.cpu
                        .register(RiscVRegister::A2)
                        .map_err(|error| error.to_string())?,
                ) | (u64::from(
                    self.cpu
                        .register(RiscVRegister::A3)
                        .map_err(|error| error.to_string())?,
                ) << 32);
                let left = f64::from_bits(left_bits);
                let right = f64::from_bits(right_bits);
                let nan_result = if matches!(self.cpu.pc(), 0x4000_0a44 | 0x4000_0a4c) {
                    -1_i32
                } else {
                    1_i32
                };
                let result = if left.is_nan() || right.is_nan() {
                    nan_result
                } else if left < right {
                    -1
                } else if left > right {
                    1
                } else {
                    0
                };
                self.complete_host_call(result as u32)?;
                Ok(true)
            }
            0x4000_0a48 | 0x4000_0a50 | 0x4000_0a58 | 0x4000_0a60 => {
                let left = f32::from_bits(
                    self.cpu
                        .register(RiscVRegister::A0)
                        .map_err(|error| error.to_string())?,
                );
                let right = f32::from_bits(
                    self.cpu
                        .register(RiscVRegister::A1)
                        .map_err(|error| error.to_string())?,
                );
                let nan_result = if matches!(self.cpu.pc(), 0x4000_0a48 | 0x4000_0a50) {
                    -1_i32
                } else {
                    1_i32
                };
                let result = if left.is_nan() || right.is_nan() {
                    nan_result
                } else if left < right {
                    -1
                } else if left > right {
                    1
                } else {
                    0
                };
                self.complete_host_call(result as u32)?;
                Ok(true)
            }
            0x4000_089c | 0x4000_08a0 | 0x4000_0950 => {
                let value = u64::from(
                    self.cpu
                        .register(RiscVRegister::A0)
                        .map_err(|error| error.to_string())?,
                ) | (u64::from(
                    self.cpu
                        .register(RiscVRegister::A1)
                        .map_err(|error| error.to_string())?,
                ) << 32);
                let shift = self
                    .cpu
                    .register(RiscVRegister::A2)
                    .map_err(|error| error.to_string())?
                    & 63;
                let result = match self.cpu.pc() {
                    0x4000_089c => value.wrapping_shl(shift),
                    0x4000_08a0 => ((value as i64) >> shift) as u64,
                    0x4000_0950 => value >> shift,
                    _ => unreachable!(),
                };
                self.cpu
                    .set_register(RiscVRegister::A1, (result >> 32) as u32)
                    .map_err(|error| error.to_string())?;
                self.complete_host_call(result as u32)?;
                Ok(true)
            }
            0x4000_08a4 => {
                let value = u64::from(
                    self.cpu
                        .register(RiscVRegister::A0)
                        .map_err(|error| error.to_string())?,
                ) | (u64::from(
                    self.cpu
                        .register(RiscVRegister::A1)
                        .map_err(|error| error.to_string())?,
                ) << 32);
                self.complete_host_call_u64(value.swap_bytes())?;
                Ok(true)
            }
            0x4000_08a8 => {
                let value = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                self.complete_host_call(value.swap_bytes())?;
                Ok(true)
            }
            0x4000_08b8 | 0x4000_08c4 | 0x4000_08f0 | 0x4000_09a4 => {
                let value = u64::from(
                    self.cpu
                        .register(RiscVRegister::A0)
                        .map_err(|error| error.to_string())?,
                ) | (u64::from(
                    self.cpu
                        .register(RiscVRegister::A1)
                        .map_err(|error| error.to_string())?,
                ) << 32);
                let result = match self.cpu.pc() {
                    0x4000_08b8 => value.leading_zeros(),
                    0x4000_08c4 => value.trailing_zeros(),
                    0x4000_08f0 => {
                        if value == 0 {
                            0
                        } else {
                            value.trailing_zeros() + 1
                        }
                    }
                    0x4000_09a4 => value.count_ones(),
                    _ => unreachable!(),
                };
                self.complete_host_call(result)?;
                Ok(true)
            }
            0x4000_08bc | 0x4000_08c8 | 0x4000_08f4 | 0x4000_09a0 | 0x4000_09a8 => {
                let value = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let result = match self.cpu.pc() {
                    0x4000_08bc => value.leading_zeros(),
                    0x4000_08c8 => value.trailing_zeros(),
                    0x4000_08f4 => {
                        if value == 0 {
                            0
                        } else {
                            value.trailing_zeros() + 1
                        }
                    }
                    0x4000_09a0 => value.count_ones() & 1,
                    0x4000_09a8 => value.count_ones(),
                    _ => unreachable!(),
                };
                self.complete_host_call(result)?;
                Ok(true)
            }
            0x4000_08c0 | 0x4000_09c8 => {
                let left = u64::from(
                    self.cpu
                        .register(RiscVRegister::A0)
                        .map_err(|error| error.to_string())?,
                ) | (u64::from(
                    self.cpu
                        .register(RiscVRegister::A1)
                        .map_err(|error| error.to_string())?,
                ) << 32);
                let right = u64::from(
                    self.cpu
                        .register(RiscVRegister::A2)
                        .map_err(|error| error.to_string())?,
                ) | (u64::from(
                    self.cpu
                        .register(RiscVRegister::A3)
                        .map_err(|error| error.to_string())?,
                ) << 32);
                let ordering = if self.cpu.pc() == 0x4000_08c0 {
                    (left as i64).cmp(&(right as i64))
                } else {
                    left.cmp(&right)
                };
                let result = match ordering {
                    std::cmp::Ordering::Less => 0,
                    std::cmp::Ordering::Equal => 1,
                    std::cmp::Ordering::Greater => 2,
                };
                self.complete_host_call(result)?;
                Ok(true)
            }
            0x4000_08d4 | 0x4000_095c => {
                let numerator = (u64::from(
                    self.cpu
                        .register(RiscVRegister::A0)
                        .map_err(|error| error.to_string())?,
                ) | (u64::from(
                    self.cpu
                        .register(RiscVRegister::A1)
                        .map_err(|error| error.to_string())?,
                ) << 32)) as i64;
                let denominator = (u64::from(
                    self.cpu
                        .register(RiscVRegister::A2)
                        .map_err(|error| error.to_string())?,
                ) | (u64::from(
                    self.cpu
                        .register(RiscVRegister::A3)
                        .map_err(|error| error.to_string())?,
                ) << 32)) as i64;
                let result = if denominator == 0 {
                    if self.cpu.pc() == 0x4000_08d4 {
                        -1
                    } else {
                        numerator
                    }
                } else if numerator == i64::MIN && denominator == -1 {
                    if self.cpu.pc() == 0x4000_08d4 {
                        i64::MIN
                    } else {
                        0
                    }
                } else if self.cpu.pc() == 0x4000_08d4 {
                    numerator / denominator
                } else {
                    numerator % denominator
                };
                self.complete_host_call_u64(result as u64)?;
                Ok(true)
            }
            0x4000_08e0 | 0x4000_0960 => {
                let numerator = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())? as i32;
                let denominator = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())? as i32;
                let result = if denominator == 0 {
                    if self.cpu.pc() == 0x4000_08e0 {
                        -1
                    } else {
                        numerator
                    }
                } else if numerator == i32::MIN && denominator == -1 {
                    if self.cpu.pc() == 0x4000_08e0 {
                        i32::MIN
                    } else {
                        0
                    }
                } else if self.cpu.pc() == 0x4000_08e0 {
                    numerator / denominator
                } else {
                    numerator % denominator
                };
                self.complete_host_call(result as u32)?;
                Ok(true)
            }
            0x4000_098c => {
                let value = u64::from(
                    self.cpu
                        .register(RiscVRegister::A0)
                        .map_err(|error| error.to_string())?,
                ) | (u64::from(
                    self.cpu
                        .register(RiscVRegister::A1)
                        .map_err(|error| error.to_string())?,
                ) << 32);
                self.complete_host_call_u64(value.wrapping_neg())?;
                Ok(true)
            }
            0x4000_096c => {
                let left = u64::from(
                    self.cpu
                        .register(RiscVRegister::A0)
                        .map_err(|error| error.to_string())?,
                ) | (u64::from(
                    self.cpu
                        .register(RiscVRegister::A1)
                        .map_err(|error| error.to_string())?,
                ) << 32);
                let right = u64::from(
                    self.cpu
                        .register(RiscVRegister::A2)
                        .map_err(|error| error.to_string())?,
                ) | (u64::from(
                    self.cpu
                        .register(RiscVRegister::A3)
                        .map_err(|error| error.to_string())?,
                ) << 32);
                let result = left.wrapping_mul(right);
                self.cpu
                    .set_register(RiscVRegister::A1, (result >> 32) as u32)
                    .map_err(|error| error.to_string())?;
                self.complete_host_call(result as u32)?;
                Ok(true)
            }
            0x4000_09cc | 0x4000_09dc => {
                let numerator = u64::from(
                    self.cpu
                        .register(RiscVRegister::A0)
                        .map_err(|error| error.to_string())?,
                ) | (u64::from(
                    self.cpu
                        .register(RiscVRegister::A1)
                        .map_err(|error| error.to_string())?,
                ) << 32);
                let denominator = u64::from(
                    self.cpu
                        .register(RiscVRegister::A2)
                        .map_err(|error| error.to_string())?,
                ) | (u64::from(
                    self.cpu
                        .register(RiscVRegister::A3)
                        .map_err(|error| error.to_string())?,
                ) << 32);
                let result = if denominator == 0 {
                    u64::MAX
                } else if self.cpu.pc() == 0x4000_09cc {
                    numerator / denominator
                } else {
                    numerator % denominator
                };
                self.cpu
                    .set_register(RiscVRegister::A1, (result >> 32) as u32)
                    .map_err(|error| error.to_string())?;
                self.complete_host_call(result as u32)?;
                Ok(true)
            }
            0x4000_09d4 | 0x4000_09e0 => {
                let numerator = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let denominator = self
                    .cpu
                    .register(RiscVRegister::A1)
                    .map_err(|error| error.to_string())?;
                let result = if denominator == 0 {
                    u32::MAX
                } else if self.cpu.pc() == 0x4000_09d4 {
                    numerator / denominator
                } else {
                    numerator % denominator
                };
                self.complete_host_call(result)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}
