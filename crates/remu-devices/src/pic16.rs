use super::{GpioHandle, GpioState, SignalHub, refresh_gpio, vendor_gpio};
use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use remu_signals::{Logic, SignalId, SignalValue};
use serde::{Deserialize, Serialize};
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
const PPS_OUTPUT_TX1: u8 = 0x0f;
const PPS_OUTPUT_TMR0: u8 = 0x19;
const PPS_OUTPUT_MASK: u8 = 0x1f;
const PPSLOCKED: u8 = 1;

/// Named PIC16F15376 PPS registers used by the functional output-routing model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
#[allow(missing_docs)]
pub enum Pic16PpsRegister {
    /// PPS lock state register.
    Ppslock,
    /// PORTA PPS output registers.
    Ra0Pps,
    Ra1Pps,
    Ra2Pps,
    Ra3Pps,
    Ra4Pps,
    Ra5Pps,
    Ra6Pps,
    Ra7Pps,
    /// PORTB PPS output registers.
    Rb0Pps,
    Rb1Pps,
    Rb2Pps,
    Rb3Pps,
    Rb4Pps,
    Rb5Pps,
    Rb6Pps,
    Rb7Pps,
    /// PORTC PPS output registers.
    Rc0Pps,
    Rc1Pps,
    Rc2Pps,
    Rc3Pps,
    Rc4Pps,
    Rc5Pps,
    Rc6Pps,
    Rc7Pps,
    /// PORTD PPS output registers.
    Rd0Pps,
    Rd1Pps,
    Rd2Pps,
    Rd3Pps,
    Rd4Pps,
    Rd5Pps,
    Rd6Pps,
    Rd7Pps,
    /// PORTE PPS output registers.
    Re0Pps,
    Re1Pps,
    Re2Pps,
    Re3Pps,
}

impl Pic16PpsRegister {
    /// Stable register order.
    pub const ALL: [Self; 37] = [
        Self::Ppslock,
        Self::Ra0Pps,
        Self::Ra1Pps,
        Self::Ra2Pps,
        Self::Ra3Pps,
        Self::Ra4Pps,
        Self::Ra5Pps,
        Self::Ra6Pps,
        Self::Ra7Pps,
        Self::Rb0Pps,
        Self::Rb1Pps,
        Self::Rb2Pps,
        Self::Rb3Pps,
        Self::Rb4Pps,
        Self::Rb5Pps,
        Self::Rb6Pps,
        Self::Rb7Pps,
        Self::Rc0Pps,
        Self::Rc1Pps,
        Self::Rc2Pps,
        Self::Rc3Pps,
        Self::Rc4Pps,
        Self::Rc5Pps,
        Self::Rc6Pps,
        Self::Rc7Pps,
        Self::Rd0Pps,
        Self::Rd1Pps,
        Self::Rd2Pps,
        Self::Rd3Pps,
        Self::Rd4Pps,
        Self::Rd5Pps,
        Self::Rd6Pps,
        Self::Rd7Pps,
        Self::Re0Pps,
        Self::Re1Pps,
        Self::Re2Pps,
        Self::Re3Pps,
    ];

    /// Canonical data-space address.
    pub const fn offset(self) -> usize {
        match self as u8 {
            0 => 0x1e8f,
            1..=8 => 0x1f10 + (self as usize - 1),
            9..=16 => 0x1f18 + (self as usize - 9),
            17..=24 => 0x1f20 + (self as usize - 17),
            25..=32 => 0x1f28 + (self as usize - 25),
            33..=36 => 0x1f30 + (self as usize - 33),
            _ => unreachable!(),
        }
    }

    /// Stable numeric index for metadata tables.
    pub const fn index(self) -> usize {
        self as usize
    }

