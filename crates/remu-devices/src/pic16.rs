use super::{GpioHandle, GpioState, SignalHub, refresh_gpio, vendor_gpio};
use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use remu_signals::{Logic, SignalId, SignalValue};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

include!("pic16/registers.rs");

/// A functional MSSP1 I²C host transaction observed by the emulator.
///
/// Addresses are represented as 7-bit addresses. The model deliberately
/// reports byte-level transactions rather than SCL edges; it is intended for
/// deterministic firmware tests, not electrical or cycle-accurate simulation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pic16I2cEvent {
    /// A normal bus START condition.
    Start,
    /// A repeated START condition without releasing the bus.
    RepeatedStart,
    /// A byte transmitted after the address byte.
    Write {
        /// Seven-bit slave address.
        address: u8,
        /// Transmitted data byte.
        value: u8,
    },
    /// A byte returned by the queued slave response.
    Read {
        /// Seven-bit slave address.
        address: u8,
        /// Received data byte.
        value: u8,
    },
    /// An acknowledge or not-acknowledge bit emitted after a host read.
    Ack {
        /// `true` for ACK (`ACKDT = 0`), `false` for NACK (`ACKDT = 1`).
        acknowledge: bool,
    },
    /// A normal bus STOP condition.
    Stop,
}

struct Pic16State {
    registers: Vec<u8>,
    ports: [Arc<Mutex<GpioState>>; 5],
    port_signals: [Vec<SignalId>; 5],
    hub: SignalHub,
    uart: Vec<u8>,
    spi: Vec<u8>,
    spi_incoming: VecDeque<u8>,
    i2c_events: Vec<Pic16I2cEvent>,
    i2c_responses: BTreeMap<u8, VecDeque<u8>>,
    i2c_acknowledgements: BTreeMap<u8, bool>,
    i2c_address: Option<u8>,
    i2c_read: bool,
    i2c_byte_signal: SignalId,
    i2c_strobe_signal: SignalId,
    timer0_epoch: u64,
    timer1_epoch: u64,
    timer2_epoch: u64,
    timer2_postscale: u8,
    nco_epoch: u64,
    nco_increment_active: u32,
    nco_increment_pending: bool,
    nco_raw_output: bool,
    nco_pulse_remaining: u64,
    watchdog_epoch: u64,
    clock_reference_epoch: u64,
    watchdog_reset: bool,
    adc_inputs: [u16; 64],
    adc_started: Option<(u8, u64)>,
    uart_byte_signal: SignalId,
    uart_strobe_signal: SignalId,
    spi_byte_signal: SignalId,
    spi_strobe_signal: SignalId,
    spi_irq_signal: SignalId,
    timer0_irq_signal: SignalId,
    timer1_irq_signal: SignalId,
    timer2_irq_signal: SignalId,
    nco1_output_signal: SignalId,
    dac1_value_signal: SignalId,
    dac1_active_signal: SignalId,
    comparator1_output_signal: SignalId,
    interrupt_signal: SignalId,
    watchdog_reset_signal: SignalId,
    clock_reference_signal: SignalId,
}

impl Pic16State {
    fn set_signal(&self, signal: SignalId, value: u64, width: u16, at: SimTime) {
        self.hub
            .set(
                signal,
                SignalValue::from_u64(value, width).expect("fixed PIC16 signal width is valid"),
                at,
            )
            .expect("PIC16 signal identity is fixed at construction");
    }

    /// Publishes the functional CLKR waveform.
    ///
    /// The emulator timeline is an abstract instruction/action tick, so the
    /// oscillator sources use deterministic relative periods rather than a
    /// claimed silicon frequency. NCO/CLC sources remain low until their
    /// coupling to CLKR is modelled.
    fn refresh_clock_reference(&self, at: SimTime) {
        let control = self.registers[CLKRCON];
        let source = self.registers[CLKRCLK] & CLKRCLK_WRITABLE_MASK;
        let output = if control & CLKRCON_ENABLE == 0 {
            false
        } else if let Some(source_period) = match source {
            0 | 1 => Some(1_u64),
            2 => Some(512_u64),
            3 => Some(32_u64),
            4 => Some(512_u64),
            5 => Some(1024_u64),
            6..=10 => None,
            _ => None,
        } {
            let divider = 1_u64 << u32::from(control & 0x07);
            let period = 4_u64
                .saturating_mul(source_period)
                .saturating_mul(divider)
                .max(1);
            let duty = u64::from((control >> 3) & 0x03);
            let high_ticks = period.saturating_mul(duty) / 4;
            let phase = at.ticks().saturating_sub(self.clock_reference_epoch) % period;
            phase < high_ticks
        } else {
            false
        };
        self.set_signal(self.clock_reference_signal, u64::from(output), 1, at);
    }

    fn resolved_port(&self, port: usize) -> u8 {
        self.ports[port]
            .lock()
            .expect("PIC16 GPIO lock poisoned")
            .nets
            .iter()
            .enumerate()
            .fold(0_u8, |value, (pin, net)| {
                value | (u8::from(net.resolved() == Logic::One) << pin)
            })
            & PORT_MASKS[port]
    }

    fn nco_accumulator(&self) -> u32 {
        u32::from(self.registers[Pic16NcoRegister::Nco1Accl.offset()])
            | (u32::from(self.registers[Pic16NcoRegister::Nco1Acch.offset()]) << 8)
            | (u32::from(self.registers[Pic16NcoRegister::Nco1Accu.offset()] & 0x0f) << 16)
    }

    fn nco_increment(&self) -> u32 {
        self.nco_increment_active
    }

    fn nco_enabled(&self) -> bool {
        self.registers[Pic16NcoRegister::Nco1Con.offset()] & NCO1EN != 0
    }

    fn nco_output(&self) -> bool {
        self.registers[Pic16NcoRegister::Nco1Con.offset()] & NCO1OUT != 0
    }

    fn nco_pulse_width(&self) -> u64 {
        1_u64 << u32::from((self.registers[Pic16NcoRegister::Nco1Clk.offset()] >> 5) & 0x07)
    }

    fn publish_nco_output(&mut self, at: SimTime) {
        let control = Pic16NcoRegister::Nco1Con.offset();
        let visible =
            self.nco_enabled() && (self.nco_raw_output ^ (self.registers[control] & NCO1POL != 0));
        self.registers[control] =
            (self.registers[control] & !NCO1OUT) | (u8::from(visible) * NCO1OUT);
        self.set_signal(self.nco1_output_signal, u64::from(visible), 1, at);
    }

