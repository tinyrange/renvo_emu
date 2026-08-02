//! Functional WCH `CH32V00x` power-control and automatic-wakeup registers.

use super::{AccessWidth, Arc, Device, DeviceError, Mutex, ResetKind, SimTime};

const CTLR_PDDS: u32 = 1 << 1;
const CTLR_PVDE: u32 = 1 << 4;
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
    ctlr: u32,
    awucsr: u32,
    awuwr: u32,
    awupsc: u32,
    supply_low: bool,
    standby_requested: bool,
}

impl WchPowerState {
    fn reset() -> Self {
        Self {
            ctlr: 0,
            awucsr: 0,
            awuwr: 0,
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
}

/// Functional WCH `PWR_TypeDef` register block.
pub struct WchPower {
    name: String,
    state: Arc<Mutex<WchPowerState>>,
}

impl WchPower {
    /// Creates a reset power controller and host-facing handle.
    pub fn new(name: impl Into<String>) -> (Self, WchPowerHandle) {
        let state = Arc::new(Mutex::new(WchPowerState::reset()));
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
        let state = self.state.lock().expect("WCH power lock poisoned");
        let value = match offset {
            0x00 => state.ctlr,
            0x04 => state.csr(),
            0x08 => state.awucsr,
            0x0c => state.awuwr,
            0x10 => state.awupsc,
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled WCH PWR read at offset {offset:#x}"
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
        Self::require_access(offset, width)?;
        let value = u32::try_from(value & u64::from(u32::MAX))
            .expect("masked WCH power register value fits u32");
        let mut state = self.state.lock().expect("WCH power lock poisoned");
        match offset {
            0x00 => {
                state.ctlr = value;
                state.standby_requested = value & CTLR_PDDS != 0;
            }
            // CSR.PVDO is a read-only status bit driven by the host supply.
            0x04 => {}
            0x08 => state.awucsr = value & AWUCSR_AWUEN,
            0x0c => state.awuwr = value & AWUWR_MASK,
            0x10 => state.awupsc = value & AWUPSC_MASK,
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled WCH PWR write at offset {offset:#x}"
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("WCH power lock poisoned") = WchPowerState::reset();
    }
}
