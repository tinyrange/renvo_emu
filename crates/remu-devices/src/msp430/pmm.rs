use super::{LOCKLPM5, Msp430PeripheralsHandle, Msp430State, PM5CTL0, overlaps};
use remu_core::{ResetKind, SimTime};

pub(super) const PMMCTL0: usize = 0x0120;
pub(super) const PMMCTL1: usize = 0x0122;
pub(super) const PMMCTL2: usize = 0x0124;
pub(super) const PMMIFG: usize = 0x012a;
pub(super) const PMMIE: usize = 0x012e;
pub(super) const LPM5SW: u16 = 0x0010;
pub(super) const LPM5SM: u16 = 0x0020;
pub(super) const PMMCTL0_SVSHE: u16 = 0x0040;
pub(super) const PMMCTL0_REG_OFF: u16 = 0x0010;
pub(super) const PMMCTL0_SWPOR: u16 = 0x0008;
pub(super) const PMMCTL0_SWBOR: u16 = 0x0004;
const PMMCTL0_VALUE_MASK: u16 = PMMCTL0_SVSHE | PMMCTL0_REG_OFF | PMMCTL0_SWPOR | PMMCTL0_SWBOR;
const PMMCTL2_VALUE_MASK: u16 = 0x00fb;
pub(super) const PMMIFG_VALUE_MASK: u16 = 0xa700;
const PMMPW: u16 = 0x9600;
pub(super) const PMM_UNLOCK: u8 = 0xa5;

/// Functional low-power mode selected by the MSP430 status register and PMM.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Msp430LowPowerMode {
    /// CPU and clocks are running.
    Active,
    /// CPUOFF is set without clock gating.
    Lpm0,
    /// CPUOFF and SCG0 are set.
    Lpm1,
    /// CPUOFF and SCG1 are set.
    Lpm2,
    /// CPUOFF, SCG0, and SCG1 are set.
    Lpm3,
    /// LPM3 with OSCOFF.
    Lpm4,
    /// LPM3 with the regulator switched off.
    Lpm3_5,
    /// LPM4 with the regulator switched off.
    Lpm4_5,
}

impl Msp430State {
    fn normalized_pmmctl0(&self) -> u16 {
        PMMPW | (self.word(PMMCTL0) & PMMCTL0_VALUE_MASK)
    }
    fn normalized_pm5ctl0(&self) -> u16 {
        let value = self.word(PM5CTL0) & (LPM5SM | LPM5SW | LOCKLPM5);
        if value & LPM5SM == 0 {
            value | LPM5SW
        } else {
            value
        }
    }
    fn request_pmm_reset(&mut self, kind: ResetKind, flag: u16, at: SimTime) {
        self.pmm_reset = Some(kind);
        self.pmm_reset_flags |= flag;
        self.set_signal(self.pmm_reset_signal, 1, 1, at);
    }
    fn pmm_write_fault(&mut self, at: SimTime) {
        self.request_pmm_reset(ResetKind::Software, 0, at);
    }
    fn write_pmmctl0_word(&mut self, value: u16, at: SimTime) {
        if value.to_be_bytes()[0] != PMM_UNLOCK {
            self.pmm_unlocked = false;
            self.pmm_write_fault(at);
            return;
        }
        self.pmm_unlocked = true;
        self.apply_pmmctl0(value, at);
    }
    fn write_pmmctl0_byte(&mut self, address: usize, value: u8, at: SimTime) {
        if address == PMMCTL0 + 1 {
            self.pmm_unlocked = value == PMM_UNLOCK;
            return;
        }
        if !self.pmm_unlocked {
            self.pmm_write_fault(at);
            return;
        }
        self.apply_pmmctl0(PMMPW | u16::from(value), at);
    }
    fn apply_pmmctl0(&mut self, value: u16, at: SimTime) {
        let value = value & PMMCTL0_VALUE_MASK;
        self.set_word(PMMCTL0, value & !(PMMCTL0_SWPOR | PMMCTL0_SWBOR));
        if value & PMMCTL0_SWPOR != 0 {
            self.request_pmm_reset(ResetKind::Software, 1 << 10, at);
        } else if value & PMMCTL0_SWBOR != 0 {
            self.request_pmm_reset(ResetKind::Software, 1 << 8, at);
        }
    }
    fn write_protected_pmm_register(
        &mut self,
        register: usize,
        value: u16,
        width: usize,
        at: SimTime,
    ) {
        if !self.pmm_unlocked {
            self.pmm_write_fault(at);
            return;
        }
        match register {
            PMMCTL1 if width == 2 => self.set_word(PMMCTL1, PMMPW),
            PMMCTL1 => {}
            PMMCTL2 => self.set_word(PMMCTL2, (value & PMMCTL2_VALUE_MASK) & !0x00c0),
            PMMIFG => self.set_word(PMMIFG, value & PMMIFG_VALUE_MASK),
            PMMIE => self.set_word(PMMIE, 0),
            _ => unreachable!("unsupported PMM register"),
        }
    }
    fn write_protected_pmm_byte(
        &mut self,
        register: usize,
        address: usize,
        value: u8,
        at: SimTime,
    ) {
        let current = self.word(register);
        let merged = if address == register {
            (current & 0xff00) | u16::from(value)
        } else {
            (current & 0x00ff) | (u16::from(value) << 8)
        };
        self.write_protected_pmm_register(register, merged, 1, at);
    }
    pub(super) fn reset_pmm(&mut self, at: SimTime) {
        let flags = self.pmm_reset_flags;
        self.set_word(PMMCTL0, PMMCTL0_SVSHE);
        self.set_word(PMMCTL1, PMMPW);
        self.set_word(PMMIFG, flags & PMMIFG_VALUE_MASK);
        self.set_word(PM5CTL0, LPM5SW | LOCKLPM5);
        self.pmm_unlocked = false;
        self.pmm_reset = None;
        self.pmm_reset_flags = 0;
        self.set_signal(self.pmm_reset_signal, 0, 1, at);
    }
    fn low_power_mode(&self, status: u16) -> Msp430LowPowerMode {
        const CPUOFF: u16 = 1 << 4;
        const SCG0: u16 = 1 << 5;
        const SCG1: u16 = 1 << 6;
        const OSCOFF: u16 = 1 << 7;
        let bits = status & (SCG0 | SCG1 | OSCOFF);
        let mode = match (status & CPUOFF != 0, bits) {
            (false, _) => Msp430LowPowerMode::Active,
            (true, 0) => Msp430LowPowerMode::Lpm0,
            (true, b) if b == SCG0 => Msp430LowPowerMode::Lpm1,
            (true, b) if b == SCG1 => Msp430LowPowerMode::Lpm2,
            (true, b) if b == SCG0 | SCG1 => Msp430LowPowerMode::Lpm3,
            (true, _) => Msp430LowPowerMode::Lpm4,
        };
        if self.word(PMMCTL0) & PMMCTL0_REG_OFF == 0 {
            return mode;
        }
        match mode {
            Msp430LowPowerMode::Lpm3 => Msp430LowPowerMode::Lpm3_5,
            Msp430LowPowerMode::Lpm4 => Msp430LowPowerMode::Lpm4_5,
            _ => mode,
        }
    }
}