    fn update_nco(&mut self, now: SimTime) {
        let control = Pic16NcoRegister::Nco1Con.offset();
        if self.nco_increment_pending {
            self.nco_increment_active = self.nco_increment_registers();
            self.nco_increment_pending = false;
        }
        let elapsed = now.ticks().saturating_sub(self.nco_epoch);
        self.nco_epoch = now.ticks();
        if !self.nco_enabled() {
            self.nco_raw_output = false;
            self.nco_pulse_remaining = 0;
            self.publish_nco_output(now);
            return;
        }
        if elapsed == 0 {
            self.publish_nco_output(now);
            return;
        }
        let increment = u64::from(self.nco_increment());
        let total = u64::from(self.nco_accumulator()) + increment.saturating_mul(elapsed);
        let overflows = total >> 20;
        let accumulator = (total as u32) & NCO_ACC_MASK;
        self.registers[Pic16NcoRegister::Nco1Accl.offset()] = accumulator as u8;
        self.registers[Pic16NcoRegister::Nco1Acch.offset()] = (accumulator >> 8) as u8;
        self.registers[Pic16NcoRegister::Nco1Accu.offset()] = (accumulator >> 16) as u8 & 0x0f;
        if overflows != 0 {
            self.registers[Pic16NcoRegister::Pir7.offset()] |= NCO1IF;
            if self.registers[control] & NCO1PFM == 0 {
                if overflows & 1 != 0 {
                    self.nco_raw_output = !self.nco_raw_output;
                }
            } else {
                self.nco_pulse_remaining = self
                    .nco_pulse_remaining
                    .saturating_sub(elapsed)
                    .max(self.nco_pulse_width());
            }
        } else if self.registers[control] & NCO1PFM != 0 {
            self.nco_pulse_remaining = self.nco_pulse_remaining.saturating_sub(elapsed);
        }
        if self.registers[control] & NCO1PFM != 0 {
            self.nco_raw_output = self.nco_pulse_remaining != 0;
        }
        self.publish_nco_output(now);
    }

    fn nco_increment_registers(&self) -> u32 {
        u32::from(self.registers[Pic16NcoRegister::Nco1Incl.offset()])
            | (u32::from(self.registers[Pic16NcoRegister::Nco1Inch.offset()]) << 8)
            | (u32::from(self.registers[Pic16NcoRegister::Nco1Incu.offset()] & 0x0f) << 16)
    }

    fn update_dac_signals(&self, at: SimTime) {
        let enabled = self.registers[Pic16DacRegister::Dac1Con0.index()] & DAC1EN != 0;
        let code = if enabled {
            self.registers[Pic16DacRegister::Dac1Con1.index()] & DAC1R_MASK
        } else {
            0
        };
        self.set_signal(self.dac1_value_signal, u64::from(code), 5, at);
        self.set_signal(self.dac1_active_signal, u64::from(enabled), 1, at);
    }

    fn comparator_pin(&self, channel: u8, positive: bool) -> Option<Logic> {
        let (port, pin) = if positive {
            match channel {
                0 => (0, 2), // C1IN0+
                1 => (0, 3), // C1IN1+
                _ => return None,
            }
        } else {
            match channel {
                0 => (0, 0), // C1IN0-
                1 => (0, 1), // C1IN1-
                2 => (3, 3), // C1IN2- on RB3
                3 => (1, 1), // C1IN3- on RB1
                _ => return None,
            }
        };
        Some(
            self.ports[port]
                .lock()
                .expect("PIC16 GPIO lock poisoned")
                .nets[pin]
                .resolved(),
        )
    }

    fn comparator_input(&self, channel: u8, positive: bool) -> Logic {
        match channel {
            5 if positive => Logic::Zero, // DAC output is not part of this slice.
            6 => Logic::One,              // FVR buffer 2 is a deterministic high reference.
            7 => Logic::Zero,             // AVSS.
            _ => self
                .comparator_pin(channel, positive)
                .unwrap_or(Logic::Zero),
        }
    }

    fn update_comparator(&mut self, at: SimTime) {
        let enabled = self.registers[Pic16ComparatorRegister::Cm1Con0.index()] & C1ON != 0;
        let previous = self.registers[Pic16ComparatorRegister::Cm1Con0.index()] & CM1CON0_OUT != 0;
        let positive = self.comparator_input(
            self.registers[Pic16ComparatorRegister::Cm1Pch.index()] & CM1_CHANNEL_MASK,
            true,
        ) == Logic::One;
        let negative = self.comparator_input(
            self.registers[Pic16ComparatorRegister::Cm1Nch.index()] & CM1_CHANNEL_MASK,
            false,
        ) == Logic::One;
        let raw_output = enabled && (positive != negative) && positive;
        let output =
            if enabled && self.registers[Pic16ComparatorRegister::Cm1Con0.index()] & C1POL != 0 {
                !raw_output
            } else {
                raw_output
            };
        let cm1con0 = Pic16ComparatorRegister::Cm1Con0.index();
        self.registers[cm1con0] =
            (self.registers[cm1con0] & !CM1CON0_OUT) | (u8::from(output) * CM1CON0_OUT);
        let cmout = Pic16ComparatorRegister::Cmout.index();
        self.registers[cmout] =
            (self.registers[cmout] & !CMOUT_C1OUT) | (u8::from(output) * CMOUT_C1OUT);
        // C1IF is edge-triggered even when a transition is caused by changing
        // C1ON or C1POL; the data sheet explicitly calls out those cases.
        if output != previous {
            let edge_enable = if output {
                self.registers[Pic16ComparatorRegister::Cm1Con1.index()] & (1 << 1) != 0
            } else {
                self.registers[Pic16ComparatorRegister::Cm1Con1.index()] & 1 != 0
            };
            if edge_enable {
                self.registers[Pic16ComparatorRegister::Pir2.index()] |= C1IF;
            }
        }
        self.set_signal(self.comparator1_output_signal, u64::from(output), 1, at);
    }

    fn signal_level(&self, signal: SignalId) -> Logic {
        self.hub.with_registry(|registry| {
            registry
                .value(signal)
                .and_then(|value| value.bit(0))
                .unwrap_or(Logic::Zero)
        })
    }

    fn pps_output_level(&self, source: u8) -> Logic {
        match source {
            0 => Logic::Zero,
            PPS_OUTPUT_TX1 => self.signal_level(self.uart_strobe_signal),
            PPS_OUTPUT_TMR0 => self.signal_level(self.timer0_irq_signal),
            _ => Logic::Zero,
        }
    }

    fn refresh_port(&mut self, port: usize, at: SimTime) -> Result<(), DeviceError> {
        let direction = (!self.registers[TRIS_BASE + port]) & PORT_MASKS[port];
        let latch = self.registers[LAT_BASE + port] & PORT_MASKS[port];
        let mut output = latch;
        for pin in 0..usize::from(PORT_WIDTHS[port]) {
            let register = Pic16PpsRegister::output(port, pin).expect("PIC16 PPS pin is mapped");
            let source = self.registers[register.offset()] & PPS_OUTPUT_MASK;
            if source != 0 {
                output = (output & !(1 << pin))
                    | (u8::from(self.pps_output_level(source) == Logic::One) << pin);
            }
        }
        {
            let mut gpio = self.ports[port].lock().expect("PIC16 GPIO lock poisoned");
            gpio.direction = u32::from(direction);
            gpio.output = u32::from(output);
        }
        refresh_gpio(
            &self.ports[port],
            &self.port_signals[port],
            &self.hub,
            PORT_WIDTHS[port],
            at,
        )?;
        let digital = !self.registers[ANSEL[port]];
        self.registers[PORT_BASE + port] = self.resolved_port(port) & digital & PORT_MASKS[port];
        Ok(())
    }

