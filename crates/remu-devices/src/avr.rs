use super::{GpioHandle, GpioState, SignalHub, refresh_gpio, vendor_gpio};
use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use remu_signals::{Logic, SignalId, SignalValue};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

const IO_BASE: u16 = 0x20;
const PINB: u16 = 0x23;
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
const SPCR1: u16 = 0xac;
const SPSR1: u16 = 0xad;
const SPDR1: u16 = 0xae;

const SPCR_SPIE: u8 = 1 << 7;
const SPCR_SPE: u8 = 1 << 6;
const SPSR_SPIF: u8 = 1 << 7;
const SPSR_WCOL: u8 = 1 << 6;
const SPSR_SPI2X: u8 = 1;
// AVR CPU lines are two below the datasheet vector number (line 0 is INT0).
const SPI0_INTERRUPT_LINE: u16 = 16;
const SPI1_INTERRUPT_LINE: u16 = 38;
const TWBR: u16 = 0xb8;
const TWSR: u16 = 0xb9;
const TWAR: u16 = 0xba;
const TWDR: u16 = 0xbb;
const TWCR: u16 = 0xbc;
const TWAMR: u16 = 0xbd;
const TWBR1: u16 = 0xd8;
const TWSR1: u16 = 0xd9;
const TWAR1: u16 = 0xda;
const TWDR1: u16 = 0xdb;
const TWCR1: u16 = 0xdc;
const TWAMR1: u16 = 0xdd;
const TWI1_INTERRUPT_LINE: u16 = 39;

const TWI_STATUS_RESET: u8 = 0xf8;
const TWI_STATUS_MASK: u8 = 0xf8;
const TWI_PRESCALER_MASK: u8 = 0x03;
const TWI_ADDRESS_MASK: u8 = 0xff;
const TWI_ADDRESS_MASK_MASK: u8 = 0xfe;
const TWINT: u8 = 1 << 7;
const TWEA: u8 = 1 << 6;
const TWSTA: u8 = 1 << 5;
const TWSTO: u8 = 1 << 4;
const TWWC: u8 = 1 << 3;
const TWEN: u8 = 1 << 2;
const TWIE: u8 = 1;
const TWCR_CONFIG_MASK: u8 = TWEA | TWSTA | TWSTO | TWEN | TWIE;
const TWCR_READ_MASK: u8 = TWINT | TWEA | TWSTA | TWSTO | TWWC | TWEN | TWIE;
const ADCL: u16 = 0x78;
const ADCH: u16 = 0x79;
const ADCSRA: u16 = 0x7a;
const ADMUX: u16 = 0x7c;

const ADEN: u8 = 1 << 7;
const ADSC: u8 = 1 << 6;
const ADATE: u8 = 1 << 5;
const ADIF: u8 = 1 << 4;
const ADIE: u8 = 1 << 3;
const ADPS_MASK: u8 = 0x07;
const ADLAR: u8 = 1 << 5;
const PRADC: u8 = 1;
const ADC_INTERRUPT_LINE: u16 = 20;

#[derive(Clone, Copy)]
struct AdcConversion {
    started: u64,
    duration: u64,
    mux: u8,
}
const UDR1: u16 = 0xce;
const UCSR1A: u16 = 0xc8;
const UCSR1B: u16 = 0xc9;
const UDRE1: u8 = 1 << 5;
const TXC1: u8 = 1 << 6;
const TXEN1: u8 = 1 << 3;
const ACSR_ACI: u8 = 1 << 4;
const ACSR_ACO: u8 = 1 << 5;
const ACSR_ACIE: u8 = 1 << 3;
const ACSR_ACIS_MASK: u8 = 0x03;

/// Named ATmega328PB analog-comparator register identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[repr(u16)]
pub enum AtmegaComparatorRegister {
    /// Analog Comparator Control and Status Register (ACSR).
    Acsr = 0x50,
}

impl AtmegaComparatorRegister {
    /// Stable list of modeled comparator register IDs.
    pub const ALL: [Self; 1] = [Self::Acsr];

    /// Returns the native data-space address.
    pub const fn offset(self) -> u16 {
        self as u16
    }

    /// Returns the I/O-device offset used by `AtmegaIo`.
    pub const fn io_offset(self) -> u16 {
        self.offset() - IO_BASE
    }

    /// Returns the vendor register name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Acsr => "acsr",
        }
    }

    /// Resolves a native data-space address to a named register.
    pub const fn from_data_address(address: u16) -> Option<Self> {
        match address {
            0x50 => Some(Self::Acsr),
            _ => None,
        }
    }
}

