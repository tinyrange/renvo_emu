use super::*;

impl RiscVMachine {
    pub(super) fn service_rp2350_bootrom(&mut self) -> Result<bool, String> {
        if self.target != TargetId::Rp2350 {
            return Ok(false);
        }
        let pc = self.cpu.pc();
        match pc {
            0x20 => {
                let code = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let address = self
                    .bootrom_services
                    .iter()
                    .find_map(|(address, stored)| (*stored == code).then_some(*address))
                    .unwrap_or_else(|| {
                        let address = 0x100
                            + u32::try_from(self.bootrom_services.len())
                                .expect("RP ROM service count fits u32")
                                * 4;
                        self.bootrom_services.insert(address, code);
                        address
                    });
                self.complete_host_call(address)?;
                Ok(true)
            }
            address if self.bootrom_services.contains_key(&address) => {
                let code = self.bootrom_services[&address];
                let argument = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let result = match code {
                    0x334c => argument.leading_zeros(),
                    0x3350 => argument.count_ones(),
                    0x3352 => argument.reverse_bits(),
                    0x3354 => argument.trailing_zeros(),
                    0x4649 | 0x5845 | 0x4346 | 0x5843 => argument,
                    0x4552 => {
                        let length = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        self.bus
                            .load(
                                u64::from(0x1000_0000_u32.wrapping_add(argument)),
                                &vec![
                                    0xff;
                                    usize::try_from(length)
                                        .map_err(|_| "flash erase length overflow")?
                                ],
                            )
                            .map_err(|error| error.to_string())?;
                        argument
                    }
                    0x5052 => {
                        let source = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        let length = self
                            .cpu
                            .register(RiscVRegister::A2)
                            .map_err(|error| error.to_string())?;
                        let mut bytes = Vec::with_capacity(
                            usize::try_from(length).map_err(|_| "flash program length overflow")?,
                        );
                        for index in 0..length {
                            bytes.push(
                                self.bus
                                    .read(
                                        u64::from(source.wrapping_add(index)),
                                        renvo_core::AccessWidth::Byte,
                                        renvo_core::AccessKind::Read,
                                        self.now,
                                    )
                                    .map_err(|error| error.to_string())?
                                    as u8,
                            );
                        }
                        self.bus
                            .load(u64::from(0x1000_0000_u32.wrapping_add(argument)), &bytes)
                            .map_err(|error| error.to_string())?;
                        argument
                    }
                    0x5347 => {
                        let capacity = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        let flags = self
                            .cpu
                            .register(RiscVRegister::A2)
                            .map_err(|error| error.to_string())?;
                        let words: Vec<u32> = if flags & 0x0001 != 0 {
                            vec![0x0001, 0x2350_0001, 0x5245_4e56, 0x4f52_5032]
                        } else if flags & 0x0010 != 0 {
                            vec![0x0010, 0x7265_6e76, 0x6f2d_7270, 0x3233_3530, 0x5eed_2026]
                        } else {
                            vec![0]
                        };
                        if capacity
                            < u32::try_from(words.len()).expect("sys-info response is small")
                        {
                            u32::MAX - 12
                        } else {
                            for (index, word) in words.iter().copied().enumerate() {
                                self.bus
                                    .write(
                                        u64::from(argument.wrapping_add(
                                            u32::try_from(index).expect("small index fits u32") * 4,
                                        )),
                                        renvo_core::AccessWidth::Word,
                                        u64::from(word),
                                        self.now,
                                    )
                                    .map_err(|error| error.to_string())?;
                            }
                            u32::try_from(words.len()).expect("sys-info response is small")
                        }
                    }
                    0x434d | 0x3443 => {
                        let source = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        let length = self
                            .cpu
                            .register(RiscVRegister::A2)
                            .map_err(|error| error.to_string())?;
                        for index in 0..length {
                            let byte = self
                                .bus
                                .read(
                                    u64::from(source.wrapping_add(index)),
                                    renvo_core::AccessWidth::Byte,
                                    renvo_core::AccessKind::Read,
                                    self.now,
                                )
                                .map_err(|error| error.to_string())?;
                            self.bus
                                .write(
                                    u64::from(argument.wrapping_add(index)),
                                    renvo_core::AccessWidth::Byte,
                                    byte,
                                    self.now,
                                )
                                .map_err(|error| error.to_string())?;
                        }
                        argument
                    }
                    0x534d | 0x3453 => {
                        let byte = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?
                            & 0xff;
                        let length = self
                            .cpu
                            .register(RiscVRegister::A2)
                            .map_err(|error| error.to_string())?;
                        for index in 0..length {
                            self.bus
                                .write(
                                    u64::from(argument.wrapping_add(index)),
                                    renvo_core::AccessWidth::Byte,
                                    u64::from(byte),
                                    self.now,
                                )
                                .map_err(|error| error.to_string())?;
                        }
                        argument
                    }
                    // BOOTROM_STATE_RESET and other lifecycle operations are deterministic
                    // ordering points in the functional single-core model.
                    0x5253 | 0x4252 | 0x5353 | 0x4152 => 0,
                    _ => {
                        return Err(format!(
                            "unsupported RP2350 RISC-V boot-ROM service code {code:#06x}"
                        ));
                    }
                };
                self.complete_host_call(result)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}
