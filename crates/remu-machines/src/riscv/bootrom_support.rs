use super::*;

impl RiscVMachine {
    pub(super) fn complete_host_call(&mut self, result: u32) -> Result<(), String> {
        let return_address = self
            .cpu
            .register(RiscVRegister::Ra)
            .map_err(|error| error.to_string())?;
        self.cpu
            .set_register(RiscVRegister::A0, result)
            .map_err(|error| error.to_string())?;
        self.cpu
            .set_pc(return_address)
            .map_err(|error| error.to_string())
    }

    pub(super) fn complete_host_call_u64(&mut self, result: u64) -> Result<(), String> {
        self.cpu
            .set_register(RiscVRegister::A1, (result >> 32) as u32)
            .map_err(|error| error.to_string())?;
        self.complete_host_call(result as u32)
    }

    pub(super) fn read_guest_c_string(
        &mut self,
        address: u32,
        limit: usize,
    ) -> Result<Vec<u8>, String> {
        let mut bytes = Vec::new();
        for offset in 0..limit {
            let byte = self
                .bus
                .read(
                    u64::from(address.wrapping_add(offset as u32)),
                    remu_core::AccessWidth::Byte,
                    remu_core::AccessKind::Read,
                    self.now,
                )
                .map_err(|error| error.to_string())? as u8;
            if byte == 0 {
                return Ok(bytes);
            }
            bytes.push(byte);
        }
        Err(format!(
            "guest string at {address:#010x} exceeds {limit} bytes"
        ))
    }

    pub(super) fn read_guest_bytes(
        &mut self,
        address: u32,
        length: usize,
    ) -> Result<Vec<u8>, String> {
        let mut bytes = Vec::with_capacity(length);
        for offset in 0..length {
            bytes.push(
                self.bus
                    .read(
                        u64::from(address.wrapping_add(offset as u32)),
                        remu_core::AccessWidth::Byte,
                        remu_core::AccessKind::Read,
                        self.now,
                    )
                    .map_err(|error| error.to_string())? as u8,
            );
        }
        Ok(bytes)
    }

