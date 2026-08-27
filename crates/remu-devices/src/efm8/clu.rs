use super::*;

pub(super) const CLEN0: usize = (PAGE3 << 8) | 0xc6;
pub(super) const CLIE0: usize = (PAGE3 << 8) | 0xc7;
pub(super) const CLIF0: usize = (PAGE3 << 8) | 0xe8;
pub(super) const CLOUT0: usize = (PAGE3 << 8) | 0xd1;
pub(super) const CLU_MX: [usize; 4] = [
    (PAGE3 << 8) | 0x84,
    (PAGE3 << 8) | 0x85,
    (PAGE3 << 8) | 0x91,
    (PAGE3 << 8) | 0xae,
];
pub(super) const CLU_FN: [usize; 4] = [
    (PAGE3 << 8) | 0xaf,
    (PAGE3 << 8) | 0xb2,
    (PAGE3 << 8) | 0xb5,
    (PAGE3 << 8) | 0xbe,
];
pub(super) const CLU_CF: [usize; 4] = [
    (PAGE3 << 8) | 0xb1,
    (PAGE3 << 8) | 0xb3,
    (PAGE3 << 8) | 0xb6,
    (PAGE3 << 8) | 0xbf,
];
pub(super) const EIE2_CL0: u8 = 0x10;
pub(super) const CLU_CF_OUTSEL: u8 = 0x80;
pub(super) const CLU_CF_OEN: u8 = 0x40;
pub(super) const CLU_CF_RST: u8 = 0x08;
pub(super) const CLU_CF_CLKINV: u8 = 0x04;
pub(super) const CLU_CF_CLKSEL: u8 = 0x03;

impl Efm8State {
    fn clu_pin(unit: usize, input: usize, selector: u8) -> Option<(usize, u8)> {
        // The EFM8BB52 manual's CLUnMX tables enumerate the external pin
        // choices as selectors 8..15. Internal timer/PWM/serial sources are
        // intentionally left low until their peripheral models are present.
        const A: [[(usize, u8); 8]; 4] = [
            [
                (0, 0),
                (0, 2),
                (0, 4),
                (0, 6),
                (1, 0),
                (1, 2),
                (1, 4),
                (1, 6),
            ],
            [
                (0, 4),
                (0, 5),
                (1, 0),
                (1, 2),
                (1, 4),
                (1, 5),
                (2, 0),
                (2, 2),
            ],
            [
                (0, 0),
                (0, 1),
                (1, 0),
                (1, 1),
                (1, 6),
                (1, 7),
                (2, 0),
                (2, 1),
            ],
            [
                (0, 2),
                (0, 3),
                (0, 6),
                (0, 7),
                (1, 2),
                (1, 3),
                (2, 2),
                (2, 3),
            ],
        ];
        const B: [[(usize, u8); 8]; 4] = [
            [
                (0, 1),
                (0, 3),
                (0, 5),
                (0, 7),
                (1, 1),
                (1, 3),
                (1, 5),
                (1, 7),
            ],
            [
                (0, 6),
                (0, 7),
                (1, 1),
                (1, 3),
                (1, 6),
                (1, 7),
                (2, 1),
                (2, 3),
            ],
            [
                (0, 2),
                (0, 3),
                (1, 2),
                (1, 3),
                (1, 4),
                (1, 5),
                (2, 2),
                (2, 3),
            ],
            [
                (0, 0),
                (0, 1),
                (0, 4),
                (0, 5),
                (1, 0),
                (1, 1),
                (2, 0),
                (2, 1),
            ],
        ];
        let table = if input == 0 { &A } else { &B };
        table
            .get(unit)
            .and_then(|unit_table| unit_table.get(usize::from(selector.saturating_sub(8))))
            .copied()
    }

