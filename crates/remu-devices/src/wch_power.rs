//! Functional WCH `CH32V00x` power-control and automatic-wakeup registers.

use super::{AccessWidth, Arc, Device, DeviceError, Mutex, ResetKind, SimTime};

/// Register identifiers for the WCH power-control block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum WchPowerRegister {
    /// Power control register.
    Ctlr = 0x00,
    /// Power control/status register.
    Csr = 0x04,
    /// Automatic-wakeup control/status register.
    Awucsr = 0x08,
    /// Automatic-wakeup window register.
    Awuwr = 0x0c,
    /// Automatic-wakeup prescaler register.
    Awupsc = 0x10,
}

impl TryFrom<u64> for WchPowerRegister {
    type Error = DeviceError;

    fn try_from(offset: u64) -> Result<Self, Self::Error> {
        match offset {
            0x00 => Ok(Self::Ctlr),
            0x04 => Ok(Self::Csr),
            0x08 => Ok(Self::Awucsr),
            0x0c => Ok(Self::Awuwr),
            0x10 => Ok(Self::Awupsc),
            _ => Err(DeviceError::new(format!(
                "unmodeled WCH PWR register at offset {offset:#x}"
            ))),
        }
    }
}

/// Register-layout variants for the WCH power block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WchPowerVariant {
    /// CH32V003 layout: three PLS bits and no FLASH/LDO controls.
    Ch32v003,
    /// CH32V006 layout: two PLS bits, FLASH low-power and LDO controls.
    Ch32v006,
}

const CTLR_PDDS: u32 = 1 << 1;
const CTLR_PVDE: u32 = 1 << 4;
const CTLR_CH32V003_MASK: u32 = 0x00f2;
const CTLR_CH32V006_MASK: u32 = 0x0e7e;
const CSR_PVDO: u32 = 1 << 2;
const AWUCSR_AWUEN: u32 = 1 << 1;
const AWUWR_MASK: u32 = 0x3f;
const AWUPSC_MASK: u32 = 0x0f;

/// Host-facing controls for a WCH power peripheral.
#[derive(Clone)]
pub struct WchPowerHandle {
    state: Arc<Mutex<WchPowerState>>,
}

impl WchPowerHandle {
    /// Sets the deterministic supply condition used by the PVD status bit.
    pub fn set_supply_low(&self, low: bool) {
        self.state
            .lock()
            .expect("WCH power lock poisoned")
            .supply_low = low;
    }

    /// Returns whether a standby request is currently latched.
    pub fn standby_requested(&self) -> bool {
        self.state
            .lock()
            .expect("WCH power lock poisoned")
            .standby_requested
    }

    /// Clears a host-observed standby request, modelling a wake event.
    pub fn clear_standby(&self) {
        self.state
            .lock()
            .expect("WCH power lock poisoned")
            .standby_requested = false;
    }
}

struct WchPowerState {
    variant: WchPowerVariant,
    ctlr: u32,
    awucsr: u32,
    awuwr: u32,
    awupsc: u32,
    supply_low: bool,
    standby_requested: bool,
}

impl WchPowerState {
    fn reset(variant: WchPowerVariant) -> Self {
        Self {
            variant,
            ctlr: match variant {
                WchPowerVariant::Ch32v003 => 0,
                WchPowerVariant::Ch32v006 => 0x0408,
            },
            awucsr: 0,
            awuwr: AWUWR_MASK,
            awupsc: 0,
            supply_low: false,
            standby_requested: false,
        }
    }

    fn csr(&self) -> u32 {
        (self.supply_low && self.ctlr & CTLR_PVDE != 0)
            .then_some(CSR_PVDO)
            .unwrap_or(0)
    }

    fn ctlr_mask(&self) -> u32 {
        match self.variant {
            WchPowerVariant::Ch32v003 => CTLR_CH32V003_MASK,
            WchPowerVariant::Ch32v006 => CTLR_CH32V006_MASK,
        }
    }
}

/// Functional WCH `PWR_TypeDef` register block.
pub struct WchPower {
    name: String,
    state: Arc<Mutex<WchPowerState>>,
}

impl WchPower {
    /// Creates a reset CH32V003 power controller and host-facing handle.
    pub fn new(name: impl Into<String>) -> (Self, WchPowerHandle) {
        Self::new_for_variant(name, WchPowerVariant::Ch32v003)
    }

    /// Creates a reset power controller for a specific WCH register layout.
    pub fn new_for_variant(
        name: impl Into<String>,
        variant: WchPowerVariant,
    ) -> (Self, WchPowerHandle) {
        let state = Arc::new(Mutex::new(WchPowerState::reset(variant)));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            WchPowerHandle { state },
        )
    }

    fn require_access(offset: u64, width: AccessWidth) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("WCH PWR requires aligned word access"));
        }
        Ok(())
    }
}

impl Device for WchPower {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        Self::require_access(offset, width)?;
        let register = WchPowerRegister::try_from(offset)?;
        let state = self.state.lock().expect("WCH power lock poisoned");
        let value = match register {
            WchPowerRegister::Ctlr => state.ctlr,
            WchPowerRegister::Csr => state.csr(),
            WchPowerRegister::Awucsr => state.awucsr,
            WchPowerRegister::Awuwr => state.awuwr,
            WchPowerRegister::Awupsc => state.awupsc,
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
        Self::require_access(offset, width)?;
        let register = WchPowerRegister::try_from(offset)?;
        let value = u32::try_from(value & u64::from(u32::MAX))
            .expect("masked WCH power register value fits u32");
        let mut state = self.state.lock().expect("WCH power lock poisoned");
        match register {
            WchPowerRegister::Ctlr => {
                state.ctlr = value & state.ctlr_mask();
                state.standby_requested = value & CTLR_PDDS != 0;
            }
            // CSR.PVDO is a read-only status bit driven by the host supply.
            WchPowerRegister::Csr => {}
            WchPowerRegister::Awucsr => state.awucsr = value & AWUCSR_AWUEN,
            WchPowerRegister::Awuwr => state.awuwr = value & AWUWR_MASK,
            WchPowerRegister::Awupsc => state.awupsc = value & AWUPSC_MASK,
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.lock().expect("WCH power lock poisoned");
        let variant = state.variant;
        *state = WchPowerState::reset(variant);
    }
}