    fn i2c_master_enabled(&self) -> bool {
        self.registers[SSP1CON1] & SSP1CON1_SSPEN != 0
            && matches!(
                self.registers[SSP1CON1] & 0x0f,
                SSP1_I2C_MASTER_7BIT | SSP1_I2C_MASTER_10BIT
            )
    }

    fn emit_i2c_byte(&mut self, value: u8, at: SimTime) {
        self.set_signal(self.i2c_byte_signal, u64::from(value), 8, at);
        let previous = self.hub.with_registry(|registry| {
            registry
                .value(self.i2c_strobe_signal)
                .and_then(|signal| signal.bit(0))
                .map_or(0, |logic| u64::from(logic == Logic::One))
        });
        self.set_signal(self.i2c_strobe_signal, previous ^ 1, 1, at);
    }

    fn i2c_command(&mut self, value: u8, at: SimTime) {
        const COMMANDS: u8 =
            SSP1CON2_SEN | SSP1CON2_RSEN | SSP1CON2_PEN | SSP1CON2_RCEN | SSP1CON2_ACKEN;
        let commands = value & COMMANDS;
        // ACKSTAT is hardware-owned. Firmware may clear it, but a write must
        // not manufacture a NACK that was never observed on the bus.
        self.registers[SSP1CON2] =
            (value & !SSP1CON2_ACKSTAT) | (self.registers[SSP1CON2] & SSP1CON2_ACKSTAT);
        if !self.i2c_master_enabled() {
            self.registers[SSP1CON2] &= !COMMANDS;
            return;
        }

        // The hardware has no event queue: setting more than one command bit
        // while an operation is being requested is a collision rather than a
        // sequence of operations.
        if commands.count_ones() > 1 {
            self.registers[SSP1CON1] |= SSP1CON1_WCOL;
            self.registers[SSP1CON2] &= !COMMANDS;
            return;
        }

        if self.registers[SSP1STAT] & SSP1STAT_BF != 0 && commands != SSP1CON2_RCEN {
            self.registers[SSP1CON1] |= SSP1CON1_WCOL;
            self.registers[SSP1CON2] &= !COMMANDS;
            return;
        }

        match commands {
            SSP1CON2_SEN => {
                self.registers[SSP1STAT] |= SSP1STAT_S;
                self.registers[SSP1STAT] &= !SSP1STAT_P;
                self.i2c_address = None;
                self.i2c_read = false;
                self.i2c_events.push(Pic16I2cEvent::Start);
                self.registers[PIR3] |= SSP1IF;
            }
            SSP1CON2_RSEN => {
                self.registers[SSP1STAT] |= SSP1STAT_S;
                self.registers[SSP1STAT] &= !SSP1STAT_P;
                self.i2c_address = None;
                self.i2c_read = false;
                self.i2c_events.push(Pic16I2cEvent::RepeatedStart);
                self.registers[PIR3] |= SSP1IF;
            }
            SSP1CON2_PEN => {
                self.registers[SSP1STAT] |= SSP1STAT_P;
                self.registers[SSP1STAT] &= !SSP1STAT_S;
                self.i2c_address = None;
                self.i2c_read = false;
                self.i2c_events.push(Pic16I2cEvent::Stop);
                self.registers[PIR3] |= SSP1IF;
            }
            SSP1CON2_RCEN => {
                if let (Some(address), true) = (self.i2c_address, self.i2c_read) {
                    if self.registers[SSP1STAT] & SSP1STAT_BF != 0
                        && self.registers[SSP1CON3] & SSP1CON3_BOEN == 0
                    {
                        self.registers[SSP1CON1] |= 1 << 6;
                    } else {
                        let value = self
                            .i2c_responses
                            .get_mut(&address)
                            .and_then(VecDeque::pop_front)
                            .unwrap_or(0xff);
                        self.registers[SSP1BUF] = value;
                        self.registers[SSP1STAT] |= SSP1STAT_BF;
                        self.registers[SSP1STAT] &= !(SSP1STAT_DA | SSP1STAT_RW);
                        self.emit_i2c_byte(value, at);
                        self.i2c_events.push(Pic16I2cEvent::Read { address, value });
                        self.registers[PIR3] |= SSP1IF;
                    }
                } else {
                    self.registers[SSP1CON1] |= SSP1CON1_WCOL;
                }
            }
            SSP1CON2_ACKEN => {
                let acknowledge = self.registers[SSP1CON2] & SSP1CON2_ACKDT == 0;
                self.i2c_events.push(Pic16I2cEvent::Ack { acknowledge });
                self.registers[PIR3] |= SSP1IF;
                if !acknowledge {
                    self.i2c_address = None;
                    self.i2c_read = false;
                }
            }
            0 => {}
            _ => unreachable!("MSSP command mask is one-hot"),
        }

        // SEN/RSEN/PEN/RCEN/ACKEN are command strobes. Firmware waits for
        // SSP1IF and observes these bits cleared by the peripheral.
        self.registers[SSP1CON2] &= !COMMANDS;
    }

    fn i2c_acknowledged(&self, address: u8) -> bool {
        self.i2c_acknowledgements
            .get(&address)
            .copied()
            .unwrap_or(true)
    }

    fn set_i2c_ackstat(&mut self, acknowledged: bool) {
        if acknowledged {
            self.registers[SSP1CON2] &= !SSP1CON2_ACKSTAT;
        } else {
            self.registers[SSP1CON2] |= SSP1CON2_ACKSTAT;
        }
    }

    fn i2c_buffer_write(&mut self, value: u8, at: SimTime) {
        if !self.i2c_master_enabled() {
            self.registers[SSP1BUF] = value;
            return;
        }

        if self.registers[SSP1STAT] & SSP1STAT_BF != 0 {
            self.registers[SSP1CON1] |= SSP1CON1_WCOL;
            return;
        }

        if self.i2c_address.is_none() {
            if self.registers[SSP1CON1] & 0x0f == SSP1_I2C_MASTER_10BIT {
                // The functional host slice intentionally accepts only the
                // common 7-bit address form; preserve the documented WCOL
                // diagnostic for a 10-bit transaction.
                self.registers[SSP1CON1] |= SSP1CON1_WCOL;
                return;
            }
            self.i2c_address = Some(value >> 1);
            self.i2c_read = value & 1 != 0;
            self.registers[SSP1BUF] = value;
            self.registers[SSP1STAT] |= SSP1STAT_BF | SSP1STAT_RW;
            self.emit_i2c_byte(value, at);
            let acknowledged = self.i2c_acknowledged(value >> 1);
            self.set_i2c_ackstat(acknowledged);
            self.registers[SSP1STAT] &= !(SSP1STAT_BF | SSP1STAT_DA | SSP1STAT_RW);
            self.registers[PIR3] |= SSP1IF;
            return;
        }

        if self.i2c_read {
            self.registers[SSP1CON1] |= SSP1CON1_WCOL;
            return;
        }
        let address = self.i2c_address.expect("I²C address was checked above");
        self.registers[SSP1BUF] = value;
        self.registers[SSP1STAT] |= SSP1STAT_BF | SSP1STAT_RW;
        self.emit_i2c_byte(value, at);
        self.i2c_events
            .push(Pic16I2cEvent::Write { address, value });
        let acknowledged = self.i2c_acknowledged(address);
        self.set_i2c_ackstat(acknowledged);
        self.registers[SSP1STAT] &= !(SSP1STAT_BF | SSP1STAT_DA | SSP1STAT_RW);
        self.registers[PIR3] |= SSP1IF;
    }

