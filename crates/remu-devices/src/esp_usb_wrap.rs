use super::{AccessWidth, Device, DeviceError, Rc, RefCell, ResetKind, SimTime};

const OTG_CONF_RESET: u32 = 0x001c_0000;
const OTG_CONF_MASK: u32 = 0x807f_ffff;
const TEST_CONF_READ_MASK: u32 = 0x7f;
const TEST_CONF_WRITE_MASK: u32 = 0x0f;
const DATE_RESET: u32 = 0x0210_2010;

/// Native ESP32-S3 USB external-control register identifiers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Esp32S3UsbWrapRegister {
    /// PHY, pad, clock, pull, session, and FIFO power configuration.
    OtgConfig,
    /// Direct USB pad test input/output control.
    TestConfig,
    /// Hardware version/date word.
    Date,
}

impl Esp32S3UsbWrapRegister {
    /// Returns the native byte offset within the USB wrapper page.
    pub const fn offset(self) -> u64 {
        match self {
            Self::OtgConfig => 0,
            Self::TestConfig => 4,
            Self::Date => 0x3fc,
        }
    }

    /// Resolves an aligned documented register offset.
    pub const fn from_offset(offset: u64) -> Option<Self> {
        match offset {
            0 => Some(Self::OtgConfig),
            4 => Some(Self::TestConfig),
            0x3fc => Some(Self::Date),
            _ => None,
        }
    }

    /// Hardware reset value from Espressif's register definition.
    pub const fn reset_value(self) -> u32 {
        match self {
            Self::OtgConfig => OTG_CONF_RESET,
            Self::TestConfig => 0,
            Self::Date => DATE_RESET,
        }
    }

    /// Bits visible on a native read.
    pub const fn read_mask(self) -> u32 {
        match self {
            Self::OtgConfig => OTG_CONF_MASK,
            Self::TestConfig => TEST_CONF_READ_MASK,
            Self::Date => u32::MAX,
        }
    }

    /// Bits accepted by a native write.
    pub const fn write_mask(self) -> u32 {
        match self {
            Self::OtgConfig => OTG_CONF_MASK,
            Self::TestConfig => TEST_CONF_WRITE_MASK,
            Self::Date => u32::MAX,
        }
    }
}

/// Decoded USB wrapper configuration for host-side inspection.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Esp32S3UsbWrapConfig {
    /// Disables automatic CSR clock gating.
    pub clock_enabled: bool,
    /// Keeps the DWC2 data FIFO powered up.
    pub fifo_force_power_up: bool,
    /// Selects the PHY transmit clock edge.
    pub phy_tx_falling_edge: bool,
    /// Forces the PHY clock on.
    pub phy_clock_force_on: bool,
    /// Forces the AHB clock on.
    pub ahb_clock_force_on: bool,
    /// Enables the dedicated USB pads.
    pub usb_pad_enabled: bool,
    /// Software pull resistor value shared by the pull fields.
    pub pullup_value: bool,
    /// D- pulldown control.
    pub dm_pulldown: bool,
    /// D- pullup control.
    pub dm_pullup: bool,
    /// D+ pulldown control.
    pub dp_pulldown: bool,
    /// D+ pullup control.
    pub dp_pullup: bool,
    /// Enables software pull control.
    pub pad_pull_override: bool,
    /// Enables software threshold control.
    pub vref_override: bool,
    /// Low input threshold selector.
    pub vref_low: u8,
    /// High input threshold selector.
    pub vref_high: u8,
    /// Swaps D+ and D- when exchange override is active.
    pub exchange_pins: bool,
    /// Enables software D+/D- exchange control.
    pub exchange_pins_override: bool,
    /// Bypasses session debounce filters.
    pub debounce_filter_bypass: bool,
    /// Forces the DWC2 data FIFO into low power.
    pub fifo_force_power_down: bool,
    /// Selects an external PHY instead of the on-chip full-speed PHY.
    pub external_phy: bool,
    /// Software session-end value.
    pub session_end: bool,
    /// Enables software session-end control.
    pub session_end_override: bool,
}

