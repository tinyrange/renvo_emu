use super::{Msp430PeripheralsHandle, Msp430State, overlaps};

pub(super) const CSCTL0: usize = 0x0180;
pub(super) const CSCTL1: usize = 0x0182;
pub(super) const CSCTL2: usize = 0x0184;
pub(super) const CSCTL3: usize = 0x0186;
pub(super) const CSCTL4: usize = 0x0188;
pub(super) const CSCTL5: usize = 0x018a;
pub(super) const CSCTL6: usize = 0x018c;
pub(super) const CSCTL7: usize = 0x018e;
pub(super) const CSCTL8: usize = 0x0190;

pub(super) const CSCTL0_RESET: u16 = 0x0000;
pub(super) const CSCTL1_RESET: u16 = 0x0033;
pub(super) const CSCTL2_RESET: u16 = 0x101f;
pub(super) const CSCTL3_RESET: u16 = 0x0000;
pub(super) const CSCTL4_RESET: u16 = 0x0100;
pub(super) const CSCTL5_RESET: u16 = 0x1000;
pub(super) const CSCTL6_RESET: u16 = 0x08c1;
pub(super) const CSCTL7_RESET: u16 = 0x0740;
pub(super) const CSCTL8_RESET: u16 = 0x0007;

impl Msp430PeripheralsHandle {
    /// Returns the functional MCLK divider selected by CSCTL5.DIVM.
    pub fn mclk_divider(&self) -> u64 {
        let state = self.0.lock().expect("MSP430 peripheral lock poisoned");
        1_u64 << u32::from(state.word(CSCTL5) & 0x0007)
    }
    /// Returns the functional SMCLK divider selected by CSCTL5.DIVS.
    pub fn smclk_divider(&self) -> u64 {
        let state = self.0.lock().expect("MSP430 peripheral lock poisoned");
        1_u64 << u32::from((state.word(CSCTL5) >> 4) & 0x0003)
    }
    /// Returns the programmed FLL multiplier with zero normalized to one.
    pub fn fll_multiplier(&self) -> u16 {
        let state = self.0.lock().expect("MSP430 peripheral lock poisoned");
        (state.word(CSCTL2) & 0x03ff).max(1)
    }
    /// Returns the selected MCLK/SMCLK source encoding from CSCTL4.SELMS.
    pub fn mclk_source(&self) -> u16 {
        let state = self.0.lock().expect("MSP430 peripheral lock poisoned");
        state.word(CSCTL4) & 0x0007
    }
}

fn normalize_clock_register(register: usize, value: u16) -> u16 {
    match register {
        CSCTL0 => value & 0x3fff,
        CSCTL1 => value & 0x00ff,
        CSCTL2 => {
            let value = value & 0x7fff;
            if value & 0x03ff == 0 {
                value | 1
            } else {
                value
            }
        }
        CSCTL3 => value & 0x00b0,
        CSCTL4 => value & 0x0707,
        CSCTL5 => value & 0x10f7,
        CSCTL6 => value & 0x2fd3,
        CSCTL7 => (value & 0x3c53) | (CSCTL7_RESET & 0x0304),
        CSCTL8 => value & 0x000f,
        _ => value,
    }
}

pub(super) fn normalize_clock_registers(state: &mut Msp430State, start: usize, length: usize) {
    for register in [
        CSCTL0, CSCTL1, CSCTL2, CSCTL3, CSCTL4, CSCTL5, CSCTL6, CSCTL7, CSCTL8,
    ] {
        if overlaps(start, length, register, 2) {
            let current = state.word(register);
            state.set_word(register, normalize_clock_register(register, current));
        }
    }
}
