use super::*;

const ESP_INTERRUPT_ROUTE_COUNT: usize = 99;
const ESP_INTERRUPT_CORE1_BASE: u64 = 0x800;
const ESP_INTERRUPT_ROUTE_MASK: u32 = 0x1f;
const ESP_INTERRUPT_ROUTE_DISABLED: u8 = 0x1f;
const ESP_INTERRUPT_ROUTE_RESET: u8 = 16;
const ESP_INTERRUPT_DATE_RESET: u32 = 0x0201_2300;

/// Native ESP32-S3 interrupt-matrix register identifiers.
///
/// Espressif exposes 99 five-bit route registers for each CPU, four
/// read-only status words, a clock gate, and a writable version/date word.
/// The gaps between those groups are reserved and are rejected by
/// [`Self::from_offset`].
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Esp32S3InterruptRegister {
    /// CPU0 source-to-interrupt route, source index 0..=98.
    Core0Route(u8),
    /// CPU1 source-to-interrupt route, source index 0..=98.
    Core1Route(u8),
    /// CPU0 interrupt status word, bank 0..=3.
    Core0Status(u8),
    /// CPU1 interrupt status word, bank 0..=3.
    Core1Status(u8),
    /// CPU0 interrupt-matrix clock gate.
    Core0ClockGate,
    /// CPU1 interrupt-matrix clock gate.
    Core1ClockGate,
    /// CPU0 interrupt-matrix version/date word.
    Core0Date,
    /// CPU1 interrupt-matrix version/date word.
    Core1Date,
}

impl Esp32S3InterruptRegister {
    /// Returns the native byte offset within the interrupt-matrix page.
    pub const fn offset(self) -> u64 {
        match self {
            Self::Core0Route(source) => source as u64 * 4,
            Self::Core1Route(source) => ESP_INTERRUPT_CORE1_BASE + source as u64 * 4,
            Self::Core0Status(bank) => 0x18c + bank as u64 * 4,
            Self::Core1Status(bank) => 0x98c + bank as u64 * 4,
            Self::Core0ClockGate => 0x19c,
            Self::Core1ClockGate => 0x99c,
            Self::Core0Date => 0x7fc,
            Self::Core1Date => 0xffc,
        }
    }

    /// Resolves an aligned native byte offset; reserved holes return `None`.
    pub const fn from_offset(offset: u64) -> Option<Self> {
        if offset & 3 != 0 {
            return None;
        }
        match offset {
            0x000..=0x188 => Some(Self::Core0Route((offset / 4) as u8)),
            0x18c..=0x198 => Some(Self::Core0Status(((offset - 0x18c) / 4) as u8)),
            0x19c => Some(Self::Core0ClockGate),
            0x7fc => Some(Self::Core0Date),
            0x800..=0x988 => Some(Self::Core1Route(((offset - 0x800) / 4) as u8)),
            0x98c..=0x998 => Some(Self::Core1Status(((offset - 0x98c) / 4) as u8)),
            0x99c => Some(Self::Core1ClockGate),
            0xffc => Some(Self::Core1Date),
            _ => None,
        }
    }

    /// Bits returned by a native read of this register.
    pub const fn read_mask(self) -> u32 {
        match self {
            Self::Core0Route(_) | Self::Core1Route(_) => ESP_INTERRUPT_ROUTE_MASK,
            Self::Core0Status(_) | Self::Core1Status(_) => u32::MAX,
            Self::Core0ClockGate | Self::Core1ClockGate => 1,
            Self::Core0Date | Self::Core1Date => 0x0fff_ffff,
        }
    }

    /// Bits accepted by a native write of this register.
    pub const fn write_mask(self) -> u32 {
        match self {
            Self::Core0Route(_) | Self::Core1Route(_) => ESP_INTERRUPT_ROUTE_MASK,
            Self::Core0ClockGate | Self::Core1ClockGate => 1,
            Self::Core0Date | Self::Core1Date => 0x0fff_ffff,
            Self::Core0Status(_) | Self::Core1Status(_) => 0,
        }
    }

