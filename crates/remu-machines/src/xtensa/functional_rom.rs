use super::*;

impl XtensaMachine {
    fn complete_functional_rom_call(&mut self, result: u32) -> Result<(), String> {
        self.cpu
            .complete_functional_call(result)
            .map_err(|error| error.to_string())
    }

    fn complete_functional_rom_call_u64(&mut self, result: u64) -> Result<(), String> {
        self.cpu
            .set_register(XtensaRegister::A3, (result >> 32) as u32);
        self.complete_functional_rom_call(result as u32)
    }

    fn read_guest_bytes(&mut self, address: u32, length: usize) -> Result<Vec<u8>, String> {
        let mut bytes = Vec::with_capacity(length);
        for offset in 0..length {
            bytes.push(
                self.bus
                    .read(
                        u64::from(address.wrapping_add(offset as u32)),
                        AccessWidth::Byte,
                        AccessKind::Read,
                        self.now,
                    )
                    .map_err(|error| error.to_string())? as u8,
            );
        }
        Ok(bytes)
    }

    fn read_guest_c_string(&mut self, address: u32, limit: usize) -> Result<Vec<u8>, String> {
        let mut bytes = Vec::new();
        for offset in 0..limit {
            let byte = self
                .bus
                .read(
                    u64::from(address.wrapping_add(offset as u32)),
                    AccessWidth::Byte,
                    AccessKind::Read,
                    self.now,
                )
                .map_err(|error| error.to_string())? as u8;
            if byte == 0 {
                break;
            }
            bytes.push(byte);
        }
        Ok(bytes)
    }

    fn functional_rom_printf(&mut self) -> Result<u32, String> {
        let format_address = self.cpu.register(XtensaRegister::A2);
        let format = self.read_guest_c_string(format_address, 4096)?;
        let mut arguments = Vec::with_capacity(5);
        for register in [
            XtensaRegister::A3,
            XtensaRegister::A4,
            XtensaRegister::A5,
            XtensaRegister::A6,
            XtensaRegister::A7,
        ] {
            arguments.push(self.cpu.register(register));
        }
        let mut next_argument = 0;
        let mut output = Vec::new();
        let mut index = 0;
        while index < format.len() {
            if format[index] != b'%' {
                output.push(format[index]);
                index += 1;
                continue;
            }
            index += 1;
            if format.get(index) == Some(&b'%') {
                output.push(b'%');
                index += 1;
                continue;
            }
            let mut zero_pad = false;
            if format.get(index) == Some(&b'0') {
                zero_pad = true;
                index += 1;
            }
            let mut width = 0_usize;
            while let Some(byte @ b'0'..=b'9') = format.get(index).copied() {
                width = width
                    .saturating_mul(10)
                    .saturating_add(usize::from(byte - b'0'));
                index += 1;
            }
            while matches!(format.get(index), Some(b'l' | b'h' | b'z')) {
                index += 1;
            }
            let conversion = format.get(index).copied().unwrap_or_default();
            index += usize::from(index < format.len());
            let argument = arguments.get(next_argument).copied().unwrap_or_default();
            next_argument += 1;
            let rendered = match conversion {
                b's' => self.read_guest_c_string(argument, 4096)?,
                b'c' => vec![argument as u8],
                b'd' | b'i' => (argument as i32).to_string().into_bytes(),
                b'u' => argument.to_string().into_bytes(),
                b'x' => format!("{argument:x}").into_bytes(),
                b'X' => format!("{argument:X}").into_bytes(),
                b'p' => format!("0x{argument:08x}").into_bytes(),
                unknown => vec![b'%', unknown],
            };
            if width > rendered.len() {
                output.extend(std::iter::repeat_n(
                    if zero_pad { b'0' } else { b' ' },
                    width - rendered.len(),
                ));
            }
            output.extend(rendered);
        }
        self.chip_uart.transmit(&output);
        Ok(output.len() as u32)
    }

