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
const NCO1ACCL: usize = 0x58c;
const NCO1ACCH: usize = 0x58d;
const NCO1ACCU: usize = 0x58e;
const NCO1INCL: usize = 0x58f;
const NCO1INCH: usize = 0x590;
const NCO1INCU: usize = 0x591;
const NCO1CON: usize = 0x592;
const NCO1CLK: usize = 0x593;
const TMR0L: usize = 0x59c;
const TMR0H: usize = 0x59d;
const T0CON0: usize = 0x59e;
const PIR0: usize = 0x70c;
const PIR3: usize = 0x70f;
const PIR4: usize = 0x710;
const PIR7: usize = 0x713;
const PIE0: usize = 0x716;
const PIE3: usize = 0x719;
const PIE4: usize = 0x71a;
const PIE7: usize = 0x71d;
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
const NCO1IF: u8 = 1 << 4;
const NCO1IE: u8 = 1 << 4;
const NCO1EN: u8 = 1 << 7;
const NCO1OUT: u8 = 1 << 5;
const NCO1POL: u8 = 1 << 4;
const NCO1PFM: u8 = 1;
const NCO_ACC_MASK: u32 = 0x0f_ffff;

struct Pic16State {
    registers: Vec<u8>,
    ports: [Arc<Mutex<GpioState>>; 5],
    port_signals: [Vec<SignalId>; 5],
    hub: SignalHub,
    uart: Vec<u8>,
    timer0_epoch: u64,
    timer1_epoch: u64,
    nco_epoch: u64,
    watchdog_epoch: u64,
    watchdog_reset: bool,
    uart_byte_signal: SignalId,
    uart_strobe_signal: SignalId,
    timer0_irq_signal: SignalId,
    timer1_irq_signal: SignalId,
    nco1_output_signal: SignalId,
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

    fn nco_accumulator(&self) -> u32 {
        u32::from(self.registers[NCO1ACCL])
            | (u32::from(self.registers[NCO1ACCH]) << 8)
            | (u32::from(self.registers[NCO1ACCU] & 0x0f) << 16)
    }

    fn nco_increment(&self) -> u32 {
        u32::from(self.registers[NCO1INCL])
            | (u32::from(self.registers[NCO1INCH]) << 8)
            | (u32::from(self.registers[NCO1INCU] & 0x0f) << 16)
    }

    fn nco_output(&self) -> bool {
        let output = self.registers[NCO1CON] & NCO1OUT != 0;
        if self.registers[NCO1CON] & NCO1POL != 0 {
            !output
        } else {
            output
        }
    }

    fn update_nco(&mut self, now: SimTime) {
        let elapsed = now.ticks().saturating_sub(self.nco_epoch);
        self.nco_epoch = now.ticks();
        if self.registers[NCO1CON] & NCO1EN == 0 || elapsed == 0 {
            let output = self.registers[NCO1CON] & NCO1EN != 0 && self.nco_output();
            self.set_signal(self.nco1_output_signal, u64::from(output), 1, now);
            return;
        }
        let increment = u64::from(self.nco_increment());
        let total = u64::from(self.nco_accumulator()) + increment.saturating_mul(elapsed);
        let overflows = total >> 20;
        let accumulator = (total as u32) & NCO_ACC_MASK;
        self.registers[NCO1ACCL] = accumulator as u8;
        self.registers[NCO1ACCH] = (accumulator >> 8) as u8;
        self.registers[NCO1ACCU] = (accumulator >> 16) as u8 & 0x0f;
        if overflows != 0 {
            self.registers[PIR7] |= NCO1IF;
            if self.registers[NCO1CON] & NCO1PFM == 0 && overflows & 1 != 0 {
                self.registers[NCO1CON] ^= NCO1OUT;
            }
        }
        let output = self.registers[NCO1CON] & NCO1EN != 0 && self.nco_output();
        self.registers[NCO1CON] = (self.registers[NCO1CON] & !NCO1OUT)
            | (u8::from(output ^ (self.registers[NCO1CON] & NCO1POL != 0)) * NCO1OUT);
        self.set_signal(self.nco1_output_signal, u64::from(output), 1, now);
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
        self.nco_epoch = at.ticks();
        self.watchdog_epoch = at.ticks();
        self.watchdog_reset = false;
        self.set_signal(self.uart_strobe_signal, 0, 1, at);
        self.set_signal(self.timer0_irq_signal, 0, 1, at);
        self.set_signal(self.timer1_irq_signal, 0, 1, at);
        self.set_signal(self.nco1_output_signal, 0, 1, at);
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
                || (self.registers[PIR7] & self.registers[PIE7] & NCO1IF != 0)
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

    /// Returns the current logical NCO1 output.
    pub fn nco1_output(&self) -> bool {
        let state = self.0.lock().expect("PIC16 peripheral lock poisoned");
        state.registers[NCO1CON] & NCO1EN != 0 && state.nco_output()
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
        state.update_nco(now);
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
        let nco1_output_signal = hub.declare(
            "board.pic16f15376.nco1.output",
            SignalValue::from_u64(0, 1)?,
            Some("functional NCO1 output".to_owned()),
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
            nco_epoch: 0,
            watchdog_epoch: 0,
            watchdog_reset: false,
            uart_byte_signal,
            uart_strobe_signal,
            timer0_irq_signal,
            timer1_irq_signal,
            nco1_output_signal,
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
            NCO1ACCL | NCO1ACCH | NCO1ACCU | NCO1INCL | NCO1INCH | NCO1INCU => {
                state.update_nco(at);
                state.registers[address] = if address == NCO1ACCU || address == NCO1INCU {
                    value & 0x0f
                } else {
                    value
                };
            }
            NCO1CON => {
                let was_enabled = state.registers[NCO1CON] & NCO1EN != 0;
                state.update_nco(at);
                let output = state.registers[NCO1CON] & NCO1OUT;
                state.registers[NCO1CON] = (value & (NCO1EN | NCO1POL | NCO1PFM)) | output;
                if !was_enabled && state.registers[NCO1CON] & NCO1EN != 0 {
                    state.nco_epoch = at.ticks();
                }
                state.update_nco(at);
            }
            NCO1CLK => {
                state.update_nco(at);
                state.registers[address] = value & 0xef;
            }
            PIR7 => {
                state.registers[address] = value;
            }
            PIE7 => {
                state.registers[address] = value & NCO1IE;
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
    fn nco1_accumulates_and_routes_overflow_interrupt() {
        let hub = SignalHub::new();
        let (mut device, handle, _ports) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
        device
            .write(NCO1INCU as u64, AccessWidth::Byte, 0x0f, SimTime::ZERO)
            .unwrap();
        device
            .write(NCO1INCH as u64, AccessWidth::Byte, 0xff, SimTime::ZERO)
            .unwrap();
        device
            .write(NCO1INCL as u64, AccessWidth::Byte, 0xff, SimTime::ZERO)
            .unwrap();
        device
            .write(PIE7 as u64, AccessWidth::Byte, NCO1IE.into(), SimTime::ZERO)
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
                NCO1CON as u64,
                AccessWidth::Byte,
                NCO1EN.into(),
                SimTime::ZERO,
            )
            .unwrap();

        assert!(!handle.nco1_output());
        assert!(handle.poll(SimTime::from_ticks(2)));
        assert!(handle.nco1_output());
        assert_eq!(
            device
                .read(PIR7 as u64, AccessWidth::Byte, SimTime::from_ticks(2))
                .unwrap() as u8
                & NCO1IF,
            NCO1IF
        );
    }
}