fn comparator_index() -> usize {
    usize::from(AtmegaComparatorRegister::Acsr.io_offset())
}

#[path = "avr_registers.rs"]
mod registers;
pub use registers::AtmegaTimerRegister;
use registers::{
    CLKPR_CHANGE_ENABLE, CLKPR_DIVIDER_MASK, PRR0_PRTIM0, PRR0_PRTIM1, PRR0_PRUSART0,
    PRR1_WRITABLE_MASK, SMCR_WRITABLE_MASK,
};

struct AtmegaState {
    registers: [u8; 224],
    ports: [Arc<Mutex<GpioState>>; 3],
    port_signals: [Vec<SignalId>; 3],
    hub: SignalHub,
    uart: Vec<u8>,
    uart1: Vec<u8>,
    eeprom: Vec<u8>,
    timer_started: u64,
    timer_pending: bool,
    timer1_started: u64,
    timer1_pending: bool,
    timer2_started: u64,
    timer2_pending: bool,
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
    previous_pinc: u8,
    previous_pind: u8,
    clock_prescaler_armed_at: Option<u64>,
    watchdog_started: u64,
    watchdog_reset: bool,
    spi_tx: Vec<u8>,
    spi_rx: Vec<u8>,
    spi_status_read: bool,
    spi1_tx: Vec<u8>,
    spi1_rx: Vec<u8>,
    spi1_status_read: bool,
    twi_tx: Vec<u8>,
    twi_rx: VecDeque<u8>,
    twi_started: bool,
    twi1_tx: Vec<u8>,
    twi1_rx: VecDeque<u8>,
    twi1_started: bool,
    adc_inputs: [u16; 8],
    adc_conversion: Option<AdcConversion>,
    adc_first_conversion: bool,
    adc_result_locked: bool,
    comparator_positive: bool,
    comparator_negative: bool,
    comparator_previous: bool,
    uart_tx_signal: SignalId,
    uart1_tx_signal: SignalId,
    timer0_irq_signal: SignalId,
    timer1_irq_signal: SignalId,
    timer2_irq_signal: SignalId,
    timer3_compare_irq_signal: SignalId,
    timer3_overflow_irq_signal: SignalId,
    timer4_compare_irq_signal: SignalId,
    timer4_overflow_irq_signal: SignalId,
    pcint0_irq_signal: SignalId,
    pcint1_irq_signal: SignalId,
    pcint2_irq_signal: SignalId,
    int0_irq_signal: SignalId,
    int1_irq_signal: SignalId,
    adc_irq_signal: SignalId,
    watchdog_reset_signal: SignalId,
    comparator_signal: SignalId,
}

/// Machine-facing ATmega328PB I/O state.
#[derive(Clone)]
pub struct AtmegaIoHandle(Arc<Mutex<AtmegaState>>);

include!("avr_handle.rs");

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