    fn clu_input(&self, unit: usize, input: usize, lut: &[bool; 4]) -> bool {
        if let Some(override_inputs) = self.clu_input_overrides[unit] {
            return override_inputs[input];
        }
        let selector = if input == 0 {
            self.registers[CLU_MX[unit]] >> 4
        } else {
            self.registers[CLU_MX[unit]] & 0x0f
        };
        match selector {
            0..=3 => lut[usize::from(selector)],
            8..=15 => Self::clu_pin(unit, input, selector).map_or(false, |(port, pin)| {
                self.resolved_port(port) & (1 << pin) != 0
            }),
            _ => false,
        }
    }

    fn clu_enabled(&self, unit: usize) -> bool {
        self.registers[CLEN0] & (1 << unit) != 0
    }

    pub(super) fn clu_output(&self, unit: usize) -> bool {
        self.registers[CLOUT0] & (1 << unit) != 0
    }

    pub(super) fn refresh_clu(&mut self, at: SimTime) {
        let previous = [
            self.clu_output(0),
            self.clu_output(1),
            self.clu_output(2),
            self.clu_output(3),
        ];
        let mut lut = self.clu_lut_outputs;
        // A CLU can consume the preceding CLU's output and CLU0 wraps from
        // CLU3. Iterate the ring to a deterministic fixed point so simple
        // cascades settle without a clock-accurate event simulator.
        for _ in 0..4 {
            let old = lut;
            for unit in 0..4 {
                if !self.clu_enabled(unit) {
                    lut[unit] = false;
                    continue;
                }
                let a = self.clu_input(unit, 0, &old);
                let b = self.clu_input(unit, 1, &old);
                let carry = old[if unit == 0 { 3 } else { unit - 1 }];
                let index = usize::from(u8::from(carry) | (u8::from(b) << 1) | (u8::from(a) << 2));
                lut[unit] = self.registers[CLU_FN[unit]] & (1 << index) != 0;
            }
        }
        self.clu_lut_outputs = lut;
        for unit in 0..4 {
            let config = self.registers[CLU_CF[unit]];
            if self.clu_enabled(unit) && config & CLU_CF_OUTSEL == 0 {
                // The functional scheduler treats each refresh as one
                // SYSCLK opportunity for the D flip-flop. CLKSEL and CLKINV
                // remain metadata until timer/clock sources are modelled.
                self.clu_ff[unit] = lut[unit];
            }
            let output = if !self.clu_enabled(unit) {
                false
            } else if config & CLU_CF_OUTSEL != 0 {
                lut[unit]
            } else {
                self.clu_ff[unit]
            };
            self.registers[CLOUT0] =
                (self.registers[CLOUT0] & !(1 << unit)) | (u8::from(output) << unit);
            if output != previous[unit] {
                let rising = 1 << (unit * 2 + 1);
                let falling = 1 << (unit * 2);
                self.registers[CLIF0] |= if output { rising } else { falling };
            }
            self.set_signal(self.clu_output_signals[unit], u64::from(output), 1, at);
        }
    }

    pub(super) fn write_clu_register(&mut self, address: usize, value: u8, at: SimTime) -> bool {
        if address == CLEN0 {
            self.registers[address] = value & 0x0f;
        } else if address == CLIE0 {
            self.registers[address] = value;
        } else if address == CLIF0 {
            self.registers[address] = value;
        } else if address == CLOUT0 {
            return true;
        } else if let Some(unit) = CLU_MX.iter().position(|register| *register == address) {
            self.registers[address] = value;
            let _ = unit;
        } else if let Some(unit) = CLU_FN.iter().position(|register| *register == address) {
            self.registers[address] = value;
            let _ = unit;
        } else if let Some(unit) = CLU_CF.iter().position(|register| *register == address) {
            if value & CLU_CF_RST != 0 {
                self.clu_ff[unit] = false;
            }
            self.registers[address] =
                value & (CLU_CF_OUTSEL | CLU_CF_OEN | CLU_CF_CLKINV | CLU_CF_CLKSEL);
        } else {
            return false;
        }
        self.refresh_clu(at);
        true
    }
}
