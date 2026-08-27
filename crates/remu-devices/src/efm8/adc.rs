use super::*;

pub(super) const ADC0CN0: usize = 0xe8;
pub(super) const ADC0CN1: usize = 0xb2;
pub(super) const ADC0CN2: usize = 0xb3;
pub(super) const ADC0CF1: usize = 0xb9;
pub(super) const ADC0CF2: usize = 0xdf;
pub(super) const ADC0L: usize = 0xbd;
pub(super) const ADC0H: usize = 0xbe;
pub(super) const ADC0GTH: usize = 0xc4;
pub(super) const ADC0GTL: usize = 0xc3;
pub(super) const ADC0LTH: usize = 0xc6;
pub(super) const ADC0LTL: usize = 0xc5;
pub(super) const ADC0MX: usize = 0xbb;

pub(super) const ADC0_ADEN: u8 = 0x80;
pub(super) const ADC0_ADINT: u8 = 0x20;
pub(super) const ADC0_ADBUSY: u8 = 0x10;
pub(super) const ADC0_ADWINT: u8 = 0x08;
pub(super) const ADC0_EADC0: u8 = 0x08;
pub(super) const ADC0_EWADC0: u8 = 0x04;

impl Efm8State {
    pub(super) fn complete_adc_conversion(&mut self, at: SimTime) {
        let control = self.registers[ADC0CN0];
        if control & ADC0_ADEN == 0 {
            self.registers[ADC0CN0] &= !ADC0_ADBUSY;
            return;
        }
        let channel = usize::from(self.registers[ADC0MX] & 0x1f);
        let mut sample = self.adc_inputs[channel.min(self.adc_inputs.len() - 1)];
        match (self.registers[ADC0CN1] >> 5) & 0x03 {
            0x01 => sample >>= 2,
            0x02 => sample >>= 4,
            _ => {}
        }
        let repeat = match self.registers[ADC0CN1] & 0x07 {
            0x01 => 4,
            0x02 => 8,
            0x03 => 16,
            0x04 => 32,
            _ => 1,
        };
        let mut result = sample.saturating_mul(repeat);
        result >>= (self.registers[ADC0CN1] >> 3) & 0x03;
        let [low, high] = result.to_le_bytes();
        self.registers[ADC0L] = low;
        self.registers[ADC0H] = high;
        self.registers[ADC0CN0] &= !ADC0_ADBUSY;
        self.registers[ADC0CN0] |= ADC0_ADINT;
        let greater = u16::from_be_bytes([self.registers[ADC0GTH], self.registers[ADC0GTL]]);
        let less = u16::from_be_bytes([self.registers[ADC0LTH], self.registers[ADC0LTL]]);
        if result > greater || result < less {
            self.registers[ADC0CN0] |= ADC0_ADWINT;
        }
        self.set_signal(self.adc_result_signal, u64::from(result), 16, at);
        self.set_signal(self.adc_eoc_signal, 1, 1, at);
        self.set_signal(
            self.adc_window_signal,
            u64::from(self.registers[ADC0CN0] & ADC0_ADWINT != 0),
            1,
            at,
        );
    }
}
