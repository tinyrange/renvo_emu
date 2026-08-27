impl AtmegaIoHandle {
    /// Captured USART0 bytes.
    pub fn uart_bytes(&self) -> Vec<u8> {
        self.0
            .lock()
            .expect("ATmega I/O lock poisoned")
            .uart
            .clone()
    }

    /// Captured USART1 bytes.
    pub fn uart1_bytes(&self) -> Vec<u8> {
        self.0
            .lock()
            .expect("ATmega I/O lock poisoned")
            .uart1
            .clone()
    }

    /// Sets the functional analog-comparator input levels.
    pub fn set_comparator_inputs(&self, positive: bool, negative: bool, at: SimTime) {
        let mut state = self.0.lock().expect("ATmega I/O lock poisoned");
        state.comparator_positive = positive;
        state.comparator_negative = negative;
        state.update_comparator(at);
    }

    /// Returns the deterministic CPU/peripheral tick divisor selected by CLKPR.
    pub fn clock_divider(&self) -> u64 {
        let state = self.0.lock().expect("ATmega I/O lock poisoned");
        1_u64 << u32::from(state.registers[usize::from(CLKPR - IO_BASE)] & CLKPR_DIVIDER_MASK)
    }

    /// Whether the next SLEEP instruction is permitted to enter sleep.
    pub fn sleep_enabled(&self) -> bool {
        self.0.lock().expect("ATmega I/O lock poisoned").registers[usize::from(SMCR - IO_BASE)] & 1
            != 0
    }

    /// Current documented sleep-mode selection (`SM[2:0]`).
    pub fn sleep_mode(&self) -> u8 {
        (self.0.lock().expect("ATmega I/O lock poisoned").registers[usize::from(SMCR - IO_BASE)]
            >> 1)
            & 0x07
    }

    /// Advances timers, edge detection and watchdog; returns asserted AVR interrupt lines.
    pub fn poll(&self, now: SimTime) -> Vec<u16> {
        let mut state = self.0.lock().expect("ATmega I/O lock poisoned");
        let mut lines = Vec::new();
        if let Some(conversion) = state.adc_conversion
            && now.ticks().saturating_sub(conversion.started) >= conversion.duration
        {
            let sample = adc_sample(&state, conversion.mux);
            let adcl = usize::from(ADCL - IO_BASE);
            let adch = usize::from(ADCH - IO_BASE);
            if !state.adc_result_locked {
                if state.registers[usize::from(ADMUX - IO_BASE)] & ADLAR != 0 {
                    state.registers[adch] = (sample >> 2) as u8;
                    state.registers[adcl] = ((sample & 3) << 6) as u8;
                } else {
                    state.registers[adcl] = sample as u8;
                    state.registers[adch] = (sample >> 8) as u8;
                }
            }
            let control = usize::from(ADCSRA - IO_BASE);
            state.registers[control] = (state.registers[control] & !ADSC) | ADIF;
            state.adc_conversion = None;
            state.adc_first_conversion = false;
            set_bit_signal(&state, state.adc_irq_signal, true, now);
        }
        if state
            .clock_prescaler_armed_at
            .is_some_and(|armed| now.ticks().saturating_sub(armed) > 4)
        {
            state.clock_prescaler_armed_at = None;
            state.registers[usize::from(CLKPR - IO_BASE)] &= CLKPR_DIVIDER_MASK;
        }
        let prr0 = state.registers[usize::from(PRR0 - IO_BASE)];
        let tccr = state.registers[usize::from(TCCR0B - IO_BASE)];
        if prr0 & PRR0_PRTIM0 == 0 && tccr & 7 != 0 {
            let compare = state.registers[usize::from(OCR0A - IO_BASE)];
            let period = u64::from(if compare == 0 { u8::MAX } else { compare }) + 1;
            if now.ticks().saturating_sub(state.timer_started) >= period {
                state.timer_started = now.ticks();
                state.timer_pending = true;
                state.registers[usize::from(TIFR0 - IO_BASE)] |= 1;
                state.registers[usize::from(TCNT0 - IO_BASE)] = 0;
                set_bit_signal(&state, state.timer0_irq_signal, true, now);
            }
        }
        if state.timer_pending && state.registers[usize::from(TIMSK0 - IO_BASE)] & 1 != 0 {
            lines.push(15);
        }
        let tccr1 = state.registers[usize::from(TCCR1B - IO_BASE)];
        if prr0 & PRR0_PRTIM1 == 0 && tccr1 & 7 != 0 {
            let compare = u16::from(state.registers[usize::from(OCR1AL - IO_BASE)])
                | (u16::from(state.registers[usize::from(OCR1AH - IO_BASE)]) << 8);
            let period = u64::from(if compare == 0 { u16::MAX } else { compare }) + 1;
            if now.ticks().saturating_sub(state.timer1_started) >= period {
                state.timer1_started = now.ticks();
                state.timer1_pending = true;
                state.registers[usize::from(TIFR1 - IO_BASE)] |= 1 << 1;
                state.registers[usize::from(TCNT1L - IO_BASE)] = 0;
                state.registers[usize::from(TCNT1H - IO_BASE)] = 0;
                set_bit_signal(&state, state.timer1_irq_signal, true, now);
            }
        }
        if state.timer1_pending && state.registers[usize::from(TIMSK1 - IO_BASE)] & (1 << 1) != 0 {
            // TIMER1_COMPA is vector 11, represented by CPU interrupt line 10.
            lines.push(10);
        }
        let tccr2 = state.registers[usize::from(TCCR2B - IO_BASE)];
        if tccr2 & 7 != 0 {
            let ctc = state.registers[usize::from(TCCR2A - IO_BASE)] & 3 == 2;
            let compare = state.registers[usize::from(OCR2A - IO_BASE)];
            let period = if ctc && compare != 0 {
                u64::from(compare) + 1
            } else {
                256
            };
            if now.ticks().saturating_sub(state.timer2_started) >= period {
                state.timer2_started = now.ticks();
                state.timer2_pending = true;
                let flag = if ctc { 1 << 1 } else { 1 };
                state.registers[usize::from(TIFR2 - IO_BASE)] |= flag;
                state.registers[usize::from(TCNT2 - IO_BASE)] = 0;
                set_bit_signal(&state, state.timer2_irq_signal, true, now);
            }
        }
        let timer2_flags = state.registers[usize::from(TIFR2 - IO_BASE)];
        let timer2_mask = state.registers[usize::from(TIMSK2 - IO_BASE)];
        if timer2_flags & timer2_mask & (1 << 1) != 0 {
            lines.push(6);
        }
        if timer2_flags & timer2_mask & (1 << 2) != 0 {
            lines.push(7);
        }
        if timer2_flags & timer2_mask & 1 != 0 {
            lines.push(8);
        }
        let tccr3 = state.registers[AtmegaTimerRegister::Tccr3b.index()];
        if tccr3 & 7 != 0 {
            let elapsed = state
                .timer3_base
                .saturating_add(now.ticks().saturating_sub(state.timer3_started));
            let compare = u16::from(state.registers[AtmegaTimerRegister::Ocr3al.index()])
                | (u16::from(state.registers[AtmegaTimerRegister::Ocr3ah.index()]) << 8);
            let compare_period = u64::from(compare).saturating_add(1);
            if elapsed / compare_period > state.timer3_elapsed / compare_period {
                state.timer3_compare_pending = true;
                state.registers[AtmegaTimerRegister::Tifr3.index()] |= 1 << 1;
                set_bit_signal(&state, state.timer3_compare_irq_signal, true, now);
            }
            if elapsed / 0x1_0000 > state.timer3_elapsed / 0x1_0000 {
                state.timer3_overflow_pending = true;
                state.registers[AtmegaTimerRegister::Tifr3.index()] |= 1;
                set_bit_signal(&state, state.timer3_overflow_irq_signal, true, now);
            }
            state.timer3_elapsed = elapsed;
            let counter = elapsed as u16;
            state.registers[AtmegaTimerRegister::Tcnt3l.index()] = counter as u8;
            state.registers[AtmegaTimerRegister::Tcnt3h.index()] = (counter >> 8) as u8;
        }
        if state.timer3_compare_pending
            && state.registers[AtmegaTimerRegister::Timsk3.index()] & (1 << 1) != 0
        {
            // TIMER3_COMPA is vector 33, represented by CPU interrupt line 32.
            lines.push(32);
        }
        if state.timer3_overflow_pending
            && state.registers[AtmegaTimerRegister::Timsk3.index()] & 1 != 0
        {
            // TIMER3_OVF is vector 35, represented by CPU interrupt line 34.
            lines.push(34);
        }
        let tccr4 = state.registers[AtmegaTimerRegister::Tccr4b.index()];
        if tccr4 & 7 != 0 {
            let elapsed = state
                .timer4_base
                .saturating_add(now.ticks().saturating_sub(state.timer4_started));
            let compare = u16::from(state.registers[AtmegaTimerRegister::Ocr4al.index()])
                | (u16::from(state.registers[AtmegaTimerRegister::Ocr4ah.index()]) << 8);
            let compare_period = u64::from(compare).saturating_add(1);
            if elapsed / compare_period > state.timer4_elapsed / compare_period {
                state.timer4_compare_pending = true;
                state.registers[AtmegaTimerRegister::Tifr4.index()] |= 1 << 1;
                set_bit_signal(&state, state.timer4_compare_irq_signal, true, now);
            }
            if elapsed / 0x1_0000 > state.timer4_elapsed / 0x1_0000 {
                state.timer4_overflow_pending = true;
                state.registers[AtmegaTimerRegister::Tifr4.index()] |= 1;
                set_bit_signal(&state, state.timer4_overflow_irq_signal, true, now);
            }
            state.timer4_elapsed = elapsed;
            let counter = elapsed as u16;
            state.registers[AtmegaTimerRegister::Tcnt4l.index()] = counter as u8;
            state.registers[AtmegaTimerRegister::Tcnt4h.index()] = (counter >> 8) as u8;
        }
        if state.timer4_compare_pending
            && state.registers[AtmegaTimerRegister::Timsk4.index()] & (1 << 1) != 0
        {
            // TIMER4_COMPA is vector 42, represented by CPU interrupt line 41.
            lines.push(41);
        }
        if state.timer4_overflow_pending
            && state.registers[AtmegaTimerRegister::Timsk4.index()] & 1 != 0
        {
            // TIMER4_OVF is vector 44, represented by CPU interrupt line 43.
            lines.push(43);
        }
        if prr0 & PRR0_PRUSART0 == 0
            && state.registers[usize::from(UCSR0B - IO_BASE)] & (1 << 5) != 0
        {
            lines.push(18);
        }
        if state.registers[usize::from(SPCR0 - IO_BASE)] & SPCR_SPIE != 0
            && state.registers[usize::from(SPSR0 - IO_BASE)] & SPSR_SPIF != 0
        {
            lines.push(SPI0_INTERRUPT_LINE);
        }
        if state.registers[usize::from(SPCR1 - IO_BASE)] & SPCR_SPIE != 0
            && state.registers[usize::from(SPSR1 - IO_BASE)] & SPSR_SPIF != 0
        {
            lines.push(SPI1_INTERRUPT_LINE);
        }
        let twcr = state.registers[usize::from(TWCR - IO_BASE)];
        if twcr & TWINT != 0 && twcr & TWIE != 0 {
            lines.push(24);
        }
        let twcr1 = state.registers[usize::from(TWCR1 - IO_BASE)];
        if twcr1 & TWINT != 0 && twcr1 & TWIE != 0 {
            lines.push(TWI1_INTERRUPT_LINE);
        }
        state.update_comparator(now);
        if state.registers[comparator_index()] & ACSR_ACI != 0
            && state.registers[comparator_index()] & ACSR_ACIE != 0
        {
            // ANALOG_COMP is vector 23, represented by AVR line 22.
            lines.push(22);
        }
        let pinb = resolved(&state.ports[0]);
        let changed = pinb ^ state.previous_pinb;
        state.previous_pinb = pinb;
        if changed & state.registers[usize::from(PCMSK0 - IO_BASE)] != 0
            && state.registers[usize::from(PCICR - IO_BASE)] & 1 != 0
        {
            state.registers[usize::from(PCIFR - IO_BASE)] |= 1;
            set_bit_signal(&state, state.pcint0_irq_signal, true, now);
            lines.push(2);
        }
        let pinc = resolved(&state.ports[1]);
        let changed = pinc ^ state.previous_pinc;
        state.previous_pinc = pinc;
        if changed & state.registers[usize::from(PCMSK1 - IO_BASE)] != 0
            && state.registers[usize::from(PCICR - IO_BASE)] & (1 << 1) != 0
        {
            state.registers[usize::from(PCIFR - IO_BASE)] |= 1 << 1;
            set_bit_signal(&state, state.pcint1_irq_signal, true, now);
            lines.push(3);
        }
        let pind = resolved(&state.ports[2]);
        let changed = pind ^ state.previous_pind;
        if changed & state.registers[usize::from(PCMSK2 - IO_BASE)] != 0
            && state.registers[usize::from(PCICR - IO_BASE)] & (1 << 2) != 0
        {
            state.registers[usize::from(PCIFR - IO_BASE)] |= 1 << 2;
            set_bit_signal(&state, state.pcint2_irq_signal, true, now);
            lines.push(4);
        }
        let old_int0 = state.previous_pind & (1 << 2) != 0;
        let new_int0 = pind & (1 << 2) != 0;
        let old_int1 = state.previous_pind & (1 << 3) != 0;
        let new_int1 = pind & (1 << 3) != 0;
        state.previous_pind = pind;
        let sense = state.registers[usize::from(EICRA - IO_BASE)] & 3;
        let int0_event = match sense {
            0 => !new_int0,
            1 => old_int0 != new_int0,
            2 => old_int0 && !new_int0,
            _ => !old_int0 && new_int0,
        };
        if int0_event && state.registers[usize::from(EIMSK - IO_BASE)] & 1 != 0 {
            state.registers[usize::from(EIFR - IO_BASE)] |= 1;
            set_bit_signal(&state, state.int0_irq_signal, true, now);
            lines.push(0);
        }
        let sense = (state.registers[usize::from(EICRA - IO_BASE)] >> 4) & 3;
        let int1_event = match sense {
            0 => !new_int1,
            1 => old_int1 != new_int1,
            2 => old_int1 && !new_int1,
            _ => !old_int1 && new_int1,
        };
        if int1_event && state.registers[usize::from(EIMSK - IO_BASE)] & (1 << 1) != 0 {
            state.registers[usize::from(EIFR - IO_BASE)] |= 1 << 1;
            set_bit_signal(&state, state.int1_irq_signal, true, now);
            lines.push(1);
        }
        if state.registers[usize::from(WDTCSR - IO_BASE)] & (1 << 3) != 0
            && now.ticks().saturating_sub(state.watchdog_started) >= 2048
        {
            state.watchdog_reset = true;
            set_bit_signal(&state, state.watchdog_reset_signal, true, now);
        }
        if state.registers[usize::from(ADCSRA - IO_BASE)] & (ADIF | ADIE) == (ADIF | ADIE) {
            // CPU line 20 is vector number 22 (program address 0x002a).
            lines.push(ADC_INTERRUPT_LINE);
            set_bit_signal(&state, state.adc_irq_signal, true, now);
        }
        lines
    }

    /// Consumes a watchdog reset request.
    pub fn take_watchdog_reset(&self) -> bool {
        std::mem::take(
            &mut self
                .0
                .lock()
                .expect("ATmega I/O lock poisoned")
                .watchdog_reset,
        )
    }

    /// Supplies the next byte returned by a functional SPI0 master transfer.
    pub fn inject_spi_rx(&self, value: u8) {
        self.0
            .lock()
            .expect("ATmega I/O lock poisoned")
            .spi_rx
            .push(value);
    }

    /// Captured bytes written to the SPI0 data register.
    pub fn spi_bytes(&self) -> Vec<u8> {
        self.0
            .lock()
            .expect("ATmega I/O lock poisoned")
            .spi_tx
            .clone()
    }

    /// Supplies the next byte returned by a functional SPI1 master transfer.
    pub fn inject_spi1_rx(&self, value: u8) {
        self.0
            .lock()
            .expect("ATmega I/O lock poisoned")
            .spi1_rx
            .push(value);
    }

    /// Captured bytes written to the SPI1 data register.
    pub fn spi1_bytes(&self) -> Vec<u8> {
        self.0
            .lock()
            .expect("ATmega I/O lock poisoned")
            .spi1_tx
            .clone()
    }

    /// Queues one byte that the TWI controller should receive from its host.
    pub fn queue_twi_rx(&self, byte: u8) {
        self.0
            .lock()
            .expect("ATmega I/O lock poisoned")
            .twi_rx
            .push_back(byte);
    }

    /// Returns bytes transferred by the functional TWI0 controller.
    pub fn take_twi_tx(&self) -> Vec<u8> {
        let mut state = self.0.lock().expect("ATmega I/O lock poisoned");
        std::mem::take(&mut state.twi_tx)
    }

    /// Queues one byte that the TWI1 controller should receive from its host.
    pub fn queue_twi1_rx(&self, byte: u8) {
        self.0
            .lock()
            .expect("ATmega I/O lock poisoned")
            .twi1_rx
            .push_back(byte);
    }

    /// Returns bytes transferred by the functional TWI1 controller.
    pub fn take_twi1_tx(&self) -> Vec<u8> {
        let mut state = self.0.lock().expect("ATmega I/O lock poisoned");
        std::mem::take(&mut state.twi1_tx)
    }

    /// Sets the deterministic 10-bit analog sample for one ADC channel.
    pub fn set_adc_input(&self, channel: u8, value: u16) {
        if let Some(sample) = self
            .0
            .lock()
            .expect("ATmega I/O lock poisoned")
            .adc_inputs
            .get_mut(usize::from(channel))
        {
            *sample = value & 0x03ff;
        }
    }

    /// Applies the native ADC side effect of entering the conversion-complete
    /// interrupt vector: hardware clears ADIF during vectoring.
    pub fn acknowledge_adc_interrupt(&self, at: SimTime) {
        let mut state = self.0.lock().expect("ATmega I/O lock poisoned");
        state.registers[usize::from(ADCSRA - IO_BASE)] &= !ADIF;
        set_bit_signal(&state, state.adc_irq_signal, false, at);
    }

    /// Returns the most recently completed ADC result in right-adjusted form.
    pub fn adc_value(&self) -> u16 {
        let state = self.0.lock().expect("ATmega I/O lock poisoned");
        let low = state.registers[usize::from(ADCL - IO_BASE)];
        let high = state.registers[usize::from(ADCH - IO_BASE)];
        if state.registers[usize::from(ADMUX - IO_BASE)] & ADLAR != 0 {
            (u16::from(high) << 2) | u16::from(low >> 6)
        } else {
            u16::from(low) | (u16::from(high) << 8)
        }
    }
}

fn adc_sample(state: &AtmegaState, mux: u8) -> u16 {
    match mux & 0x0f {
        channel @ 0..=7 => state.adc_inputs[usize::from(channel)],
        // Temperature sensor, bandgap, and GND are not electrical models in
        // this functional slice. Keep them deterministic and do not alias
        // them to an external ADC channel.
        8 | 14 | 15 => 0,
        _ => 0,
    }
}

fn adc_prescaler(control: u8) -> u64 {
    match control & ADPS_MASK {
        0 | 1 => 2,
        2 => 4,
        3 => 8,
        4 => 16,
        5 => 32,
        6 => 64,
        _ => 128,
    }
}
