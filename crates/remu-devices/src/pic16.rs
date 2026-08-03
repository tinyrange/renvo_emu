use super::{GpioHandle, GpioState, SignalHub, refresh_gpio, vendor_gpio};
use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use remu_signals::{Logic, SignalId, SignalValue};
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
const TXEN: u8 = 1 << 5;
const SPEN: u8 = 1 << 7;
const C1IF: u8 = 1;
const CM1CON0_OUT: u8 = 1 << 6;
const CMOUT_C1OUT: u8 = 1;
const C1ON: u8 = 1 << 7;
const C1POL: u8 = 1 << 4;
const CM1CON0_WRITE_MASK: u8 = C1ON | C1POL | (1 << 1) | 1;
const CM1CON1_MASK: u8 = 0x03;
const CM1_CHANNEL_MASK: u8 = 0x07;

/// PIC16F15376 Comparator C1 and interrupt register identifiers.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
#[repr(usize)]
pub enum Pic16ComparatorRegister {
    /// Comparator interrupt flag register (C1IF is bit 0).
    Pir2 = 0x70e,
    /// Comparator interrupt enable register (C1IE is bit 0).
    Pie2 = 0x718,
    /// Read-only mirror of comparator outputs.
    Cmout = 0x98f,
    /// Comparator C1 enable, output, polarity, hysteresis and sync control.
    Cm1Con0 = 0x990,
    /// Comparator C1 edge interrupt enables.
    Cm1Con1 = 0x991,
    /// Comparator C1 negative input selection.
    Cm1Nch = 0x992,
    /// Comparator C1 positive input selection.
    Cm1Pch = 0x993,
}

impl Pic16ComparatorRegister {
    /// All comparator-related registers modelled by this peripheral slice.
    pub const ALL: [Self; 7] = [
        Self::Pir2,
        Self::Pie2,
        Self::Cmout,
        Self::Cm1Con0,
        Self::Cm1Con1,
        Self::Cm1Nch,
        Self::Cm1Pch,
    ];

    /// Data-space address of this register.
    pub const fn offset(self) -> usize {
        self as usize
    }

    /// Backing register-array index for this register.
    pub const fn index(self) -> usize {
        self.offset()
    }

    /// Lowercase datasheet register name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Pir2 => "pir2",
            Self::Pie2 => "pie2",
            Self::Cmout => "cmout",
            Self::Cm1Con0 => "cm1con0",
            Self::Cm1Con1 => "cm1con1",
            Self::Cm1Nch => "cm1nch",
            Self::Cm1Pch => "cm1pch",
        }
    }

    /// Converts a data-space address into a known comparator register.
    pub const fn from_data_address(address: usize) -> Option<Self> {
        match address {
            0x70e => Some(Self::Pir2),
            0x718 => Some(Self::Pie2),
            0x98f => Some(Self::Cmout),
            0x990 => Some(Self::Cm1Con0),
            0x991 => Some(Self::Cm1Con1),
            0x992 => Some(Self::Cm1Nch),
            0x993 => Some(Self::Cm1Pch),
            _ => None,
        }
    }
}

struct Pic16State {
    registers: Vec<u8>,
    ports: [Arc<Mutex<GpioState>>; 5],
    port_signals: [Vec<SignalId>; 5],
    hub: SignalHub,
    uart: Vec<u8>,
    timer0_epoch: u64,
    timer1_epoch: u64,
    watchdog_epoch: u64,
    watchdog_reset: bool,
    uart_byte_signal: SignalId,
    uart_strobe_signal: SignalId,
    timer0_irq_signal: SignalId,
    timer1_irq_signal: SignalId,
    comparator1_output_signal: SignalId,
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

    fn reset_registers(&mut self, at: SimTime) {
        self.registers.fill(0);
        for port in 0..5 {
            self.registers[TRIS_BASE + port] = PORT_MASKS[port];
            self.registers[ANSEL[port]] = PORT_MASKS[port];
        }
        self.registers[PIR3] = TX1IF;
        self.registers[TX1STA] = 1 << 1; // TRMT
        self.registers[OSCSTAT] = 1 << 6; // internal HF oscillator ready
        self.registers[PPSLOCK] = 1;
        self.uart.clear();
        self.timer0_epoch = at.ticks();
        self.timer1_epoch = at.ticks();
        self.watchdog_epoch = at.ticks();
        self.watchdog_reset = false;
        self.set_signal(self.uart_strobe_signal, 0, 1, at);
        self.set_signal(self.timer0_irq_signal, 0, 1, at);
        self.set_signal(self.timer1_irq_signal, 0, 1, at);
        self.set_signal(self.comparator1_output_signal, 0, 1, at);
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
                || (self.registers[Pic16ComparatorRegister::Pir2.index()]
                    & self.registers[Pic16ComparatorRegister::Pie2.index()]
                    & C1IF
                    != 0)
                || (self.registers[PIR3] & self.registers[PIE3] & (TX1IF | RC1IF) != 0));
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

    /// Returns the current logical C1 comparator output.
    pub fn comparator1_output(&self) -> bool {
        self.0
            .lock()
            .expect("PIC16 peripheral lock poisoned")
            .registers[Pic16ComparatorRegister::Cm1Con0.index()]
            & CM1CON0_OUT
            != 0
    }

