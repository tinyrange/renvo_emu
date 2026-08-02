//! Functional CH32V006 touch-key/ADC conversion model.

use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use std::cell::RefCell;
use std::rc::Rc;

const CHANNEL_COUNT: usize = 8;
const STATUS_MASK: u32 = 0x1f;
const EOC: u32 = 1 << 1;
const STRT: u32 = 1 << 4;
const ADON: u32 = 1;
const EOCIE: u32 = 1 << 5;
const TKENABLE: u32 = 1 << 24;

/// Host-facing control and observation handle for a WCH touch-key block.
#[derive(Clone)]
pub struct WchTouchKeyHandle {
    state: Rc<RefCell<WchTouchKeyState>>,
}

impl WchTouchKeyHandle {
    /// Sets the deterministic converted value returned for one touch channel.
    /// Values use the peripheral's 12-bit ADC scale, although the register is
    /// exposed as a 16-bit value by the WCH manual.
    pub fn set_channel_value(&self, channel: u8, value: u16) {
        if let Some(sample) = self
            .state
            .borrow_mut()
            .samples
            .get_mut(usize::from(channel))
        {
            *sample = value & 0x0fff;
        }
    }

    /// Returns whether a completed conversion is enabled for ADC interrupt 15.
    pub fn pending(&self, now: SimTime) -> bool {
        let mut state = self.state.borrow_mut();
        state.update(now);
        state.statr & EOC != 0 && state.ctlr1 & EOCIE != 0
    }

    /// Returns the most recently converted value for host-side assertions.
    pub fn converted_value(&self) -> u16 {
        self.state.borrow().rdatar
    }
}

struct WchTouchKeyState {
    statr: u32,
    ctlr1: u32,
    ctlr2: u32,
    rsqr3: u32,
    ctlr3: u32,
    tk_charge: u16,
    rdatar: u16,
    samples: [u16; CHANNEL_COUNT],
    conversion_due: Option<u64>,
}

impl WchTouchKeyState {
    fn reset() -> Self {
        Self {
            statr: 0,
            ctlr1: 0,
            ctlr2: 0,
            rsqr3: 0,
            ctlr3: 1,
            tk_charge: 0,
            rdatar: 0,
            samples: [0x0800; CHANNEL_COUNT],
            conversion_due: None,
        }
    }

    fn update(&mut self, now: SimTime) {
        let Some(due) = self.conversion_due else {
            return;
        };
        if now.ticks() < due {
            return;
        }
        let channel = usize::try_from(self.rsqr3 & 0x1f).expect("touch channel fits usize");
        self.rdatar = self.samples.get(channel).copied().unwrap_or_default();
        self.statr = (self.statr & !STRT) | EOC;
        self.conversion_due = None;
    }

    fn start(&mut self, now: SimTime, discharge: u16) {
        if self.ctlr1 & TKENABLE == 0 || self.ctlr2 & ADON == 0 {
            return;
        }
        let channel = self.rsqr3 & 0x1f;
        if channel as usize >= CHANNEL_COUNT {
            return;
        }
        self.statr = (self.statr & !EOC) | STRT;
        // The model deliberately uses abstract ticks rather than HBCLK. The
        // charge, discharge, and ADC conversion terms preserve deterministic
        // ordering while leaving exact clock fidelity as a documented gap.
        let duration = u64::from(discharge & 0x07ff)
            .saturating_add(u64::from(self.tk_charge))
            .saturating_add(13)
            .max(1);
        self.conversion_due = Some(now.ticks().saturating_add(duration));
    }
}

/// CH32V006 ADC/TKEY register slice at `0x4001_2400`.
///
/// The WCH reference manual documents TKEY as an ADC extension: software
/// selects `ADC_RSQR3.SQ1`, writes charge and discharge durations, and reads
/// the converted result from `ADC_RDATAR`. This model keeps that sequence and
/// exposes host-controlled deterministic channel samples; analogue capacitance
/// physics and DMA are intentionally outside the functional slice.
pub struct WchTouchKey {
    name: String,
    state: Rc<RefCell<WchTouchKeyState>>,
}

impl WchTouchKey {
    /// Creates the reset peripheral and a host-facing sample handle.
    pub fn new(name: impl Into<String>) -> (Self, WchTouchKeyHandle) {
        let state = Rc::new(RefCell::new(WchTouchKeyState::reset()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            WchTouchKeyHandle { state },
        )
    }

    fn require_access(offset: u64, width: AccessWidth) -> Result<(), DeviceError> {
        if !matches!(width, AccessWidth::HalfWord | AccessWidth::Word) || offset & 3 != 0 {
            return Err(DeviceError::new(
                "WCH ADC/TKEY requires halfword or word access at a register boundary",
            ));
        }
        Ok(())
    }
}

impl Device for WchTouchKey {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        Self::require_access(offset, width)?;
        let mut state = self.state.borrow_mut();
        state.update(at);
        let value = match offset {
            0x00 => state.statr,
            0x04 => state.ctlr1,
            0x08 => state.ctlr2,
            0x10 => 0,
            0x34 => state.rsqr3,
            // A read of the aliased injection-data address is not the charge
            // configuration register; it exposes the retained data register.
            0x3c => 0,
            0x4c => {
                let value = state.rdatar;
                state.statr &= !EOC;
                value.into()
            }
            0x50 => state.ctlr3,
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled WCH ADC/TKEY read at offset {offset:#x}"
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
        at: SimTime,
    ) -> Result<(), DeviceError> {
        Self::require_access(offset, width)?;
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("value fits u32");
        let mut state = self.state.borrow_mut();
        state.update(at);
        match offset {
            0x00 => state.statr &= (value & STATUS_MASK) | !STATUS_MASK,
            0x04 => state.ctlr1 = value,
            0x08 => state.ctlr2 = value,
            0x34 => state.rsqr3 = value & 0x3fff_ffff,
            0x3c => state.tk_charge = (value & 0x07ff) as u16,
            0x4c => state.start(at, (value & 0x07ff) as u16),
            0x50 => state.ctlr3 = value,
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled WCH ADC/TKEY write at offset {offset:#x}"
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.borrow_mut() = WchTouchKeyState::reset();
    }
}