impl Esp32S3UsbWrapConfig {
    fn decode(value: u32) -> Self {
        Self {
            clock_enabled: value & (1 << 31) != 0,
            fifo_force_power_up: value & (1 << 22) != 0,
            phy_tx_falling_edge: value & (1 << 21) != 0,
            phy_clock_force_on: value & (1 << 20) != 0,
            ahb_clock_force_on: value & (1 << 19) != 0,
            usb_pad_enabled: value & (1 << 18) != 0,
            pullup_value: value & (1 << 17) != 0,
            dm_pulldown: value & (1 << 16) != 0,
            dm_pullup: value & (1 << 15) != 0,
            dp_pulldown: value & (1 << 14) != 0,
            dp_pullup: value & (1 << 13) != 0,
            pad_pull_override: value & (1 << 12) != 0,
            vref_override: value & (1 << 11) != 0,
            vref_low: ((value >> 9) & 3) as u8,
            vref_high: ((value >> 7) & 3) as u8,
            exchange_pins: value & (1 << 6) != 0,
            exchange_pins_override: value & (1 << 5) != 0,
            debounce_filter_bypass: value & (1 << 4) != 0,
            fifo_force_power_down: value & (1 << 3) != 0,
            external_phy: value & (1 << 2) != 0,
            session_end: value & (1 << 1) != 0,
            session_end_override: value & 1 != 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Esp32S3UsbWrapState {
    otg_config: u32,
    test_config: u32,
    test_rx: u8,
    date: u32,
}

impl Esp32S3UsbWrapState {
    const fn new() -> Self {
        Self {
            otg_config: OTG_CONF_RESET,
            test_config: 0,
            test_rx: 0,
            date: DATE_RESET,
        }
    }

    fn register(self, register: Esp32S3UsbWrapRegister) -> u32 {
        match register {
            Esp32S3UsbWrapRegister::OtgConfig => self.otg_config,
            Esp32S3UsbWrapRegister::TestConfig => self.test_config | (u32::from(self.test_rx) << 4),
            Esp32S3UsbWrapRegister::Date => self.date,
        }
    }
}

/// Host and DWC2-facing view of USB PHY/pad wrapper state.
#[derive(Clone)]
pub struct Esp32S3UsbWrapHandle {
    state: Rc<RefCell<Esp32S3UsbWrapState>>,
}

impl Esp32S3UsbWrapHandle {
    /// Returns all decoded wrapper fields.
    pub fn config(&self) -> Esp32S3UsbWrapConfig {
        Esp32S3UsbWrapConfig::decode(self.state.borrow().otg_config)
    }

    /// Returns whether the functional on-chip PHY can expose DWC2 to a host.
    pub fn host_link_active(&self) -> bool {
        let state = self.state.borrow();
        let config = Esp32S3UsbWrapConfig::decode(state.otg_config);
        let pull_attach = !config.pad_pull_override
            || (config.dp_pullup
                && !config.dm_pullup
                && !config.dp_pulldown
                && !config.dm_pulldown);
        let session_active = !config.session_end_override || !config.session_end;
        let fifo_active = !config.fifo_force_power_down || config.fifo_force_power_up;
        config.usb_pad_enabled
            && !config.external_phy
            && session_active
            && fifo_active
            && state.test_config & 1 == 0
            && pull_attach
    }

    /// Returns whether the DWC2 data FIFO is available.
    pub fn fifo_powered(&self) -> bool {
        let config = self.config();
        !config.fifo_force_power_down || config.fifo_force_power_up
    }

    /// Drives D+, D-, and differential receive values reported in test mode.
    pub fn drive_test_inputs(&self, dp: bool, dm: bool, differential: bool) {
        self.state.borrow_mut().test_rx =
            (u8::from(dm) << 2) | (u8::from(dp) << 1) | u8::from(differential);
    }

    /// Returns direct test output `(dp, dm, output_enable)`, or `None` outside test mode.
    pub fn test_output(&self) -> Option<(bool, bool, bool)> {
        let test = self.state.borrow().test_config;
        (test & 1 != 0).then_some((test & 4 != 0, test & 8 != 0, test & 2 != 0))
    }
}

/// Functional ESP32-S3 USB external-control wrapper.
pub struct Esp32S3UsbWrap {
    name: String,
    state: Rc<RefCell<Esp32S3UsbWrapState>>,
}

impl Esp32S3UsbWrap {
    /// Creates reset wrapper state and its host-facing handle.
    pub fn new(name: impl Into<String>) -> (Self, Esp32S3UsbWrapHandle) {
        let state = Rc::new(RefCell::new(Esp32S3UsbWrapState::new()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Esp32S3UsbWrapHandle { state },
        )
    }
}

impl Device for Esp32S3UsbWrap {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || !offset.is_multiple_of(4) {
            return Err(DeviceError::new(
                "ESP32-S3 USB wrapper requires aligned word access",
            ));
        }
        let register = Esp32S3UsbWrapRegister::from_offset(offset).ok_or_else(|| {
            DeviceError::new(format!("{} read at reserved offset {offset:#x}", self.name))
        })?;
        Ok(u64::from(
            self.state.borrow().register(register) & register.read_mask(),
        ))
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
                "ESP32-S3 USB wrapper requires aligned word access",
            ));
        }
        let register = Esp32S3UsbWrapRegister::from_offset(offset).ok_or_else(|| {
            DeviceError::new(format!(
                "{} write at reserved offset {offset:#x}",
                self.name
            ))
        })?;
        let value = u32::try_from(value)
            .map_err(|_| DeviceError::new(format!("{} word write exceeds 32 bits", self.name)))?;
        let mut state = self.state.borrow_mut();
        let old = state.register(register);
        let value = (old & !register.write_mask()) | (value & register.write_mask());
        match register {
            Esp32S3UsbWrapRegister::OtgConfig => state.otg_config = value,
            Esp32S3UsbWrapRegister::TestConfig => state.test_config = value & TEST_CONF_WRITE_MASK,
            Esp32S3UsbWrapRegister::Date => state.date = value,
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.borrow_mut() = Esp32S3UsbWrapState::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(device: &mut Esp32S3UsbWrap, register: Esp32S3UsbWrapRegister) -> u32 {
        device
            .read(register.offset(), AccessWidth::Word, SimTime::ZERO)
            .unwrap() as u32
    }

    fn write(device: &mut Esp32S3UsbWrap, register: Esp32S3UsbWrapRegister, value: u32) {
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
    fn exact_register_contract_and_reserved_holes() {
        let (mut device, _) = Esp32S3UsbWrap::new("usb-wrap");
        for register in [
            Esp32S3UsbWrapRegister::OtgConfig,
            Esp32S3UsbWrapRegister::TestConfig,
            Esp32S3UsbWrapRegister::Date,
        ] {
            assert_eq!(read(&mut device, register), register.reset_value());
            write(&mut device, register, u32::MAX);
            assert_eq!(read(&mut device, register), register.write_mask());
        }
        for offset in [0x08, 0x100, 0x3f8] {
            assert!(
                device
                    .read(offset, AccessWidth::Word, SimTime::ZERO)
                    .is_err()
            );
            assert!(
                device
                    .write(offset, AccessWidth::Word, 0, SimTime::ZERO)
                    .is_err()
            );
        }
        assert!(device.read(0, AccessWidth::Byte, SimTime::ZERO).is_err());
        assert!(device.read(2, AccessWidth::Word, SimTime::ZERO).is_err());
        device.reset(ResetKind::Software);
        assert_eq!(
            read(&mut device, Esp32S3UsbWrapRegister::OtgConfig),
            OTG_CONF_RESET
        );
        assert_eq!(read(&mut device, Esp32S3UsbWrapRegister::Date), DATE_RESET);
    }

    #[test]
    fn wrapper_fields_gate_host_attach_and_fifo_power() {
        let (mut device, handle) = Esp32S3UsbWrap::new("usb-wrap");
        assert!(handle.host_link_active());
        assert!(handle.fifo_powered());
        write(
            &mut device,
            Esp32S3UsbWrapRegister::OtgConfig,
            1 << 18 | 1 << 2,
        );
        assert!(!handle.host_link_active());
        write(
            &mut device,
            Esp32S3UsbWrapRegister::OtgConfig,
            (1 << 18) | (1 << 12) | (1 << 13),
        );
        assert!(handle.host_link_active());
        write(
            &mut device,
            Esp32S3UsbWrapRegister::OtgConfig,
            (1 << 18) | (1 << 3),
        );
        assert!(!handle.fifo_powered());
        assert!(!handle.host_link_active());
        write(
            &mut device,
            Esp32S3UsbWrapRegister::OtgConfig,
            (1 << 22) | (1 << 18) | (1 << 3),
        );
        assert!(handle.fifo_powered());
        assert!(handle.host_link_active());
        write(
            &mut device,
            Esp32S3UsbWrapRegister::OtgConfig,
            (1 << 18) | (1 << 1) | 1,
        );
        assert!(!handle.host_link_active());
        write(
            &mut device,
            Esp32S3UsbWrapRegister::OtgConfig,
            (1 << 18) | 1,
        );
        assert!(handle.host_link_active());
    }

    #[test]
    fn direct_pad_test_mode_has_independent_inputs_and_outputs() {
        let (mut device, handle) = Esp32S3UsbWrap::new("usb-wrap");
        handle.drive_test_inputs(false, false, true);
        assert_eq!(read(&mut device, Esp32S3UsbWrapRegister::TestConfig), 0x10);
        handle.drive_test_inputs(true, false, false);
        assert_eq!(read(&mut device, Esp32S3UsbWrapRegister::TestConfig), 0x20);
        handle.drive_test_inputs(false, true, false);
        assert_eq!(read(&mut device, Esp32S3UsbWrapRegister::TestConfig), 0x40);
        handle.drive_test_inputs(true, false, true);
        assert_eq!(read(&mut device, Esp32S3UsbWrapRegister::TestConfig), 0x30);
        write(&mut device, Esp32S3UsbWrapRegister::TestConfig, 0x0f);
        assert_eq!(handle.test_output(), Some((true, true, true)));
        assert_eq!(read(&mut device, Esp32S3UsbWrapRegister::TestConfig), 0x3f);
        assert!(!handle.host_link_active());
    }
}