    fn reset_registers(&mut self, at: SimTime) {
        self.registers.fill(0);
        for port in 0..5 {
            self.registers[TRIS_BASE + port] = PORT_MASKS[port];
            self.registers[ANSEL[port]] = PORT_MASKS[port];
        }
        self.registers[SSP1ADD] = 0;
        self.registers[SSP1MSK] = 0xff;
        self.registers[SSP1CON2] = 0;
        self.registers[SSP1CON3] = 0;
        // NCO1INCL's bit zero powers up set on the PIC16F15376.
        self.registers[Pic16NcoRegister::Nco1Incl.offset()] = 1;
        self.registers[PIR3] = TX1IF;
        self.registers[Pic16Timer2Register::T2Pr.index()] = u8::MAX;
        self.registers[TX1STA] = 1 << 1; // TRMT
        self.registers[OSCSTAT] = 1 << 6; // internal HF oscillator ready
        // CLKRDC1 resets high, selecting the documented deterministic 50%
        // duty default while the module remains disabled.
        self.registers[CLKRCON] = 0x08;
        self.registers[Pic16PpsRegister::Ppslock.offset()] = 0;
        self.uart.clear();
        self.spi.clear();
        self.spi_incoming.clear();
        self.i2c_events.clear();
        self.i2c_responses.clear();
        self.i2c_acknowledgements.clear();
        self.i2c_address = None;
        self.i2c_read = false;
        self.timer0_epoch = at.ticks();
        self.timer1_epoch = at.ticks();
        self.timer2_epoch = at.ticks();
        self.timer2_postscale = 0;
        self.nco_epoch = at.ticks();
        self.nco_increment_active = 1;
        self.nco_increment_pending = false;
        self.nco_raw_output = false;
        self.nco_pulse_remaining = 0;
        self.watchdog_epoch = at.ticks();
        self.clock_reference_epoch = at.ticks();
        self.watchdog_reset = false;
        self.adc_inputs = [0; 64];
        self.adc_started = None;
        self.set_signal(self.uart_strobe_signal, 0, 1, at);
        self.set_signal(self.spi_byte_signal, 0, 8, at);
        self.set_signal(self.spi_strobe_signal, 0, 1, at);
        self.set_signal(self.spi_irq_signal, 0, 1, at);
        self.set_signal(self.i2c_byte_signal, 0, 8, at);
        self.set_signal(self.i2c_strobe_signal, 0, 1, at);
        self.set_signal(self.timer0_irq_signal, 0, 1, at);
        self.set_signal(self.timer1_irq_signal, 0, 1, at);
        self.set_signal(self.timer2_irq_signal, 0, 1, at);
        self.update_dac_signals(at);
        self.set_signal(self.comparator1_output_signal, 0, 1, at);
        self.publish_nco_output(at);
        self.set_signal(self.interrupt_signal, 0, 1, at);
        self.set_signal(self.watchdog_reset_signal, 0, 1, at);
        self.set_signal(self.clock_reference_signal, 0, 1, at);
        for port in 0..5 {
            let _ = self.refresh_port(port, at);
        }
    }

    fn interrupt_pending(&self) -> bool {
        let peripheral = self.registers[INTCON] & INTCON_PEIE != 0
            && ((self.registers[PIR0] & self.registers[PIE0] & TMR0IF != 0)
                || (self.registers[Pic16Timer2Register::Pir4.index()]
                    & self.registers[Pic16Timer2Register::Pie4.index()]
                    & TMR1IF
                    != 0)
                || (self.registers[PIR3] & self.registers[PIE3] & (TX1IF | RC1IF) != 0)
                || (self.registers[PIR1] & ADIF != 0 && self.registers[PIE1] & ADIE != 0)
                || (self.registers[Pic16Timer2Register::Pir4.index()]
                    & self.registers[Pic16Timer2Register::Pie4.index()]
                    & TMR2IF
                    != 0)
                || (self.registers[Pic16ComparatorRegister::Pir2.index()]
                    & self.registers[Pic16ComparatorRegister::Pie2.index()]
                    & C1IF
                    != 0)
                || (self.registers[Pic16NcoRegister::Pir7.offset()]
                    & self.registers[Pic16NcoRegister::Pie7.offset()]
                    & NCO1IF
                    != 0)
                || (self.registers[PIR3] & SSP1IF != 0 && self.registers[PIE3] & SSP1IE != 0));
        self.registers[INTCON] & INTCON_GIE != 0 && peripheral
    }

    fn update_interrupt_signals(&self, at: SimTime) {
        self.set_signal(
            self.spi_irq_signal,
            u64::from(self.registers[PIR3] & SSP1IF != 0),
            1,
            at,
        );
        self.set_signal(
            self.interrupt_signal,
            u64::from(self.interrupt_pending()),
            1,
            at,
        );
    }
}

/// Host-facing PIC16F15376 peripheral state.
#[derive(Clone)]
pub struct Pic16PeripheralsHandle(Arc<Mutex<Pic16State>>);

impl Pic16PeripheralsHandle {
    /// Captured EUSART1 transmit bytes.
    pub fn uart_bytes(&self) -> Vec<u8> {
        self.0
            .lock()
            .expect("PIC16 peripheral lock poisoned")
            .uart
            .clone()
    }

    /// Captured MSSP1 MOSI bytes from functional SPI master transfers.
    pub fn spi_bytes(&self) -> Vec<u8> {
        self.0
            .lock()
            .expect("PIC16 peripheral lock poisoned")
            .spi
            .clone()
    }

    /// Queues one MISO byte for the next completed MSSP1 transfer.
    pub fn inject_spi_rx(&self, value: u8, at: SimTime) {
        let mut state = self.0.lock().expect("PIC16 peripheral lock poisoned");
        state.spi_incoming.push_back(value);
        state.update_interrupt_signals(at);
    }