    /// Advances functional timers and returns the combined interrupt request.
    pub fn poll(&self, now: SimTime) -> bool {
        let mut state = self.0.lock().expect("PIC16 peripheral lock poisoned");
        for port in 0..5 {
            let _ = state.refresh_port(port, now);
        }
        state.update_comparator(now);
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
        let state = Arc::new(Mutex::new(Pic16State {
            registers: vec![0; DATA_BYTES],
            ports: [porta, portb, portc, portd, porte],
            port_signals: [signals_a, signals_b, signals_c, signals_d, signals_e],
            hub,
            uart: Vec::new(),
            timer0_epoch: 0,
            timer1_epoch: 0,
            watchdog_epoch: 0,
            watchdog_reset: false,
            uart_byte_signal,
            uart_strobe_signal,
            timer0_irq_signal,
            timer1_irq_signal,
            comparator1_output_signal,
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
    fn comparator_register_ids_are_named_and_stable() {
        assert_eq!(Pic16ComparatorRegister::ALL.len(), 7);
        assert_eq!(Pic16ComparatorRegister::Pir2.offset(), 0x70e);
        assert_eq!(Pic16ComparatorRegister::Cm1Con0.index(), 0x990);
        assert_eq!(Pic16ComparatorRegister::Cm1Con0.name(), "cm1con0");
        assert_eq!(
            Pic16ComparatorRegister::from_data_address(0x993),
            Some(Pic16ComparatorRegister::Cm1Pch)
        );
        assert_eq!(Pic16ComparatorRegister::from_data_address(0x994), None);
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
    fn comparator1_selects_gpio_inputs_and_latches_edge_interrupts() {
        let hub = SignalHub::new();
        let (mut device, handle, ports) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
        ports[0].set_input(0, Logic::Zero, SimTime::ZERO).unwrap(); // C1IN0-
        ports[0].set_input(2, Logic::One, SimTime::ZERO).unwrap(); // C1IN0+
        device
            .write(
                Pic16ComparatorRegister::Pie2.offset() as u64,
                AccessWidth::Byte,
                C1IF.into(),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Pic16ComparatorRegister::Cm1Con1.offset() as u64,
                AccessWidth::Byte,
                0x02,
                SimTime::ZERO,
            )
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
            .write(
                Pic16ComparatorRegister::Cm1Con0.offset() as u64,
                AccessWidth::Byte,
                C1ON.into(),
                SimTime::ZERO,
            )
            .unwrap();
        assert!(handle.comparator1_output());
        assert!(handle.poll(SimTime::from_ticks(1)));
        assert_eq!(
            device
                .read(
                    Pic16ComparatorRegister::Cm1Con0.offset() as u64,
                    AccessWidth::Byte,
                    SimTime::from_ticks(1),
                )
                .unwrap(),
            u64::from(C1ON | CM1CON0_OUT)
        );
        assert_eq!(
            device
                .read(
                    Pic16ComparatorRegister::Cmout.offset() as u64,
                    AccessWidth::Byte,
                    SimTime::from_ticks(1),
                )
                .unwrap(),
            u64::from(CMOUT_C1OUT)
        );
        device
            .write(
                Pic16ComparatorRegister::Cmout.offset() as u64,
                AccessWidth::Byte,
                u8::MAX.into(),
                SimTime::from_ticks(1),
            )
            .unwrap();
        assert_eq!(
            device
                .read(
                    Pic16ComparatorRegister::Cmout.offset() as u64,
                    AccessWidth::Byte,
                    SimTime::from_ticks(1),
                )
                .unwrap(),
            u64::from(CMOUT_C1OUT)
        );

        device
            .write(
                Pic16ComparatorRegister::Pir2.offset() as u64,
                AccessWidth::Byte,
                0,
                SimTime::from_ticks(1),
            )
            .unwrap();
        ports[0]
            .set_input(2, Logic::Zero, SimTime::from_ticks(2))
            .unwrap();
        assert!(!handle.poll(SimTime::from_ticks(2)));
        assert!(!handle.comparator1_output());
    }

    #[test]
    fn comparator1_stays_low_when_disabled_even_if_polarity_is_inverted() {
        let hub = SignalHub::new();
        let (mut device, handle, ports) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
        ports[0].set_input(0, Logic::Zero, SimTime::ZERO).unwrap();
        ports[0].set_input(2, Logic::One, SimTime::ZERO).unwrap();
        device
            .write(
                Pic16ComparatorRegister::Cm1Con0.offset() as u64,
                AccessWidth::Byte,
                C1POL.into(),
                SimTime::ZERO,
            )
            .unwrap();
        assert!(!handle.comparator1_output());
        device
            .write(
                Pic16ComparatorRegister::Cm1Con0.offset() as u64,
                AccessWidth::Byte,
                u64::from(C1ON | C1POL),
                SimTime::from_ticks(1),
            )
            .unwrap();
        assert!(!handle.comparator1_output());
        device
            .write(
                Pic16ComparatorRegister::Cm1Con0.offset() as u64,
                AccessWidth::Byte,
                C1ON.into(),
                SimTime::from_ticks(2),
            )
            .unwrap();
        assert!(handle.comparator1_output());
    }
}