impl Msp430PeripheralsHandle {
    /// Returns whether the PMM password currently permits writes.
    pub fn pmm_unlocked(&self) -> bool {
        self.0
            .lock()
            .expect("MSP430 peripheral lock poisoned")
            .pmm_unlocked
    }
    /// Classifies an MSP430 status value using the regulator setting.
    pub fn low_power_mode(&self, status: u16) -> Msp430LowPowerMode {
        self.0
            .lock()
            .expect("MSP430 peripheral lock poisoned")
            .low_power_mode(status)
    }
    /// Consumes a pending PMM reset request.
    pub fn take_pmm_reset(&self) -> Option<ResetKind> {
        self.0
            .lock()
            .expect("MSP430 peripheral lock poisoned")
            .pmm_reset
            .take()
    }
}

pub(super) fn normalize_pmm_read(state: &mut Msp430State, start: usize, length: usize) {
    if overlaps(start, length, PMMCTL0, 2) {
        let v = state.normalized_pmmctl0();
        state.set_word(PMMCTL0, v);
    }
    if overlaps(start, length, PMMCTL1, 2) {
        state.set_word(PMMCTL1, PMMPW);
    }
    if overlaps(start, length, PMMCTL2, 2) {
        let v = state.word(PMMCTL2) & PMMCTL2_VALUE_MASK;
        state.set_word(PMMCTL2, v);
    }
    if overlaps(start, length, PMMIFG, 2) {
        let v = state.word(PMMIFG) & PMMIFG_VALUE_MASK;
        state.set_word(PMMIFG, v);
    }
    if overlaps(start, length, PMMIE, 2) {
        state.set_word(PMMIE, 0);
    }
    if overlaps(start, length, PM5CTL0, 2) {
        let v = state.normalized_pm5ctl0();
        state.set_word(PM5CTL0, v);
    }
}

pub(super) fn handle_pmm_write(
    state: &mut Msp430State,
    start: usize,
    length: usize,
    value: u64,
    at: SimTime,
) -> bool {
    if start == PMMCTL0 && length == 2 {
        state.write_pmmctl0_word(value as u16, at);
        return true;
    }
    if (start == PMMCTL0 || start == PMMCTL0 + 1) && length == 1 {
        state.write_pmmctl0_byte(start, value as u8, at);
        return true;
    }
    for register in [PMMCTL1, PMMCTL2, PMMIFG, PMMIE] {
        if start == register && length == 2 {
            state.write_protected_pmm_register(register, value as u16, 2, at);
            return true;
        }
        if (start == register || start == register + 1) && length == 1 {
            state.write_protected_pmm_byte(register, start, value as u8, at);
            return true;
        }
    }
    false
}

pub(super) fn normalize_pm5_write(state: &mut Msp430State, start: usize, length: usize) {
    if overlaps(start, length, PM5CTL0, 2) {
        let v = state.normalized_pm5ctl0();
        state.set_word(PM5CTL0, v);
    }
}