    /// Returns the normalized 5-bit DAC code, or zero while DAC1 is disabled.
    pub fn dac1_code(&self) -> u8 {
        let state = self.0.lock().expect("PIC16 peripheral lock poisoned");
        if state.registers[Pic16DacRegister::Dac1Con0.index()] & DAC1EN != 0 {
            state.registers[Pic16DacRegister::Dac1Con1.index()] & DAC1R_MASK
        } else {
            0
        }
    }

    /// Returns whether DAC1 is enabled.
    pub fn dac1_enabled(&self) -> bool {
        self.0
            .lock()
            .expect("PIC16 peripheral lock poisoned")
            .registers[Pic16DacRegister::Dac1Con0.index()]
            & DAC1EN
            != 0
    }

    /// Returns the current logical C1 comparator output.
    pub fn comparator1_output(&self) -> bool {
        self.0
            .lock()
            .expect("PIC16 peripheral lock poisoned")
            .registers[Pic16ComparatorRegister::Cm1Con0.index()]
            & CM1CON0_OUT
            != 0
    }

    /// Returns the current logical NCO1 output.
    pub fn nco1_output(&self) -> bool {
        let state = self.0.lock().expect("PIC16 peripheral lock poisoned");
        state.nco_output()
    }

    /// Queues deterministic bytes returned by a 7-bit MSSP1 I²C slave.
    ///
    /// The queue is keyed by the 7-bit address used in the address byte. A
    /// missing response returns `0xff`, which keeps firmware runs bounded and
    /// reproducible without pretending to model an electrical bus.
    pub fn queue_i2c_read(&self, address: u8, bytes: impl IntoIterator<Item = u8>) {
        let mut state = self.0.lock().expect("PIC16 peripheral lock poisoned");
        let address = address & 0x7f;
        state.i2c_acknowledgements.insert(address, true);
        state
            .i2c_responses
            .entry(address)
            .or_default()
            .extend(bytes);
    }

    /// Configures whether the deterministic host should observe an ACK for a
    /// seven-bit address. Addresses ACK by default; this hook lets firmware
    /// tests exercise the documented `ACKSTAT` NACK path without electrical
    /// bus timing.
    pub fn set_i2c_ack(&self, address: u8, acknowledge: bool) {
        self.0
            .lock()
            .expect("PIC16 peripheral lock poisoned")
            .i2c_acknowledgements
            .insert(address & 0x7f, acknowledge);
    }

    /// Returns the byte-level MSSP1 I²C host events observed since reset or
    /// [`Self::clear_i2c`].
    pub fn i2c_events(&self) -> Vec<Pic16I2cEvent> {
        self.0
            .lock()
            .expect("PIC16 peripheral lock poisoned")
            .i2c_events
            .clone()
    }

    /// Clears captured I²C events while leaving queued slave responses intact.
    pub fn clear_i2c(&self) {
        self.0
            .lock()
            .expect("PIC16 peripheral lock poisoned")
            .i2c_events
            .clear();
    }

    /// Advances functional timers and returns the combined interrupt request.
    pub fn poll(&self, now: SimTime) -> bool {
        let mut state = self.0.lock().expect("PIC16 peripheral lock poisoned");
        for port in 0..5 {
            let _ = state.refresh_port(port, now);
        }
        state.update_comparator(now);
        state.refresh_clock_reference(now);
        if state.registers[T0CON0] & 0x80 != 0 {
            let period = u64::from(state.registers[TMR0H]).saturating_add(1).max(1);
            let elapsed = now.ticks().saturating_sub(state.timer0_epoch);
            state.registers[TMR0L] = (elapsed % period) as u8;
            if elapsed >= period {
                state.timer0_epoch = now.ticks();
                state.registers[PIR0] |= TMR0IF;
                state.set_signal(state.timer0_irq_signal, 1, 1, now);
            }
        }
        if state.registers[T1CON] & 1 != 0 {
            let initial =
                u16::from(state.registers[TMR1L]) | (u16::from(state.registers[TMR1H]) << 8);
            let elapsed = now.ticks().saturating_sub(state.timer1_epoch);
            let total = u64::from(initial).saturating_add(elapsed);
            let value = total as u16;
            state.registers[TMR1L] = value as u8;
            state.registers[TMR1H] = (value >> 8) as u8;
            state.timer1_epoch = now.ticks();
            if total > u64::from(u16::MAX) {
                state.registers[Pic16Timer2Register::Pir4.index()] |= TMR1IF;
                state.set_signal(state.timer1_irq_signal, 1, 1, now);
            }
        }
        state.update_nco(now);
        if state.registers[Pic16Timer2Register::T2Con.index()] & T2ON != 0 {
            let prescaler = 1_u64
                << u32::from(
                    (state.registers[Pic16Timer2Register::T2Con.index()] & T2CKPS_MASK) >> 4,
                );
            let period = u64::from(state.registers[Pic16Timer2Register::T2Pr.index()])
                .saturating_add(1)
                .max(1);
            let elapsed = now.ticks().saturating_sub(state.timer2_epoch);
            let increments = elapsed / prescaler;
            if increments != 0 {
                let total = u64::from(state.registers[Pic16Timer2Register::T2Tmr.index()])
                    .saturating_add(increments);
                let matches = total / period;
                state.registers[Pic16Timer2Register::T2Tmr.index()] = (total % period) as u8;
                state.timer2_epoch = state
                    .timer2_epoch
                    .saturating_add(increments.saturating_mul(prescaler));
                if matches != 0 {
                    let postscaler = u64::from(
                        state.registers[Pic16Timer2Register::T2Con.index()] & T2OUTPS_MASK,
                    ) + 1;
                    let accumulated = u64::from(state.timer2_postscale) + matches;
                    if accumulated >= postscaler {
                        state.registers[Pic16Timer2Register::Pir4.index()] |= TMR2IF;
                        state.set_signal(state.timer2_irq_signal, 1, 1, now);
                    }
                    state.timer2_postscale = (accumulated % postscaler) as u8;
                }
            }
        }
        if state.registers[WDTCON0] & 1 != 0 {
            let exponent = u32::from((state.registers[WDTCON0] >> 1) & 0x1f).min(20);
            let period = 32_u64.checked_shl(exponent).unwrap_or(u64::MAX);
            if now.ticks().saturating_sub(state.watchdog_epoch) >= period {
                state.watchdog_reset = true;
                state.set_signal(state.watchdog_reset_signal, 1, 1, now);
            }
        }
        if let Some((channel, started)) = state.adc_started {
            if now.ticks() > started {
                let sample = state.adc_inputs[usize::from(channel.min(63))] & 0x03ff;
                if state.registers[ADCON1] & (1 << 7) != 0 {
                    state.registers[ADRESL] = sample as u8;
                    state.registers[ADRESH] = (sample >> 8) as u8;
                } else {
                    state.registers[ADRESH] = (sample >> 2) as u8;
                    state.registers[ADRESL] = ((sample & 0x3) << 6) as u8;
                }
                state.registers[ADCON0] &= !ADCON0_GO;
                state.registers[PIR1] |= ADIF;
                state.adc_started = None;
            }
        }
        for port in 0..5 {
            let _ = state.refresh_port(port, now);
        }
        let pending = state.interrupt_pending();
        state.update_interrupt_signals(now);
        pending
    }

