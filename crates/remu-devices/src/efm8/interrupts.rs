use super::*;

impl Efm8State {
    pub(super) fn interrupt_levels(&self) -> [bool; 30] {
        let enabled = self.registers[IE];
        if enabled & IE_EA == 0 {
            return [false; 30];
        }
        let active = [
            enabled & IE_ET0 != 0 && self.registers[TCON] & TCON_TF0 != 0,
            enabled & IE_ES0 != 0 && self.registers[SCON0] & (SCON0_RI | SCON0_TI) != 0,
            enabled & IE_ET2 != 0 && self.registers[TMR2CN0] & TMR2_TF2H != 0,
            enabled & IE_ESPI0 != 0 && self.registers[SPI0CN0] & SPI0_SPIF != 0,
            enabled & IE_ET1 != 0 && self.registers[TCON] & TCON_TF1 != 0,
            self.registers[Efm8SmbusRegister::Eie1.offset()] & EIE1_ESMB0 != 0
                && self.registers[Efm8SmbusRegister::Smb0Cn0.offset()] & SMB0CN0_SI != 0
                && (self.registers[Efm8SmbusRegister::Smb0Cn0.offset()] & SMB0CN0_MASTER != 0
                    || self.registers[Efm8SmbusRegister::Smb0Cf.offset()] & SMB0CF_INH == 0),
        ];
        let priorities = [
            self.registers[IP] & IE_ET0 != 0,
            self.registers[IP] & IE_ES0 != 0,
            self.registers[IP] & IE_ET2 != 0,
            self.registers[IP] & IE_ESPI0 != 0,
            self.registers[IP] & IE_ET1 != 0,
            self.registers[Efm8SmbusRegister::Eip1.offset()] & EIP1_PSMB0 != 0,
        ];
        const LOW_LINES: [usize; 6] = [0, 1, 2, 6, 8, 10];
        const HIGH_LINES: [usize; 6] = [3, 4, 5, 7, 9, 11];
        let mut levels = [false; 30];
        for source in 0..active.len() {
            if active[source] {
                levels[if priorities[source] {
                    HIGH_LINES[source]
                } else {
                    LOW_LINES[source]
                }] = true;
            }
        }
        if self.pca_interrupt_pending() {
            levels[if self.pca_high_priority() { 7 } else { 6 }] = true;
        }
        let uart1_pending = self.registers[SCON1] & (SCON1_RI | SCON1_TI) != 0;
        let uart1_enabled = self.registers[EIE2] & 1 != 0 || self.registers[EIE2_PAGE10] & 1 != 0;
        if uart1_pending && uart1_enabled {
            let high = self.registers[EIP2] & 1 != 0 || self.registers[EIP2H] & 1 != 0;
            levels[12 + usize::from(high)] = true;
        }
        let timer3 = self.registers[EIE1] & EIE1_ET3 != 0
            && ((self.registers[TMR3CN0] & TMR3_TF3H != 0
                && self.registers[TMR3CN0] & TMR3_TF3CEN != 0)
                || (self.registers[TMR3CN0] & TMR3_TF3L != 0
                    && self.registers[TMR3CN0] & TMR3_TF3LEN != 0));
        let timer4 = self.registers[EIE2] & EIE2_ET4 != 0
            && ((self.registers[TMR4CN0] & TMR4_TF4H != 0
                && self.registers[TMR4CN0] & TMR4_TF4CEN != 0)
                || (self.registers[TMR4CN0] & TMR4_TF4L != 0
                    && self.registers[TMR4CN0] & TMR4_TF4LEN != 0));
        let timer5 = self.registers[EIE2] & EIE2_ET5 != 0
            && ((self.registers[TMR5CN0] & TMR5_TF5H != 0
                && self.registers[TMR5CN0] & TMR5_TF5CEN != 0)
                || (self.registers[TMR5CN0] & TMR5_TF5L != 0
                    && self.registers[TMR5CN0] & TMR5_TF5LEN != 0));
        let timer3_high = self.registers[EIP1] & 0x80 != 0 || self.registers[EIP1H] & 0x80 != 0;
        let timer4_high = self.registers[EIP2] & 0x04 != 0 || self.registers[EIP2H] & 0x04 != 0;
        let timer5_high = self.registers[EIP2] & 0x08 != 0 || self.registers[EIP2H] & 0x08 != 0;
        if timer3 {
            levels[14 + usize::from(timer3_high)] = true;
        }
        if timer4 {
            levels[16 + usize::from(timer4_high)] = true;
        }
        if timer5 {
            levels[18 + usize::from(timer5_high)] = true;
        }
        let adc_window =
            self.registers[EIE1] & ADC0_EWADC0 != 0 && self.registers[ADC0CN0] & ADC0_ADWINT != 0;
        let adc_complete =
            self.registers[EIE1] & ADC0_EADC0 != 0 && self.registers[ADC0CN0] & ADC0_ADINT != 0;
        let adc_window_high = self.registers[EIP1] & 0x04 != 0 || self.registers[EIP1H] & 0x04 != 0;
        let adc_complete_high =
            self.registers[EIP1] & 0x08 != 0 || self.registers[EIP1H] & 0x08 != 0;
        if adc_window {
            levels[20 + usize::from(adc_window_high)] = true;
        }
        if adc_complete {
            levels[22 + usize::from(adc_complete_high)] = true;
        }
        for comparator in 0..2 {
            if self.comparator_interrupt_active(comparator) {
                let priority_bit = if comparator == 0 { 0x20 } else { 0x40 };
                let high = self.registers[EIP1] & priority_bit != 0
                    || self.registers[EIP1H] & priority_bit != 0;
                levels[24 + comparator * 2 + usize::from(high)] = true;
            }
        }
        if self.registers[EIE2] & EIE2_CL0 != 0
            && self.registers[CLIE0] & self.registers[CLIF0] != 0
        {
            let high =
                self.registers[EIP2] & EIE2_CL0 != 0 || self.registers[EIP2H] & EIE2_CL0 != 0;
            levels[28 + usize::from(high)] = true;
        }
        levels
    }