    fn route(self) -> Option<(usize, usize)> {
        match self {
            Self::Core0Route(source) => Some((0, source as usize)),
            Self::Core1Route(source) => Some((1, source as usize)),
            _ => None,
        }
    }

    fn status(self) -> Option<(usize, usize)> {
        match self {
            Self::Core0Status(bank) => Some((0, bank as usize)),
            Self::Core1Status(bank) => Some((1, bank as usize)),
            _ => None,
        }
    }
}

struct EspInterruptMatrixState {
    routes: [[u8; ESP_INTERRUPT_ROUTE_COUNT]; 2],
    pending: [[bool; ESP_INTERRUPT_ROUTE_COUNT]; 2],
    status: [[u32; 4]; 2],
    clock_gate: [bool; 2],
    date: [u32; 2],
}

impl EspInterruptMatrixState {
    fn new() -> Self {
        Self {
            // Every route field resets to interrupt line 16 per the official
            // CPU0/CPU1 register definitions. The host handle uses 0xff as
            // its out-of-range/disabled sentinel, while native reads retain
            // the five-bit 0x1f encoding.
            routes: [[ESP_INTERRUPT_ROUTE_RESET; ESP_INTERRUPT_ROUTE_COUNT]; 2],
            pending: [[false; ESP_INTERRUPT_ROUTE_COUNT]; 2],
            status: [[0; 4]; 2],
            clock_gate: [true; 2],
            date: [ESP_INTERRUPT_DATE_RESET; 2],
        }
    }

    fn route(&self, core: usize, source: usize) -> u8 {
        self.routes
            .get(core)
            .and_then(|routes| routes.get(source))
            .copied()
            .map_or(u8::MAX, |route| {
                if route == ESP_INTERRUPT_ROUTE_DISABLED {
                    u8::MAX
                } else {
                    route
                }
            })
    }

    fn set_route(&mut self, core: usize, source: usize, interrupt: u32) {
        if let Some(route) = self
            .routes
            .get_mut(core)
            .and_then(|routes| routes.get_mut(source))
        {
            *route = (interrupt & ESP_INTERRUPT_ROUTE_MASK) as u8;
            self.recompute_status(core);
        }
    }

    fn set_pending(&mut self, core: usize, source: usize, pending: bool) {
        if let Some(source_pending) = self
            .pending
            .get_mut(core)
            .and_then(|sources| sources.get_mut(source))
        {
            *source_pending = pending;
            self.recompute_status(core);
        }
    }

    fn recompute_status(&mut self, core: usize) {
        let Some(routes) = self.routes.get(core) else {
            return;
        };
        let Some(pending) = self.pending.get(core) else {
            return;
        };
        let Some(status) = self.status.get_mut(core) else {
            return;
        };
        status.fill(0);
        for (route, is_pending) in routes.iter().zip(pending.iter()) {
            if *is_pending && *route != ESP_INTERRUPT_ROUTE_DISABLED {
                let bank = usize::from(*route) / 32;
                let bit = u32::from(*route % 32);
                status[bank] |= 1 << bit;
            }
        }
    }

    fn status(&self, core: usize, bank: usize) -> u32 {
        self.status
            .get(core)
            .and_then(|status| status.get(bank))
            .copied()
            .unwrap_or(0)
    }
}

/// Scheduler-facing ESP32-S3 interrupt-matrix route and status view.
#[derive(Clone)]
pub struct EspInterruptMatrixHandle {
    state: Rc<RefCell<EspInterruptMatrixState>>,
}

impl EspInterruptMatrixHandle {
    /// Returns the CPU interrupt line for a source, or `u8::MAX` when disabled.
    pub fn route(&self, core: usize, source: usize) -> u8 {
        self.state.borrow().route(core, source)
    }

