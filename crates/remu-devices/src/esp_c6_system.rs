use super::*;

/// ESP32-C6 UHCI adapter for the native register layout.
///
/// The C6 removed the S3 application-interrupt register at offset `0x14`,
/// shifting the common framing/UART engine down by one word.
pub struct EspC6Uhci {
    name: String,
    inner: Esp32S3Uhci,
    date: u32,
}

impl EspC6Uhci {
    /// Creates a UHCI engine attached to UART0, UART1, and LP UART.
    pub fn new(name: impl Into<String>, uarts: [UartHandle; 3]) -> (Self, Esp32S3UhciHandle) {
        let name = name.into();
        let (inner, handle) = Esp32S3Uhci::new(name.clone(), uarts);
        (
            Self {
                name,
                inner,
                date: 35_655_936,
            },
            handle,
        )
    }

    fn translate(offset: u64) -> u64 {
        if (0x14..=0x80).contains(&offset) {
            offset + 4
        } else {
            offset
        }
    }
}

impl Device for EspC6Uhci {
    fn name(&self) -> &str {
        &self.name
    }
    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if offset == 0x80 && width == AccessWidth::Word {
            Ok(u64::from(self.date))
        } else {
            self.inner.read(Self::translate(offset), width, at)
        }
    }
    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        if offset == 0x80 && width == AccessWidth::Word {
            self.date = value as u32;
            Ok(())
        } else {
            self.inner.write(Self::translate(offset), width, value, at)
        }
    }
    fn reset(&mut self, kind: ResetKind) {
        self.inner.reset(kind);
        self.date = 35_655_936;
    }
}

/// Host view of the ESP32-C6 IO MUX pad configuration.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EspC6IoMuxPinConfig {
    /// Whether the pad input path is enabled.
    pub input_enable: bool,
    /// Whether the functional pull-up is enabled.
    pub pullup: bool,
    /// Whether the functional pull-down is enabled.
    pub pulldown: bool,
    /// Two-bit drive-strength selector.
    pub drive: u8,
    /// Three-bit direct-function selector.
    pub function: u8,
}

/// Functional 31-pad ESP32-C6 IO MUX.
pub struct EspC6IoMux {
    name: String,
    pin_ctrl: u32,
    pads: [u32; 31],
    modem_diagnostic: u32,
    date: u32,
}

impl EspC6IoMux {
    /// Creates a reset IO MUX.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            pin_ctrl: 0,
            pads: [0; 31],
            modem_diagnostic: 0,
            date: 35_655_776,
        }
    }

    /// Decodes one bonded pad configuration.
    pub fn pin_config(&self, pin: u8) -> Option<EspC6IoMuxPinConfig> {
        let value = *self.pads.get(usize::from(pin))?;
        Some(EspC6IoMuxPinConfig {
            input_enable: value & (1 << 9) != 0,
            pullup: value & (1 << 8) != 0,
            pulldown: value & (1 << 7) != 0,
            drive: ((value >> 10) & 3) as u8,
            function: ((value >> 12) & 7) as u8,
        })
    }
}