    pub(super) fn update_interrupt_signals(&self, at: SimTime) {
        self.set_signal(
            self.timer0_irq_signal,
            u64::from(self.registers[TCON] & TCON_TF0 != 0),
            1,
            at,
        );
        self.set_signal(
            self.timer1_irq_signal,
            u64::from(self.registers[TCON] & TCON_TF1 != 0),
            1,
            at,
        );
        self.set_signal(
            self.timer2_irq_signal,
            u64::from(self.registers[TMR2CN0] & TMR2_TF2H != 0),
            1,
            at,
        );
        self.set_signal(
            self.timer3_irq_signal,
            u64::from(self.registers[TMR3CN0] & (TMR3_TF3L | TMR3_TF3H) != 0),
            1,
            at,
        );
        self.set_signal(
            self.timer4_irq_signal,
            u64::from(self.registers[TMR4CN0] & (TMR4_TF4L | TMR4_TF4H) != 0),
            1,
            at,
        );
        self.set_signal(
            self.timer5_irq_signal,
            u64::from(self.registers[TMR5CN0] & (TMR5_TF5L | TMR5_TF5H) != 0),
            1,
            at,
        );
        self.set_signal(
            self.adc_eoc_signal,
            u64::from(self.registers[ADC0CN0] & ADC0_ADINT != 0),
            1,
            at,
        );
        self.set_signal(
            self.adc_window_signal,
            u64::from(self.registers[ADC0CN0] & ADC0_ADWINT != 0),
            1,
            at,
        );
        self.set_signal(
            self.interrupt_signal,
            u64::from(self.interrupt_levels().iter().any(|level| *level)),
            1,
            at,
        );
        self.set_signal(
            self.pca_interrupt_signal,
            u64::from(self.pca_interrupt_pending()),
            1,
            at,
        );
    }
}