    /// Drives a deterministic 10-bit analog value for one ADC channel.
    pub fn set_adc_input(&self, channel: u8, value: u16) {
        let mut state = self.0.lock().expect("PIC16 peripheral lock poisoned");
        state.adc_inputs[usize::from(channel.min(63))] = value & 0x03ff;
    }

    /// Restarts the functional watchdog interval after CLRWDT.
    pub fn clear_watchdog(&self, now: SimTime) {
        self.0
            .lock()
            .expect("PIC16 peripheral lock poisoned")
            .watchdog_epoch = now.ticks();
    }

    /// Consumes a watchdog reset request.
    pub fn take_watchdog_reset(&self) -> bool {
        std::mem::take(
            &mut self
                .0
                .lock()
                .expect("PIC16 peripheral lock poisoned")
                .watchdog_reset,
        )
    }
}

/// PIC16F15376 banked data and peripheral window.
pub struct Pic16Peripherals {
    name: String,
    state: Arc<Mutex<Pic16State>>,
}

impl Pic16Peripherals {
    /// Creates the documented peripheral slice and five package port handles.
    pub fn new(
        name: impl Into<String>,
        hub: SignalHub,
    ) -> Result<(Self, Pic16PeripheralsHandle, [GpioHandle; 5]), remu_signals::SignalError> {
        let (porta, signals_a, handle_a) = vendor_gpio(8, "board.pic16f15376.porta", &hub)?;
        let (portb, signals_b, handle_b) = vendor_gpio(8, "board.pic16f15376.portb", &hub)?;
        let (portc, signals_c, handle_c) = vendor_gpio(8, "board.pic16f15376.portc", &hub)?;
        let (portd, signals_d, handle_d) = vendor_gpio(8, "board.pic16f15376.portd", &hub)?;
        let (porte, signals_e, handle_e) = vendor_gpio(4, "board.pic16f15376.porte", &hub)?;
        let uart_byte_signal = hub.declare(
            "board.pic16f15376.eusart1.tx_byte",
            SignalValue::from_u64(0, 8)?,
            Some("last byte written to EUSART1 TXREG".to_owned()),
        )?;
        let uart_strobe_signal = hub.declare(
            "board.pic16f15376.eusart1.tx_strobe",
            SignalValue::from_u64(0, 1)?,
            Some("toggles for each EUSART1 byte".to_owned()),
        )?;
        let spi_byte_signal = hub.declare(
            "board.pic16f15376.mssp1.tx_byte",
            SignalValue::from_u64(0, 8)?,
            Some("last byte written to MSSP1 SSPBUF".to_owned()),
        )?;
        let spi_strobe_signal = hub.declare(
            "board.pic16f15376.mssp1.tx_strobe",
            SignalValue::from_u64(0, 1)?,
            Some("toggles for each functional MSSP1 transfer".to_owned()),
        )?;
        let spi_irq_signal = hub.declare(
            "board.pic16f15376.mssp1.irq",
            SignalValue::from_u64(0, 1)?,
            Some("MSSP1 transfer-complete interrupt flag".to_owned()),
        )?;
        let i2c_byte_signal = hub.declare(
            "board.pic16f15376.mssp1.i2c_byte",
            SignalValue::from_u64(0, 8)?,
            Some("last byte observed on the functional MSSP1 I²C host".to_owned()),
        )?;
        let i2c_strobe_signal = hub.declare(
            "board.pic16f15376.mssp1.i2c_strobe",
            SignalValue::from_u64(0, 1)?,
            Some("toggles for each functional MSSP1 I²C byte".to_owned()),
        )?;
        let timer0_irq_signal = hub.declare(
            "board.pic16f15376.timer0.irq",
            SignalValue::from_u64(0, 1)?,
            Some("functional Timer0 interrupt flag".to_owned()),
        )?;
        let timer1_irq_signal = hub.declare(
            "board.pic16f15376.timer1.irq",
            SignalValue::from_u64(0, 1)?,
            Some("functional Timer1 interrupt flag".to_owned()),
        )?;
        let timer2_irq_signal = hub.declare(
            "board.pic16f15376.timer2.irq",
            SignalValue::from_u64(0, 1)?,
            Some("functional Timer2 period-match interrupt flag".to_owned()),
        )?;
        let nco1_output_signal = hub.declare(
            "board.pic16f15376.nco1.output",
            SignalValue::from_u64(0, 1)?,
            Some("functional NCO1 output".to_owned()),
        )?;
        let dac1_value_signal = hub.declare(
            "board.pic16f15376.dac1.value",
            SignalValue::from_u64(0, 5)?,
            Some("normalized 5-bit DAC1 code while enabled".to_owned()),
        )?;
        let dac1_active_signal = hub.declare(
            "board.pic16f15376.dac1.active",
            SignalValue::from_u64(0, 1)?,
            Some("DAC1 enable state".to_owned()),
        )?;
        let comparator1_output_signal = hub.declare(
            "board.pic16f15376.comparator1.output",
            SignalValue::from_u64(0, 1)?,
            Some("functional C1 comparator output".to_owned()),
        )?;
        let interrupt_signal = hub.declare(
            "board.pic16f15376.interrupt.request",
            SignalValue::from_u64(0, 1)?,
            Some("combined enabled peripheral interrupt request".to_owned()),
        )?;
        let watchdog_reset_signal = hub.declare(
            "board.pic16f15376.watchdog.reset",
            SignalValue::from_u64(0, 1)?,
            Some("functional watchdog reset request".to_owned()),
        )?;
        let clock_reference_signal = hub.declare(
            "board.pic16f15376.clkr",
            SignalValue::from_u64(0, 1)?,
            Some("functional PIC16F15376 CLKR reference-clock output".to_owned()),
        )?;
        let state = Arc::new(Mutex::new(Pic16State {
            registers: vec![0; DATA_BYTES],
            ports: [porta, portb, portc, portd, porte],
            port_signals: [signals_a, signals_b, signals_c, signals_d, signals_e],
            hub,
            uart: Vec::new(),
            spi: Vec::new(),
            spi_incoming: VecDeque::new(),
            i2c_events: Vec::new(),
            i2c_responses: BTreeMap::new(),
            i2c_acknowledgements: BTreeMap::new(),
            i2c_address: None,
            i2c_read: false,
            timer0_epoch: 0,
            timer1_epoch: 0,
            timer2_epoch: 0,
            timer2_postscale: 0,
            nco_epoch: 0,
            nco_increment_active: 0,
            nco_increment_pending: false,
            nco_raw_output: false,
            nco_pulse_remaining: 0,
            watchdog_epoch: 0,
            clock_reference_epoch: 0,
            watchdog_reset: false,
            adc_inputs: [0; 64],
            adc_started: None,
            uart_byte_signal,
            uart_strobe_signal,
            spi_byte_signal,
            spi_strobe_signal,
            spi_irq_signal,
            i2c_byte_signal,
            i2c_strobe_signal,
            timer0_irq_signal,
            timer1_irq_signal,
            timer2_irq_signal,
            nco1_output_signal,
            dac1_value_signal,
            dac1_active_signal,
            comparator1_output_signal,
            interrupt_signal,
            watchdog_reset_signal,
            clock_reference_signal,
        }));
        state
            .lock()
            .expect("new PIC16 peripheral lock poisoned")
            .reset_registers(SimTime::ZERO);
        Ok((
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Pic16PeripheralsHandle(state),
            [handle_a, handle_b, handle_c, handle_d, handle_e],
        ))
    }