    pub(super) fn write_guest_bytes(&mut self, address: u32, bytes: &[u8]) -> Result<(), String> {
        for (offset, byte) in bytes.iter().copied().enumerate() {
            self.bus
                .write(
                    u64::from(address.wrapping_add(offset as u32)),
                    remu_core::AccessWidth::Byte,
                    u64::from(byte),
                    self.now,
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    pub(super) fn esp_printf_argument(&mut self, slot: u32) -> Result<u32, String> {
        if slot <= 7 {
            let register =
                RiscVRegister::argument(slot as u8).expect("slot range was checked above");
            return self
                .cpu
                .register(register)
                .map_err(|error| error.to_string());
        }
        let stack = self
            .cpu
            .register(RiscVRegister::Sp)
            .map_err(|error| error.to_string())?;
        self.bus
            .read(
                u64::from(stack.wrapping_add((slot - 8) * 4)),
                remu_core::AccessWidth::Word,
                remu_core::AccessKind::Read,
                self.now,
            )
            .map(|value| value as u32)
            .map_err(|error| error.to_string())
    }

    pub(super) fn service_esp_printf(&mut self) -> Result<u32, String> {
        let format_address = self
            .cpu
            .register(RiscVRegister::A0)
            .map_err(|error| error.to_string())?;
        let format = self.read_guest_c_string(format_address, 16 * 1024)?;
        let mut output = Vec::new();
        let mut cursor = 0;
        let mut argument_slot = 1_u32;

        while cursor < format.len() {
            if format[cursor] != b'%' {
                output.push(format[cursor]);
                cursor += 1;
                continue;
            }
            cursor += 1;
            if format.get(cursor) == Some(&b'%') {
                output.push(b'%');
                cursor += 1;
                continue;
            }

            let mut left = false;
            let mut plus = false;
            let mut alternate = false;
            let mut zero = false;
            while let Some(flag) = format.get(cursor).copied() {
                match flag {
                    b'-' => left = true,
                    b'+' => plus = true,
                    b'#' => alternate = true,
                    b'0' => zero = true,
                    b' ' => {}
                    _ => break,
                }
                cursor += 1;
            }

            let mut width = 0_usize;
            if format.get(cursor) == Some(&b'*') {
                width = self.esp_printf_argument(argument_slot)? as usize;
                argument_slot += 1;
                cursor += 1;
            } else {
                while let Some(digit @ b'0'..=b'9') = format.get(cursor).copied() {
                    width = width
                        .saturating_mul(10)
                        .saturating_add(usize::from(digit - b'0'));
                    cursor += 1;
                }
            }

            let mut precision = None;
            if format.get(cursor) == Some(&b'.') {
                cursor += 1;
                let mut value = 0_usize;
                if format.get(cursor) == Some(&b'*') {
                    value = self.esp_printf_argument(argument_slot)? as usize;
                    argument_slot += 1;
                    cursor += 1;
                } else {
                    while let Some(digit @ b'0'..=b'9') = format.get(cursor).copied() {
                        value = value
                            .saturating_mul(10)
                            .saturating_add(usize::from(digit - b'0'));
                        cursor += 1;
                    }
                }
                precision = Some(value);
            }

            let mut bits = 32_u8;
            match format.get(cursor).copied() {
                Some(b'h') => {
                    cursor += 1;
                    if format.get(cursor) == Some(&b'h') {
                        bits = 8;
                        cursor += 1;
                    } else {
                        bits = 16;
                    }
                }
                Some(b'l') => {
                    cursor += 1;
                    if format.get(cursor) == Some(&b'l') {
                        bits = 64;
                        cursor += 1;
                    }
                }
                Some(b'j') => {
                    bits = 64;
                    cursor += 1;
                }
                Some(b'z' | b't') => cursor += 1,
                _ => {}
            }
            let conversion = *format
                .get(cursor)
                .ok_or_else(|| "unterminated ets_printf conversion".to_owned())?;
            cursor += 1;

            let value = if bits == 64 {
                if argument_slot & 1 != 0 {
                    argument_slot += 1;
                }
                let low = u64::from(self.esp_printf_argument(argument_slot)?);
                let high = u64::from(self.esp_printf_argument(argument_slot + 1)?);
                argument_slot += 2;
                low | (high << 32)
            } else {
                let value = u64::from(self.esp_printf_argument(argument_slot)?);
                argument_slot += 1;
                value
            };

            let mut rendered = match conversion {
                b'c' => String::from(char::from(value as u8)),
                b's' => {
                    let mut bytes = self.read_guest_c_string(value as u32, 16 * 1024)?;
                    if let Some(precision) = precision {
                        bytes.truncate(precision);
                    }
                    String::from_utf8_lossy(&bytes).into_owned()
                }
                b'd' | b'i' => {
                    let signed = match bits {
                        8 => i64::from(value as i8),
                        16 => i64::from(value as i16),
                        64 => value as i64,
                        _ => i64::from(value as i32),
                    };
                    if plus && signed >= 0 {
                        format!("+{signed}")
                    } else {
                        signed.to_string()
                    }
                }
                b'u' => value.to_string(),
                b'x' => {
                    if alternate {
                        format!("{value:#x}")
                    } else {
                        format!("{value:x}")
                    }
                }
                b'X' => {
                    if alternate {
                        format!("{value:#X}")
                    } else {
                        format!("{value:X}")
                    }
                }
                b'o' => {
                    if alternate {
                        format!("{value:#o}")
                    } else {
                        format!("{value:o}")
                    }
                }
                b'p' => format!("0x{value:08x}"),
                other => {
                    return Err(format!(
                        "unsupported ets_printf conversion %{other}",
                        other = char::from(other)
                    ));
                }
            };
            if rendered.len() < width {
                let fill = if zero && !left { '0' } else { ' ' };
                let padding: String = std::iter::repeat_n(fill, width - rendered.len()).collect();
                rendered = if left {
                    rendered + &padding
                } else {
                    padding + &rendered
                };
            }
            output.extend_from_slice(rendered.as_bytes());
        }

        if let Some(uart) = self.chip_uarts.first() {
            uart.transmit(&output);
        } else {
            self.uart.transmit(&output);
        }
        Ok(output.len() as u32)
    }

    pub(super) fn esp_heap_allocate(
        &mut self,
        handle: u32,
        size: u32,
        alignment: u32,
        offset: u32,
    ) -> Result<u32, String> {
        let heap = self
            .esp_heaps
            .get_mut(&handle)
            .ok_or_else(|| format!("unknown ESP heap handle {handle:#010x}"))?;
        Ok(heap.allocate(size, alignment, offset).unwrap_or(0))
    }

    pub(super) fn esp_heap_reallocate(
        &mut self,
        handle: u32,
        pointer: u32,
        size: u32,
    ) -> Result<u32, String> {
        if pointer == 0 {
            return self.esp_heap_allocate(handle, size, 4, 0);
        }
        if size == 0 {
            if let Some(heap) = self.esp_heaps.get_mut(&handle) {
                heap.release(pointer);
            }
            return Ok(0);
        }
        let old_size = self
            .esp_heaps
            .get(&handle)
            .and_then(|heap| heap.allocations.get(&pointer))
            .copied()
            .ok_or_else(|| format!("unknown allocation {pointer:#010x}"))?;
        if size <= old_size {
            return Ok(pointer);
        }
        let replacement = self.esp_heap_allocate(handle, size, 4, 0)?;
        if replacement == 0 {
            return Ok(0);
        }
        let mut bytes = Vec::with_capacity(old_size as usize);
        for offset in 0..old_size {
            bytes.push(
                self.bus
                    .read(
                        u64::from(pointer + offset),
                        remu_core::AccessWidth::Byte,
                        remu_core::AccessKind::Read,
                        self.now,
                    )
                    .map_err(|error| error.to_string())? as u8,
            );
        }
        self.bus
            .load(u64::from(replacement), &bytes)
            .map_err(|error| error.to_string())?;
        self.esp_heaps
            .get_mut(&handle)
            .expect("validated heap")
            .release(pointer);
        Ok(replacement)
    }

    pub(super) fn write_guest_words(&mut self, address: u32, words: &[u32]) -> Result<(), String> {
        for (index, value) in words.iter().copied().enumerate() {
            self.bus
                .write(
                    u64::from(address.wrapping_add(index as u32 * 4)),
                    remu_core::AccessWidth::Word,
                    u64::from(value),
                    self.now,
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }
}
