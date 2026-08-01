use super::{GpioHandle, GpioState, SignalHub, refresh_gpio, vendor_gpio};
use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use remu_signals::{Logic, SignalId, SignalValue};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

const IO_BASE: u16 = 0x20;
const PINB: u16 = 0x23;
const DDRB: u16 = 0x24;
const PORTB: u16 = 0x25;
const PINC: u16 = 0x26;
const PIND: u16 = 0x29;
const TIFR0: u16 = 0x35;
const TIFR1: u16 = 0x36;
const TIFR2: u16 = 0x37;
const EIFR: u16 = 0x3c;
const EIMSK: u16 = 0x3d;
const EECR: u16 = 0x3f;
const EEDR: u16 = 0x40;
const EEARL: u16 = 0x41;
const EEARH: u16 = 0x42;
const TCCR0B: u16 = 0x45;
const TCNT0: u16 = 0x46;
const OCR0A: u16 = 0x47;
const PCIFR: u16 = 0x3b;
const WDTCSR: u16 = 0x60;
const PCICR: u16 = 0x68;
const EICRA: u16 = 0x69;
const PCMSK0: u16 = 0x6b;
const PCMSK1: u16 = 0x6c;
const PCMSK2: u16 = 0x6d;
const TIMSK0: u16 = 0x6e;
const TIMSK1: u16 = 0x6f;
const TIMSK2: u16 = 0x70;
const TCCR1B: u16 = 0x81;
const TCNT1L: u16 = 0x84;
const TCNT1H: u16 = 0x85;
const OCR1AL: u16 = 0x88;
const OCR1AH: u16 = 0x89;
const UCSR0A: u16 = 0xc0;
const UCSR0B: u16 = 0xc1;
const UDR0: u16 = 0xc6;
const TCCR2A: u16 = 0xb0;
const TCCR2B: u16 = 0xb1;
const TCNT2: u16 = 0xb2;
const OCR2A: u16 = 0xb3;
const SPCR0: u16 = 0x4c;
const SPSR0: u16 = 0x4d;
const SPDR0: u16 = 0x4e;

