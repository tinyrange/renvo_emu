use super::*;

pub(super) const CMP0CN0: usize = (0x30 << 8) | 0x9b;
pub(super) const CMP0CN1: usize = (0x30 << 8) | 0x99;
pub(super) const CMP0MD: usize = (0x30 << 8) | 0x9d;
pub(super) const CMP0MX: usize = (0x30 << 8) | 0x9f;
pub(super) const CMP1CN0: usize = (0x30 << 8) | 0xbf;
pub(super) const CMP1CN1: usize = (0x30 << 8) | 0xac;
pub(super) const CMP1MD: usize = (0x30 << 8) | 0xab;
pub(super) const CMP1MX: usize = (0x30 << 8) | 0xaa;

const CMP_CPEN: u8 = 0x80;
const CMP_CPOUT: u8 = 0x40;
const CMP_CPRIF: u8 = 0x20;
const CMP_CPFIF: u8 = 0x10;
const CMP_CPINV: u8 = 0x40;
const CMP_CPRIE: u8 = 0x20;
const CMP_CPFIE: u8 = 0x10;

impl Efm8State {
    fn comparator_registers(comparator: usize) -> (usize, usize, usize, usize) {
        match comparator {
            0 => (CMP0CN0, CMP0CN1, CMP0MD, CMP0MX),
            1 => (CMP1CN0, CMP1CN1, CMP1MD, CMP1MX),
            _ => unreachable!("validated EFM8 comparator index"),
        }
    }

    fn comparator_output(&self, comparator: usize) -> bool {
        let (control, _, mode, _) = Self::comparator_registers(comparator);
        if self.registers[control] & CMP_CPEN == 0 {
            return false;
        }
        let positive = self.comparator_inputs[comparator][0];
        let negative = self.comparator_inputs[comparator][1];
        let mut output = positive > negative;
        if self.registers[mode] & CMP_CPINV != 0 {
            output = !output;
        }
        output
    }

    fn refresh_comparator(&mut self, comparator: usize, at: SimTime) {
        let (control, _, _, _) = Self::comparator_registers(comparator);
        let previous = self.registers[control] & CMP_CPOUT != 0;
        let output = self.comparator_output(comparator);
        if output != previous {
            self.registers[control] =
                (self.registers[control] & !CMP_CPOUT) | if output { CMP_CPOUT } else { 0 };
            self.registers[control] |= if output { CMP_CPRIF } else { CMP_CPFIF };
        }
        self.set_signal(
            self.comparator_output_signals[comparator],
            u64::from(output),
            1,
            at,
        );
    }

    pub(super) fn refresh_comparators(&mut self, at: SimTime) {
        self.refresh_comparator(0, at);
        self.refresh_comparator(1, at);
    }

    pub(super) fn write_comparator_register(&mut self, address: usize, value: u8, at: SimTime) {
        let (comparator, control, control1, mode, mux) =
            if [CMP0CN0, CMP0CN1, CMP0MD, CMP0MX].contains(&address) {
                (0, CMP0CN0, CMP0CN1, CMP0MD, CMP0MX)
            } else {
                (1, CMP1CN0, CMP1CN1, CMP1MD, CMP1MX)
            };
        match address {
            address if address == control => {
                self.registers[address] =
                    (self.registers[address] & CMP_CPOUT) | (value & (CMP_CPEN | 0x3f));
            }
            address if address == control1 => self.registers[address] = value & 0xaf,
            address if address == mode => {
                self.registers[address] = (self.registers[address] & 0x80) | (value & 0x77);
            }
            address if address == mux => self.registers[address] = value,
            _ => unreachable!("validated EFM8 comparator register"),
        }
        self.refresh_comparator(comparator, at);
    }

    pub(super) fn comparator_interrupt_active(&self, comparator: usize) -> bool {
        let (control, _, mode, _) = Self::comparator_registers(comparator);
        let enable = if comparator == 0 { 0x20 } else { 0x40 };
        self.registers[EIE1] & enable != 0
            && ((self.registers[control] & CMP_CPRIF != 0 && self.registers[mode] & CMP_CPRIE != 0)
                || (self.registers[control] & CMP_CPFIF != 0
                    && self.registers[mode] & CMP_CPFIE != 0))
    }
}