impl Device for EspC6IoMux {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || !offset.is_multiple_of(4) {
            return Err(DeviceError::new(
                "ESP32-C6 IO MUX requires aligned word access",
            ));
        }
        let value = match offset {
            0 => self.pin_ctrl,
            0x04..=0x7c => self.pads[(offset as usize - 4) / 4],
            0xbc => self.modem_diagnostic,
            0xfc => self.date,
            _ => {
                return Err(DeviceError::new(format!(
                    "{} reserved read {offset:#x}",
                    self.name
                )));
            }
        };
        Ok(u64::from(value))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || !offset.is_multiple_of(4) {
            return Err(DeviceError::new(
                "ESP32-C6 IO MUX requires aligned word access",
            ));
        }
        let value = u32::try_from(value)
            .map_err(|_| DeviceError::new("ESP32-C6 IO MUX rejects wide writes"))?;
        match offset {
            0 => self.pin_ctrl = value & 0xffff,
            0x04..=0x7c => self.pads[(offset as usize - 4) / 4] = value & 0xffff,
            0xbc => self.modem_diagnostic = value & 1,
            0xfc => self.date = value & 0x0fff_ffff,
            _ => {
                return Err(DeviceError::new(format!(
                    "{} reserved write {offset:#x}",
                    self.name
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.pin_ctrl = 0;
        self.pads.fill(0);
        self.modem_diagnostic = 0;
    }
}

struct InterruptMatrixState {
    // Native source numbers are the register index. Some inputs have no named
    // peripheral in the public header, but the ESP-IDF ROM handoff clears all
    // 77 route words and hardware accepts those writes.
    routes: [u8; 77],
    sources: [u32; 3],
    clock_enabled: bool,
}

impl Default for InterruptMatrixState {
    fn default() -> Self {
        Self {
            routes: [0; 77],
            sources: [0; 3],
            clock_enabled: false,
        }
    }
}

/// Host injection and inspection handle for the ESP32-C6 interrupt matrix.
#[derive(Clone)]
pub struct EspC6InterruptMatrixHandle {
    state: Rc<RefCell<InterruptMatrixState>>,
}

impl EspC6InterruptMatrixHandle {
    /// Sets or clears one peripheral source in the native status banks.
    pub fn set_source(&self, source: u8, asserted: bool) {
        if source >= 77 || !valid_interrupt_route(u64::from(source) * 4) {
            return;
        }
        let mut state = self.state.borrow_mut();
        let word = usize::from(source / 32);
        let bit = 1_u32 << (source % 32);
        if asserted {
            state.sources[word] |= bit;
        } else {
            state.sources[word] &= !bit;
        }
    }

    /// Returns whether any asserted source is routed to the named CPU interrupt.
    pub fn cpu_interrupt_pending(&self, interrupt: u8) -> bool {
        let state = self.state.borrow();
        state.routes.iter().enumerate().any(|(source, route)| {
            *route == interrupt && state.sources[source / 32] & (1 << (source % 32)) != 0
        })
    }

    /// Returns the CPU interrupt currently selected for a peripheral source.
    pub fn route(&self, source: u8) -> Option<u8> {
        self.state.borrow().routes.get(usize::from(source)).copied()
    }

    /// Returns a bit mask of CPU interrupt lines with at least one asserted source.
    pub fn pending_cpu_interrupts(&self) -> u32 {
        let state = self.state.borrow();
        if state.sources.iter().all(|sources| *sources == 0) {
            return 0;
        }
        state
            .routes
            .iter()
            .enumerate()
            .fold(0_u32, |pending, (source, route)| {
                if *route < 32 && state.sources[source / 32] & (1 << (source % 32)) != 0 {
                    pending | (1_u32 << *route)
                } else {
                    pending
                }
            })
    }
}

/// ESP32-C6 peripheral-source interrupt routing matrix.
pub struct EspC6InterruptMatrix {
    name: String,
    state: Rc<RefCell<InterruptMatrixState>>,
    date: u32,
}

fn valid_interrupt_route(offset: u64) -> bool {
    offset <= 0x130 && offset.is_multiple_of(4)
}

impl EspC6InterruptMatrix {
    /// Creates a matrix with all sources routed to interrupt zero.
    pub fn new(name: impl Into<String>) -> (Self, EspC6InterruptMatrixHandle) {
        let state = Rc::new(RefCell::new(InterruptMatrixState {
            clock_enabled: true,
            ..Default::default()
        }));
        (
            Self {
                name: name.into(),
                state: state.clone(),
                date: 35_664_144,
            },
            EspC6InterruptMatrixHandle { state },
        )
    }
}

impl Device for EspC6InterruptMatrix {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || !offset.is_multiple_of(4) {
            return Err(DeviceError::new(
                "ESP32-C6 interrupt matrix requires aligned word access",
            ));
        }
        let state = self.state.borrow();
        let value = match offset {
            0x000..=0x130 if valid_interrupt_route(offset) => {
                u32::from(state.routes[offset as usize / 4])
            }
            0x134..=0x13c => state.sources[(offset as usize - 0x134) / 4],
            0x140 => u32::from(state.clock_enabled),
            0x7fc => self.date,
            _ => {
                return Err(DeviceError::new(format!(
                    "{} reserved read {offset:#x}",
                    self.name
                )));
            }
        };
        Ok(u64::from(value))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || !offset.is_multiple_of(4) {
            return Err(DeviceError::new(
                "ESP32-C6 interrupt matrix requires aligned word access",
            ));
        }
        let value = u32::try_from(value)
            .map_err(|_| DeviceError::new("ESP32-C6 interrupt matrix rejects wide writes"))?;
        match offset {
            0x000..=0x130 if valid_interrupt_route(offset) => {
                self.state.borrow_mut().routes[offset as usize / 4] = (value & 0x1f) as u8
            }
            0x140 => self.state.borrow_mut().clock_enabled = value & 1 != 0,
            0x7fc => self.date = value & 0x0fff_ffff,
            0x134..=0x13c => {
                return Err(DeviceError::new("ESP32-C6 interrupt status is read-only"));
            }
            _ => {
                return Err(DeviceError::new(format!(
                    "{} reserved write {offset:#x}",
                    self.name
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.borrow_mut() = InterruptMatrixState {
            clock_enabled: true,
            ..Default::default()
        };
    }
}

/// Functional ESP32-C6 CPU interrupt priority controller register slice.
pub struct EspC6InterruptPriority {
    name: String,
    registers: [u32; 0x400 / 4],
}

impl EspC6InterruptPriority {
    /// Creates the native reset state.
    pub fn new(name: impl Into<String>) -> Self {
        let mut registers = [0; 0x400 / 4];
        registers[0xa0 / 4] = 35_655_824;
        registers[0xa4 / 4] = 1;
        registers[0x3fc / 4] = u32::MAX;
        Self {
            name: name.into(),
            registers,
        }
    }

    /// Returns the highest enabled pending interrupt above the threshold.
    pub fn highest_pending(&self) -> Option<u8> {
        let enabled_pending = self.registers[0] & self.registers[2];
        let threshold = self.registers[0x8c / 4] & 0xff;
        (0_u8..32)
            .filter(|interrupt| enabled_pending & (1 << interrupt) != 0)
            .filter(|interrupt| self.registers[(0x0c / 4) + usize::from(*interrupt)] > threshold)
            .max_by_key(|interrupt| self.registers[(0x0c / 4) + usize::from(*interrupt)])
    }
}

impl Device for EspC6InterruptPriority {
    fn name(&self) -> &str {
        &self.name
    }
    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || !offset.is_multiple_of(4) {
            return Err(DeviceError::new(
                "ESP32-C6 interrupt priority requires aligned word access",
            ));
        }
        let index = offset as usize / 4;
        let value = match offset {
            0x00 | 0x04 | 0x08 | 0x8c..=0xb0 | 0x3fc => self.registers[index],
            0x0c..=0x88 => self.registers[index] & 0xf,
            _ => {
                return Err(DeviceError::new(format!(
                    "{} reserved read {offset:#x}",
                    self.name
                )));
            }
        };
        Ok(u64::from(value))
    }
    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || !offset.is_multiple_of(4) {
            return Err(DeviceError::new(
                "ESP32-C6 interrupt priority requires aligned word access",
            ));
        }
        let value = u32::try_from(value)
            .map_err(|_| DeviceError::new("ESP32-C6 interrupt priority rejects wide writes"))?;
        let index = offset as usize / 4;
        match offset {
            0x00 | 0x04 => self.registers[index] = value,
            0x08 => {
                return Err(DeviceError::new(
                    "ESP32-C6 pending interrupt status is read-only",
                ));
            }
            0x0c..=0x88 => self.registers[index] = value & 0xf,
            0x8c => self.registers[index] = value & 0xff,
            0x90..=0x9c => {
                self.registers[index] = value & 1;
                if value & 1 != 0 {
                    self.registers[2] |= 1 << ((offset - 0x90) / 4);
                }
            }
            0xa0 => self.registers[index] = value & 0x0fff_ffff,
            0xa4 => self.registers[index] = value & 1,
            0xa8 => self.registers[2] &= !value,
            0xac => self.registers[index] = value & 1,
            0xb0 | 0x3fc => self.registers[index] = value,
            _ => {
                return Err(DeviceError::new(format!(
                    "{} reserved write {offset:#x}",
                    self.name
                )));
            }
        }
        Ok(())
    }
    fn reset(&mut self, _kind: ResetKind) {
        *self = Self::new(self.name.clone());
    }
}

/// Target-specific register facade for ESP32-C6 power, clock, protection and retention blocks.
pub struct EspC6ControlBlock {
    name: String,
    registers: Vec<u32>,
    reset: Vec<u32>,
}

impl EspC6ControlBlock {
    /// Creates a strict aligned register block with native clock/date reset words.
    pub fn new(name: impl Into<String>, size: usize, date_offset: Option<u64>, date: u32) -> Self {
        let name = name.into();
        let mut reset = vec![0; size / 4];
        if let Some(offset) = date_offset {
            reset[offset as usize / 4] = date;
        }
        Self {
            name,
            registers: reset.clone(),
            reset,
        }
    }

    /// Applies one documented hardware reset word to this register facade.
    ///
    /// This builder is used for control blocks whose non-zero reset state is
    /// consumed by vendor startup code before firmware has written the block.
    pub fn with_reset_word(mut self, offset: u64, value: u32) -> Self {
        let index = usize::try_from(offset / 4).expect("control-block offset fits usize");
        assert!(offset.is_multiple_of(4) && index < self.reset.len());
        self.reset[index] = value;
        self.registers[index] = value;
        self
    }
}

impl Device for EspC6ControlBlock {
    fn name(&self) -> &str {
        &self.name
    }
    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || !offset.is_multiple_of(4) {
            return Err(DeviceError::new(
                "ESP32-C6 control blocks require aligned word access",
            ));
        }
        self.registers
            .get(offset as usize / 4)
            .copied()
            .map(u64::from)
            .ok_or_else(|| DeviceError::new(format!("{} read outside native page", self.name)))
    }
    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || !offset.is_multiple_of(4) {
            return Err(DeviceError::new(
                "ESP32-C6 control blocks require aligned word access",
            ));
        }
        let value = u32::try_from(value)
            .map_err(|_| DeviceError::new("ESP32-C6 control blocks reject wide writes"))?;
        *self.registers.get_mut(offset as usize / 4).ok_or_else(|| {
            DeviceError::new(format!("{} write outside native page", self.name))
        })? = value;
        Ok(())
    }
    fn reset(&mut self, _kind: ResetKind) {
        self.registers.clone_from(&self.reset);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c6_uhci_quick_send_uses_shifted_vendor_offsets() {
        let uarts = std::array::from_fn(|_| UartHandle::default());
        let terminal = uarts[0].clone();
        let (mut uhci, _) = EspC6Uhci::new("uhci", uarts);
        uhci.write(0, AccessWidth::Word, 1 << 2, SimTime::ZERO)
            .unwrap();
        uhci.write(0x44, AccessWidth::Word, 0x4443_4241, SimTime::ZERO)
            .unwrap();
        uhci.write(0x48, AccessWidth::Word, 0x4847_4645, SimTime::ZERO)
            .unwrap();
        uhci.write(0x30, AccessWidth::Word, (1 << 3) | 2, SimTime::ZERO)
            .unwrap();
        assert_eq!(terminal.bytes(), b"ABCDEFGH");
    }

    #[test]
    fn c6_iomux_covers_all_bonded_pads_and_masks_fields() {
        let mut iomux = EspC6IoMux::new("iomux");
        iomux
            .write(
                4 + 30 * 4,
                AccessWidth::Word,
                u32::MAX.into(),
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            iomux.read(4 + 30 * 4, AccessWidth::Word, SimTime::ZERO),
            Ok(0xffff)
        );
        assert!(iomux.pin_config(30).unwrap().input_enable);
        assert!(iomux.pin_config(31).is_none());
    }

    #[test]
    fn c6_interrupt_routes_and_priority_are_observable() {
        let (mut matrix, handle) = EspC6InterruptMatrix::new("matrix");
        matrix
            .write(8 * 4, AccessWidth::Word, 5, SimTime::ZERO)
            .unwrap();
        handle.set_source(8, true);
        assert!(handle.cpu_interrupt_pending(5));
        let mut priority = EspC6InterruptPriority::new("priority");
        priority
            .write(0, AccessWidth::Word, 1 << 5, SimTime::ZERO)
            .unwrap();
        priority
            .write(0x0c + 5 * 4, AccessWidth::Word, 3, SimTime::ZERO)
            .unwrap();
        priority.registers[2] = 1 << 5;
        assert_eq!(priority.highest_pending(), Some(5));
    }
}
