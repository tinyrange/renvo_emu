use super::{Msp430PeripheralsHandle, Msp430State, overlaps, transfer_bytes};
use remu_bus::{Device, DeviceError, SharedMemory};
use remu_core::{AccessWidth, ResetKind, SimTime};

pub(super) const SYSCFG0: usize = 0x0160;
pub(super) const FRCTL0: usize = 0x01a0;
pub(super) const GCCTL0: usize = 0x01a4;
pub(super) const GCCTL1: usize = 0x01a6;
const FRAM_PASSWORD: u16 = 0xa500;
const REGISTER_READ_PASSWORD: u16 = 0x9600;
const SYSCFG0_VALUE_MASK: u16 = 0x0003;
const FRCTL0_VALUE_MASK: u16 = 0x0070;
const GCCTL0_VALUE_MASK: u16 = 0x00e6;
const GCCTL1_VALUE_MASK: u16 = 0x000e;
const GCCTL0_UBDRSTEN: u16 = 0x0080;
const GCCTL0_UBDIE: u16 = 0x0040;
const GCCTL0_FRPWR: u16 = 0x0004;
const GCCTL0_FRLPMPWR: u16 = 0x0002;

/// Main program FRAM as exposed by the compatibility memory map.
pub const MSP430_PROGRAM_FRAM_START: u64 = 0xc000;
/// Main program FRAM compatibility-window size.
pub const MSP430_PROGRAM_FRAM_SIZE: usize = 16 * 1024;
/// Information FRAM start address on the FR2433.
pub const MSP430_INFO_FRAM_START: u64 = 0x1800;
/// Information FRAM size on the FR2433.
pub const MSP430_INFO_FRAM_SIZE: usize = 512;

impl Msp430State {
    fn fram_program_write_protected(&self) -> bool {
        self.word(SYSCFG0) & 0x0001 != 0
    }

    fn fram_info_write_protected(&self) -> bool {
        self.word(SYSCFG0) & 0x0002 != 0
    }

    fn write_frctl0(&mut self, value: u16) {
        if value & 0xff00 != FRAM_PASSWORD {
            self.frctl_unlocked = false;
            self.frctl_reset = true;
            return;
        }
        self.frctl_unlocked = true;
        self.set_word(FRCTL0, value & FRCTL0_VALUE_MASK);
    }

    fn write_gcctl0(&mut self, value: u16) {
        if !self.frctl_unlocked {
            self.frctl_reset = true;
            return;
        }
        let mut value = value & GCCTL0_VALUE_MASK;
        if value & GCCTL0_UBDRSTEN != 0 && value & GCCTL0_UBDIE != 0 {
            value &= !GCCTL0_UBDIE;
        }
        self.set_word(GCCTL0, value);
    }

    fn write_gcctl1(&mut self, value: u16) {
        if !self.frctl_unlocked {
            self.frctl_reset = true;
            return;
        }
        let current = self.word(GCCTL1) & GCCTL1_VALUE_MASK;
        self.set_word(GCCTL1, current & (value & GCCTL1_VALUE_MASK));
    }

    fn write_syscfg0(&mut self, value: u16) {
        if value & 0xff00 == FRAM_PASSWORD {
            self.set_word(
                SYSCFG0,
                REGISTER_READ_PASSWORD | (value & SYSCFG0_VALUE_MASK),
            );
        }
    }

    pub(super) fn reset_fram(&mut self) {
        self.set_word(SYSCFG0, REGISTER_READ_PASSWORD | SYSCFG0_VALUE_MASK);
        self.set_word(FRCTL0, REGISTER_READ_PASSWORD);
        self.set_word(GCCTL0, GCCTL0_FRPWR | GCCTL0_FRLPMPWR);
        self.set_word(GCCTL1, 0);
        self.frctl_reset = false;
        self.frctl_unlocked = false;
        self.fram_write_ignored = 0;
    }
}

impl Msp430PeripheralsHandle {
    /// Returns whether protected FRAM controller registers may currently be written.
    pub fn frctl_unlocked(&self) -> bool {
        self.0
            .lock()
            .expect("MSP430 peripheral lock poisoned")
            .frctl_unlocked
    }

    /// Returns whether program or information FRAM writes are protected.
    pub fn fram_write_protected(&self, information: bool) -> bool {
        let state = self.0.lock().expect("MSP430 peripheral lock poisoned");
        if information {
            state.fram_info_write_protected()
        } else {
            state.fram_program_write_protected()
        }
    }

    /// Current FRAM wait-state setting (`FRCTL0.NWAITS`).
    pub fn fram_wait_states(&self) -> u8 {
        let state = self.0.lock().expect("MSP430 peripheral lock poisoned");
        ((state.word(FRCTL0) & FRCTL0_VALUE_MASK) >> 4) as u8
    }

    /// Returns whether the FRAM array power bit is enabled.
    pub fn fram_powered(&self) -> bool {
        self.0
            .lock()
            .expect("MSP430 peripheral lock poisoned")
            .word(GCCTL0)
            & GCCTL0_FRPWR
            != 0
    }

    /// Models the device's automatic FRAM array wake-up on access.
    pub fn power_fram(&self) {
        let mut state = self.0.lock().expect("MSP430 peripheral lock poisoned");
        let value = state.word(GCCTL0) | GCCTL0_FRPWR;
        state.set_word(GCCTL0, value & GCCTL0_VALUE_MASK);
    }

    /// Number of runtime writes ignored by FRAM write protection.
    pub fn fram_write_ignored(&self) -> u64 {
        self.0
            .lock()
            .expect("MSP430 peripheral lock poisoned")
            .fram_write_ignored
    }