    fn write_guest_bytes(&mut self, address: u32, bytes: &[u8]) -> Result<(), String> {
        for (offset, byte) in bytes.iter().copied().enumerate() {
            self.bus
                .write(
                    u64::from(address.wrapping_add(offset as u32)),
                    AccessWidth::Byte,
                    u64::from(byte),
                    self.now,
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    pub(super) fn service_functional_rom(&mut self) -> Result<bool, String> {
        let pc = self.cpu.pc();
        // The verified-image handoff has the same externally visible flash
        // state as a completed second-stage bootloader. IDF's early probe has
        // already initialized the static host/chip-driver fields; publish the
        // default chip before FreeRTOS launches application tasks.
        if pc == 0x4213_8764 {
            self.write_guest_bytes(0x3fca_6850, &0x3fca_1dfc_u32.to_le_bytes())?;
            self.write_guest_bytes(0x3fca_1dfc + 20, &(16_u32 * 1024 * 1024).to_le_bytes())?;
            self.write_guest_bytes(0x3fca_1dfc + 24, &0x0016_40c8_u32.to_le_bytes())?;
            return Ok(false);
        }
        // CPU1 has accepted the nonblocking IPC request and entered
        // spi_flash_op_block_func, which publishes s_flash_op_can_start.
        if pc == 0x4037_f1d8 && self.appcpu_boot_address.is_some() {
            self.write_guest_bytes(0x3fca_6847, &[1])?;
            return Ok(false);
        }
        // Execute CPU1 IPC requests synchronously in the functional dual-core
        // model. The cache-block callback's externally visible action is to
        // acknowledge that CPU1 has disabled its scheduler and caches.
        if pc == 0x4200_8718
            && self.cpu.register(XtensaRegister::A2) == 1
            && self.appcpu_boot_address.is_some()
        {
            let callback = self.cpu.register(XtensaRegister::A3);
            if callback == 0x4037_f154 {
                self.write_guest_bytes(0x3fca_6847, &[1])?;
            }
            self.complete_functional_rom_call(0)?;
            return Ok(true);
        }
        // Surface IDF abort diagnostics as stable emulator faults instead of
        // executing the noreturn panic tail into its deliberate trap.
        if pc == 0x4038_5d7c {
            let details = self.cpu.register(XtensaRegister::A2);
            let message = self
                .read_guest_c_string(details, 16 * 1024)
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                .unwrap_or_else(|_| format!("details at {details:#010x}"));
            return Err(format!("ESP-IDF panic: {message}"));
        }
        match pc {
            // ESP-IDF flash API. The functional controller operates on the
            // exact merged image installed by the loader and preserves NOR
            // program/erase behavior for partitions and filesystems.
            0x4038_c624 => {
                let size_out = self.cpu.register(XtensaRegister::A3);
                self.write_guest_bytes(size_out, &(self.flash.len() as u32).to_le_bytes())?;
                self.complete_functional_rom_call(0)?;
            }
            0x4038_c824 => {
                let destination = self.cpu.register(XtensaRegister::A3);
                let offset = self.cpu.register(XtensaRegister::A4) as usize;
                let length = self.cpu.register(XtensaRegister::A5) as usize;
                let end = offset
                    .checked_add(length)
                    .filter(|end| *end <= self.flash.len())
                    .ok_or_else(|| {
                        format!(
                            "ESP flash read {offset:#x}+{length:#x} exceeds {:#x}",
                            self.flash.len()
                        )
                    })?;
                let bytes = self.flash[offset..end].to_vec();
                self.write_guest_bytes(destination, &bytes)?;
                self.complete_functional_rom_call(0)?;
            }
            0x4038_c9d4 => {
                let source = self.cpu.register(XtensaRegister::A3);
                let offset = self.cpu.register(XtensaRegister::A4) as usize;
                let length = self.cpu.register(XtensaRegister::A5) as usize;
                let end = offset
                    .checked_add(length)
                    .filter(|end| *end <= self.flash.len())
                    .ok_or_else(|| {
                        format!(
                            "ESP flash write {offset:#x}+{length:#x} exceeds {:#x}",
                            self.flash.len()
                        )
                    })?;
                let bytes = self.read_guest_bytes(source, length)?;
                for (destination, requested) in
                    self.flash[offset..end].iter_mut().zip(bytes.into_iter())
                {
                    *destination &= requested;
                }
                self.complete_functional_rom_call(0)?;
            }
            0x4038_c418 => {
                let offset = self.cpu.register(XtensaRegister::A3) as usize;
                let length = self.cpu.register(XtensaRegister::A4) as usize;
                let end = offset
                    .checked_add(length)
                    .filter(|end| *end <= self.flash.len())
                    .ok_or_else(|| {
                        format!(
                            "ESP flash erase {offset:#x}+{length:#x} exceeds {:#x}",
                            self.flash.len()
                        )
                    })?;
                self.flash[offset..end].fill(0xff);
                self.complete_functional_rom_call(0)?;
            }
            // rtc_get_reset_reason(cpu): deterministic power-on reset.
            0x4000_057c => self.complete_functional_rom_call(1)?,
            // ets_delay_us: virtual time advances at instruction granularity,
            // so the functional delay completes deterministically.
            0x4000_0600 => self.complete_functional_rom_call(0)?,
            // ets_printf, with the integer/string subset used by ROM and IDF
            // startup diagnostics.
            0x4000_05d0 => {
                let written = self.functional_rom_printf()?;
                self.complete_functional_rom_call(written)?;
            }
            // ROM console byte writers used before the IDF UART driver starts.
            0x4000_0648 | 0x4000_0654 | 0x4000_06b4 => {
                let byte = self.cpu.register(XtensaRegister::A2) as u8;
                self.chip_uart.transmit(&[byte]);
                self.complete_functional_rom_call(0)?;
            }
            // Flush/wait/divisor/console-selection calls complete immediately
            // against the host-drained functional UART.
            0x4000_05e8 | 0x4000_0630 | 0x4000_0690 | 0x4000_069c | 0x4000_06a8 | 0x4000_06c0 => {
                self.complete_functional_rom_call(0)?;
            }
            // Watchdog disable.
            0x4000_0714 => self.complete_functional_rom_call(0)?,
            // ets_set_appcpu_boot_addr(entry). The mask ROM releases CPU1 at
            // this address; Renvo Emulator then interprets both cores over the shared
            // address space.
            0x4000_0720 => {
                let entry = self.cpu.register(XtensaRegister::A2);
                if std::env::var_os("REMU_DEBUG_INTERRUPTS").is_some() {
                    eprintln!(
                        "set appcpu boot entry={entry:#010x} stack={:#010x}",
                        self.stack.wrapping_sub(0x1000)
                    );
                }
                if entry != 0 && self.appcpu_boot_address.is_none() {
                    self.appcpu_boot_address = Some(entry);
                    self.cpu1
                        .set_direct_state(self.stack.wrapping_sub(0x1000), entry);
                    self.cpu1.set_processor_id(1);
                }
                self.complete_functional_rom_call(0)?;
            }
            // ROM watchdog HAL. The emulator has no wall-clock watchdog;
            // configuration, feed, and write-protect operations are therefore
            // deterministic no-ops, while `is_enabled` reports disabled.
            0x4000_0dbc..=0x4000_0e34 => self.complete_functional_rom_call(0)?,
            // Initializes ROM newlib lock indirections. Renvo Emulator's deterministic
            // single-host-thread execution does not require host mutexes.
            0x4000_11dc => self.complete_functional_rom_call(0)?,
            // memset(destination, byte, length)
            0x4000_11e8 => {
                let destination = self.cpu.register(XtensaRegister::A2);
                let byte = self.cpu.register(XtensaRegister::A3) as u8;
                let length = self.cpu.register(XtensaRegister::A4);
                for index in 0..length {
                    self.bus
                        .write(
                            u64::from(destination.wrapping_add(index)),
                            AccessWidth::Byte,
                            u64::from(byte),
                            self.now,
                        )
                        .map_err(|error| error.to_string())?;
                }
                self.complete_functional_rom_call(destination)?;
            }
            // bzero(destination, length)
            0x4000_1260 => {
                let destination = self.cpu.register(XtensaRegister::A2);
                let length = self.cpu.register(XtensaRegister::A3);
                for index in 0..length {
                    self.bus
                        .write(
                            u64::from(destination.wrapping_add(index)),
                            AccessWidth::Byte,
                            0,
                            self.now,
                        )
                        .map_err(|error| error.to_string())?;
                }
                self.complete_functional_rom_call(0)?;
            }
            0x4000_11f4 | 0x4000_1200 => {
                let destination = self.cpu.register(XtensaRegister::A2);
                let source = self.cpu.register(XtensaRegister::A3);
                let length = self.cpu.register(XtensaRegister::A4) as usize;
                let bytes = self.read_guest_bytes(source, length)?;
                self.write_guest_bytes(destination, &bytes)?;
                self.complete_functional_rom_call(destination)?;
            }
            0x4000_120c => {
                let left = self.cpu.register(XtensaRegister::A2);
                let right = self.cpu.register(XtensaRegister::A3);
                let length = self.cpu.register(XtensaRegister::A4) as usize;
                let left = self.read_guest_bytes(left, length)?;
                let right = self.read_guest_bytes(right, length)?;
                let result = left
                    .iter()
                    .zip(right.iter())
                    .find_map(|(left, right)| {
                        (left != right).then(|| i32::from(*left) - i32::from(*right))
                    })
                    .unwrap_or_default();
                self.complete_functional_rom_call(result as u32)?;
            }
            // strcpy/strncpy
            0x4000_1218 | 0x4000_1224 => {
                let destination = self.cpu.register(XtensaRegister::A2);
                let source = self.cpu.register(XtensaRegister::A3);
                let limit = if pc == 0x4000_1224 {
                    self.cpu.register(XtensaRegister::A4) as usize
                } else {
                    1024 * 1024
                };
                let mut terminated = false;
                for offset in 0..limit {
                    let byte = if terminated {
                        0
                    } else {
                        let byte = self
                            .read_guest_bytes(source.wrapping_add(offset as u32), 1)?
                            .into_iter()
                            .next()
                            .unwrap_or_default();
                        terminated = byte == 0;
                        byte
                    };
                    self.write_guest_bytes(destination.wrapping_add(offset as u32), &[byte])?;
                    if pc == 0x4000_1218 && terminated {
                        break;
                    }
                }
                self.complete_functional_rom_call(destination)?;
            }
            // memchr(buffer, byte, length)
            0x4000_1344 => {
                let source = self.cpu.register(XtensaRegister::A2);
                let byte = self.cpu.register(XtensaRegister::A3) as u8;
                let length = self.cpu.register(XtensaRegister::A4);
                let mut found = 0;
                for offset in 0..length {
                    let candidate = self
                        .read_guest_bytes(source.wrapping_add(offset), 1)?
                        .into_iter()
                        .next()
                        .unwrap_or_default();
                    if candidate == byte {
                        found = source.wrapping_add(offset);
                        break;
                    }
                }
                self.complete_functional_rom_call(found)?;
            }
            // strcmp/strncmp. Return the first unsigned-byte difference, as
            // required by newlib, rather than merely a normalized ordering.
            0x4000_1230 | 0x4000_123c => {
                let left = self.cpu.register(XtensaRegister::A2);
                let right = self.cpu.register(XtensaRegister::A3);
                let limit = if pc == 0x4000_123c {
                    self.cpu.register(XtensaRegister::A4) as usize
                } else {
                    1024 * 1024
                };
                let mut result = 0_i32;
                for index in 0..limit {
                    let left_byte = self
                        .read_guest_bytes(left.wrapping_add(index as u32), 1)?
                        .into_iter()
                        .next()
                        .unwrap_or_default();
                    let right_byte = self
                        .read_guest_bytes(right.wrapping_add(index as u32), 1)?
                        .into_iter()
                        .next()
                        .unwrap_or_default();
                    if left_byte != right_byte {
                        result = i32::from(left_byte) - i32::from(right_byte);
                        break;
                    }
                    if left_byte == 0 {
                        break;
                    }
                }
                self.complete_functional_rom_call(result as u32)?;
            }
            // strcat
            0x4000_1374 => {
                let destination = self.cpu.register(XtensaRegister::A2);
                let source = self.cpu.register(XtensaRegister::A3);
                let destination_length =
                    self.read_guest_c_string(destination, 1024 * 1024)?.len() as u32;
                let mut suffix = self.read_guest_c_string(source, 1024 * 1024)?;
                suffix.push(0);
                self.write_guest_bytes(destination.wrapping_add(destination_length), &suffix)?;
                self.complete_functional_rom_call(destination)?;
            }
            // strchr
            0x4000_138c => {
                let string = self.cpu.register(XtensaRegister::A2);
                let needle = (self.cpu.register(XtensaRegister::A3) & 0xff) as u8;
                let bytes = self.read_guest_c_string(string, 1024 * 1024)?;
                let offset = if needle == 0 {
                    Some(bytes.len())
                } else {
                    bytes.iter().position(|byte| *byte == needle)
                };
                let result = offset.map_or(0, |offset| string.wrapping_add(offset as u32));
                self.complete_functional_rom_call(result)?;
            }
            // strcspn
            0x4000_1398 => {
                let string = self.cpu.register(XtensaRegister::A2);
                let reject = self.cpu.register(XtensaRegister::A3);
                let bytes = self.read_guest_c_string(string, 1024 * 1024)?;
                let rejected = self.read_guest_c_string(reject, 1024 * 1024)?;
                let length = bytes
                    .iter()
                    .position(|byte| rejected.contains(byte))
                    .unwrap_or(bytes.len());
                self.complete_functional_rom_call(length as u32)?;
            }
            // strlcpy
            0x4000_13bc => {
                let destination = self.cpu.register(XtensaRegister::A2);
                let source = self.cpu.register(XtensaRegister::A3);
                let capacity = self.cpu.register(XtensaRegister::A4) as usize;
                let bytes = self.read_guest_c_string(source, 1024 * 1024)?;
                if capacity != 0 {
                    let copied = bytes.len().min(capacity - 1);
                    let mut output = bytes[..copied].to_vec();
                    output.push(0);
                    self.write_guest_bytes(destination, &output)?;
                }
                self.complete_functional_rom_call(bytes.len() as u32)?;
            }
            // strrchr
            0x4000_1404 => {
                let string = self.cpu.register(XtensaRegister::A2);
                let needle = (self.cpu.register(XtensaRegister::A3) & 0xff) as u8;
                let bytes = self.read_guest_c_string(string, 1024 * 1024)?;
                let offset = if needle == 0 {
                    Some(bytes.len())
                } else {
                    bytes.iter().rposition(|byte| *byte == needle)
                };
                let result = offset.map_or(0, |offset| string.wrapping_add(offset as u32));
                self.complete_functional_rom_call(result)?;
            }
            // strspn
            0x4000_141c => {
                let string = self.cpu.register(XtensaRegister::A2);
                let accepted = self.cpu.register(XtensaRegister::A3);
                let bytes = self.read_guest_c_string(string, 1024 * 1024)?;
                let accepted = self.read_guest_c_string(accepted, 1024 * 1024)?;
                let length = bytes
                    .iter()
                    .position(|byte| !accepted.contains(byte))
                    .unwrap_or(bytes.len());
                self.complete_functional_rom_call(length as u32)?;
            }
            // strlen
            0x4000_1248 => {
                let string = self.cpu.register(XtensaRegister::A2);
                let length = self.read_guest_c_string(string, 1024 * 1024)?.len();
                self.complete_functional_rom_call(length as u32)?;
            }
            // qsort. IDF startup sorts eight-byte reserved-memory ranges by
            // their first signed address field. Keep that ROM callback
            // contract explicit; singleton arrays are naturally unchanged.
            0x4000_1488 => {
                let base = self.cpu.register(XtensaRegister::A2);
                let count = self.cpu.register(XtensaRegister::A3) as usize;
                let size = self.cpu.register(XtensaRegister::A4) as usize;
                let comparator = self.cpu.register(XtensaRegister::A5);
                if count > 1 {
                    if comparator != 0x4212_82dc || size < 4 {
                        return Err(format!(
                            "unsupported functional qsort comparator {comparator:#010x}, size {size}"
                        ));
                    }
                    let mut records = (0..count)
                        .map(|index| {
                            self.read_guest_bytes(base.wrapping_add((index * size) as u32), size)
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    records.sort_by_key(|record| {
                        i32::from_le_bytes(
                            record[..4].try_into().expect("record is at least 4 bytes"),
                        )
                    });
                    for (index, record) in records.iter().enumerate() {
                        self.write_guest_bytes(base.wrapping_add((index * size) as u32), record)?;
                    }
                }
                self.complete_functional_rom_call(0)?;
            }
            // Newlib non-local control transfer. Save the logical windowed
            // context after setjmp has returned to its caller; longjmp
            // reinstates it and supplies the non-zero call8 return value.
            0x4000_144c => {
                let environment = self.cpu.register(XtensaRegister::A2);
                self.complete_functional_rom_call(0)?;
                self.setjmp_contexts.insert(environment, self.cpu.clone());
            }
            // ESP-IDF wraps the ESP32-S3 ROM longjmp so WINDOWSTART can be
            // repaired in a critical section. The interpreter keeps logical
            // register windows directly, so both entry points perform the
            // same non-local restoration. This must not return through the
            // wrapper: doing so would leave MicroPython's NLR frame active.
            0x4000_1440 | 0x4212_b548 => {
                let environment = self.cpu.register(XtensaRegister::A2);
                let value = self.cpu.register(XtensaRegister::A3).max(1);
                let mut restored = self
                    .setjmp_contexts
                    .get(&environment)
                    .cloned()
                    .ok_or_else(|| format!("longjmp used unknown environment {environment:#x}"))?;
                restored.set_register(XtensaRegister::A10, value);
                self.cpu = restored;
            }
            // Newlib expands sqrtf into the Xtensa coprocessor's reciprocal
            // approximation/refinement sequence. At the platform's declared
            // functional fidelity, evaluate the public helper atomically
            // while retaining IEEE-754 single-precision behavior.
            0x4212_e2c4 => {
                let value = f32::from_bits(self.cpu.register(XtensaRegister::A2));
                self.complete_functional_rom_call(value.sqrt().to_bits())?;
            }
            // Mbed TLS SHA-224/SHA-256 API backed by deterministic host
            // hashing. This is the functional equivalent of the ESP32-S3
            // SHA accelerator, including incremental and cloned contexts.
            0x420d_37a8 => {
                let context = self.cpu.register(XtensaRegister::A2);
                self.sha256_contexts
                    .insert(context, FunctionalSha256::default());
                self.complete_functional_rom_call(0)?;
            }
            0x4212_cfe8 => {
                let context = self.cpu.register(XtensaRegister::A2);
                self.sha256_contexts.remove(&context);
                self.complete_functional_rom_call(0)?;
            }
            0x420d_37bc => {
                let destination = self.cpu.register(XtensaRegister::A2);
                let source = self.cpu.register(XtensaRegister::A3);
                let state = self
                    .sha256_contexts
                    .get(&source)
                    .cloned()
                    .unwrap_or_default();
                self.sha256_contexts.insert(destination, state);
                self.complete_functional_rom_call(0)?;
            }
            0x420d_37d0 => {
                let context = self.cpu.register(XtensaRegister::A2);
                let sha224 = self.cpu.register(XtensaRegister::A3) != 0;
                self.sha256_contexts.insert(
                    context,
                    FunctionalSha256 {
                        sha224,
                        input: Vec::new(),
                    },
                );
                self.complete_functional_rom_call(0)?;
            }
            0x420d_37f0 => {
                let context = self.cpu.register(XtensaRegister::A2);
                let input = self.cpu.register(XtensaRegister::A3);
                let length = self.cpu.register(XtensaRegister::A4) as usize;
                let bytes = self.read_guest_bytes(input, length)?;
                self.sha256_contexts
                    .entry(context)
                    .or_default()
                    .input
                    .extend_from_slice(&bytes);
                self.complete_functional_rom_call(0)?;
            }
            0x420d_3938 => {
                let context = self.cpu.register(XtensaRegister::A2);
                let output = self.cpu.register(XtensaRegister::A3);
                let state = self
                    .sha256_contexts
                    .get(&context)
                    .cloned()
                    .unwrap_or_default();
                let digest = if state.sha224 {
                    Sha224::digest(&state.input).to_vec()
                } else {
                    Sha256::digest(&state.input).to_vec()
                };
                self.write_guest_bytes(output, &digest)?;
                self.complete_functional_rom_call(0)?;
            }
            // Newlib integer helpers.
            0x4000_1458 | 0x4000_1470 => {
                let value = self.cpu.register(XtensaRegister::A2) as i32;
                self.complete_functional_rom_call(value.wrapping_abs() as u32)?;
            }
            0x4000_1464 | 0x4000_147c => {
                let numerator = self.cpu.register(XtensaRegister::A2) as i32;
                let denominator = self.cpu.register(XtensaRegister::A3) as i32;
                let quotient = if denominator == 0 {
                    0
                } else {
                    numerator.checked_div(denominator).unwrap_or(i32::MIN)
                };
                let remainder = if denominator == 0 {
                    numerator
                } else {
                    numerator.checked_rem(denominator).unwrap_or_default()
                };
                self.complete_functional_rom_call_u64(
                    u64::from(quotient as u32) | (u64::from(remainder as u32) << 32),
                )?;
            }
            // utoa/itoa
            0x4000_14b8 | 0x4000_14c4 => {
                let raw = self.cpu.register(XtensaRegister::A2);
                let destination = self.cpu.register(XtensaRegister::A3);
                let radix = self.cpu.register(XtensaRegister::A4);
                let signed = pc == 0x4000_14c4 && radix == 10 && (raw as i32) < 0;
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
                self.complete_functional_rom_call(destination)?;
            }
            // Instruction/data cache configuration and resume routines. Direct
            // application loading has already established coherent mappings.
            0x4000_19b0 => {
                let virtual_address = self.cpu.register(XtensaRegister::A3);
                let physical_address = self.cpu.register(XtensaRegister::A4) as usize;
                let page_size_kib = self.cpu.register(XtensaRegister::A5) as usize;
                let pages = self.cpu.register(XtensaRegister::A6) as usize;
                let fixed = self.cpu.register(XtensaRegister::A7) != 0;
                let page_size = page_size_kib.saturating_mul(1024);
                if page_size != 64 * 1024 {
                    self.complete_functional_rom_call(3)?;
                } else {
                    for page in 0..pages {
                        let source = physical_address
                            .checked_add(if fixed { 0 } else { page * page_size })
                            .ok_or_else(|| "ESP flash MMU source overflow".to_owned())?;
                        let end = source
                            .checked_add(page_size)
                            .filter(|end| *end <= self.flash.len())
                            .ok_or_else(|| {
                                format!(
                                    "ESP flash MMU map {source:#x}..{:#x} exceeds image",
                                    source.saturating_add(page_size)
                                )
                            })?;
                        let destination = virtual_address.wrapping_add((page * page_size) as u32);
                        self.bus
                            .load(u64::from(destination), &self.flash[source..end])
                            .map_err(|error| error.to_string())?;
                    }
                    self.complete_functional_rom_call(0)?;
                }
            }
            // rom_config_instruction_cache_mode(0x4000, 8, 32) enables the
            // application IROM window after the IRAM bootstrap has validated
            // its 64-KiB cache mode. Keep the handoff gated until this call.
            0x4000_1a1c => {
                let valid = self.cpu.register(XtensaRegister::A2) == 0x4000
                    && self.cpu.register(XtensaRegister::A3) == 8
                    && self.cpu.register(XtensaRegister::A4) == 32;
                if valid {
                    self.instruction_cache_configured = true;
                    self.extmem.configure_boot_caches();
                }
                self.complete_functional_rom_call(u32::from(!valid))?;
            }
            0x4000_15fc..=0x4000_1a28 => {
                self.complete_functional_rom_call(0)?;
            }
            // ROM clock query/update services.
            0x4000_1a34 => self.complete_functional_rom_call(80_000_000)?,
            0x4000_1a40 => self.complete_functional_rom_call(240)?,
            0x4000_1a4c => self.complete_functional_rom_call(0)?,
            // ROM GPIO matrix and pad helpers. The register-level GPIO model
            // retains pin state; routing and pad policy complete immediately.
            0x4000_1a58..=0x4000_1b48 => self.complete_functional_rom_call(0)?,
            // Mask-ROM MD5 API used to validate partition metadata.
            0x4000_1c5c => {
                let context = self.cpu.register(XtensaRegister::A2);
                self.md5_contexts.insert(context, Vec::new());
                self.write_guest_bytes(context, &[0; 88])?;
                self.complete_functional_rom_call(0)?;
            }
            0x4000_1c68 => {
                let context = self.cpu.register(XtensaRegister::A2);
                let input = self.cpu.register(XtensaRegister::A3);
                let length = self.cpu.register(XtensaRegister::A4) as usize;
                let bytes = self.read_guest_bytes(input, length)?;
                self.md5_contexts
                    .entry(context)
                    .or_default()
                    .extend_from_slice(&bytes);
                self.complete_functional_rom_call(0)?;
            }
            0x4000_1c74 => {
                let digest_address = self.cpu.register(XtensaRegister::A2);
                let context = self.cpu.register(XtensaRegister::A3);
                let message = self.md5_contexts.remove(&context).unwrap_or_default();
                let digest = Md5::digest(message);
                self.write_guest_bytes(digest_address, digest.as_slice())?;
                self.complete_functional_rom_call(0)?;
            }
            // ROM CRC32 helpers retain the caller-supplied accumulator.
            0x4000_1c98 | 0x4000_1ca4 => {
                let mut crc = self.cpu.register(XtensaRegister::A2);
                let input = self.cpu.register(XtensaRegister::A3);
                let length = self.cpu.register(XtensaRegister::A4) as usize;
                for byte in self.read_guest_bytes(input, length)? {
                    if pc == 0x4000_1c98 {
                        crc ^= u32::from(byte);
                        for _ in 0..8 {
                            crc = (crc >> 1) ^ (0xedb8_8320 & 0_u32.wrapping_sub(crc & 1));
                        }
                    } else {
                        crc ^= u32::from(byte) << 24;
                        for _ in 0..8 {
                            crc = (crc << 1) ^ (0x04c1_1db7 & 0_u32.wrapping_sub(crc >> 31));
                        }
                    }
                }
                self.complete_functional_rom_call(crc)?;
            }
            // Functional blank/default eFuse policy: unsecured boot, default
            // SPI pads, USB enabled, and no burned feature-disable bits.
            0x4000_1ef0..=0x4000_2028 => self.complete_functional_rom_call(0)?,
            // _xtos_set_intlevel(level)
            0x4000_1c38 => {
                let level = self.cpu.register(XtensaRegister::A2);
                let previous = self.cpu.set_interrupt_level(level);
                self.complete_functional_rom_call(previous)?;
            }
            // intr_matrix_set(cpu, source, interrupt): retain deterministic
            // routing state for subsequent peripheral interrupt delivery.
            0x4000_1b54 => {
                let cpu = self.cpu.register(XtensaRegister::A2);
                let source = self.cpu.register(XtensaRegister::A3);
                let interrupt = self.cpu.register(XtensaRegister::A4);
                if std::env::var_os("REMU_DEBUG_INTERRUPTS").is_some() {
                    eprintln!("interrupt route cpu={cpu} source={source} line={interrupt}");
                }
                if let (Ok(core), Ok(source)) = (usize::try_from(cpu), usize::try_from(source)) {
                    self.interrupt_matrix.set_route(core, source, interrupt);
                }
                self.complete_functional_rom_call(0)?;
            }
            // ROM analog-I2C register helpers used during clock/PHY setup.
            0x4000_5cd0 | 0x4000_5cdc | 0x4000_5d48 | 0x4000_5d54 | 0x4000_5d60 | 0x4000_5d6c => {
                self.complete_functional_rom_call(0)?
            }
            // Coexistence-ROM build identifier. The real ROM returns a
            // persistent C string; expose an emulator-owned string in the
            // modeled ROM data window so IDF can copy and report it normally.
            0x4000_5b68 => self.complete_functional_rom_call(0x3ff1_e000)?,
            // ROM libgcc scalar floating-point helpers. Keep arguments and
            // results as raw ABI payloads so NaNs and signed zero survive.
            0x4000_2190 | 0x4000_2274 | 0x4000_243c | 0x4000_2508 => {
                let left = f32::from_bits(self.cpu.register(XtensaRegister::A2));
                let right = f32::from_bits(self.cpu.register(XtensaRegister::A3));
                let result = match pc {
                    0x4000_2190 => left + right,
                    0x4000_2274 => left / right,
                    0x4000_243c => left * right,
                    0x4000_2508 => left - right,
                    _ => unreachable!(),
                };
                self.complete_functional_rom_call(result.to_bits())?;
            }
            0x4000_2184 | 0x4000_2250 | 0x4000_2418 | 0x4000_24fc => {
                let left = f64::from_bits(
                    u64::from(self.cpu.register(XtensaRegister::A2))
                        | (u64::from(self.cpu.register(XtensaRegister::A3)) << 32),
                );
                let right = f64::from_bits(
                    u64::from(self.cpu.register(XtensaRegister::A4))
                        | (u64::from(self.cpu.register(XtensaRegister::A5)) << 32),
                );
                let result = match pc {
                    0x4000_2184 => left + right,
                    0x4000_2250 => left / right,
                    0x4000_2418 => left * right,
                    0x4000_24fc => left - right,
                    _ => unreachable!(),
                };
                self.complete_functional_rom_call_u64(result.to_bits())?;
            }
            0x4000_2490 => {
                let value = self.cpu.register(XtensaRegister::A2);
                self.complete_functional_rom_call(value ^ (1 << 31))?;
            }
            0x4000_2478 => {
                let value = u64::from(self.cpu.register(XtensaRegister::A2))
                    | (u64::from(self.cpu.register(XtensaRegister::A3)) << 32);
                self.complete_functional_rom_call_u64(value ^ (1_u64 << 63))?;
            }
            0x4000_22a4 => {
                let value = f32::from_bits(self.cpu.register(XtensaRegister::A2));
                self.complete_functional_rom_call_u64(f64::from(value).to_bits())?;
            }
            0x4000_252c => {
                let value = f64::from_bits(
                    u64::from(self.cpu.register(XtensaRegister::A2))
                        | (u64::from(self.cpu.register(XtensaRegister::A3)) << 32),
                );
                self.complete_functional_rom_call((value as f32).to_bits())?;
            }
            0x4000_22ec | 0x4000_2310 => {
                let value = f32::from_bits(self.cpu.register(XtensaRegister::A2));
                let result = if pc == 0x4000_22ec {
                    (value as i32) as u32
                } else {
                    value as u32
                };
                self.complete_functional_rom_call(result)?;
            }
            0x4000_22e0 | 0x4000_2304 => {
                let value = f32::from_bits(self.cpu.register(XtensaRegister::A2));
                let result = if pc == 0x4000_22e0 {
                    (value as i64) as u64
                } else {
                    value as u64
                };
                self.complete_functional_rom_call_u64(result)?;
            }
            0x4000_22d4 | 0x4000_22f8 => {
                let value = f64::from_bits(
                    u64::from(self.cpu.register(XtensaRegister::A2))
                        | (u64::from(self.cpu.register(XtensaRegister::A3)) << 32),
                );
                let result = if pc == 0x4000_22d4 {
                    (value as i32) as u32
                } else {
                    value as u32
                };
                self.complete_functional_rom_call(result)?;
            }
            0x4000_2340 | 0x4000_2370 => {
                let value = self.cpu.register(XtensaRegister::A2);
                let result = if pc == 0x4000_2340 {
                    (value as i32) as f32
                } else {
                    value as f32
                };
                self.complete_functional_rom_call(result.to_bits())?;
            }
            0x4000_2334 | 0x4000_2364 => {
                let value = self.cpu.register(XtensaRegister::A2);
                let result = if pc == 0x4000_2334 {
                    f64::from(value as i32)
                } else {
                    f64::from(value)
                };
                self.complete_functional_rom_call_u64(result.to_bits())?;
            }
            0x4000_2328 | 0x4000_2358 => {
                let value = u64::from(self.cpu.register(XtensaRegister::A2))
                    | (u64::from(self.cpu.register(XtensaRegister::A3)) << 32);
                let result = if pc == 0x4000_2328 {
                    (value as i64) as f32
                } else {
                    value as f32
                };
                self.complete_functional_rom_call(result.to_bits())?;
            }
            0x4000_24f0 => {
                let value = f32::from_bits(self.cpu.register(XtensaRegister::A2));
                let exponent = self.cpu.register(XtensaRegister::A3) as i32;
                self.complete_functional_rom_call(value.powi(exponent).to_bits())?;
            }
            0x4000_2298 | 0x4000_2394 | 0x4000_23ac | 0x4000_23c4 | 0x4000_23e8 | 0x4000_24b4
            | 0x4000_2598 => {
                let left = f32::from_bits(self.cpu.register(XtensaRegister::A2));
                let right = f32::from_bits(self.cpu.register(XtensaRegister::A3));
                let unordered = left.is_nan() || right.is_nan();
                let result = match pc {
                    0x4000_2298 | 0x4000_24b4 => i32::from(left != right || unordered),
                    0x4000_2394 => {
                        if unordered || left < right {
                            -1
                        } else {
                            i32::from(left > right)
                        }
                    }
                    0x4000_23ac => {
                        if unordered {
                            -1
                        } else {
                            i32::from(left > right)
                        }
                    }
                    0x4000_23c4 => {
                        if unordered || left > right {
                            1
                        } else {
                            -i32::from(left < right)
                        }
                    }
                    0x4000_23e8 => {
                        if unordered {
                            1
                        } else {
                            -i32::from(left < right)
                        }
                    }
                    0x4000_2598 => i32::from(unordered),
                    _ => unreachable!(),
                };
                self.complete_functional_rom_call(result as u32)?;
            }
            // Compiler-runtime unsigned division and remainder helpers.
            0x4000_225c | 0x4000_23f4 => {
                let numerator = (u64::from(self.cpu.register(XtensaRegister::A2))
                    | (u64::from(self.cpu.register(XtensaRegister::A3)) << 32))
                    as i64;
                let denominator = (u64::from(self.cpu.register(XtensaRegister::A4))
                    | (u64::from(self.cpu.register(XtensaRegister::A5)) << 32))
                    as i64;
                let result = if denominator == 0 {
                    if pc == 0x4000_225c { -1 } else { numerator }
                } else if pc == 0x4000_225c {
                    numerator.checked_div(denominator).unwrap_or(i64::MIN)
                } else {
                    numerator.checked_rem(denominator).unwrap_or_default()
                };
                self.complete_functional_rom_call_u64(result as u64)?;
            }
            0x4000_2280 | 0x4000_2400 => {
                let numerator = self.cpu.register(XtensaRegister::A2) as i32;
                let denominator = self.cpu.register(XtensaRegister::A3) as i32;
                let result = if denominator == 0 {
                    if pc == 0x4000_2280 { -1 } else { numerator }
                } else if pc == 0x4000_2280 {
                    numerator.checked_div(denominator).unwrap_or(i32::MIN)
                } else {
                    numerator.checked_rem(denominator).unwrap_or_default()
                };
                self.complete_functional_rom_call(result as u32)?;
            }
            0x4000_2544 | 0x4000_2574 => {
                let numerator = u64::from(self.cpu.register(XtensaRegister::A2))
                    | (u64::from(self.cpu.register(XtensaRegister::A3)) << 32);
                let denominator = u64::from(self.cpu.register(XtensaRegister::A4))
                    | (u64::from(self.cpu.register(XtensaRegister::A5)) << 32);
                let result = if denominator == 0 {
                    u64::MAX
                } else if pc == 0x4000_2544 {
                    numerator / denominator
                } else {
                    numerator % denominator
                };
                self.complete_functional_rom_call_u64(result)?;
            }
            0x4000_255c | 0x4000_2580 => {
                let numerator = self.cpu.register(XtensaRegister::A2);
                let denominator = self.cpu.register(XtensaRegister::A3);
                let result = if denominator == 0 {
                    u32::MAX
                } else if pc == 0x4000_255c {
                    numerator / denominator
                } else {
                    numerator % denominator
                };
                self.complete_functional_rom_call(result)?;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
}