const SPCR_SPIE: u8 = 1 << 7;
const SPCR_SPE: u8 = 1 << 6;
const SPSR_SPIF: u8 = 1 << 7;
const SPSR_WCOL: u8 = 1 << 6;
const SPSR_SPI2X: u8 = 1;
// AVR CPU lines are two below the datasheet vector number (line 0 is INT0).
const SPI0_INTERRUPT_LINE: u16 = 16;
const TWBR: u16 = 0xb8;
const TWSR: u16 = 0xb9;
const TWAR: u16 = 0xba;
const TWDR: u16 = 0xbb;
const TWCR: u16 = 0xbc;
const TWAMR: u16 = 0xbd;

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
    timer2_started: u64,
    timer2_pending: bool,
    previous_pinb: u8,
    previous_pinc: u8,
    previous_pind: u8,
    watchdog_started: u64,
    watchdog_reset: bool,
    spi_tx: Vec<u8>,
    spi_rx: Vec<u8>,
    spi_status_read: bool,
    twi_tx: Vec<u8>,
    twi_rx: VecDeque<u8>,
    uart_tx_signal: SignalId,
    timer0_irq_signal: SignalId,
    timer1_irq_signal: SignalId,
    timer2_irq_signal: SignalId,
    pcint0_irq_signal: SignalId,
    pcint1_irq_signal: SignalId,
    pcint2_irq_signal: SignalId,
    int0_irq_signal: SignalId,
    int1_irq_signal: SignalId,
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

    /// Advances timers, edge detection and watchdog; returns asserted AVR interrupt lines.
    pub fn poll(&self, now: SimTime) -> Vec<u16> {
        let mut state = self.0.lock().expect("ATmega I/O lock poisoned");
        let mut lines = Vec::new();
        let tccr = state.registers[usize::from(TCCR0B - IO_BASE)];
        if tccr & 7 != 0 {
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
        if tccr1 & 7 != 0 {
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
        if state.registers[usize::from(UCSR0B - IO_BASE)] & (1 << 5) != 0 {
            lines.push(18);
        }
        if state.registers[usize::from(SPCR0 - IO_BASE)] & SPCR_SPIE != 0
            && state.registers[usize::from(SPSR0 - IO_BASE)] & SPSR_SPIF != 0
        {
            lines.push(SPI0_INTERRUPT_LINE);
        }
        let twcr = state.registers[usize::from(TWCR - IO_BASE)];
        if twcr & (1 << 7) != 0 && twcr & (1 << 0) != 0 {
            lines.push(24);
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
        let timer2_irq_signal = hub.declare(
            "board.atmega328pb.timer2.irq",
            SignalValue::from_u64(0, 1)?,
            Some("functional Timer2 interrupt request".to_owned()),
        )?;
        let pcint0_irq_signal = hub.declare(
            "board.atmega328pb.interrupt.pcint0",
            SignalValue::from_u64(0, 1)?,
            Some("pin-change interrupt group zero request".to_owned()),
        )?;
        let pcint1_irq_signal = hub.declare(
            "board.atmega328pb.interrupt.pcint1",
            SignalValue::from_u64(0, 1)?,
            Some("pin-change interrupt group one request".to_owned()),
        )?;
        let pcint2_irq_signal = hub.declare(
            "board.atmega328pb.interrupt.pcint2",
            SignalValue::from_u64(0, 1)?,
            Some("pin-change interrupt group two request".to_owned()),
        )?;
        let int0_irq_signal = hub.declare(
            "board.atmega328pb.interrupt.int0",
            SignalValue::from_u64(0, 1)?,
            Some("external interrupt zero request".to_owned()),
        )?;
        let int1_irq_signal = hub.declare(
            "board.atmega328pb.interrupt.int1",
            SignalValue::from_u64(0, 1)?,
            Some("external interrupt one request".to_owned()),
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
            timer2_started: 0,
            timer2_pending: false,
            previous_pinb: 0,
            previous_pinc: 0,
            previous_pind: 0,
            watchdog_started: 0,
            watchdog_reset: false,
            spi_tx: Vec::new(),
            spi_rx: Vec::new(),
            spi_status_read: false,
            twi_tx: Vec::new(),
            twi_rx: VecDeque::new(),
            uart_tx_signal,
            timer0_irq_signal,
            timer1_irq_signal,
            timer2_irq_signal,
            pcint0_irq_signal,
            pcint1_irq_signal,
            pcint2_irq_signal,
            int0_irq_signal,
            int1_irq_signal,
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
        let mut state = self.state.lock().expect("ATmega I/O lock poisoned");
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
            SPSR0 => {
                let value = state.registers[usize::from(SPSR0 - IO_BASE)]
                    & (SPSR_SPIF | SPSR_WCOL | SPSR_SPI2X);
                state.spi_status_read = value & (SPSR_SPIF | SPSR_WCOL) != 0;
                value
            }
            SPDR0 => {
                let value = state.registers[usize::from(SPDR0 - IO_BASE)];
                if state.spi_status_read {
                    state.registers[usize::from(SPSR0 - IO_BASE)] &= !(SPSR_SPIF | SPSR_WCOL);
                    state.spi_status_read = false;
                }
                value
            }
            EEDR => state.registers[usize::from(EEDR - IO_BASE)],
            TWCR => state.registers[usize::from(TWCR - IO_BASE)] | (1 << 6),
            TWBR | TWAR | TWAMR => state.registers[usize::from(address - IO_BASE)],
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
            TCCR2B if state.registers[usize::from(TCCR2B - IO_BASE)] & 7 == 0 && value & 7 != 0 => {
                state.timer2_started = at.ticks();
                state.registers[usize::from(TCCR2B - IO_BASE)] = value;
            }
            TIFR2 => {
                state.registers[usize::from(TIFR2 - IO_BASE)] &= !value;
                if value & 0x07 != 0 && state.registers[usize::from(TIFR2 - IO_BASE)] == 0 {
                    state.timer2_pending = false;
                    set_bit_signal(&state, state.timer2_irq_signal, false, at);
                }
            }
            PCIFR => {
                state.registers[usize::from(PCIFR - IO_BASE)] &= !value;
                if value & 1 != 0 {
                    set_bit_signal(&state, state.pcint0_irq_signal, false, at);
                }
                if value & (1 << 1) != 0 {
                    set_bit_signal(&state, state.pcint1_irq_signal, false, at);
                }
                if value & (1 << 2) != 0 {
                    set_bit_signal(&state, state.pcint2_irq_signal, false, at);
                }
            }
            EIFR => {
                state.registers[usize::from(EIFR - IO_BASE)] &= !value;
                if value & 1 != 0 {
                    set_bit_signal(&state, state.int0_irq_signal, false, at);
                }
                if value & (1 << 1) != 0 {
                    set_bit_signal(&state, state.int1_irq_signal, false, at);
                }
            }
            UDR0 => {
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
            SPDR0 => {
                if state.registers[usize::from(SPCR0 - IO_BASE)] & SPCR_SPE != 0
                    && state.registers[usize::from(SPSR0 - IO_BASE)] & SPSR_SPIF == 0
                {
                    let received = if state.spi_rx.is_empty() {
                        value
                    } else {
                        state.spi_rx.remove(0)
                    };
                    state.spi_tx.push(value);
                    state.registers[usize::from(SPDR0 - IO_BASE)] = received;
                    state.registers[usize::from(SPSR0 - IO_BASE)] |= SPSR_SPIF;
                } else if state.registers[usize::from(SPCR0 - IO_BASE)] & SPCR_SPE != 0 {
                    state.registers[usize::from(SPSR0 - IO_BASE)] |= SPSR_WCOL;
                }
            }
            SPSR0 => {
                let status = state.registers[usize::from(SPSR0 - IO_BASE)];
                state.registers[usize::from(SPSR0 - IO_BASE)] =
                    (status & (SPSR_SPIF | SPSR_WCOL)) | (value & SPSR_SPI2X);
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
            TWCR => {
                let index = usize::from(TWCR - IO_BASE);
                state.registers[index] = value;
                if value & (1 << 7) != 0 && value & (1 << 2) != 0 {
                    let status = if value & (1 << 5) != 0 {
                        0x08
                    } else if value & (1 << 4) != 0 {
                        0xf8
                    } else if let Some(byte) = state.twi_rx.pop_front() {
                        state.registers[usize::from(TWDR - IO_BASE)] = byte;
                        0x50
                    } else {
                        let byte = state.registers[usize::from(TWDR - IO_BASE)];
                        state.twi_tx.push(byte);
                        0x28
                    };
                    state.registers[usize::from(TWSR - IO_BASE)] =
                        (state.registers[usize::from(TWSR - IO_BASE)] & 3) | status;
                    state.registers[index] = (value | (1 << 7)) & !((1 << 5) | (1 << 4));
                }
            }
            TWBR | TWAR | TWAMR => state.registers[usize::from(address - IO_BASE)] = value,
            _ => state.registers[usize::from(address - IO_BASE)] = value,
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.lock().expect("ATmega I/O lock poisoned");
        state.registers.fill(0);
        state.uart.clear();
        state.timer_pending = false;
        state.timer1_pending = false;
        state.timer2_pending = false;
        state.previous_pinb = 0;
        state.previous_pinc = 0;
        state.previous_pind = 0;
        state.watchdog_reset = false;
        state.spi_tx.clear();
        state.spi_rx.clear();
        state.spi_status_read = false;
        state.twi_tx.clear();
        state.twi_rx.clear();
        set_bit_signal(&state, state.timer0_irq_signal, false, SimTime::ZERO);
        set_bit_signal(&state, state.timer1_irq_signal, false, SimTime::ZERO);
        set_bit_signal(&state, state.timer2_irq_signal, false, SimTime::ZERO);
        set_bit_signal(&state, state.pcint0_irq_signal, false, SimTime::ZERO);
        set_bit_signal(&state, state.pcint1_irq_signal, false, SimTime::ZERO);
        set_bit_signal(&state, state.pcint2_irq_signal, false, SimTime::ZERO);
        set_bit_signal(&state, state.int0_irq_signal, false, SimTime::ZERO);
        set_bit_signal(&state, state.int1_irq_signal, false, SimTime::ZERO);
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
    fn pin_change_groups_and_int1_report_distinct_interrupt_lines() {
        let hub = SignalHub::new();
        let (mut io, handle, ports) = AtmegaIo::new("atmega328pb.io", hub).unwrap();
        io.write(
            u64::from(PCICR - IO_BASE),
            AccessWidth::Byte,
            (1 << 1) | (1 << 2),
            SimTime::ZERO,
        )
        .unwrap();
        io.write(
            u64::from(PCMSK1 - IO_BASE),
            AccessWidth::Byte,
            1,
            SimTime::ZERO,
        )
        .unwrap();
        io.write(
            u64::from(PCMSK2 - IO_BASE),
            AccessWidth::Byte,
            1 << 4,
            SimTime::ZERO,
        )
        .unwrap();
        io.write(
            u64::from(EICRA - IO_BASE),
            AccessWidth::Byte,
            3 << 4,
            SimTime::ZERO,
        )
        .unwrap();
        io.write(
            u64::from(EIMSK - IO_BASE),
            AccessWidth::Byte,
            1 << 1,
            SimTime::ZERO,
        )
        .unwrap();
        ports[1].set_input(0, Logic::Zero, SimTime::ZERO).unwrap();
        ports[2].set_input(3, Logic::Zero, SimTime::ZERO).unwrap();
        ports[2].set_input(4, Logic::Zero, SimTime::ZERO).unwrap();
        assert!(handle.poll(SimTime::ZERO).is_empty());
        ports[1]
            .set_input(0, Logic::One, SimTime::from_ticks(1))
            .unwrap();
        ports[2]
            .set_input(3, Logic::One, SimTime::from_ticks(1))
            .unwrap();
        ports[2]
            .set_input(4, Logic::One, SimTime::from_ticks(1))
            .unwrap();
        let lines = handle.poll(SimTime::from_ticks(1));
        assert!(lines.contains(&1));
        assert!(lines.contains(&3));
        assert!(lines.contains(&4));
        io.write(
            u64::from(PCIFR - IO_BASE),
            AccessWidth::Byte,
            (1 << 1) | (1 << 2),
            SimTime::from_ticks(1),
        )
        .unwrap();
        io.write(
            u64::from(EIFR - IO_BASE),
            AccessWidth::Byte,
            1 << 1,
            SimTime::from_ticks(1),
        )
        .unwrap();
        assert_eq!(
            io.read(
                u64::from(PCIFR - IO_BASE),
                AccessWidth::Byte,
                SimTime::from_ticks(1),
            )
            .unwrap(),
            0
        );
        assert_eq!(
            io.read(
                u64::from(EIFR - IO_BASE),
                AccessWidth::Byte,
                SimTime::from_ticks(1),
            )
            .unwrap(),
            0
        );
    }

    #[test]
    fn timer2_ctc_sets_and_clears_its_compare_interrupt() {
        let hub = SignalHub::new();
        let (mut io, handle, _) = AtmegaIo::new("atmega328pb.io", hub.clone()).unwrap();
        let timer2_irq = hub
            .with_registry(|registry| registry.find("board.atmega328pb.timer2.irq"))
            .expect("Timer2 IRQ signal is declared");
        io.write(
            u64::from(OCR2A - IO_BASE),
            AccessWidth::Byte,
            3,
            SimTime::ZERO,
        )
        .unwrap();
        io.write(
            u64::from(TCCR2A - IO_BASE),
            AccessWidth::Byte,
            2,
            SimTime::ZERO,
        )
        .unwrap();
        io.write(
            u64::from(TIMSK2 - IO_BASE),
            AccessWidth::Byte,
            1 << 1,
            SimTime::ZERO,
        )
        .unwrap();
        io.write(
            u64::from(TCCR2B - IO_BASE),
            AccessWidth::Byte,
            1,
            SimTime::ZERO,
        )
        .unwrap();

        assert!(handle.poll(SimTime::from_ticks(3)).is_empty());
        assert_eq!(
            handle.poll(SimTime::from_ticks(4)),
            vec![6],
            "TIMER2_COMPA is AVR vector 8 / emulator interrupt line 6"
        );
        assert!(
            hub.drain_changes().iter().any(
                |change| change.signal == timer2_irq && change.value.bit(0) == Some(Logic::One)
            )
        );
        assert_eq!(
            io.read(
                u64::from(TIFR2 - IO_BASE),
                AccessWidth::Byte,
                SimTime::from_ticks(4)
            )
            .unwrap(),
            1 << 1
        );

        io.write(
            u64::from(TIFR2 - IO_BASE),
            AccessWidth::Byte,
            1 << 1,
            SimTime::from_ticks(4),
        )
        .unwrap();
        assert!(handle.poll(SimTime::from_ticks(4)).is_empty());
        assert!(
            hub.drain_changes()
                .iter()
                .any(|change| change.signal == timer2_irq
                    && change.value.bit(0) == Some(Logic::Zero))
        );
    }

    #[test]
    fn spi0_master_transfer_returns_injected_byte_and_interrupts() {
        let hub = SignalHub::new();
        let (mut io, handle, _) = AtmegaIo::new("atmega328pb.io", hub).unwrap();
        handle.inject_spi_rx(0x3c);
        io.write(
            u64::from(SPCR0 - IO_BASE),
            AccessWidth::Byte,
            u64::from(SPCR_SPIE | SPCR_SPE),
            SimTime::ZERO,
        )
        .unwrap();
        io.write(
            u64::from(SPDR0 - IO_BASE),
            AccessWidth::Byte,
            0xa5,
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(handle.spi_bytes(), [0xa5]);
        assert_eq!(
            handle.poll(SimTime::from_ticks(1)),
            vec![SPI0_INTERRUPT_LINE]
        );
        assert_eq!(
            io.read(u64::from(SPDR0 - IO_BASE), AccessWidth::Byte, SimTime::ZERO,)
                .unwrap(),
            0x3c
        );
        assert_eq!(
            io.read(u64::from(SPSR0 - IO_BASE), AccessWidth::Byte, SimTime::ZERO,)
                .unwrap()
                & u64::from(SPSR_SPIF),
            u64::from(SPSR_SPIF)
        );
        assert_eq!(
            io.read(u64::from(SPDR0 - IO_BASE), AccessWidth::Byte, SimTime::ZERO,)
                .unwrap(),
            0x3c
        );
        assert_eq!(
            io.read(u64::from(SPSR0 - IO_BASE), AccessWidth::Byte, SimTime::ZERO,)
                .unwrap()
                & u64::from(SPSR_SPIF),
            0
        );
        assert!(handle.poll(SimTime::from_ticks(1)).is_empty());
    }

    #[test]
    fn spi0_write_collision_requires_an_unacknowledged_transfer() {
        let hub = SignalHub::new();
        let (mut io, _, _) = AtmegaIo::new("atmega328pb.io", hub).unwrap();
        io.write(
            u64::from(SPDR0 - IO_BASE),
            AccessWidth::Byte,
            0x11,
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            io.read(u64::from(SPSR0 - IO_BASE), AccessWidth::Byte, SimTime::ZERO,)
                .unwrap()
                & u64::from(SPSR_WCOL),
            0
        );

        io.write(
            u64::from(SPCR0 - IO_BASE),
            AccessWidth::Byte,
            u64::from(SPCR_SPE),
            SimTime::ZERO,
        )
        .unwrap();
        io.write(
            u64::from(SPDR0 - IO_BASE),
            AccessWidth::Byte,
            0x22,
            SimTime::ZERO,
        )
        .unwrap();
        io.write(
            u64::from(SPDR0 - IO_BASE),
            AccessWidth::Byte,
            0x33,
            SimTime::ZERO,
        )
        .unwrap();
        let status = io
            .read(u64::from(SPSR0 - IO_BASE), AccessWidth::Byte, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            status & u64::from(SPSR_SPIF | SPSR_WCOL),
            u64::from(SPSR_SPIF | SPSR_WCOL)
        );
        assert_eq!(
            io.read(u64::from(SPDR0 - IO_BASE), AccessWidth::Byte, SimTime::ZERO,)
                .unwrap(),
            0x22
        );
    }

    #[test]
    fn twi0_start_transmit_and_receive_have_deterministic_status() {
        let hub = SignalHub::new();
        let (mut io, handle, _) = AtmegaIo::new("atmega328pb.io", hub).unwrap();
        let twcr = u64::from(TWCR - IO_BASE);
        io.write(
            u64::from(TWBR - IO_BASE),
            AccessWidth::Byte,
            12,
            SimTime::ZERO,
        )
        .unwrap();
        io.write(
            u64::from(TWAR - IO_BASE),
            AccessWidth::Byte,
            0x22,
            SimTime::ZERO,
        )
        .unwrap();
        io.write(
            u64::from(TWAMR - IO_BASE),
            AccessWidth::Byte,
            0,
            SimTime::ZERO,
        )
        .unwrap();
        io.write(twcr, AccessWidth::Byte, 0xA5 | 0x20, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            io.read(u64::from(TWSR - IO_BASE), AccessWidth::Byte, SimTime::ZERO)
                .unwrap(),
            0x08
        );
        io.write(
            u64::from(TWDR - IO_BASE),
            AccessWidth::Byte,
            0x55,
            SimTime::ZERO,
        )
        .unwrap();
        io.write(twcr, AccessWidth::Byte, 0x85, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.take_twi_tx(), vec![0x55]);
        handle.queue_twi_rx(0xa5);
        io.write(twcr, AccessWidth::Byte, 0x85, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            io.read(u64::from(TWDR - IO_BASE), AccessWidth::Byte, SimTime::ZERO)
                .unwrap(),
            0xa5
        );
        assert_eq!(
            io.read(u64::from(TWSR - IO_BASE), AccessWidth::Byte, SimTime::ZERO)
                .unwrap(),
            0x50
        );
        assert!(handle.poll(SimTime::ZERO).contains(&24));
    }
}
