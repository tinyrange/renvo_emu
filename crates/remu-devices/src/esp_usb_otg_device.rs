use super::*;

impl Device for EspUsbOtg {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP USB OTG core requires aligned word access",
            ));
        }
        let mut state = self.state.lock().expect("ESP USB OTG state lock poisoned");
        let value = match offset {
            offset if offset == EspUsbOtgRegister::GintSts.offset() => state.interrupt_status(),
            offset if offset == EspUsbOtgRegister::GrxStsR.offset() => {
                state.rx_status.front().copied().unwrap_or(0)
            }
            offset if offset == EspUsbOtgRegister::GrxStsP.offset() => state.pop_rx_status(),
            offset if offset == EspUsbOtgRegister::Daint.offset() => state.endpoint_interrupts(),
            offset if (0x1000..0x1_0000).contains(&offset) => {
                state.rx_fifo.pop_front().unwrap_or(0)
            }
            offset
                if matches!(
                    EspUsbOtgRegister::from_offset(offset),
                    Some(EspUsbOtgRegister::DiepInt(_))
                ) =>
            {
                let register = EspUsbOtgRegister::from_offset(offset).expect("DIEPINT offset");
                let EspUsbOtgRegister::DiepInt(endpoint) = register else {
                    unreachable!();
                };
                let mut value = state.register(register);
                if state.register(EspUsbOtgRegister::DiepEmpMsk) & (1 << endpoint) != 0
                    && state.register(EspUsbOtgRegister::DiepCtl(endpoint)) & DWC2_EPENA != 0
                {
                    value |= 1 << 7;
                }
                value
            }
            _ => state
                .registers
                .get(usize::try_from(offset / 4).expect("USB OTG offset fits"))
                .copied()
                .ok_or_else(|| DeviceError::new(format!("{} read at {offset:#x}", self.name)))?,
        };
        if std::env::var_os("REMU_DEBUG_USB").is_some()
            && (offset == EspUsbOtgRegister::GintSts.offset()
                || offset == EspUsbOtgRegister::Daint.offset()
                || matches!(
                    EspUsbOtgRegister::from_offset(offset),
                    Some(
                        EspUsbOtgRegister::DiepInt(_)
                            | EspUsbOtgRegister::DoepInt(_)
                            | EspUsbOtgRegister::DiepCtl(_)
                            | EspUsbOtgRegister::DoepCtl(_)
                    )
                ))
        {
            eprintln!("dwc2 reg read {offset:#x} -> {value:#x}");
        }
        Ok(u64::from(value))
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
                "ESP USB OTG core requires aligned word access",
            ));
        }
        let index = usize::try_from(offset / 4).expect("USB OTG offset fits");
        let mut state = self.state.lock().expect("ESP USB OTG state lock poisoned");
        if std::env::var_os("REMU_DEBUG_USB").is_some()
            && (offset == EspUsbOtgRegister::GintSts.offset()
                || offset == EspUsbOtgRegister::Daint.offset()
                || matches!(
                    EspUsbOtgRegister::from_offset(offset),
                    Some(
                        EspUsbOtgRegister::DiepInt(_)
                            | EspUsbOtgRegister::DoepInt(_)
                            | EspUsbOtgRegister::DiepCtl(_)
                            | EspUsbOtgRegister::DoepCtl(_)
                    )
                ))
        {
            eprintln!("dwc2 reg write {offset:#x} <- {value:#x}");
        }
        if (0x1000..0x1_0000).contains(&offset) {
            let endpoint =
                usize::try_from((offset - 0x1000) / 0x1000).expect("endpoint number fits usize");
            state.write_fifo(endpoint, value as u32);
            return Ok(());
        }
        if index >= state.registers.len() {
            return Err(DeviceError::new(format!(
                "{} write at {offset:#x}",
                self.name
            )));
        }
        let register = EspUsbOtgRegister::from_offset(offset);
        if offset == EspUsbOtgRegister::GotgInt.offset()
            || offset == EspUsbOtgRegister::GintSts.offset()
        {
            // GOTGINT and writable GINTSTS causes are write-one-to-clear. RO summary bits
            // (including RXFLVL, IEPINT, and OEPINT) cannot be cleared by the application.
            let clear_mask = if offset == EspUsbOtgRegister::GotgInt.offset() {
                DWC2_GOTGINT_W1C_MASK
            } else {
                DWC2_GINTSTS_W1C_MASK
            };
            state.registers[index] &= !(value as u32 & clear_mask);
        } else if offset == EspUsbOtgRegister::GrstCtl.offset() {
            // CSRST and the FIFO flush strobes self-clear once the functional
            // operation has completed. AHB remains idle for the next access.
            state.registers[index] = value as u32 & !((1 << 0) | (1 << 4) | (1 << 5));
            state.registers[index] |= 1 << 31;
            if value & (1 << 4) != 0 {
                state.rx_status.clear();
                state.rx_fifo.clear();
            }
            if value & (1 << 5) != 0 {
                for fifo in &mut state.tx_fifo {
                    fifo.clear();
                }
            }
        } else if offset == EspUsbOtgRegister::Dctl.offset() {
            let value = value as u32;
            let mut next = (state.registers[index] & DWC2_DCTL_NAK_STATUS_MASK)
                | (value & DWC2_DCTL_CONFIG_MASK);
            if value & (1 << 7) != 0 || value & (1 << 9) != 0 {
                // Global NAK effective is observable synchronously.
                next |= 1 << 2;
                *state.register_mut(EspUsbOtgRegister::GintSts) |= 1 << 7;
            }
            if value & (1 << 8) != 0 || value & (1 << 10) != 0 {
                next &= !(1 << 2);
                *state.register_mut(EspUsbOtgRegister::GintSts) &= !(1 << 7);
            }
            state.registers[index] = next;
        } else if matches!(
            register,
            Some(EspUsbOtgRegister::DiepInt(_) | EspUsbOtgRegister::DoepInt(_))
        ) {
            // Endpoint interrupt registers are write-one-to-clear.
            let clear_mask = match register {
                Some(EspUsbOtgRegister::DiepInt(_)) => DWC2_DIEPINT_W1C_MASK,
                Some(EspUsbOtgRegister::DoepInt(_)) => DWC2_DOEPINT_W1C_MASK,
                _ => unreachable!(),
            };
            state.registers[index] &= !(value as u32 & clear_mask);
        } else if let Some(
            fifo_register @ (EspUsbOtgRegister::GrxFsiz
            | EspUsbOtgRegister::GnptxFsiz
            | EspUsbOtgRegister::GdfifoCfg
            | EspUsbOtgRegister::HptxFsiz
            | EspUsbOtgRegister::DiepTxFifo(_)),
        ) = register
        {
            let value = if fifo_register == EspUsbOtgRegister::GrxFsiz {
                value as u32 & 0xffff
            } else {
                value as u32
            };
            state.configure_fifo(fifo_register, value)?;
        } else if let Some(EspUsbOtgRegister::DiepCtl(endpoint)) = register {
            let endpoint = usize::from(endpoint);
            let value = value as u32;
            if value & DWC2_EPENA != 0 && state.transmit_fifo_words(endpoint as u8) == 0 {
                return Err(DeviceError::new(format!(
                    "ESP32-S3 DWC2 endpoint {endpoint} has no configured transmit FIFO"
                )));
            }
            let current = state.registers[index];
            // MPS, STALL, and TxFIFO number are ordinary configuration fields. Active,
            // endpoint type, and NAK status are core-owned; SNAK/CNAK/EPDIS/EPENA are command
            // bits with the documented immediate side effects.
            let mut next =
                (current & !DWC2_DIEPCTL_CONFIG_MASK) | (value & DWC2_DIEPCTL_CONFIG_MASK);
            if value & (1 << 27) != 0 {
                next |= 1 << 17;
            }
            if value & (1 << 26) != 0 {
                next &= !(1 << 17);
            }
            if value & DWC2_EPDIS != 0 {
                next &= !DWC2_EPENA;
                *state.register_mut(EspUsbOtgRegister::DiepInt(endpoint as u8)) |= 1 << 1;
            }
            if value & DWC2_EPENA != 0 {
                next |= DWC2_EPENA;
                let size = usize::try_from(
                    state.register(EspUsbOtgRegister::DiepTsiz(endpoint as u8))
                        & dwc2_xfer_size_mask(endpoint as u8),
                )
                .expect("DWC2 transfer size fits usize");
                state.in_transfer_size[endpoint] = size;
                state.tx_fifo[endpoint].clear();
            }
            state.registers[index] = next;
        } else if let Some(EspUsbOtgRegister::DoepCtl(endpoint)) = register {
            let endpoint = usize::from(endpoint);
            let value = value as u32;
            let current = state.registers[index];
            let mut next =
                (current & !DWC2_DOEPCTL_CONFIG_MASK) | (value & DWC2_DOEPCTL_CONFIG_MASK);
            if value & (1 << 27) != 0 {
                next |= 1 << 17;
            }
            if value & (1 << 26) != 0 {
                next &= !(1 << 17);
            }
            if value & DWC2_EPDIS != 0 {
                next &= !DWC2_EPENA;
                *state.register_mut(EspUsbOtgRegister::DoepInt(endpoint as u8)) |= 1 << 1;
            }
            if value & DWC2_EPENA != 0 {
                next |= DWC2_EPENA;
            }
            state.registers[index] = next;
        } else if let Some(EspUsbOtgRegister::Dcfg) = register {
            state.registers[index] = value as u32 & DWC2_DCFG_MASK;
        } else if let Some(EspUsbOtgRegister::GintMsk) = register {
            state.registers[index] = value as u32 & DWC2_GINTMSK_MASK;
        } else if let Some(EspUsbOtgRegister::DiepMsk) = register {
            state.registers[index] = value as u32 & DWC2_DIEPMSK_MASK;
        } else if let Some(EspUsbOtgRegister::DoepMsk) = register {
            state.registers[index] = value as u32 & DWC2_DOEPMSK_MASK;
        } else if matches!(
            register,
            Some(
                EspUsbOtgRegister::GsnpsId
                    | EspUsbOtgRegister::GhwCfg2
                    | EspUsbOtgRegister::Dsts
                    | EspUsbOtgRegister::Daint
                    | EspUsbOtgRegister::GrxStsR
                    | EspUsbOtgRegister::GrxStsP
                    | EspUsbOtgRegister::DtxfSts(_)
            )
        ) {
            return Err(DeviceError::new("ESP USB OTG register is read-only"));
        } else if let Some(EspUsbOtgRegister::DaintMsk) = register {
            state.registers[index] = value as u32 & DWC2_DAINT_MASK;
        } else if let Some(EspUsbOtgRegister::DiepEmpMsk) = register {
            state.registers[index] = value as u32 & DWC2_DIEPEMP_MASK;
        } else if let Some(EspUsbOtgRegister::DiepTsiz(endpoint)) = register {
            let value = value as u32;
            let size_mask = dwc2_xfer_size_mask(endpoint);
            state.registers[index] = value & (size_mask | dwc2_in_pkt_count_mask(endpoint));
            state.in_transfer_size[usize::from(endpoint)] =
                usize::try_from(value & size_mask).expect("DWC2 transfer size fits");
        } else if let Some(EspUsbOtgRegister::DoepTsiz(endpoint)) = register {
            if endpoint == 0 && value as u32 & DWC2_OUT_SETUP_COUNT_MASK != 0 {
                state.setup_receive_enabled = true;
            }
            let setup_mask = if endpoint == 0 {
                DWC2_OUT_SETUP_COUNT_MASK
            } else {
                0
            };
            state.registers[index] = value as u32
                & (dwc2_xfer_size_mask(endpoint) | dwc2_out_pkt_count_mask(endpoint) | setup_mask);
        } else {
            state.registers[index] = value as u32;
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("ESP USB OTG state lock poisoned") = EspUsbOtgState::reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(device: &mut EspUsbOtg, register: EspUsbOtgRegister, value: u32) {
        device
            .write(
                register.offset(),
                AccessWidth::Word,
                u64::from(value),
                SimTime::ZERO,
            )
            .unwrap();
    }

    #[test]
    fn shared_fifo_rejects_overlap_and_capacity_overflow() {
        let (mut device, _) = EspUsbOtg::new("usb");
        write(&mut device, EspUsbOtgRegister::GrxFsiz, 80);
        write(&mut device, EspUsbOtgRegister::GnptxFsiz, (64 << 16) | 0x50);
        write(
            &mut device,
            EspUsbOtgRegister::DiepTxFifo(1),
            (32 << 16) | 0x90,
        );

        assert!(
            device
                .write(
                    EspUsbOtgRegister::DiepTxFifo(2).offset(),
                    AccessWidth::Word,
                    u64::from((32_u32 << 16) | 0xa0),
                    SimTime::ZERO,
                )
                .is_err()
        );
        assert!(
            device
                .write(
                    EspUsbOtgRegister::DiepTxFifo(2).offset(),
                    AccessWidth::Word,
                    u64::from((32_u32 << 16) | 0xf0),
                    SimTime::ZERO,
                )
                .is_err()
        );
    }

    #[test]
    fn host_fifo_does_not_configure_device_endpoint_one() {
        let (mut device, _) = EspUsbOtg::new("usb");
        write(&mut device, EspUsbOtgRegister::GrxFsiz, 64);
        write(&mut device, EspUsbOtgRegister::GnptxFsiz, (64 << 16) | 0x40);
        write(&mut device, EspUsbOtgRegister::HptxFsiz, (32 << 16) | 0x80);

        assert!(
            device
                .write(
                    EspUsbOtgRegister::DiepCtl(1).offset(),
                    AccessWidth::Word,
                    u64::from(DWC2_EPENA),
                    SimTime::ZERO,
                )
                .is_err()
        );
        write(
            &mut device,
            EspUsbOtgRegister::DiepTxFifo(1),
            (32 << 16) | 0xa0,
        );
        write(&mut device, EspUsbOtgRegister::DiepCtl(1), DWC2_EPENA);
    }

    #[test]
    fn data_endpoints_keep_wide_transfer_size_and_packet_count_fields() {
        let (mut device, _) = EspUsbOtg::new("usb");
        let input = 192 | (3 << 19);
        let output = 512 | (8 << 19);
        write(&mut device, EspUsbOtgRegister::DiepTsiz(1), input);
        write(&mut device, EspUsbOtgRegister::DoepTsiz(2), output);

        assert_eq!(
            device
                .read(
                    EspUsbOtgRegister::DiepTsiz(1).offset(),
                    AccessWidth::Word,
                    SimTime::ZERO,
                )
                .unwrap(),
            u64::from(input)
        );
        assert_eq!(
            device
                .read(
                    EspUsbOtgRegister::DoepTsiz(2).offset(),
                    AccessWidth::Word,
                    SimTime::ZERO,
                )
                .unwrap(),
            u64::from(output)
        );
    }

    #[test]
    fn setup_completion_releases_endpoint_zero_receive_arm() {
        let (mut device, handle) = EspUsbOtg::new("usb");
        write(&mut device, EspUsbOtgRegister::DoepTsiz(0), 64);
        write(&mut device, EspUsbOtgRegister::DoepCtl(0), DWC2_EPENA);
        handle.inject_setup([0x80, 6, 0, 1, 0, 0, 18, 0]);

        device
            .read(
                EspUsbOtgRegister::GrxStsP.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .read(
                EspUsbOtgRegister::GrxStsP.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap();
        assert!(!handle.output_ready(0));
    }

    #[test]
    fn slave_mode_setup_arm_survives_ep0_status_sizing() {
        let (mut device, handle) = EspUsbOtg::new("usb");
        write(&mut device, EspUsbOtgRegister::DoepTsiz(0), 3 << 29);
        assert!(handle.setup_ready());

        write(&mut device, EspUsbOtgRegister::DoepTsiz(0), 1 << 19);
        assert!(handle.setup_ready());

        handle.inject_bus_reset();
        assert!(!handle.setup_ready());
    }

    #[test]
    fn dma_mode_setup_also_requires_endpoint_enable() {
        let (mut device, handle) = EspUsbOtg::new("usb");
        write(&mut device, EspUsbOtgRegister::GahbCfg, 1 << 5);
        write(&mut device, EspUsbOtgRegister::DoepTsiz(0), 3 << 29);
        assert!(!handle.setup_ready());

        write(&mut device, EspUsbOtgRegister::DoepCtl(0), DWC2_EPENA);
        assert!(handle.setup_ready());
    }
}
