use super::{GpioHandle, GpioState, SignalHub, refresh_gpio, vendor_gpio};
use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use remu_signals::{Logic, SignalId, SignalValue};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

const DATA_BYTES: usize = 0x2000;
const INTCON: usize = 0x00b;
const PORT_BASE: usize = 0x00c;
const TRIS_BASE: usize = 0x012;
const LAT_BASE: usize = 0x018;
const RC1REG: usize = 0x119;
const TX1REG: usize = 0x11a;
const RC1STA: usize = 0x11d;
const TX1STA: usize = 0x11e;
/// Native PIC16F15376 MSSP1 register identifiers.
///
/// The enum keeps the peripheral register window typed at the API boundary;
/// callers should not have to carry unlabelled data-space offsets when
/// inspecting or driving the functional I²C host model.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(usize)]
pub enum Pic16Mssp1Register {
    /// SSP1BUF synchronous serial input/output buffer.
    Buffer = 0x18c,
    /// SSP1ADD baud divider/address register.
    Address = 0x18d,
    /// SSP1MSK address-mask register.
    Mask = 0x18e,
    /// SSP1STAT status register.
    Status = 0x18f,
    /// SSP1CON1 mode and enable control.
    Control1 = 0x190,
    /// SSP1CON2 I²C command and acknowledge control.
    Control2 = 0x191,
    /// SSP1CON3 I²C auxiliary control.
    Control3 = 0x192,
}

impl Pic16Mssp1Register {
    /// Native banked data-space offset for this MSSP1 register.
    pub const fn offset(self) -> usize {
        self as usize
    }

    /// Converts a native data-space offset into a typed MSSP1 identifier.
    pub const fn from_offset(offset: usize) -> Option<Self> {
        match offset {
            0x18c => Some(Self::Buffer),
            0x18d => Some(Self::Address),
            0x18e => Some(Self::Mask),
            0x18f => Some(Self::Status),
            0x190 => Some(Self::Control1),
            0x191 => Some(Self::Control2),
            0x192 => Some(Self::Control3),
            _ => None,
        }
    }
}

// Keep the internal register-array indexing readable while deriving every
// offset from the named enum above rather than duplicating numeric IDs.
const SSP1BUF: usize = Pic16Mssp1Register::Buffer.offset();
const SSP1ADD: usize = Pic16Mssp1Register::Address.offset();
const SSP1MSK: usize = Pic16Mssp1Register::Mask.offset();
const SSP1STAT: usize = Pic16Mssp1Register::Status.offset();
const SSP1CON1: usize = Pic16Mssp1Register::Control1.offset();
const SSP1CON2: usize = Pic16Mssp1Register::Control2.offset();
const SSP1CON3: usize = Pic16Mssp1Register::Control3.offset();
const TMR1L: usize = 0x20c;
const TMR1H: usize = 0x20d;
const T1CON: usize = 0x20e;
const TMR0L: usize = 0x59c;
const TMR0H: usize = 0x59d;
const T0CON0: usize = 0x59e;
const PIR0: usize = 0x70c;
const PIR3: usize = 0x70f;
const PIR4: usize = 0x710;
const PIE0: usize = 0x716;
const PIE3: usize = 0x719;
const PIE4: usize = 0x71a;
const WDTCON0: usize = 0x80c;
const OSCSTAT: usize = 0x890;
const PPSLOCK: usize = 0x1e8f;
const PPS_OUTPUT_BASES: [usize; 5] = [0x1f10, 0x1f18, 0x1f20, 0x1f28, 0x1f30];
const ANSEL: [usize; 5] = [0x1f38, 0x1f43, 0x1f4e, 0x1f59, 0x1f64];

