use super::{GpioHandle, GpioState, SignalHub, refresh_gpio, vendor_gpio};
use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use remu_signals::{Logic, SignalId, SignalValue};
use std::sync::{Arc, Mutex};

const IO_BASE: u16 = 0x20;
const PINB: u16 = 0x23;
const DDRB: u16 = 0x24;
const PORTB: u16 = 0x25;
const PINC: u16 = 0x26;
const PIND: u16 = 0x29;
const TIFR0: u16 = 0x35;
const TIFR1: u16 = 0x36;
const EIFR: u16 = 0x3c;
const EIMSK: u16 = 0x3d;
const EECR: u16 = 0x3f;
const EEDR: u16 = 0x40;
const EEARL: u16 = 0x41;
const EEARH: u16 = 0x42;
// ATmega328PB power-management registers (DS40001906C §13.12).
const SMCR: u16 = 0x53;
const CLKPR: u16 = 0x61;
const PRR0: u16 = 0x64;
const PRR1: u16 = 0x65;
const TCCR0B: u16 = 0x45;
const TCNT0: u16 = 0x46;
const OCR0A: u16 = 0x47;
const PCIFR: u16 = 0x3b;
const WDTCSR: u16 = 0x60;
const PCICR: u16 = 0x68;
const EICRA: u16 = 0x69;
const PCMSK0: u16 = 0x6b;
const TIMSK0: u16 = 0x6e;
const TIMSK1: u16 = 0x6f;
const TCCR1B: u16 = 0x81;
const TCNT1L: u16 = 0x84;
const TCNT1H: u16 = 0x85;
const OCR1AL: u16 = 0x88;
const OCR1AH: u16 = 0x89;
const UCSR0A: u16 = 0xc0;
const UCSR0B: u16 = 0xc1;
const UDR0: u16 = 0xc6;

const SMCR_WRITABLE_MASK: u8 = 0x0f;
const CLKPR_CHANGE_ENABLE: u8 = 1 << 7;
const CLKPR_DIVIDER_MASK: u8 = 0x0f;
const PRR1_WRITABLE_MASK: u8 = 0x3d;
const PRR0_PRTIM0: u8 = 1 << 5;
const PRR0_PRTIM1: u8 = 1 << 3;
const PRR0_PRUSART0: u8 = 1 << 1;

struct AtmegaState {
    registers: [u8; 224],
    ports: [Arc<Mutex<GpioState>>; 3],
    port_signals: [Vec<SignalId>; 3],
    hub: SignalHub,
    uart: Vec<u8>,
    eeprom: Vec<u8>,
    timer_started: u64,
    timer_pending: bool,
    timer1_started: u64,
    timer1_pending: bool,
    previous_pinb: u8,
    previous_pind: u8,
    clock_prescaler_armed_at: Option<u64>,
    watchdog_started: u64,
    watchdog_reset: bool,
    uart_tx_signal: SignalId,
    timer0_irq_signal: SignalId,
    timer1_irq_signal: SignalId,
    pcint0_irq_signal: SignalId,
    int0_irq_signal: SignalId,
    watchdog_reset_signal: SignalId,
}

/// Machine-facing ATmega328PB I/O state.
#[derive(Clone)]
pub struct AtmegaIoHandle(Arc<Mutex<AtmegaState>>);