    fn note_ignored_fram_write(&self) {
        let mut state = self.0.lock().expect("MSP430 peripheral lock poisoned");
        state.fram_write_ignored = state.fram_write_ignored.saturating_add(1);
    }

    /// Consumes a FRAM-controller protection fault, which is a PUC-like reset.
    pub fn take_frctl_reset(&self) -> bool {
        std::mem::take(
            &mut self
                .0
                .lock()
                .expect("MSP430 peripheral lock poisoned")
                .frctl_reset,
        )
    }
}

/// Functional, persistent FRAM window used by the FR2433 machine.
pub struct Msp430Fram {
    name: String,
    storage: SharedMemory,
    peripherals: Msp430PeripheralsHandle,
    information: bool,
}

impl Msp430Fram {
    /// Creates a program or information FRAM window over shared backing bytes.
    pub fn new(
        name: impl Into<String>,
        storage: SharedMemory,
        peripherals: Msp430PeripheralsHandle,
        information: bool,
    ) -> Self {
        Self {
            name: name.into(),
            storage,
            peripherals,
            information,
        }
    }

    fn range(&self, offset: u64, width: AccessWidth) -> Result<(usize, usize), DeviceError> {
        let start = usize::try_from(offset)
            .map_err(|_| DeviceError::new("MSP430 FRAM address does not fit usize"))?;
        let length = transfer_bytes(width);
        let end = start
            .checked_add(length)
            .ok_or_else(|| DeviceError::new("MSP430 FRAM access overflow"))?;
        if end > self.storage.len() {
            return Err(DeviceError::new(format!(
                "MSP430 FRAM access outside mapped window at {offset:#x}"
            )));
        }
        Ok((start, length))
    }
}

impl Device for Msp430Fram {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let (start, length) = self.range(offset, width)?;
        self.peripherals.power_fram();
        let bytes = self
            .storage
            .read_range(start, length)
            .expect("checked MSP430 FRAM read range");
        Ok(bytes.iter().enumerate().fold(0, |value, (index, byte)| {
            value | (u64::from(*byte) << (index * 8))
        }))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        let (start, length) = self.range(offset, width)?;
        self.peripherals.power_fram();
        if self.peripherals.fram_write_protected(self.information) {
            self.peripherals.note_ignored_fram_write();
            return Ok(());
        }
        let bytes = (0..length)
            .map(|index| (value >> (index * 8)) as u8)
            .collect::<Vec<_>>();
        if self.storage.write_range(start, &bytes) {
            Ok(())
        } else {
            Err(DeviceError::new("MSP430 FRAM write backing range failed"))
        }
    }

    fn reset(&mut self, _kind: ResetKind) {
        // FRAM contents persist across PUC, watchdog and software resets.
    }
}

pub(super) fn normalize_fram_read(state: &mut Msp430State, start: usize, length: usize) {
    if overlaps(start, length, SYSCFG0, 2) {
        let value = REGISTER_READ_PASSWORD | (state.word(SYSCFG0) & SYSCFG0_VALUE_MASK);
        state.set_word(SYSCFG0, value);
    }
    if overlaps(start, length, FRCTL0, 2) {
        let value = REGISTER_READ_PASSWORD | (state.word(FRCTL0) & FRCTL0_VALUE_MASK);
        state.set_word(FRCTL0, value);
    }
    if overlaps(start, length, GCCTL0, 2) {
        let value = state.word(GCCTL0) & GCCTL0_VALUE_MASK;
        state.set_word(GCCTL0, value);
    }
    if overlaps(start, length, GCCTL1, 2) {
        let value = state.word(GCCTL1) & GCCTL1_VALUE_MASK;
        state.set_word(GCCTL1, value);
    }
}

pub(super) fn handle_fram_write(
    state: &mut Msp430State,
    start: usize,
    length: usize,
    value: u64,
) -> Result<bool, DeviceError> {
    if overlaps(start, length, SYSCFG0, 2) {
        if start == SYSCFG0 && length == 2 {
            state.write_syscfg0(value as u16);
        }
        return Ok(true);
    }
    if start == FRCTL0 && length == 2 {
        state.write_frctl0(value as u16);
        return Ok(true);
    }
    if start == FRCTL0 + 1 && length == 1 {
        state.frctl_unlocked = value as u8 == (FRAM_PASSWORD >> 8) as u8;
        return Ok(true);
    }
    if start == FRCTL0 && length == 1 {
        if state.frctl_unlocked {
            state.set_word(FRCTL0, value as u16 & FRCTL0_VALUE_MASK);
        } else {
            state.frctl_reset = true;
        }
        return Ok(true);
    }
    if overlaps(start, length, FRCTL0, 2) {
        return Err(DeviceError::new(
            "MSP430 FRCTL0 requires an aligned byte or half-word access",
        ));
    }
    for register in [GCCTL0, GCCTL1] {
        if start == register && length == 2 {
            if register == GCCTL0 {
                state.write_gcctl0(value as u16);
            } else {
                state.write_gcctl1(value as u16);
            }
            return Ok(true);
        }
        if (start == register || start == register + 1) && length == 1 {
            let current = state.word(register);
            let merged = if start == register {
                (current & 0xff00) | value as u16
            } else {
                (current & 0x00ff) | ((value as u16) << 8)
            };
            if register == GCCTL0 {
                state.write_gcctl0(merged);
            } else {
                state.write_gcctl1(merged);
            }
            return Ok(true);
        }
        if overlaps(start, length, register, 2) {
            return Err(DeviceError::new(format!(
                "MSP430 controller register {register:#x} requires an aligned byte or half-word access"
            )));
        }
    }
    Ok(false)
}
