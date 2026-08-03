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

/// Named ATmega328PB Timer/Counter3 and Timer/Counter4 register identifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[repr(u16)]
pub enum AtmegaTimerRegister {
    /// Timer/Counter3 interrupt flags (TIFR3).
    Tifr3 = 0x38,
    /// Timer/Counter4 interrupt flags (TIFR4).
    Tifr4 = 0x39,
    /// Timer/Counter3 interrupt mask (TIMSK3).
    Timsk3 = 0x71,
    /// Timer/Counter4 interrupt mask (TIMSK4).
    Timsk4 = 0x72,
    /// Timer/Counter3 control register B (TCCR3B).
    Tccr3b = 0x91,
    /// Timer/Counter3 counter low byte (TCNT3L).
    Tcnt3l = 0x94,
    /// Timer/Counter3 counter high byte (TCNT3H).
    Tcnt3h = 0x95,
    /// Timer/Counter3 output compare A low byte (OCR3AL).
    Ocr3al = 0x98,
    /// Timer/Counter3 output compare A high byte (OCR3AH).
    Ocr3ah = 0x99,
    /// Timer/Counter4 control register B (TCCR4B).
    Tccr4b = 0xa1,
    /// Timer/Counter4 counter low byte (TCNT4L).
    Tcnt4l = 0xa4,
    /// Timer/Counter4 counter high byte (TCNT4H).
    Tcnt4h = 0xa5,
    /// Timer/Counter4 output compare A low byte (OCR4AL).
    Ocr4al = 0xa8,
    /// Timer/Counter4 output compare A high byte (OCR4AH).
    Ocr4ah = 0xa9,
}

impl AtmegaTimerRegister {
    /// Stable list of modeled Timer3/Timer4 register IDs.
    pub const ALL: [Self; 14] = [
        Self::Tifr3,
        Self::Tifr4,
        Self::Timsk3,
        Self::Timsk4,
        Self::Tccr3b,
        Self::Tcnt3l,
        Self::Tcnt3h,
        Self::Ocr3al,
        Self::Ocr3ah,
        Self::Tccr4b,
        Self::Tcnt4l,
        Self::Tcnt4h,
        Self::Ocr4al,
        Self::Ocr4ah,
    ];

    /// Returns the native data-space address.
    pub const fn offset(self) -> u16 {
        self as u16
    }

    /// Returns the I/O-device offset used by `AtmegaIo`.
    pub const fn io_offset(self) -> u16 {
        self.offset() - IO_BASE
    }

    /// Returns the register-array index used by `AtmegaIo`.
    pub const fn index(self) -> usize {
        self.io_offset() as usize
    }

