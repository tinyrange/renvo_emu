use super::*;

pub(super) const DAC0L: usize = (0x30 << 8) | 0x84;
pub(super) const DAC0H: usize = (0x30 << 8) | 0x85;
pub(super) const DAC0ALT: usize = (0x30 << 8) | 0x8a;
pub(super) const DAC0CF0: usize = (0x30 << 8) | 0x91;
pub(super) const DAC0CF1: usize = (0x30 << 8) | 0x92;

const DAC0_EN: u8 = 0x80;
const DAC0_LJST: u8 = 0x20;
const DAC0_UPDATE_MASK: u8 = 0x0f;

impl Efm8State {
    fn dac_code(&self) -> u16 {
        if self.registers[DAC0CF0] & DAC0_LJST != 0 {
            (u16::from(self.registers[DAC0H]) << 2) | u16::from(self.registers[DAC0L] >> 6)
        } else {
            (u16::from(self.registers[DAC0H] & 0x03) << 8) | u16::from(self.registers[DAC0L])
        }
    }

    fn update_dac_output(&mut self, at: SimTime) {
        if self.registers[DAC0CF0] & DAC0_EN == 0 || self.dac_update_inhibited {
            return;
        }
        self.dac_output = self.dac_code();
        self.set_signal(self.dac_output_signal, u64::from(self.dac_output), 10, at);
    }

    pub(super) fn write_dac_register(&mut self, address: usize, value: u8, at: SimTime) {
        match address {
            DAC0CF0 => {
                self.registers[address] = value & (DAC0_EN | 0x40 | DAC0_LJST | DAC0_UPDATE_MASK);
                self.set_signal(
                    self.dac_enabled_signal,
                    u64::from(self.registers[DAC0CF0] & DAC0_EN != 0),
                    1,
                    at,
                );
                if self.registers[DAC0CF0] & DAC0_UPDATE_MASK == 0 {
                    self.update_dac_output(at);
                }
            }
            DAC0CF1 => self.registers[address] = value & 0x0f,
            DAC0ALT | DAC0L => {
                self.registers[address] = value;
                self.dac_update_inhibited = true;
            }
            DAC0H => {
                self.registers[address] = value;
                self.dac_update_inhibited = false;
                if self.registers[DAC0CF0] & DAC0_UPDATE_MASK == 0 {
                    self.update_dac_output(at);
                }
            }
            _ => unreachable!("validated DAC0 register"),
        }
    }
}