    fn canonical_offset(offset: usize) -> usize {
        if offset & 0x7f >= 0x70 {
            offset & 0x7f
        } else {
            offset
        }
    }

    fn port_for(address: usize, bases: &[usize]) -> Option<usize> {
        bases
            .iter()
            .position(|base| (*base..*base + 5).contains(&address))
            .or_else(|| {
                if (PORT_BASE..PORT_BASE + 5).contains(&address) {
                    Some(address - PORT_BASE)
                } else if (TRIS_BASE..TRIS_BASE + 5).contains(&address) {
                    Some(address - TRIS_BASE)
                } else if (LAT_BASE..LAT_BASE + 5).contains(&address) {
                    Some(address - LAT_BASE)
                } else {
                    None
                }
            })
    }
}

impl Device for Pic16Peripherals {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Byte {
            return Err(DeviceError::new("PIC16 data space requires byte accesses"));
        }
        let raw = usize::try_from(offset).map_err(|_| DeviceError::new("PIC16 offset overflow"))?;
        let address = Self::canonical_offset(raw);
        let mut state = self.state.lock().expect("PIC16 peripheral lock poisoned");
        if (PORT_BASE..PORT_BASE + 5).contains(&address) {
            state.refresh_port(address - PORT_BASE, at)?;
        }
        if Pic16NcoRegister::from_data_address(address).is_some() {
            state.update_nco(at);
        }
        let value = match address {
            OSCSTAT => state.registers[address] | (1 << 6),
            CLKRCON => state.registers[address] & CLKRCON_WRITABLE_MASK,
            CLKRCLK => state.registers[address] & CLKRCLK_WRITABLE_MASK,
            TX1STA => state.registers[address] | (1 << 1),
            address
                if Pic16PpsRegister::from_data_address(address)
                    == Some(Pic16PpsRegister::Ppslock) =>
            {
                state.registers[address] & PPSLOCKED
            }
            address if Pic16PpsRegister::from_data_address(address).is_some() => {
                state.registers[address] & PPS_OUTPUT_MASK
            }
            RC1REG => {
                state.registers[PIR3] &= !RC1IF;
                state.registers[address]
            }
            SSP1BUF => {
                state.registers[SSP1STAT] &= !SSP1STAT_BF;
                state.registers[address]
            }
            _ => *state.registers.get(address).ok_or_else(|| {
                DeviceError::new(format!("PIC16 read outside data space: {raw:#x}"))
            })?,
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
        if width != AccessWidth::Byte {
            return Err(DeviceError::new("PIC16 data space requires byte accesses"));
        }
        let raw = usize::try_from(offset).map_err(|_| DeviceError::new("PIC16 offset overflow"))?;
        let address = Self::canonical_offset(raw);
        let value = value as u8;
        let mut state = self.state.lock().expect("PIC16 peripheral lock poisoned");
        if !(address < DATA_BYTES) {
            return Err(DeviceError::new(format!(
                "PIC16 write outside data space: {raw:#x}"
            )));
        }
        match address {
            PORT_BASE..=0x010 => {
                let port = address - PORT_BASE;
                state.registers[LAT_BASE + port] = value & PORT_MASKS[port];
                state.refresh_port(port, at)?;
            }
            TRIS_BASE..=0x016 => {
                let port = address - TRIS_BASE;
                state.registers[address] = value & PORT_MASKS[port];
                state.refresh_port(port, at)?;
            }
            LAT_BASE..=0x01c => {
                let port = address - LAT_BASE;
                state.registers[address] = value & PORT_MASKS[port];
                state.refresh_port(port, at)?;
            }
            TX1REG => {
                state.registers[address] = value;
                state.registers[PIR3] |= TX1IF;
                if state.registers[RC1STA] & SPEN != 0 && state.registers[TX1STA] & TXEN != 0 {
                    state.uart.push(value);
                    state.set_signal(state.uart_byte_signal, u64::from(value), 8, at);
                    let previous = state.hub.with_registry(|registry| {
                        registry
                            .value(state.uart_strobe_signal)
                            .and_then(|signal| signal.bit(0))
                            .map_or(0, |logic| u64::from(logic == Logic::One))
                    });
                    state.set_signal(state.uart_strobe_signal, previous ^ 1, 1, at);
                }
            }
            SSP1BUF => {
                if state.i2c_master_enabled() {
                    state.i2c_buffer_write(value, at);
                } else {
                    let enabled = state.registers[SSP1CON1] & SSP1CON1_SSPEN != 0;
                    let master_mode = state.registers[SSP1CON1] & 0x0f <= 0x03;
                    if enabled && master_mode {
                        if state.registers[SSP1STAT] & SSP1STAT_BF != 0 {
                            state.registers[SSP1CON1] |= SSP1CON1_WCOL;
                        } else {
                            let received = state.spi_incoming.pop_front().unwrap_or(value);
                            state.registers[address] = received;
                            state.registers[SSP1STAT] |= SSP1STAT_BF;
                            state.registers[PIR3] |= SSP1IF;
                            state.spi.push(value);
                            state.set_signal(state.spi_byte_signal, u64::from(value), 8, at);
                            let previous = state.hub.with_registry(|registry| {
                                registry
                                    .value(state.spi_strobe_signal)
                                    .and_then(|signal| signal.bit(0))
                                    .map_or(0, |logic| u64::from(logic == Logic::One))
                            });
                            state.set_signal(state.spi_strobe_signal, previous ^ 1, 1, at);
                        }
                    } else {
                        state.registers[address] = value;
                    }
                }
            }
            SSP1CON1 => {
                let was_enabled = state.i2c_master_enabled();
                state.registers[address] = value;
                if was_enabled && !state.i2c_master_enabled() {
                    state.i2c_address = None;
                    state.i2c_read = false;
                    state.registers[SSP1STAT] &=
                        !(SSP1STAT_BF | SSP1STAT_RW | SSP1STAT_S | SSP1STAT_P | SSP1STAT_DA);
                    state.registers[SSP1CON2] &= !(SSP1CON2_SEN
                        | SSP1CON2_RSEN
                        | SSP1CON2_PEN
                        | SSP1CON2_RCEN
                        | SSP1CON2_ACKEN);
                }
            }
            SSP1CON2 => state.i2c_command(value, at),
            PIR0 => {
                state.registers[address] = value;
                state.set_signal(
                    state.timer0_irq_signal,
                    u64::from(value & TMR0IF != 0),
                    1,
                    at,
                );
            }
            address if address == Pic16Timer2Register::Pir4.offset() => {
                state.registers[address] = value;
                state.set_signal(
                    state.timer1_irq_signal,
                    u64::from(value & TMR1IF != 0),
                    1,
                    at,
                );
                state.set_signal(
                    state.timer2_irq_signal,
                    u64::from(value & TMR2IF != 0),
                    1,
                    at,
                );
            }
            SSP1STAT => {
                // BF, R/W, D/A, S and P are maintained by the functional
                // transfer models; SMP and CKE are writable mode bits.
                state.registers[address] = (state.registers[address] & 0x3f) | (value & 0xc0);
            }
            T0CON0 => {
                if state.registers[address] & 0x80 == 0 && value & 0x80 != 0 {
                    state.timer0_epoch = at.ticks();
                }
                state.registers[address] = value;
            }
            T1CON => {
                if state.registers[address] & 1 == 0 && value & 1 != 0 {
                    state.timer1_epoch = at.ticks();
                }
                state.registers[address] = value;
            }
            address if address == Pic16Timer2Register::T2Tmr.offset() => {
                state.registers[address] = value;
                state.timer2_epoch = at.ticks();
                state.timer2_postscale = 0;
            }
            address if address == Pic16Timer2Register::T2Con.offset() => {
                state.registers[address] = value;
                state.timer2_epoch = at.ticks();
                state.timer2_postscale = 0;
            }
            address if address == Pic16DacRegister::Dac1Con0.offset() => {
                state.registers[address] = value & DAC1CON0_MASK;
                state.update_dac_signals(at);
            }
            address if address == Pic16DacRegister::Dac1Con1.offset() => {
                state.registers[address] = value & DAC1R_MASK;
                state.update_dac_signals(at);
            }
            address if address == Pic16ComparatorRegister::Pir2.offset() => {
                state.registers[address] = value & C1IF;
            }
            address if address == Pic16ComparatorRegister::Pie2.offset() => {
                state.registers[address] = value & C1IF;
            }
            address if address == Pic16ComparatorRegister::Cmout.offset() => {
                // CMOUT is a read-only mirror of comparator outputs.
            }
            address if address == Pic16ComparatorRegister::Cm1Con0.offset() => {
                state.registers[address] =
                    (state.registers[address] & CM1CON0_OUT) | (value & CM1CON0_WRITE_MASK);
                state.update_comparator(at);
            }
            address if address == Pic16ComparatorRegister::Cm1Con1.offset() => {
                state.registers[address] = value & CM1CON1_MASK;
                state.update_comparator(at);
            }
            address
                if address == Pic16ComparatorRegister::Cm1Nch.offset()
                    || address == Pic16ComparatorRegister::Cm1Pch.offset() =>
            {
                state.registers[address] = value & CM1_CHANNEL_MASK;
                state.update_comparator(at);
            }
            address if Pic16NcoRegister::from_data_address(address).is_some() => {
                let register = Pic16NcoRegister::from_data_address(address)
                    .expect("NCO register guard returned Some");
                state.update_nco(at);
                match register {
                    Pic16NcoRegister::Nco1Accl
                    | Pic16NcoRegister::Nco1Acch
                    | Pic16NcoRegister::Nco1Accu => {
                        state.registers[address] = if register == Pic16NcoRegister::Nco1Accu {
                            value & 0x0f
                        } else {
                            value
                        };
                    }
                    Pic16NcoRegister::Nco1Incl
                    | Pic16NcoRegister::Nco1Inch
                    | Pic16NcoRegister::Nco1Incu => {
                        state.registers[address] = if register == Pic16NcoRegister::Nco1Incu {
                            value & 0x0f
                        } else {
                            value
                        };
                        if state.nco_enabled() {
                            if register == Pic16NcoRegister::Nco1Incl {
                                state.nco_increment_pending = true;
                            }
                        } else {
                            state.nco_increment_active = state.nco_increment_registers();
                            state.nco_increment_pending = false;
                        }
                    }
                    Pic16NcoRegister::Nco1Con => {
                        let was_enabled = state.nco_enabled();
                        let output = state.registers[address] & NCO1OUT;
                        state.registers[address] = (value & (NCO1EN | NCO1POL | NCO1PFM)) | output;
                        if was_enabled && !state.nco_enabled() {
                            state.nco_raw_output = false;
                            state.nco_pulse_remaining = 0;
                        }
                        if !was_enabled && state.nco_enabled() {
                            state.nco_epoch = at.ticks();
                        }
                        state.publish_nco_output(at);
                    }
                    Pic16NcoRegister::Nco1Clk => {
                        state.registers[address] = value & 0xef;
                        state.publish_nco_output(at);
                    }
                    Pic16NcoRegister::Pir7 => {
                        state.registers[address] = value & NCO1IF;
                    }
                    Pic16NcoRegister::Pie7 => {
                        state.registers[address] = value & NCO1IE;
                    }
                }
            }
            WDTCON0 => {
                state.registers[address] = value & 0x3f;
                state.watchdog_epoch = at.ticks();
            }
            CLKRCON => {
                state.registers[address] = value & CLKRCON_WRITABLE_MASK;
                state.clock_reference_epoch = at.ticks();
                state.refresh_clock_reference(at);
            }
            CLKRCLK => {
                state.registers[address] = value & CLKRCLK_WRITABLE_MASK;
                state.clock_reference_epoch = at.ticks();
                state.refresh_clock_reference(at);
            }
            ADCON0 => {
                let previous = state.registers[address];
                state.registers[address] = value;
                if value & ADCON0_GO != 0 && value & ADCON0_ADON != 0 && previous & ADCON0_GO == 0 {
                    state.adc_started = Some(((value >> 2) & 0x3f, at.ticks()));
                }
            }
            PIR1 => state.registers[address] = value,
            address if Pic16PpsRegister::from_data_address(address).is_some() => {
                let register = Pic16PpsRegister::from_data_address(address)
                    .expect("PPS address was checked above");
                if register == Pic16PpsRegister::Ppslock {
                    state.registers[address] = value & PPSLOCKED;
                } else if state.registers[Pic16PpsRegister::Ppslock.offset()] & PPSLOCKED == 0 {
                    state.registers[address] = value & PPS_OUTPUT_MASK;
                    if let Some((port, _pin)) = register.port_pin() {
                        state.refresh_port(port, at)?;
                    }
                }
            }
            _ => {
                state.registers[address] = value;
                if let Some(port) = Self::port_for(address, &ANSEL) {
                    state.refresh_port(port, at)?;
                }
            }
        }
        state.update_interrupt_signals(at);
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state
            .lock()
            .expect("PIC16 peripheral lock poisoned")
            .reset_registers(SimTime::ZERO);
    }
}

#[cfg(test)]
mod tests;