impl AtmegaIoHandle {
    /// Captured USART0 bytes.
    pub fn uart_bytes(&self) -> Vec<u8> {
        self.0
            .lock()
            .expect("ATmega I/O lock poisoned")
            .uart
            .clone()
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
        if prr0 & PRR0_PRUSART0 == 0
            && state.registers[usize::from(UCSR0B - IO_BASE)] & (1 << 5) != 0
        {
            lines.push(18);
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
        let pind = resolved(&state.ports[2]);
        let old_int0 = state.previous_pind & (1 << 2) != 0;
        let new_int0 = pind & (1 << 2) != 0;
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
        if state.registers[usize::from(WDTCSR - IO_BASE)] & (1 << 3) != 0
            && now.ticks().saturating_sub(state.watchdog_started) >= 2048
        {
            state.watchdog_reset = true;
            set_bit_signal(&state, state.watchdog_reset_signal, true, now);
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
}

fn set_bit_signal(state: &AtmegaState, signal: SignalId, value: bool, at: SimTime) {
    state
        .hub
        .set(
            signal,
            SignalValue::from_u64(u64::from(value), 1).expect("one-bit signal is valid"),
            at,
        )
        .expect("ATmega signal identity and width are fixed at construction");
}

fn resolved(state: &Arc<Mutex<GpioState>>) -> u8 {
    state
        .lock()
        .expect("ATmega GPIO lock poisoned")
        .nets
        .iter()
        .enumerate()
        .fold(0_u8, |value, (pin, net)| {
            value | (u8::from(net.resolved() == Logic::One) << pin)
        })
}

/// Unified ATmega328PB memory-mapped I/O window.
pub struct AtmegaIo {
    name: String,
    state: Arc<Mutex<AtmegaState>>,
}

impl AtmegaIo {
    /// Creates the PB-specific I/O block and package GPIO handles B/C/D.
    pub fn new(
        name: impl Into<String>,
        hub: SignalHub,
    ) -> Result<(Self, AtmegaIoHandle, [GpioHandle; 3]), remu_signals::SignalError> {
        let (portb, signals_b, handle_b) = vendor_gpio(8, "board.atmega328pb.portb", &hub)?;
        let (portc, signals_c, handle_c) = vendor_gpio(7, "board.atmega328pb.portc", &hub)?;
        let (portd, signals_d, handle_d) = vendor_gpio(8, "board.atmega328pb.portd", &hub)?;
        let uart_tx_signal = hub.declare(
            "board.atmega328pb.usart0.tx_byte",
            SignalValue::from_u64(0, 8)?,
            Some("last byte written to USART0 UDR".to_owned()),
        )?;
        let timer0_irq_signal = hub.declare(
            "board.atmega328pb.timer0.overflow_irq",
            SignalValue::from_u64(0, 1)?,
            Some("functional Timer0 overflow request".to_owned()),
        )?;
        let timer1_irq_signal = hub.declare(
            "board.atmega328pb.timer1.compare_a_irq",
            SignalValue::from_u64(0, 1)?,
            Some("functional Timer1 compare-A request".to_owned()),
        )?;
        let pcint0_irq_signal = hub.declare(
            "board.atmega328pb.interrupt.pcint0",
            SignalValue::from_u64(0, 1)?,
            Some("pin-change interrupt group zero request".to_owned()),
        )?;
        let int0_irq_signal = hub.declare(
            "board.atmega328pb.interrupt.int0",
            SignalValue::from_u64(0, 1)?,
            Some("external interrupt zero request".to_owned()),
        )?;
        let watchdog_reset_signal = hub.declare(
            "board.atmega328pb.watchdog.reset",
            SignalValue::from_u64(0, 1)?,
            Some("functional watchdog reset request".to_owned()),
        )?;
        let state = Arc::new(Mutex::new(AtmegaState {
            registers: [0; 224],
            ports: [portb, portc, portd],
            port_signals: [signals_b, signals_c, signals_d],
            hub,
            uart: Vec::new(),
            eeprom: vec![0xff; 1024],
            timer_started: 0,
            timer_pending: false,
            timer1_started: 0,
            timer1_pending: false,
            previous_pinb: 0,
            previous_pind: 0,
            clock_prescaler_armed_at: None,
            watchdog_started: 0,
            watchdog_reset: false,
            uart_tx_signal,
            timer0_irq_signal,
            timer1_irq_signal,
            pcint0_irq_signal,
            int0_irq_signal,
            watchdog_reset_signal,
        }));
        Ok((
            Self {
                name: name.into(),
                state: state.clone(),
            },
            AtmegaIoHandle(state),
            [handle_b, handle_c, handle_d],
        ))
    }

    fn refresh_port(state: &AtmegaState, port: usize, at: SimTime) -> Result<(), DeviceError> {
        let pins = if port == 1 { 7 } else { 8 };
        refresh_gpio(
            &state.ports[port],
            &state.port_signals[port],
            &state.hub,
            pins,
            at,
        )
    }

    fn port_register(address: u16) -> Option<(usize, u16)> {
        match address {
            PINB..=PORTB => Some((0, PINB)),
            PINC..=0x28 => Some((1, PINC)),
            PIND..=0x2b => Some((2, PIND)),
            _ => None,
        }
    }
}

impl Device for AtmegaIo {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Byte {
            return Err(DeviceError::new("ATmega I/O requires byte accesses"));
        }
        let address = IO_BASE + offset as u16;
        let state = self.state.lock().expect("ATmega I/O lock poisoned");
        if let Some((port, base)) = Self::port_register(address) {
            let value = match address - base {
                0 => resolved(&state.ports[port]),
                1 => {
                    state.ports[port]
                        .lock()
                        .expect("ATmega GPIO lock poisoned")
                        .direction as u8
                }
                _ => {
                    state.ports[port]
                        .lock()
                        .expect("ATmega GPIO lock poisoned")
                        .output as u8
                }
            };
            return Ok(u64::from(value));
        }
        let value = match address {
            UCSR0A => state.registers[usize::from(address - IO_BASE)] | (1 << 5) | (1 << 6),
            SMCR => state.registers[usize::from(SMCR - IO_BASE)] & SMCR_WRITABLE_MASK,
            CLKPR => {
                state.registers[usize::from(CLKPR - IO_BASE)]
                    & (CLKPR_CHANGE_ENABLE | CLKPR_DIVIDER_MASK)
            }
            PRR0 => state.registers[usize::from(PRR0 - IO_BASE)],
            PRR1 => state.registers[usize::from(PRR1 - IO_BASE)] & PRR1_WRITABLE_MASK,
            EEDR => state.registers[usize::from(EEDR - IO_BASE)],
            _ => state.registers[usize::from(address - IO_BASE)],
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
            return Err(DeviceError::new("ATmega I/O requires byte accesses"));
        }
        let address = IO_BASE + offset as u16;
        let value = value as u8;
        let mut state = self.state.lock().expect("ATmega I/O lock poisoned");
        if let Some((port, base)) = Self::port_register(address) {
            let mut gpio = state.ports[port].lock().expect("ATmega GPIO lock poisoned");
            match address - base {
                0 => gpio.output ^= u32::from(value),
                1 => gpio.direction = u32::from(value),
                _ => gpio.output = u32::from(value),
            }
            drop(gpio);
            return Self::refresh_port(&state, port, at);
        }
        match address {
            SMCR => {
                state.registers[usize::from(SMCR - IO_BASE)] = value & SMCR_WRITABLE_MASK;
            }
            CLKPR => {
                let index = usize::from(CLKPR - IO_BASE);
                if value & CLKPR_CHANGE_ENABLE != 0 {
                    if value & !CLKPR_CHANGE_ENABLE == 0 {
                        state.clock_prescaler_armed_at = Some(at.ticks());
                        state.registers[index] =
                            CLKPR_CHANGE_ENABLE | (state.registers[index] & CLKPR_DIVIDER_MASK);
                    }
                } else if value & !CLKPR_DIVIDER_MASK == 0 {
                    if state
                        .clock_prescaler_armed_at
                        .is_some_and(|armed| at.ticks().saturating_sub(armed) <= 4)
                    {
                        state.registers[index] = value & CLKPR_DIVIDER_MASK;
                    }
                    state.clock_prescaler_armed_at = None;
                }
            }
            PRR0 => {
                state.registers[usize::from(PRR0 - IO_BASE)] = value;
            }
            PRR1 => {
                state.registers[usize::from(PRR1 - IO_BASE)] = value & PRR1_WRITABLE_MASK;
            }
            TCCR0B if state.registers[usize::from(TCCR0B - IO_BASE)] & 7 == 0 && value & 7 != 0 => {
                state.timer_started = at.ticks();
                state.registers[usize::from(TCCR0B - IO_BASE)] = value;
            }
            TIFR0 => {
                state.registers[usize::from(TIFR0 - IO_BASE)] &= !value;
                if value & 1 != 0 {
                    state.timer_pending = false;
                    set_bit_signal(&state, state.timer0_irq_signal, false, at);
                }
            }
            TCCR1B if state.registers[usize::from(TCCR1B - IO_BASE)] & 7 == 0 && value & 7 != 0 => {
                state.timer1_started = at.ticks();
                state.registers[usize::from(TCCR1B - IO_BASE)] = value;
            }
            TIFR1 => {
                state.registers[usize::from(TIFR1 - IO_BASE)] &= !value;
                if value & (1 << 1) != 0 {
                    state.timer1_pending = false;
                    set_bit_signal(&state, state.timer1_irq_signal, false, at);
                }
            }
            PCIFR => {
                state.registers[usize::from(PCIFR - IO_BASE)] &= !value;
                if value & 1 != 0 {
                    set_bit_signal(&state, state.pcint0_irq_signal, false, at);
                }
            }
            EIFR => {
                state.registers[usize::from(EIFR - IO_BASE)] &= !value;
                if value & 1 != 0 {
                    set_bit_signal(&state, state.int0_irq_signal, false, at);
                }
            }
            UDR0 => {
                if state.registers[usize::from(PRR0 - IO_BASE)] & PRR0_PRUSART0 == 0 {
                    state.uart.push(value);
                    state
                        .hub
                        .set(
                            state.uart_tx_signal,
                            SignalValue::from_u64(u64::from(value), 8)
                                .expect("eight-bit signal is valid"),
                            at,
                        )
                        .expect("ATmega USART signal identity and width are fixed");
                }
            }
            EECR => {
                let address = usize::from(state.registers[usize::from(EEARL - IO_BASE)])
                    | (usize::from(state.registers[usize::from(EEARH - IO_BASE)] & 3) << 8);
                if value & 1 != 0 {
                    state.registers[usize::from(EEDR - IO_BASE)] = state.eeprom[address];
                }
                if value & 2 != 0 && state.registers[usize::from(EECR - IO_BASE)] & 4 != 0 {
                    state.eeprom[address] = state.registers[usize::from(EEDR - IO_BASE)];
                }
                state.registers[usize::from(EECR - IO_BASE)] = value & !2;
            }
            WDTCSR => {
                state.registers[usize::from(WDTCSR - IO_BASE)] = value;
                state.watchdog_started = at.ticks();
            }
            _ => state.registers[usize::from(address - IO_BASE)] = value,
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.lock().expect("ATmega I/O lock poisoned");
        state.registers.fill(0);
        state.uart.clear();
        state.clock_prescaler_armed_at = None;
        state.timer_pending = false;
        state.timer1_pending = false;
        state.watchdog_reset = false;
        set_bit_signal(&state, state.timer0_irq_signal, false, SimTime::ZERO);
        set_bit_signal(&state, state.timer1_irq_signal, false, SimTime::ZERO);
        set_bit_signal(&state, state.pcint0_irq_signal, false, SimTime::ZERO);
        set_bit_signal(&state, state.int0_irq_signal, false, SimTime::ZERO);
        set_bit_signal(&state, state.watchdog_reset_signal, false, SimTime::ZERO);
        for port in &state.ports {
            let mut port = port.lock().expect("ATmega GPIO lock poisoned");
            port.direction = 0;
            port.output = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pb_ports_uart_timer_and_persistent_eeprom_are_functional() {
        let hub = SignalHub::new();
        let (mut io, handle, ports) = AtmegaIo::new("atmega328pb.io", hub).unwrap();
        io.write(
            u64::from(DDRB - IO_BASE),
            AccessWidth::Byte,
            1,
            SimTime::ZERO,
        )
        .unwrap();
        io.write(
            u64::from(PORTB - IO_BASE),
            AccessWidth::Byte,
            1,
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(ports[0].output(), 1);
        io.write(
            u64::from(UDR0 - IO_BASE),
            AccessWidth::Byte,
            b'A'.into(),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(handle.uart_bytes(), b"A");
        io.write(
            u64::from(OCR0A - IO_BASE),
            AccessWidth::Byte,
            3,
            SimTime::ZERO,
        )
        .unwrap();
        io.write(
            u64::from(TIMSK0 - IO_BASE),
            AccessWidth::Byte,
            1,
            SimTime::ZERO,
        )
        .unwrap();
        io.write(
            u64::from(TCCR0B - IO_BASE),
            AccessWidth::Byte,
            1,
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(handle.poll(SimTime::from_ticks(4)), vec![15]);
    }

    #[test]
    fn power_registers_apply_masks_and_clkpr_authorization_window() {
        let hub = SignalHub::new();
        let (mut io, handle, _) = AtmegaIo::new("atmega328pb.io", hub).unwrap();

        assert_eq!(handle.clock_divider(), 1);
        assert!(!handle.sleep_enabled());
        assert_eq!(handle.sleep_mode(), 0);

        io.write(
            u64::from(SMCR - IO_BASE),
            AccessWidth::Byte,
            0xff,
            SimTime::ZERO,
        )
        .unwrap();
        assert!(handle.sleep_enabled());
        assert_eq!(handle.sleep_mode(), 7);
        assert_eq!(
            io.read(u64::from(SMCR - IO_BASE), AccessWidth::Byte, SimTime::ZERO),
            Ok(0x0f)
        );

        // CLKPR writes without CLKPCE are ignored.
        io.write(
            u64::from(CLKPR - IO_BASE),
            AccessWidth::Byte,
            0x0f,
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(handle.clock_divider(), 1);
        io.write(
            u64::from(CLKPR - IO_BASE),
            AccessWidth::Byte,
            CLKPR_CHANGE_ENABLE.into(),
            SimTime::ZERO,
        )
        .unwrap();
        io.write(
            u64::from(CLKPR - IO_BASE),
            AccessWidth::Byte,
            2,
            SimTime::from_ticks(1),
        )
        .unwrap();
        assert_eq!(handle.clock_divider(), 4);

        // A second write after the four-tick authorization window is ignored.
        io.write(
            u64::from(CLKPR - IO_BASE),
            AccessWidth::Byte,
            CLKPR_CHANGE_ENABLE.into(),
            SimTime::from_ticks(2),
        )
        .unwrap();
        io.write(
            u64::from(CLKPR - IO_BASE),
            AccessWidth::Byte,
            4,
            SimTime::from_ticks(7),
        )
        .unwrap();
        assert_eq!(handle.clock_divider(), 4);

        io.write(
            u64::from(PRR0 - IO_BASE),
            AccessWidth::Byte,
            0xff,
            SimTime::from_ticks(8),
        )
        .unwrap();
        io.write(
            u64::from(PRR1 - IO_BASE),
            AccessWidth::Byte,
            0xff,
            SimTime::from_ticks(8),
        )
        .unwrap();
        assert_eq!(
            io.read(u64::from(PRR0 - IO_BASE), AccessWidth::Byte, SimTime::ZERO),
            Ok(0xff)
        );
        assert_eq!(
            io.read(u64::from(PRR1 - IO_BASE), AccessWidth::Byte, SimTime::ZERO),
            Ok(u64::from(PRR1_WRITABLE_MASK))
        );
    }

    #[test]
    fn power_reduction_gates_timer_and_uart_facades() {
        let hub = SignalHub::new();
        let (mut io, handle, _) = AtmegaIo::new("atmega328pb.io", hub).unwrap();
        io.write(
            u64::from(OCR0A - IO_BASE),
            AccessWidth::Byte,
            3,
            SimTime::ZERO,
        )
        .unwrap();
        io.write(
            u64::from(TIMSK0 - IO_BASE),
            AccessWidth::Byte,
            1,
            SimTime::ZERO,
        )
        .unwrap();
        io.write(
            u64::from(TCCR0B - IO_BASE),
            AccessWidth::Byte,
            1,
            SimTime::ZERO,
        )
        .unwrap();
        io.write(
            u64::from(PRR0 - IO_BASE),
            AccessWidth::Byte,
            PRR0_PRTIM0.into(),
            SimTime::ZERO,
        )
        .unwrap();
        assert!(handle.poll(SimTime::from_ticks(4)).is_empty());
        io.write(
            u64::from(PRR0 - IO_BASE),
            AccessWidth::Byte,
            0,
            SimTime::from_ticks(4),
        )
        .unwrap();
        assert_eq!(handle.poll(SimTime::from_ticks(8)), vec![15]);

        let hub = SignalHub::new();
        let (mut io, handle, _) = AtmegaIo::new("atmega328pb.io", hub).unwrap();
        io.write(
            u64::from(PRR0 - IO_BASE),
            AccessWidth::Byte,
            PRR0_PRUSART0.into(),
            SimTime::ZERO,
        )
        .unwrap();
        io.write(
            u64::from(UDR0 - IO_BASE),
            AccessWidth::Byte,
            u64::from(b'X'),
            SimTime::ZERO,
        )
        .unwrap();
        assert!(handle.uart_bytes().is_empty());
        io.write(
            u64::from(PRR0 - IO_BASE),
            AccessWidth::Byte,
            0,
            SimTime::from_ticks(1),
        )
        .unwrap();
        io.write(
            u64::from(UDR0 - IO_BASE),
            AccessWidth::Byte,
            u64::from(b'Y'),
            SimTime::from_ticks(1),
        )
        .unwrap();
        assert_eq!(handle.uart_bytes(), b"Y");
    }
}