impl AtmegaState {
    fn update_comparator(&mut self, at: SimTime) {
        let acsr = self.registers[comparator_index()];
        // The host supplies boolean levels for AIN0/AIN1.  ACO is high when
        // AIN0 is above AIN1, unless the comparator is disabled with ACD.
        let output = acsr & (1 << 7) == 0 && self.comparator_positive && !self.comparator_negative;
        let changed = output != self.comparator_previous;
        self.comparator_previous = output;
        let mut updated = acsr;
        if output {
            updated |= ACSR_ACO;
        } else {
            updated &= !ACSR_ACO;
        }
        if changed {
            let mode = updated & ACSR_ACIS_MASK;
            let edge = match mode {
                0 => true,
                2 => !output,
                3 => output,
                _ => false,
            };
            if edge {
                updated |= ACSR_ACI;
            }
        }
        self.registers[comparator_index()] = updated;
        set_bit_signal(self, self.comparator_signal, output, at);
    }
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

fn reset_registers(registers: &mut [u8; 224]) {
    registers.fill(0);
    registers[usize::from(TWSR - IO_BASE)] = TWI_STATUS_RESET;
    registers[usize::from(TWAR - IO_BASE)] = 0x02;
    registers[usize::from(TWDR - IO_BASE)] = 0x01;
    registers[usize::from(TWSR1 - IO_BASE)] = TWI_STATUS_RESET;
    registers[usize::from(TWAR1 - IO_BASE)] = 0x02;
    registers[usize::from(TWDR1 - IO_BASE)] = 0x01;
    registers[usize::from(UCSR1A - IO_BASE)] = UDRE1;
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
        let uart1_tx_signal = hub.declare(
            "board.atmega328pb.usart1.tx_byte",
            SignalValue::from_u64(0, 8)?,
            Some("last byte written to USART1 UDR".to_owned()),
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
        let adc_irq_signal = hub.declare(
            "board.atmega328pb.adc.interrupt",
            SignalValue::from_u64(0, 1)?,
            Some("functional ADC conversion-complete interrupt".to_owned()),
        )?;
        let watchdog_reset_signal = hub.declare(
            "board.atmega328pb.watchdog.reset",
            SignalValue::from_u64(0, 1)?,
            Some("functional watchdog reset request".to_owned()),
        )?;
        let mut registers = [0; 224];
        reset_registers(&mut registers);
        let comparator_signal = hub.declare(
            "board.atmega328pb.analog_comparator.output",
            SignalValue::from_u64(0, 1)?,
            Some("analog comparator ACO output".to_owned()),
        )?;
        let state = Arc::new(Mutex::new(AtmegaState {
            registers,
            ports: [portb, portc, portd],
            port_signals: [signals_b, signals_c, signals_d],
            hub,
            uart: Vec::new(),
            uart1: Vec::new(),
            eeprom: vec![0xff; 1024],
            timer_started: 0,
            timer_pending: false,
            timer1_started: 0,
            timer1_pending: false,
            timer2_started: 0,
            timer2_pending: false,
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
            previous_pinc: 0,
            previous_pind: 0,
            clock_prescaler_armed_at: None,
            watchdog_started: 0,
            watchdog_reset: false,
            spi_tx: Vec::new(),
            spi_rx: Vec::new(),
            spi_status_read: false,
            spi1_tx: Vec::new(),
            spi1_rx: Vec::new(),
            spi1_status_read: false,
            twi_tx: Vec::new(),
            twi_rx: VecDeque::new(),
            twi_started: false,
            twi1_tx: Vec::new(),
            twi1_rx: VecDeque::new(),
            twi1_started: false,
            adc_inputs: [0; 8],
            adc_conversion: None,
            adc_first_conversion: true,
            adc_result_locked: false,
            comparator_positive: false,
            comparator_negative: false,
            comparator_previous: false,
            uart_tx_signal,
            uart1_tx_signal,
            timer0_irq_signal,
            timer1_irq_signal,
            timer2_irq_signal,
            timer3_compare_irq_signal,
            timer3_overflow_irq_signal,
            timer4_compare_irq_signal,
            timer4_overflow_irq_signal,
            pcint0_irq_signal,
            pcint1_irq_signal,
            pcint2_irq_signal,
            int0_irq_signal,
            int1_irq_signal,
            adc_irq_signal,
            watchdog_reset_signal,
            comparator_signal,
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
            ADCL => {
                state.adc_result_locked = true;
                state.registers[usize::from(address - IO_BASE)]
            }
            ADCH => {
                state.adc_result_locked = false;
                state.registers[usize::from(address - IO_BASE)]
            }
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
            SPSR1 => {
                let value = state.registers[usize::from(SPSR1 - IO_BASE)]
                    & (SPSR_SPIF | SPSR_WCOL | SPSR_SPI2X);
                state.spi1_status_read = value & (SPSR_SPIF | SPSR_WCOL) != 0;
                value
            }
            SPDR1 => {
                let value = state.registers[usize::from(SPDR1 - IO_BASE)];
                if state.spi1_status_read {
                    state.registers[usize::from(SPSR1 - IO_BASE)] &= !(SPSR_SPIF | SPSR_WCOL);
                    state.spi1_status_read = false;
                }
                value
            }
            UCSR1A => state.registers[usize::from(address - IO_BASE)],
            SMCR => state.registers[usize::from(SMCR - IO_BASE)] & SMCR_WRITABLE_MASK,
            CLKPR => {
                state.registers[usize::from(CLKPR - IO_BASE)]
                    & (CLKPR_CHANGE_ENABLE | CLKPR_DIVIDER_MASK)
            }
            PRR0 => state.registers[usize::from(PRR0 - IO_BASE)],
            PRR1 => state.registers[usize::from(PRR1 - IO_BASE)] & PRR1_WRITABLE_MASK,
            EEDR => state.registers[usize::from(EEDR - IO_BASE)],
            TWCR => state.registers[usize::from(TWCR - IO_BASE)] & TWCR_READ_MASK,
            TWSR => state.registers[usize::from(TWSR - IO_BASE)],
            TWAR => state.registers[usize::from(TWAR - IO_BASE)] & TWI_ADDRESS_MASK,
            TWDR => state.registers[usize::from(TWDR - IO_BASE)],
            TWAMR => state.registers[usize::from(TWAMR - IO_BASE)] & TWI_ADDRESS_MASK_MASK,
            TWCR1 => state.registers[usize::from(TWCR1 - IO_BASE)] & TWCR_READ_MASK,
            TWSR1 => state.registers[usize::from(TWSR1 - IO_BASE)],
            TWAR1 => state.registers[usize::from(TWAR1 - IO_BASE)] & TWI_ADDRESS_MASK,
            TWDR1 => state.registers[usize::from(TWDR1 - IO_BASE)],
            TWAMR1 => state.registers[usize::from(TWAMR1 - IO_BASE)] & TWI_ADDRESS_MASK_MASK,
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
        if address == AtmegaComparatorRegister::Acsr.offset() {
            let current = state.registers[comparator_index()];
            let mut updated = value & !ACSR_ACO;
            if value & ACSR_ACI == 0 && current & ACSR_ACI != 0 {
                updated |= ACSR_ACI;
            } else {
                updated &= !ACSR_ACI;
            }
            state.registers[comparator_index()] = updated;
            state.update_comparator(at);
            return Ok(());
        }
        match address {
            UCSR1A => {
                let index = usize::from(UCSR1A - IO_BASE);
                let current = state.registers[index];
                // TXC1 is cleared by writing one. UDRE1 and the receiver
                // status/error flags are read-only; U2X1 and MPCM1 are the
                // writable configuration bits in this functional slice.
                state.registers[index] = (current & UDRE1)
                    | (if value & TXC1 == 0 { current & TXC1 } else { 0 })
                    | (value & 0x03);
            }
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
                if value & PRADC != 0 {
                    state.adc_conversion = None;
                    state.adc_first_conversion = true;
                    state.registers[usize::from(ADCSRA - IO_BASE)] &= !ADSC;
                }
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
            ADCSRA => {
                let index = usize::from(ADCSRA - IO_BASE);
                let previous = state.registers[index];
                let control_mask = ADEN | ADATE | ADIE | ADPS_MASK;
                let mut next = (previous & !control_mask) | (value & control_mask);
                if value & ADIF != 0 {
                    next &= !ADIF;
                    set_bit_signal(&state, state.adc_irq_signal, false, at);
                } else {
                    next = (next & !ADIF) | (previous & ADIF);
                }
                let powered =
                    next & ADEN != 0 && state.registers[usize::from(PRR0 - IO_BASE)] & PRADC == 0;
                if !powered {
                    state.adc_conversion = None;
                    state.adc_first_conversion = true;
                    next &= !ADSC;
                } else if value & ADSC != 0
                    && previous & ADSC == 0
                    && state.adc_conversion.is_none()
                {
                    let duration = (if state.adc_first_conversion {
                        25_u64
                    } else {
                        13_u64
                    }) * adc_prescaler(next);
                    state.adc_conversion = Some(AdcConversion {
                        started: at.ticks(),
                        duration,
                        mux: state.registers[usize::from(ADMUX - IO_BASE)] & 0x0f,
                    });
                    next |= ADSC;
                } else if state.adc_conversion.is_some() {
                    // Writing zero to ADSC has no effect while a conversion
                    // is in progress.
                    next |= ADSC;
                } else {
                    next &= !ADSC;
                }
                state.registers[index] = next;
            }
            ADCL | ADCH => {
                // ADC data registers are read-only on the ATmega328PB.
            }
            ADMUX => {
                // Bit 4 is reserved and reads as zero.
                state.registers[usize::from(ADMUX - IO_BASE)] = value & 0xef;
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
            SPDR1 => {
                if state.registers[usize::from(SPCR1 - IO_BASE)] & SPCR_SPE != 0
                    && state.registers[usize::from(SPSR1 - IO_BASE)] & SPSR_SPIF == 0
                {
                    let received = if state.spi1_rx.is_empty() {
                        value
                    } else {
                        state.spi1_rx.remove(0)
                    };
                    state.spi1_tx.push(value);
                    state.registers[usize::from(SPDR1 - IO_BASE)] = received;
                    state.registers[usize::from(SPSR1 - IO_BASE)] |= SPSR_SPIF;
                } else if state.registers[usize::from(SPCR1 - IO_BASE)] & SPCR_SPE != 0 {
                    state.registers[usize::from(SPSR1 - IO_BASE)] |= SPSR_WCOL;
                }
            }
            SPSR1 => {
                let status = state.registers[usize::from(SPSR1 - IO_BASE)];
                state.registers[usize::from(SPSR1 - IO_BASE)] =
                    (status & (SPSR_SPIF | SPSR_WCOL)) | (value & SPSR_SPI2X);
            }
            UDR1 => {
                let status = state.registers[usize::from(UCSR1A - IO_BASE)];
                let control = state.registers[usize::from(UCSR1B - IO_BASE)];
                if control & TXEN1 != 0 && status & UDRE1 != 0 {
                    state.uart1.push(value);
                    // The functional model completes the frame at the same
                    // abstract timestamp, leaving both ready and complete
                    // for the next polling iteration.
                    state.registers[usize::from(UCSR1A - IO_BASE)] |= UDRE1 | TXC1;
                    state
                        .hub
                        .set(
                            state.uart1_tx_signal,
                            SignalValue::from_u64(u64::from(value), 8)
                                .expect("eight-bit signal is valid"),
                            at,
                        )
                        .expect("ATmega USART1 signal identity and width are fixed");
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
            TWCR => {
                let index = usize::from(TWCR - IO_BASE);
                let mut twcr =
                    (state.registers[index] & (TWINT | TWWC)) | (value & TWCR_CONFIG_MASK);
                let command = value & TWINT != 0;
                if command {
                    // TWINT is cleared by writing one; hardware may set it again
                    // below once the functional operation has completed.
                    twcr &= !TWINT;
                }
                if value & TWEN == 0 {
                    state.twi_started = false;
                }
                if command && value & TWEN != 0 {
                    let status = if value & TWSTA != 0 {
                        state.twi_started = true;
                        0x08
                    } else if value & TWSTO != 0 {
                        state.twi_started = false;
                        TWI_STATUS_RESET
                    } else if let Some(byte) = state.twi_rx.pop_front() {
                        state.registers[usize::from(TWDR - IO_BASE)] = byte;
                        if value & TWEA != 0 { 0x50 } else { 0x58 }
                    } else if state.twi_started {
                        let byte = state.registers[usize::from(TWDR - IO_BASE)];
                        state.twi_tx.push(byte);
                        0x28
                    } else {
                        // No START has been observed, so there is no bus
                        // transaction to complete. Keep the controller idle.
                        TWI_STATUS_RESET
                    };
                    state.registers[usize::from(TWSR - IO_BASE)] =
                        (state.registers[usize::from(TWSR - IO_BASE)] & TWI_PRESCALER_MASK)
                            | (status & TWI_STATUS_MASK);
                    if value & TWSTO == 0 {
                        twcr |= TWINT;
                    }
                    twcr &= !(TWSTA | TWSTO);
                }
                state.registers[index] = twcr & TWCR_READ_MASK;
            }
            TWSR => {
                let index = usize::from(TWSR - IO_BASE);
                state.registers[index] =
                    (state.registers[index] & TWI_STATUS_MASK) | (value & TWI_PRESCALER_MASK);
            }
            TWDR => {
                let twcr_index = usize::from(TWCR - IO_BASE);
                if state.registers[twcr_index] & TWINT != 0 {
                    state.registers[usize::from(TWDR - IO_BASE)] = value;
                    state.registers[twcr_index] &= !TWWC;
                } else {
                    state.registers[twcr_index] |= TWWC;
                }
            }
            TWAMR => {
                state.registers[usize::from(TWAMR - IO_BASE)] = value & TWI_ADDRESS_MASK_MASK;
            }
            TWBR | TWAR => {
                state.registers[usize::from(address - IO_BASE)] = value;
            }
            TWCR1 => {
                let index = usize::from(TWCR1 - IO_BASE);
                let mut twcr =
                    (state.registers[index] & (TWINT | TWWC)) | (value & TWCR_CONFIG_MASK);
                let command = value & TWINT != 0;
                if command {
                    twcr &= !TWINT;
                }
                if value & TWEN == 0 {
                    state.twi1_started = false;
                }
                if command && value & TWEN != 0 {
                    let status = if value & TWSTA != 0 {
                        state.twi1_started = true;
                        0x08
                    } else if value & TWSTO != 0 {
                        state.twi1_started = false;
                        TWI_STATUS_RESET
                    } else if let Some(byte) = state.twi1_rx.pop_front() {
                        state.registers[usize::from(TWDR1 - IO_BASE)] = byte;
                        if value & TWEA != 0 { 0x50 } else { 0x58 }
                    } else if state.twi1_started {
                        let byte = state.registers[usize::from(TWDR1 - IO_BASE)];
                        state.twi1_tx.push(byte);
                        0x28
                    } else {
                        TWI_STATUS_RESET
                    };
                    state.registers[usize::from(TWSR1 - IO_BASE)] =
                        (state.registers[usize::from(TWSR1 - IO_BASE)] & TWI_PRESCALER_MASK)
                            | (status & TWI_STATUS_MASK);
                    if value & TWSTO == 0 {
                        twcr |= TWINT;
                    }
                    twcr &= !(TWSTA | TWSTO);
                }
                state.registers[index] = twcr & TWCR_READ_MASK;
            }
            TWSR1 => {
                let index = usize::from(TWSR1 - IO_BASE);
                state.registers[index] =
                    (state.registers[index] & TWI_STATUS_MASK) | (value & TWI_PRESCALER_MASK);
            }
            TWDR1 => {
                let twcr_index = usize::from(TWCR1 - IO_BASE);
                if state.registers[twcr_index] & TWINT != 0 {
                    state.registers[usize::from(TWDR1 - IO_BASE)] = value;
                    state.registers[twcr_index] &= !TWWC;
                } else {
                    state.registers[twcr_index] |= TWWC;
                }
            }
            TWAMR1 => {
                state.registers[usize::from(TWAMR1 - IO_BASE)] = value & TWI_ADDRESS_MASK_MASK;
            }
            TWBR1 | TWAR1 => {
                state.registers[usize::from(address - IO_BASE)] = value;
            }
            _ => state.registers[usize::from(address - IO_BASE)] = value,
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.lock().expect("ATmega I/O lock poisoned");
        reset_registers(&mut state.registers);
        state.uart.clear();
        state.uart1.clear();
        state.clock_prescaler_armed_at = None;
        state.timer_pending = false;
        state.timer1_pending = false;
        state.timer2_pending = false;
        state.previous_pinb = 0;
        state.previous_pinc = 0;
        state.previous_pind = 0;
        state.timer3_elapsed = 0;
        state.timer3_compare_pending = false;
        state.timer3_overflow_pending = false;
        state.timer4_elapsed = 0;
        state.timer4_compare_pending = false;
        state.timer4_overflow_pending = false;
        state.watchdog_reset = false;
        state.spi_tx.clear();
        state.spi_rx.clear();
        state.spi_status_read = false;
        state.spi1_tx.clear();
        state.spi1_rx.clear();
        state.spi1_status_read = false;
        state.twi_tx.clear();
        state.twi_rx.clear();
        state.twi_started = false;
        state.twi1_tx.clear();
        state.twi1_rx.clear();
        state.twi1_started = false;
        state.adc_inputs = [0; 8];
        state.adc_conversion = None;
        state.adc_first_conversion = true;
        state.adc_result_locked = false;
        state.comparator_positive = false;
        state.comparator_negative = false;
        state.comparator_previous = false;
        set_bit_signal(&state, state.timer0_irq_signal, false, SimTime::ZERO);
        set_bit_signal(&state, state.timer1_irq_signal, false, SimTime::ZERO);
        set_bit_signal(&state, state.timer2_irq_signal, false, SimTime::ZERO);
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
        set_bit_signal(&state, state.pcint1_irq_signal, false, SimTime::ZERO);
        set_bit_signal(&state, state.pcint2_irq_signal, false, SimTime::ZERO);
        set_bit_signal(&state, state.int0_irq_signal, false, SimTime::ZERO);
        set_bit_signal(&state, state.int1_irq_signal, false, SimTime::ZERO);
        set_bit_signal(&state, state.adc_irq_signal, false, SimTime::ZERO);
        set_bit_signal(&state, state.watchdog_reset_signal, false, SimTime::ZERO);
        set_bit_signal(&state, state.comparator_signal, false, SimTime::ZERO);
        for port in &state.ports {
            let mut port = port.lock().expect("ATmega GPIO lock poisoned");
            port.direction = 0;
            port.output = 0;
        }
    }
}

#[cfg(test)]
#[path = "avr_tests.rs"]
mod tests;