    /// Updates one CPU/source route using the native five-bit encoding.
    pub fn set_route(&self, core: usize, source: usize, interrupt: u32) {
        self.state.borrow_mut().set_route(core, source, interrupt);
    }

    /// Updates a source's pending state so native status words reflect the
    /// scheduler's current interrupt-line view.
    pub fn set_source_pending(&self, core: usize, source: usize, pending: bool) {
        self.state.borrow_mut().set_pending(core, source, pending);
    }
}

/// Functional ESP32-S3 CPU0/CPU1 interrupt-matrix register block.
///
/// CPU0 source routes occupy `0x000..0x188`; CPU1 routes occupy
/// `0x800..0x988`. Status, clock-gate, and version/date registers are also
/// mapped at their documented offsets. Routing is deterministic and
/// functional; priority, NMI timing, and cycle-accurate SMP arbitration
/// remain outside this model.
pub struct EspInterruptMatrix {
    name: String,
    state: Rc<RefCell<EspInterruptMatrixState>>,
}

impl EspInterruptMatrix {
    /// Creates a reset interrupt matrix and scheduler handle.
    pub fn new(name: impl Into<String>) -> (Self, EspInterruptMatrixHandle) {
        let state = Rc::new(RefCell::new(EspInterruptMatrixState::new()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            EspInterruptMatrixHandle { state },
        )
    }
}

impl Device for EspInterruptMatrix {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP interrupt matrix requires aligned word access",
            ));
        }
        let register = Esp32S3InterruptRegister::from_offset(offset)
            .ok_or_else(|| DeviceError::new(format!("{} read at {offset:#x}", self.name)))?;
        if register.read_mask() == 0 {
            return Err(DeviceError::new(format!(
                "{} register {register:?} is write-only",
                self.name
            )));
        }
        let state = self.state.borrow();
        let value = if let Some((core, source)) = register.route() {
            state.routes[core][source] as u32
        } else if let Some((core, bank)) = register.status() {
            state.status(core, bank)
        } else {
            match register {
                Esp32S3InterruptRegister::Core0ClockGate => u32::from(state.clock_gate[0]),
                Esp32S3InterruptRegister::Core1ClockGate => u32::from(state.clock_gate[1]),
                Esp32S3InterruptRegister::Core0Date => state.date[0],
                Esp32S3InterruptRegister::Core1Date => state.date[1],
                _ => unreachable!("interrupt-matrix register handled above"),
            }
        };
        Ok(u64::from(value & register.read_mask()))
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
                "ESP interrupt matrix requires aligned word access",
            ));
        }
        let register = Esp32S3InterruptRegister::from_offset(offset)
            .ok_or_else(|| DeviceError::new(format!("{} write at {offset:#x}", self.name)))?;
        let value = u32::try_from(value).map_err(|_| {
            DeviceError::new(format!(
                "{} word write exceeds 32 bits: {value:#x}",
                self.name
            ))
        })?;
        let write_mask = register.write_mask();
        if write_mask == 0 {
            return Err(DeviceError::new(format!(
                "{} register {register:?} is read-only",
                self.name
            )));
        }
        let value = value & write_mask;
        let mut state = self.state.borrow_mut();
        if let Some((core, source)) = register.route() {
            state.set_route(core, source, value);
            return Ok(());
        }
        match register {
            Esp32S3InterruptRegister::Core0ClockGate => state.clock_gate[0] = value != 0,
            Esp32S3InterruptRegister::Core1ClockGate => state.clock_gate[1] = value != 0,
            Esp32S3InterruptRegister::Core0Date => state.date[0] = value,
            Esp32S3InterruptRegister::Core1Date => state.date[1] = value,
            Esp32S3InterruptRegister::Core0Status(_) | Esp32S3InterruptRegister::Core1Status(_) => {
                unreachable!("read-only interrupt-matrix status handled above")
            }
            Esp32S3InterruptRegister::Core0Route(_) | Esp32S3InterruptRegister::Core1Route(_) => {
                unreachable!("interrupt-matrix route handled above")
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.borrow_mut() = EspInterruptMatrixState::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_word(device: &mut EspInterruptMatrix, register: Esp32S3InterruptRegister, value: u64) {
        device
            .write(register.offset(), AccessWidth::Word, value, SimTime::ZERO)
            .unwrap();
    }

    #[test]
    fn exposes_native_route_status_clock_and_date_contract() {
        let (mut device, handle) = EspInterruptMatrix::new("interrupt-matrix");
        assert_eq!(
            Esp32S3InterruptRegister::from_offset(0x188),
            Some(Esp32S3InterruptRegister::Core0Route(98))
        );
        assert_eq!(
            Esp32S3InterruptRegister::from_offset(0x18c),
            Some(Esp32S3InterruptRegister::Core0Status(0))
        );
        assert_eq!(
            Esp32S3InterruptRegister::from_offset(0x800),
            Some(Esp32S3InterruptRegister::Core1Route(0))
        );
        assert_eq!(
            Esp32S3InterruptRegister::from_offset(0x7fc),
            Some(Esp32S3InterruptRegister::Core0Date)
        );
        assert_eq!(Esp32S3InterruptRegister::from_offset(0x200), None);
        assert_eq!(
            device
                .read(
                    Esp32S3InterruptRegister::Core0Route(0).offset(),
                    AccessWidth::Word,
                    SimTime::ZERO,
                )
                .unwrap(),
            u64::from(ESP_INTERRUPT_ROUTE_RESET)
        );
        assert_eq!(
            device
                .read(
                    Esp32S3InterruptRegister::Core0Date.offset(),
                    AccessWidth::Word,
                    SimTime::ZERO,
                )
                .unwrap(),
            u64::from(ESP_INTERRUPT_DATE_RESET)
        );
        write_word(&mut device, Esp32S3InterruptRegister::Core0Route(38), 5);
        write_word(&mut device, Esp32S3InterruptRegister::Core1Route(39), 7);
        assert_eq!(handle.route(0, 38), 5);
        assert_eq!(handle.route(1, 39), 7);
        write_word(
            &mut device,
            Esp32S3InterruptRegister::Core0Route(38),
            u64::from(u32::MAX),
        );
        assert_eq!(handle.route(0, 38), u8::MAX);
        assert_eq!(
            device
                .read(
                    Esp32S3InterruptRegister::Core0Route(38).offset(),
                    AccessWidth::Word,
                    SimTime::ZERO,
                )
                .unwrap(),
            u64::from(ESP_INTERRUPT_ROUTE_DISABLED)
        );
        handle.set_source_pending(1, 39, true);
        assert_eq!(
            device
                .read(
                    Esp32S3InterruptRegister::Core1Status(0).offset(),
                    AccessWidth::Word,
                    SimTime::ZERO,
                )
                .unwrap(),
            1 << 7
        );
    }

    #[test]
    fn rejects_reserved_holes_wrong_width_and_read_only_writes() {
        let (mut device, _) = EspInterruptMatrix::new("interrupt-matrix");
        assert!(
            device
                .read(0x200, AccessWidth::Word, SimTime::ZERO)
                .is_err()
        );
        assert!(
            device
                .read(0x7f8, AccessWidth::Word, SimTime::ZERO)
                .is_err()
        );
        assert!(
            device
                .read(
                    Esp32S3InterruptRegister::Core0Route(0).offset(),
                    AccessWidth::Byte,
                    SimTime::ZERO,
                )
                .is_err()
        );
        assert!(
            device
                .write(
                    Esp32S3InterruptRegister::Core0Status(0).offset(),
                    AccessWidth::Word,
                    1,
                    SimTime::ZERO,
                )
                .is_err()
        );
        assert!(
            device
                .write(
                    Esp32S3InterruptRegister::Core0Route(0).offset(),
                    AccessWidth::Word,
                    u64::from(u32::MAX) + 1,
                    SimTime::ZERO,
                )
                .is_err()
        );
    }
}