const PORT_WIDTHS: [u8; 5] = [8, 8, 8, 8, 4];
const PORT_MASKS: [u8; 5] = [0xff, 0xff, 0xff, 0xff, 0x0f];
const INTCON_GIE: u8 = 1 << 7;
const INTCON_PEIE: u8 = 1 << 6;
const TMR0IF: u8 = 1 << 5;
const TMR1IF: u8 = 1;
const TX1IF: u8 = 1 << 4;
const RC1IF: u8 = 1 << 5;
const SSP1IF: u8 = 1;
const SSP1IE: u8 = 1;
const TXEN: u8 = 1 << 5;
const SPEN: u8 = 1 << 7;
const SSP1STAT_BF: u8 = 1;
const SSP1STAT_RW: u8 = 1 << 2;
const SSP1STAT_S: u8 = 1 << 3;
const SSP1STAT_P: u8 = 1 << 4;
const SSP1STAT_DA: u8 = 1 << 5;
const SSP1CON1_WCOL: u8 = 1 << 7;
const SSP1CON1_SSPEN: u8 = 1 << 5;
const SSP1_I2C_MASTER_7BIT: u8 = 0x08;
const SSP1_I2C_MASTER_10BIT: u8 = 0x09;
const SSP1CON2_SEN: u8 = 1 << 0;
const SSP1CON2_RSEN: u8 = 1 << 1;
const SSP1CON2_PEN: u8 = 1 << 2;
const SSP1CON2_RCEN: u8 = 1 << 3;
const SSP1CON2_ACKEN: u8 = 1 << 4;
const SSP1CON2_ACKDT: u8 = 1 << 5;
const SSP1CON2_ACKSTAT: u8 = 1 << 6;
const SSP1CON3_BOEN: u8 = 1 << 4;

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
    i2c_events: Vec<Pic16I2cEvent>,
    i2c_responses: BTreeMap<u8, VecDeque<u8>>,
    i2c_acknowledgements: BTreeMap<u8, bool>,
    i2c_address: Option<u8>,
    i2c_read: bool,
    i2c_byte_signal: SignalId,
    i2c_strobe_signal: SignalId,
    timer0_epoch: u64,
    timer1_epoch: u64,
    watchdog_epoch: u64,
    watchdog_reset: bool,
    uart_byte_signal: SignalId,
    uart_strobe_signal: SignalId,
    timer0_irq_signal: SignalId,
    timer1_irq_signal: SignalId,
    interrupt_signal: SignalId,
    watchdog_reset_signal: SignalId,
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

    fn refresh_port(&mut self, port: usize, at: SimTime) -> Result<(), DeviceError> {
        let direction = (!self.registers[TRIS_BASE + port]) & PORT_MASKS[port];
        let output = self.registers[LAT_BASE + port] & PORT_MASKS[port];
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
        self.registers[PIR3] = TX1IF;
        self.registers[SSP1ADD] = 0;
        self.registers[SSP1MSK] = 0xff;
        self.registers[SSP1CON3] = 0;
        self.registers[TX1STA] = 1 << 1; // TRMT
        self.registers[OSCSTAT] = 1 << 6; // internal HF oscillator ready
        self.registers[PPSLOCK] = 1;
        self.uart.clear();
        self.i2c_events.clear();
        self.i2c_responses.clear();
        self.i2c_acknowledgements.clear();
        self.i2c_address = None;
        self.i2c_read = false;
        self.timer0_epoch = at.ticks();
        self.timer1_epoch = at.ticks();
        self.watchdog_epoch = at.ticks();
        self.watchdog_reset = false;
        self.set_signal(self.uart_strobe_signal, 0, 1, at);
        self.set_signal(self.i2c_byte_signal, 0, 8, at);
        self.set_signal(self.i2c_strobe_signal, 0, 1, at);
        self.set_signal(self.timer0_irq_signal, 0, 1, at);
        self.set_signal(self.timer1_irq_signal, 0, 1, at);
        self.set_signal(self.interrupt_signal, 0, 1, at);
        self.set_signal(self.watchdog_reset_signal, 0, 1, at);
        for port in 0..5 {
            let _ = self.refresh_port(port, at);
        }
    }

    fn interrupt_pending(&self) -> bool {
        let peripheral = self.registers[INTCON] & INTCON_PEIE != 0
            && ((self.registers[PIR0] & self.registers[PIE0] & TMR0IF != 0)
                || (self.registers[PIR4] & self.registers[PIE4] & TMR1IF != 0)
                || (self.registers[PIR3] & self.registers[PIE3] & (TX1IF | RC1IF) != 0)
                || (self.registers[PIR3] & SSP1IF != 0 && self.registers[PIE3] & SSP1IE != 0));
        self.registers[INTCON] & INTCON_GIE != 0 && peripheral
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
                state.registers[PIR4] |= TMR1IF;
                state.set_signal(state.timer1_irq_signal, 1, 1, now);
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
        let pending = state.interrupt_pending();
        state.set_signal(state.interrupt_signal, u64::from(pending), 1, now);
        pending
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
        let state = Arc::new(Mutex::new(Pic16State {
            registers: vec![0; DATA_BYTES],
            ports: [porta, portb, portc, portd, porte],
            port_signals: [signals_a, signals_b, signals_c, signals_d, signals_e],
            hub,
            uart: Vec::new(),
            i2c_events: Vec::new(),
            i2c_responses: BTreeMap::new(),
            i2c_acknowledgements: BTreeMap::new(),
            i2c_address: None,
            i2c_read: false,
            i2c_byte_signal,
            i2c_strobe_signal,
            timer0_epoch: 0,
            timer1_epoch: 0,
            watchdog_epoch: 0,
            watchdog_reset: false,
            uart_byte_signal,
            uart_strobe_signal,
            timer0_irq_signal,
            timer1_irq_signal,
            interrupt_signal,
            watchdog_reset_signal,
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
        let value = match address {
            OSCSTAT => state.registers[address] | (1 << 6),
            TX1STA => state.registers[address] | (1 << 1),
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
            SSP1BUF => state.i2c_buffer_write(value, at),
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
            SSP1STAT => {
                // BF, R/W, D/A, S and P are maintained by the functional
                // transfer model; SMP and CKE are the writable mode bits.
                state.registers[address] = (state.registers[address] & 0x3f) | (value & 0xc0);
            }
            PIR0 => {
                state.registers[address] = value;
                state.set_signal(
                    state.timer0_irq_signal,
                    u64::from(value & TMR0IF != 0),
                    1,
                    at,
                );
            }
            PIR4 => {
                state.registers[address] = value;
                state.set_signal(
                    state.timer1_irq_signal,
                    u64::from(value & TMR1IF != 0),
                    1,
                    at,
                );
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
            WDTCON0 => {
                state.registers[address] = value & 0x3f;
                state.watchdog_epoch = at.ticks();
            }
            _ => {
                state.registers[address] = value;
                if let Some(port) = Self::port_for(address, &ANSEL) {
                    state.refresh_port(port, at)?;
                }
                // PPS output registers are retained verbatim so firmware can read them back.
                let _is_output_pps = PPS_OUTPUT_BASES
                    .iter()
                    .any(|base| (*base..*base + 8).contains(&address));
            }
        }
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
mod tests {
    use super::*;

    #[test]
    fn mssp1_register_ids_cover_the_native_window() {
        assert_eq!(
            Pic16Mssp1Register::from_offset(0x18c),
            Some(Pic16Mssp1Register::Buffer)
        );
        assert_eq!(
            Pic16Mssp1Register::Control3.offset(),
            0x192,
            "the typed ID must retain the PIC16 native offset"
        );
        assert_eq!(Pic16Mssp1Register::from_offset(0x193), None);
    }

    #[test]
    fn gpio_uart_timer_and_watchdog_slice_is_functional() {
        let hub = SignalHub::new();
        let (mut device, handle, ports) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
        device
            .write(ANSEL[0] as u64, AccessWidth::Byte, 0, SimTime::ZERO)
            .unwrap();
        device
            .write(TRIS_BASE as u64, AccessWidth::Byte, 0xfe, SimTime::ZERO)
            .unwrap();
        device
            .write(LAT_BASE as u64, AccessWidth::Byte, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(ports[0].output() & 1, 1);

        device
            .write(RC1STA as u64, AccessWidth::Byte, SPEN.into(), SimTime::ZERO)
            .unwrap();
        device
            .write(TX1STA as u64, AccessWidth::Byte, TXEN.into(), SimTime::ZERO)
            .unwrap();
        device
            .write(TX1REG as u64, AccessWidth::Byte, b'P'.into(), SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.uart_bytes(), b"P");

        device
            .write(TMR0H as u64, AccessWidth::Byte, 3, SimTime::ZERO)
            .unwrap();
        device
            .write(PIE0 as u64, AccessWidth::Byte, TMR0IF.into(), SimTime::ZERO)
            .unwrap();
        device
            .write(
                INTCON as u64,
                AccessWidth::Byte,
                (INTCON_GIE | INTCON_PEIE).into(),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(T0CON0 as u64, AccessWidth::Byte, 0x80, SimTime::ZERO)
            .unwrap();
        assert!(handle.poll(SimTime::from_ticks(4)));
    }

    #[test]
    fn mssp1_i2c_master_records_write_and_start_stop() {
        let hub = SignalHub::new();
        let (mut device, handle, _ports) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
        device
            .write(
                SSP1CON1 as u64,
                AccessWidth::Byte,
                u64::from(SSP1CON1_SSPEN | SSP1_I2C_MASTER_7BIT),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                PIE3 as u64,
                AccessWidth::Byte,
                u64::from(SSP1IE),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                INTCON as u64,
                AccessWidth::Byte,
                u64::from(INTCON_GIE | INTCON_PEIE),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                SSP1CON2 as u64,
                AccessWidth::Byte,
                u64::from(SSP1CON2_SEN),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                SSP1BUF as u64,
                AccessWidth::Byte,
                0xa0,
                SimTime::from_ticks(1),
            )
            .unwrap();
        device
            .write(
                SSP1BUF as u64,
                AccessWidth::Byte,
                0x10,
                SimTime::from_ticks(2),
            )
            .unwrap();
        device
            .write(
                SSP1CON2 as u64,
                AccessWidth::Byte,
                u64::from(SSP1CON2_PEN),
                SimTime::from_ticks(3),
            )
            .unwrap();

        assert_eq!(
            handle.i2c_events(),
            vec![
                Pic16I2cEvent::Start,
                Pic16I2cEvent::Write {
                    address: 0x50,
                    value: 0x10,
                },
                Pic16I2cEvent::Stop,
            ]
        );
        assert!(handle.poll(SimTime::from_ticks(3)));
        assert_eq!(
            device
                .read(PIR3 as u64, AccessWidth::Byte, SimTime::from_ticks(3))
                .unwrap()
                & u64::from(SSP1IF),
            u64::from(SSP1IF)
        );
    }

    #[test]
    fn mssp1_i2c_master_reads_queued_response_and_clears_bf_on_read() {
        let hub = SignalHub::new();
        let (mut device, handle, _ports) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
        device
            .write(
                SSP1CON1 as u64,
                AccessWidth::Byte,
                u64::from(SSP1CON1_SSPEN | SSP1_I2C_MASTER_7BIT),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                PIE3 as u64,
                AccessWidth::Byte,
                u64::from(SSP1IE),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                INTCON as u64,
                AccessWidth::Byte,
                u64::from(INTCON_GIE | INTCON_PEIE),
                SimTime::ZERO,
            )
            .unwrap();
        handle.queue_i2c_read(0x50, [0x42]);
        device
            .write(
                SSP1CON2 as u64,
                AccessWidth::Byte,
                u64::from(SSP1CON2_SEN),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                SSP1BUF as u64,
                AccessWidth::Byte,
                0xa1,
                SimTime::from_ticks(1),
            )
            .unwrap();
        device
            .write(
                SSP1CON2 as u64,
                AccessWidth::Byte,
                u64::from(SSP1CON2_RCEN),
                SimTime::from_ticks(2),
            )
            .unwrap();

        assert_eq!(
            device
                .read(SSP1STAT as u64, AccessWidth::Byte, SimTime::from_ticks(2))
                .unwrap()
                & u64::from(SSP1STAT_BF),
            u64::from(SSP1STAT_BF)
        );
        assert_eq!(
            device
                .read(SSP1BUF as u64, AccessWidth::Byte, SimTime::from_ticks(3))
                .unwrap(),
            0x42
        );
        assert_eq!(
            device
                .read(SSP1STAT as u64, AccessWidth::Byte, SimTime::from_ticks(3))
                .unwrap()
                & u64::from(SSP1STAT_BF),
            0
        );
        assert_eq!(
            handle.i2c_events(),
            vec![
                Pic16I2cEvent::Start,
                Pic16I2cEvent::Read {
                    address: 0x50,
                    value: 0x42,
                },
            ]
        );
        assert!(handle.poll(SimTime::from_ticks(3)));
    }

    #[test]
    fn mssp1_i2c_master_reports_ackstat_and_ack_sequence() {
        let hub = SignalHub::new();
        let (mut device, handle, _ports) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
        device
            .write(
                SSP1CON1 as u64,
                AccessWidth::Byte,
                u64::from(SSP1CON1_SSPEN | SSP1_I2C_MASTER_7BIT),
                SimTime::ZERO,
            )
            .unwrap();
        handle.set_i2c_ack(0x50, false);
        device
            .write(
                SSP1CON2 as u64,
                AccessWidth::Byte,
                u64::from(SSP1CON2_SEN),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                SSP1BUF as u64,
                AccessWidth::Byte,
                0xa0,
                SimTime::from_ticks(1),
            )
            .unwrap();
        assert_ne!(
            device
                .read(SSP1CON2 as u64, AccessWidth::Byte, SimTime::from_ticks(1))
                .unwrap()
                & u64::from(SSP1CON2_ACKSTAT),
            0,
            "a configured NACK must be visible through ACKSTAT"
        );

        handle.set_i2c_ack(0x50, true);
        handle.queue_i2c_read(0x50, [0x42]);
        device
            .write(
                SSP1CON2 as u64,
                AccessWidth::Byte,
                u64::from(SSP1CON2_RSEN),
                SimTime::from_ticks(2),
            )
            .unwrap();
        device
            .write(
                SSP1BUF as u64,
                AccessWidth::Byte,
                0xa1,
                SimTime::from_ticks(3),
            )
            .unwrap();
        device
            .write(
                SSP1CON2 as u64,
                AccessWidth::Byte,
                u64::from(SSP1CON2_RCEN),
                SimTime::from_ticks(4),
            )
            .unwrap();
        assert_eq!(
            device
                .read(SSP1BUF as u64, AccessWidth::Byte, SimTime::from_ticks(5))
                .unwrap(),
            0x42
        );
        device
            .write(
                SSP1CON2 as u64,
                AccessWidth::Byte,
                u64::from(SSP1CON2_ACKDT | SSP1CON2_ACKEN),
                SimTime::from_ticks(6),
            )
            .unwrap();
        assert_eq!(
            handle.i2c_events().last(),
            Some(&Pic16I2cEvent::Ack { acknowledge: false })
        );
        assert_eq!(
            device
                .read(SSP1CON2 as u64, AccessWidth::Byte, SimTime::from_ticks(6))
                .unwrap()
                & u64::from(SSP1CON2_ACKEN),
            0
        );
    }

    #[test]
    fn mssp1_i2c_master_rejects_queued_commands_and_preserves_receive_buffer() {
        let hub = SignalHub::new();
        let (mut device, handle, _ports) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
        device
            .write(
                SSP1CON1 as u64,
                AccessWidth::Byte,
                u64::from(SSP1CON1_SSPEN | SSP1_I2C_MASTER_7BIT),
                SimTime::ZERO,
            )
            .unwrap();
        handle.queue_i2c_read(0x50, [0x10, 0x20]);
        device
            .write(
                SSP1CON2 as u64,
                AccessWidth::Byte,
                u64::from(SSP1CON2_SEN),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                SSP1BUF as u64,
                AccessWidth::Byte,
                0xa1,
                SimTime::from_ticks(1),
            )
            .unwrap();
        device
            .write(
                SSP1CON2 as u64,
                AccessWidth::Byte,
                u64::from(SSP1CON2_RCEN),
                SimTime::from_ticks(2),
            )
            .unwrap();
        device
            .write(
                SSP1CON2 as u64,
                AccessWidth::Byte,
                u64::from(SSP1CON2_SEN | SSP1CON2_PEN),
                SimTime::from_ticks(3),
            )
            .unwrap();
        assert_ne!(
            device
                .read(SSP1CON1 as u64, AccessWidth::Byte, SimTime::from_ticks(3))
                .unwrap()
                & u64::from(SSP1CON1_WCOL),
            0
        );
        assert_eq!(
            device
                .read(SSP1BUF as u64, AccessWidth::Byte, SimTime::from_ticks(4))
                .unwrap(),
            0x10
        );
    }
}