    /// Returns the vendor register name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Tifr3 => "tifr3",
            Self::Tifr4 => "tifr4",
            Self::Timsk3 => "timsk3",
            Self::Timsk4 => "timsk4",
            Self::Tccr3b => "tccr3b",
            Self::Tcnt3l => "tcnt3l",
            Self::Tcnt3h => "tcnt3h",
            Self::Ocr3al => "ocr3al",
            Self::Ocr3ah => "ocr3ah",
            Self::Tccr4b => "tccr4b",
            Self::Tcnt4l => "tcnt4l",
            Self::Tcnt4h => "tcnt4h",
            Self::Ocr4al => "ocr4al",
            Self::Ocr4ah => "ocr4ah",
        }
    }

    /// Resolves a native data-space address to a named Timer3/Timer4 register.
    pub const fn from_data_address(address: u16) -> Option<Self> {
        match address {
            0x38 => Some(Self::Tifr3),
            0x39 => Some(Self::Tifr4),
            0x71 => Some(Self::Timsk3),
            0x72 => Some(Self::Timsk4),
            0x91 => Some(Self::Tccr3b),
            0x94 => Some(Self::Tcnt3l),
            0x95 => Some(Self::Tcnt3h),
            0x98 => Some(Self::Ocr3al),
            0x99 => Some(Self::Ocr3ah),
            0xa1 => Some(Self::Tccr4b),
            0xa4 => Some(Self::Tcnt4l),
            0xa5 => Some(Self::Tcnt4h),
            0xa8 => Some(Self::Ocr4al),
            0xa9 => Some(Self::Ocr4ah),
            _ => None,
        }
    }
}

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
    timer3_started: u64,
    timer3_base: u64,
    timer3_elapsed: u64,
    timer3_compare_pending: bool,
    timer3_overflow_pending: bool,
    timer4_started: u64,
    timer4_base: u64,
    timer4_elapsed: u64,
    timer4_compare_pending: bool,
    timer4_overflow_pending: bool,
    previous_pinb: u8,
    previous_pind: u8,
    watchdog_started: u64,
    watchdog_reset: bool,
    uart_tx_signal: SignalId,
    timer0_irq_signal: SignalId,
    timer1_irq_signal: SignalId,
    timer3_compare_irq_signal: SignalId,
    timer3_overflow_irq_signal: SignalId,
    timer4_compare_irq_signal: SignalId,
    timer4_overflow_irq_signal: SignalId,
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
        if state.registers[usize::from(UCSR0B - IO_BASE)] & (1 << 5) != 0 {
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
        let timer3_compare_irq_signal = hub.declare(
            "board.atmega328pb.timer3.compare_a_irq",
            SignalValue::from_u64(0, 1)?,
            Some("functional Timer3 compare-A request".to_owned()),
        )?;
        let timer3_overflow_irq_signal = hub.declare(
            "board.atmega328pb.timer3.overflow_irq",
            SignalValue::from_u64(0, 1)?,
            Some("functional Timer3 overflow request".to_owned()),
        )?;
        let timer4_compare_irq_signal = hub.declare(
            "board.atmega328pb.timer4.compare_a_irq",
            SignalValue::from_u64(0, 1)?,
            Some("functional Timer4 compare-A request".to_owned()),
        )?;
        let timer4_overflow_irq_signal = hub.declare(
            "board.atmega328pb.timer4.overflow_irq",
            SignalValue::from_u64(0, 1)?,
            Some("functional Timer4 overflow request".to_owned()),
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
            timer3_started: 0,
            timer3_base: 0,
            timer3_elapsed: 0,
            timer3_compare_pending: false,
            timer3_overflow_pending: false,
            timer4_started: 0,
            timer4_base: 0,
            timer4_elapsed: 0,
            timer4_compare_pending: false,
            timer4_overflow_pending: false,
            previous_pinb: 0,
            previous_pind: 0,
            watchdog_started: 0,
            watchdog_reset: false,
            uart_tx_signal,
            timer0_irq_signal,
            timer1_irq_signal,
            timer3_compare_irq_signal,
            timer3_overflow_irq_signal,
            timer4_compare_irq_signal,
            timer4_overflow_irq_signal,
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
            address if address == AtmegaTimerRegister::Tccr3b.offset() => {
                if state.registers[AtmegaTimerRegister::Tccr3b.index()] & 7 == 0 && value & 7 != 0 {
                    state.timer3_started = at.ticks();
                    state.timer3_base = u64::from(
                        u16::from(state.registers[AtmegaTimerRegister::Tcnt3l.index()])
                            | (u16::from(state.registers[AtmegaTimerRegister::Tcnt3h.index()])
                                << 8),
                    );
                    state.timer3_elapsed = state.timer3_base;
                }
                state.registers[AtmegaTimerRegister::Tccr3b.index()] = value;
            }
            address if address == AtmegaTimerRegister::Tifr3.offset() => {
                state.registers[AtmegaTimerRegister::Tifr3.index()] &= !value;
                if value & (1 << 1) != 0 {
                    state.timer3_compare_pending = false;
                    set_bit_signal(&state, state.timer3_compare_irq_signal, false, at);
                }
                if value & 1 != 0 {
                    state.timer3_overflow_pending = false;
                    set_bit_signal(&state, state.timer3_overflow_irq_signal, false, at);
                }
            }
            address if address == AtmegaTimerRegister::Tccr4b.offset() => {
                if state.registers[AtmegaTimerRegister::Tccr4b.index()] & 7 == 0 && value & 7 != 0 {
                    state.timer4_started = at.ticks();
                    state.timer4_base = u64::from(
                        u16::from(state.registers[AtmegaTimerRegister::Tcnt4l.index()])
                            | (u16::from(state.registers[AtmegaTimerRegister::Tcnt4h.index()])
                                << 8),
                    );
                    state.timer4_elapsed = state.timer4_base;
                }
                state.registers[AtmegaTimerRegister::Tccr4b.index()] = value;
            }
            address if address == AtmegaTimerRegister::Tifr4.offset() => {
                state.registers[AtmegaTimerRegister::Tifr4.index()] &= !value;
                if value & (1 << 1) != 0 {
                    state.timer4_compare_pending = false;
                    set_bit_signal(&state, state.timer4_compare_irq_signal, false, at);
                }
                if value & 1 != 0 {
                    state.timer4_overflow_pending = false;
                    set_bit_signal(&state, state.timer4_overflow_irq_signal, false, at);
                }
            }
            address if address == AtmegaTimerRegister::Tcnt3l.offset() => {
                state.registers[AtmegaTimerRegister::Tcnt3l.index()] = value;
                state.timer3_base = u64::from(
                    u16::from(state.registers[AtmegaTimerRegister::Tcnt3l.index()])
                        | (u16::from(state.registers[AtmegaTimerRegister::Tcnt3h.index()]) << 8),
                );
                state.timer3_started = at.ticks();
                state.timer3_elapsed = state.timer3_base;
            }
            address if address == AtmegaTimerRegister::Tcnt3h.offset() => {
                state.registers[AtmegaTimerRegister::Tcnt3h.index()] = value;
                state.timer3_base = u64::from(
                    u16::from(state.registers[AtmegaTimerRegister::Tcnt3l.index()])
                        | (u16::from(state.registers[AtmegaTimerRegister::Tcnt3h.index()]) << 8),
                );
                state.timer3_started = at.ticks();
                state.timer3_elapsed = state.timer3_base;
            }
            address if address == AtmegaTimerRegister::Tcnt4l.offset() => {
                state.registers[AtmegaTimerRegister::Tcnt4l.index()] = value;
                state.timer4_base = u64::from(
                    u16::from(state.registers[AtmegaTimerRegister::Tcnt4l.index()])
                        | (u16::from(state.registers[AtmegaTimerRegister::Tcnt4h.index()]) << 8),
                );
                state.timer4_started = at.ticks();
                state.timer4_elapsed = state.timer4_base;
            }
            address if address == AtmegaTimerRegister::Tcnt4h.offset() => {
                state.registers[AtmegaTimerRegister::Tcnt4h.index()] = value;
                state.timer4_base = u64::from(
                    u16::from(state.registers[AtmegaTimerRegister::Tcnt4l.index()])
                        | (u16::from(state.registers[AtmegaTimerRegister::Tcnt4h.index()]) << 8),
                );
                state.timer4_started = at.ticks();
                state.timer4_elapsed = state.timer4_base;
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
        state.timer_pending = false;
        state.timer1_pending = false;
        state.timer3_elapsed = 0;
        state.timer3_compare_pending = false;
        state.timer3_overflow_pending = false;
        state.timer4_elapsed = 0;
        state.timer4_compare_pending = false;
        state.timer4_overflow_pending = false;
        state.watchdog_reset = false;
        set_bit_signal(&state, state.timer0_irq_signal, false, SimTime::ZERO);
        set_bit_signal(&state, state.timer1_irq_signal, false, SimTime::ZERO);
        set_bit_signal(
            &state,
            state.timer3_compare_irq_signal,
            false,
            SimTime::ZERO,
        );
        set_bit_signal(
            &state,
            state.timer3_overflow_irq_signal,
            false,
            SimTime::ZERO,
        );
        set_bit_signal(
            &state,
            state.timer4_compare_irq_signal,
            false,
            SimTime::ZERO,
        );
        set_bit_signal(
            &state,
            state.timer4_overflow_irq_signal,
            false,
            SimTime::ZERO,
        );
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
    fn timer_register_ids_are_named_and_native() {
        assert_eq!(AtmegaTimerRegister::ALL.len(), 14);
        assert_eq!(AtmegaTimerRegister::Tccr3b.offset(), 0x91);
        assert_eq!(AtmegaTimerRegister::Tccr3b.io_offset(), 0x71);
        assert_eq!(AtmegaTimerRegister::Tccr3b.name(), "tccr3b");
        assert_eq!(
            AtmegaTimerRegister::from_data_address(0xa9),
            Some(AtmegaTimerRegister::Ocr4ah)
        );
        assert_eq!(AtmegaTimerRegister::from_data_address(0x93), None);
    }

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
        io.write(
            u64::from(AtmegaTimerRegister::Ocr3al.io_offset()),
            AccessWidth::Byte,
            3,
            SimTime::ZERO,
        )
        .unwrap();
        io.write(
            u64::from(AtmegaTimerRegister::Timsk3.io_offset()),
            AccessWidth::Byte,
            1 << 1,
            SimTime::ZERO,
        )
        .unwrap();
        io.write(
            u64::from(AtmegaTimerRegister::Tccr3b.io_offset()),
            AccessWidth::Byte,
            1,
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(handle.poll(SimTime::from_ticks(4)), vec![15, 32]);
        io.write(
            u64::from(AtmegaTimerRegister::Tifr3.io_offset()),
            AccessWidth::Byte,
            1 << 1,
            SimTime::from_ticks(4),
        )
        .unwrap();
        io.write(
            u64::from(AtmegaTimerRegister::Ocr4al.io_offset()),
            AccessWidth::Byte,
            2,
            SimTime::ZERO,
        )
        .unwrap();
        io.write(
            u64::from(AtmegaTimerRegister::Timsk4.io_offset()),
            AccessWidth::Byte,
            1 << 1,
            SimTime::ZERO,
        )
        .unwrap();
        io.write(
            u64::from(AtmegaTimerRegister::Tccr4b.io_offset()),
            AccessWidth::Byte,
            1,
            SimTime::ZERO,
        )
        .unwrap();
        assert!(handle.poll(SimTime::from_ticks(4)).contains(&41));
    }

    #[test]
    fn timer3_counter_preload_advances_from_written_value() {
        let hub = SignalHub::new();
        let (mut io, handle, _) = AtmegaIo::new("atmega328pb.timer3", hub).unwrap();
        io.write(
            u64::from(AtmegaTimerRegister::Ocr3al.io_offset()),
            AccessWidth::Byte,
            0xff,
            SimTime::ZERO,
        )
        .unwrap();
        io.write(
            u64::from(AtmegaTimerRegister::Ocr3ah.io_offset()),
            AccessWidth::Byte,
            0xff,
            SimTime::ZERO,
        )
        .unwrap();
        io.write(
            u64::from(AtmegaTimerRegister::Tcnt3l.io_offset()),
            AccessWidth::Byte,
            0xfe,
            SimTime::ZERO,
        )
        .unwrap();
        io.write(
            u64::from(AtmegaTimerRegister::Tcnt3h.io_offset()),
            AccessWidth::Byte,
            0xff,
            SimTime::ZERO,
        )
        .unwrap();
        io.write(
            u64::from(AtmegaTimerRegister::Timsk3.io_offset()),
            AccessWidth::Byte,
            1 << 1,
            SimTime::ZERO,
        )
        .unwrap();
        io.write(
            u64::from(AtmegaTimerRegister::Tccr3b.io_offset()),
            AccessWidth::Byte,
            1,
            SimTime::ZERO,
        )
        .unwrap();

        assert_eq!(handle.poll(SimTime::from_ticks(1)), Vec::<u16>::new());
        assert_eq!(
            io.read(
                u64::from(AtmegaTimerRegister::Tcnt3l.io_offset()),
                AccessWidth::Byte,
                SimTime::from_ticks(1),
            )
            .unwrap(),
            0xff
        );
        assert_eq!(
            io.read(
                u64::from(AtmegaTimerRegister::Tcnt3h.io_offset()),
                AccessWidth::Byte,
                SimTime::from_ticks(1),
            )
            .unwrap(),
            0xff
        );
        assert_eq!(handle.poll(SimTime::from_ticks(2)), vec![32]);
    }
}