    /// Stable lowercase register name.
    pub const fn name(self) -> &'static str {
        const NAMES: [&str; 37] = [
            "ppslock", "ra0pps", "ra1pps", "ra2pps", "ra3pps", "ra4pps", "ra5pps", "ra6pps",
            "ra7pps", "rb0pps", "rb1pps", "rb2pps", "rb3pps", "rb4pps", "rb5pps", "rb6pps",
            "rb7pps", "rc0pps", "rc1pps", "rc2pps", "rc3pps", "rc4pps", "rc5pps", "rc6pps",
            "rc7pps", "rd0pps", "rd1pps", "rd2pps", "rd3pps", "rd4pps", "rd5pps", "rd6pps",
            "rd7pps", "re0pps", "re1pps", "re2pps", "re3pps",
        ];
        NAMES[self.index()]
    }

    /// Returns the output port/pin for an RxyPPS register.
    pub const fn port_pin(self) -> Option<(usize, usize)> {
        match self as u8 {
            1..=8 => Some((0, self as usize - 1)),
            9..=16 => Some((1, self as usize - 9)),
            17..=24 => Some((2, self as usize - 17)),
            25..=32 => Some((3, self as usize - 25)),
            33..=36 => Some((4, self as usize - 33)),
            _ => None,
        }
    }

    /// Returns a named output register for a port/pin pair.
    pub fn output(port: usize, pin: usize) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|register| register.port_pin() == Some((port, pin)))
    }

    /// Resolves a raw data-space address to its named PPS register.
    pub fn from_data_address(address: usize) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|register| register.offset() == address)
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

    fn reset_registers(&mut self, at: SimTime) {
        self.registers.fill(0);
        for port in 0..5 {
            self.registers[TRIS_BASE + port] = PORT_MASKS[port];
            self.registers[ANSEL[port]] = PORT_MASKS[port];
        }
        self.registers[PIR3] = TX1IF;
        self.registers[TX1STA] = 1 << 1; // TRMT
        self.registers[OSCSTAT] = 1 << 6; // internal HF oscillator ready
        self.registers[Pic16PpsRegister::Ppslock.offset()] = 0;
        self.uart.clear();
        self.timer0_epoch = at.ticks();
        self.timer1_epoch = at.ticks();
        self.watchdog_epoch = at.ticks();
        self.watchdog_reset = false;
        self.set_signal(self.uart_strobe_signal, 0, 1, at);
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
        for port in 0..5 {
            let _ = state.refresh_port(port, now);
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
            WDTCON0 => {
                state.registers[address] = value & 0x3f;
                state.watchdog_epoch = at.ticks();
            }
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
    fn pps_routes_timer0_and_eusart_strobes_to_gpio_outputs() {
        let hub = SignalHub::new();
        let (mut device, handle, ports) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
        device
            .write(ANSEL[0] as u64, AccessWidth::Byte, 0, SimTime::ZERO)
            .unwrap();
        device
            .write(TRIS_BASE as u64, AccessWidth::Byte, 0xfc, SimTime::ZERO)
            .unwrap();
        device
            .write(
                Pic16PpsRegister::Ra0Pps.offset() as u64,
                AccessWidth::Byte,
                PPS_OUTPUT_TMR0.into(),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(TMR0H as u64, AccessWidth::Byte, 1, SimTime::ZERO)
            .unwrap();
        device
            .write(T0CON0 as u64, AccessWidth::Byte, 0x80, SimTime::ZERO)
            .unwrap();
        assert_eq!(ports[0].output() & 1, 0);
        handle.poll(SimTime::from_ticks(2));
        assert_eq!(ports[0].output() & 1, 1);

        device
            .write(
                Pic16PpsRegister::Ra0Pps.offset() as u64,
                AccessWidth::Byte,
                PPS_OUTPUT_TX1.into(),
                SimTime::from_ticks(2),
            )
            .unwrap();
        device
            .write(
                RC1STA as u64,
                AccessWidth::Byte,
                SPEN.into(),
                SimTime::from_ticks(2),
            )
            .unwrap();
        device
            .write(
                TX1STA as u64,
                AccessWidth::Byte,
                TXEN.into(),
                SimTime::from_ticks(2),
            )
            .unwrap();
        device
            .write(
                TX1REG as u64,
                AccessWidth::Byte,
                b'P'.into(),
                SimTime::from_ticks(2),
            )
            .unwrap();
        handle.poll(SimTime::from_ticks(2));
        assert_eq!(ports[0].output() & 1, 1);
    }

    #[test]
    fn pps_registers_are_named_cover_all_pins_and_honor_the_lock() {
        assert_eq!(Pic16PpsRegister::ALL.len(), 37);
        for (index, register) in Pic16PpsRegister::ALL.iter().copied().enumerate() {
            assert_eq!(register.index(), index);
            assert_eq!(
                Pic16PpsRegister::from_data_address(register.offset()),
                Some(register)
            );
        }
        assert_eq!(Pic16PpsRegister::Ra7Pps.port_pin(), Some((0, 7)));
        assert_eq!(Pic16PpsRegister::Re3Pps.port_pin(), Some((4, 3)));
        assert_eq!(
            Pic16PpsRegister::output(3, 7),
            Some(Pic16PpsRegister::Rd7Pps)
        );

        let hub = SignalHub::new();
        let (mut device, handle, ports) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
        let at = SimTime::ZERO;
        device
            .write(ANSEL[0] as u64, AccessWidth::Byte, 0, at)
            .unwrap();
        device
            .write(TRIS_BASE as u64, AccessWidth::Byte, 0x7f, at)
            .unwrap();
        device
            .write(
                Pic16PpsRegister::Ra7Pps.offset() as u64,
                AccessWidth::Byte,
                PPS_OUTPUT_TMR0.into(),
                at,
            )
            .unwrap();
        device
            .write(TMR0H as u64, AccessWidth::Byte, 1, at)
            .unwrap();
        device
            .write(T0CON0 as u64, AccessWidth::Byte, 0x80, at)
            .unwrap();
        handle.poll(SimTime::from_ticks(2));
        assert_eq!(ports[0].output() & 0x80, 0x80);

        device
            .write(
                Pic16PpsRegister::Ppslock.offset() as u64,
                AccessWidth::Byte,
                PPSLOCKED.into(),
                at,
            )
            .unwrap();
        device
            .write(
                Pic16PpsRegister::Ra7Pps.offset() as u64,
                AccessWidth::Byte,
                0,
                at,
            )
            .unwrap();
        assert_eq!(
            device.read(
                Pic16PpsRegister::Ra7Pps.offset() as u64,
                AccessWidth::Byte,
                at
            ),
            Ok(u64::from(PPS_OUTPUT_TMR0))
        );
    }
}
