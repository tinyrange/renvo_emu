use super::{GpioHandle, GpioState, SignalHub, refresh_gpio, vendor_gpio};
use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use remu_signals::{Logic, SignalId, SignalValue};
use std::sync::{Arc, Mutex};

const REGISTER_BYTES: usize = 0x1000;

const PM5CTL0: usize = 0x0130;
const FRCTL0: usize = 0x01a0;
const CRC16DI: usize = 0x01c0;
const CRCDIRB: usize = 0x01c2;
const CRCINIRES: usize = 0x01c4;
const CRCRESR: usize = 0x01c6;
const WDTCTL: usize = 0x01cc;

const PAIN: usize = 0x0200;
const PAOUT: usize = 0x0202;
const PADIR: usize = 0x0204;
const P1IV: usize = 0x020e;
const PAIES: usize = 0x0218;
const PAIE: usize = 0x021a;
const PAIFG: usize = 0x021c;
const PBIN: usize = 0x0220;
const PBOUT: usize = 0x0222;
const PBDIR: usize = 0x0224;

const TIMER_BASES: [usize; 4] = [0x0380, 0x03c0, 0x0400, 0x0440];
const TIMER_CHANNELS: [usize; 4] = [3, 3, 2, 2];
const TIMER_CTL_OFFSET: usize = 0x00;
const TIMER_CCTL0_OFFSET: usize = 0x02;
const TIMER_COUNTER_OFFSET: usize = 0x10;
const TIMER_CCR0_OFFSET: usize = 0x12;
const TIMER_IV_OFFSET: usize = 0x2e;

const UCA0CTLW0: usize = 0x0500;
const UCA0STATW: usize = 0x050a;
const UCA0RXBUF: usize = 0x050c;
const UCA0TXBUF: usize = 0x050e;
const UCA0IE: usize = 0x051a;
const UCA0IFG: usize = 0x051c;
const UCA0IV: usize = 0x051e;

const LOCKLPM5: u16 = 0x0001;
const WDTHOLD: u16 = 0x0080;
const WDTPW: u16 = 0x5a00;
const UCSWRST: u16 = 0x0001;
const UCLISTEN: u8 = 0x80;
const CCIE: u16 = 0x0010;
const CCIFG: u16 = 0x0001;
const TAIFG: u16 = 0x0001;
const TAIE: u16 = 0x0002;
const UCRXIFG: u16 = 0x0001;
const UCTXIFG: u16 = 0x0002;

/// FR2433 interrupt vector addresses consumed by the MSP430 CPU adapter.
pub const MSP430_PORT1_VECTOR: u16 = 0xffdc;
/// eUSCI_A0 receive/transmit vector address.
pub const MSP430_USCI_A0_VECTOR: u16 = 0xffe4;
/// Timer0_A0 capture/compare vector address.
pub const MSP430_TIMER0_A0_VECTOR: u16 = 0xfff8;
/// Timer0_A1 capture/compare and overflow vector address.
pub const MSP430_TIMER0_A1_VECTOR: u16 = 0xfff6;
/// Timer1_A0 capture/compare vector address.
pub const MSP430_TIMER1_A0_VECTOR: u16 = 0xfff4;
/// Timer1_A1 capture/compare and overflow vector address.
pub const MSP430_TIMER1_A1_VECTOR: u16 = 0xfff2;
/// Timer2_A0 capture/compare vector address.
pub const MSP430_TIMER2_A0_VECTOR: u16 = 0xfff0;
/// Timer2_A1 capture/compare and overflow vector address.
pub const MSP430_TIMER2_A1_VECTOR: u16 = 0xffee;
/// Timer3_A0 capture/compare vector address.
pub const MSP430_TIMER3_A0_VECTOR: u16 = 0xffec;
/// Timer3_A1 capture/compare and overflow vector address.
pub const MSP430_TIMER3_A1_VECTOR: u16 = 0xffea;
/// Timer_A CCR0 interrupt vectors in TA0..TA3 order.
pub const MSP430_TIMER_A0_VECTORS: [u16; 4] = [
    MSP430_TIMER0_A0_VECTOR,
    MSP430_TIMER1_A0_VECTOR,
    MSP430_TIMER2_A0_VECTOR,
    MSP430_TIMER3_A0_VECTOR,
];
/// Timer_A CCR1/CCR2/overflow interrupt vectors in TA0..TA3 order.
pub const MSP430_TIMER_A1_VECTORS: [u16; 4] = [
    MSP430_TIMER0_A1_VECTOR,
    MSP430_TIMER1_A1_VECTOR,
    MSP430_TIMER2_A1_VECTOR,
    MSP430_TIMER3_A1_VECTOR,
];

fn timer_register(timer: usize, offset: usize) -> usize {
    TIMER_BASES[timer] + offset
}

struct Msp430State {
    registers: [u8; REGISTER_BYTES],
    ports: [Arc<Mutex<GpioState>>; 3],
    port_signals: [Vec<SignalId>; 3],
    hub: SignalHub,
    uart: Vec<u8>,
    previous_p1: u8,
    timer_epoch: [u64; 4],
    timer_a1_delivered: [bool; 4],
    watchdog_epoch: u64,
    watchdog_reset: bool,
    loopback_pending: Option<(u8, u64)>,
    crc: u16,
    crc_data: u16,
    uart_strobe: bool,
    uart_byte_signal: SignalId,
    uart_strobe_signal: SignalId,
    timer_irq_signals: [SignalId; 4],
    port1_irq_signal: SignalId,
    watchdog_reset_signal: SignalId,
}

impl Msp430State {
    fn word(&self, address: usize) -> u16 {
        u16::from_le_bytes([self.registers[address], self.registers[address + 1]])
    }

    fn set_word(&mut self, address: usize, value: u16) {
        self.registers[address..address + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn crc_update_byte(&mut self, byte: u8) {
        self.crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            self.crc = if self.crc & 0x8000 != 0 {
                (self.crc << 1) ^ 0x1021
            } else {
                self.crc << 1
            };
        }
        self.set_word(CRCINIRES, self.crc);
        self.set_word(CRC16DI, self.crc_data);
    }

    fn crc_update(&mut self, value: u16, bytes: usize, reverse_bits: bool) {
        self.crc_data = if reverse_bits {
            let mut translated = 0_u16;
            for index in 0..bytes {
                translated |=
                    u16::from(((value >> (index * 8)) as u8).reverse_bits()) << (index * 8);
            }
            translated
        } else {
            value
        };
        self.set_word(CRC16DI, self.crc_data);
        for index in 0..bytes {
            let byte = (self.crc_data >> (index * 8)) as u8;
            self.crc_update_byte(byte);
        }
    }

    fn gpio_unlocked(&self) -> bool {
        self.word(PM5CTL0) & LOCKLPM5 == 0
    }

    fn resolved_port(&self, port: usize) -> u8 {
        self.ports[port]
            .lock()
            .expect("MSP430 GPIO lock poisoned")
            .nets
            .iter()
            .enumerate()
            .fold(0_u8, |value, (pin, net)| {
                value | (u8::from(net.resolved() == Logic::One) << pin)
            })
    }

    fn update_inputs(&mut self) {
        self.registers[PAIN] = self.resolved_port(0);
        self.registers[PAIN + 1] = self.resolved_port(1);
        self.registers[PBIN] = self.resolved_port(2) & 0x07;
    }

    fn set_signal(&self, signal: SignalId, value: u64, width: u16, at: SimTime) {
        self.hub
            .set(
                signal,
                SignalValue::from_u64(value, width).expect("fixed MSP430 signal width is valid"),
                at,
            )
            .expect("MSP430 signal identity is fixed at construction");
    }

    fn refresh_ports(&mut self, at: SimTime) -> Result<(), DeviceError> {
        let unlocked = self.gpio_unlocked();
        let directions = [
            if unlocked { self.registers[PADIR] } else { 0 },
            if unlocked {
                self.registers[PADIR + 1]
            } else {
                0
            },
            if unlocked {
                self.registers[PBDIR] & 0x07
            } else {
                0
            },
        ];
        let outputs = [
            self.registers[PAOUT],
            self.registers[PAOUT + 1],
            self.registers[PBOUT] & 0x07,
        ];
        for port in 0..3 {
            {
                let mut gpio = self.ports[port].lock().expect("MSP430 GPIO lock poisoned");
                gpio.direction = u32::from(directions[port]);
                gpio.output = u32::from(outputs[port]);
            }
            refresh_gpio(
                &self.ports[port],
                &self.port_signals[port],
                &self.hub,
                u8::try_from(self.port_signals[port].len())
                    .expect("MSP430 package port width fits u8"),
                at,
            )?;
        }
        self.update_inputs();
        Ok(())
    }

    fn reset_registers(&mut self, at: SimTime) {
        self.registers.fill(0);
        self.set_word(PM5CTL0, LOCKLPM5);
        self.set_word(WDTCTL, 0x6900);
        self.set_word(UCA0CTLW0, UCSWRST);
        self.set_word(UCA0IFG, UCTXIFG);
        self.previous_p1 = 0;
        self.timer_epoch = [at.ticks(); 4];
        self.timer_a1_delivered = [false; 4];
        self.watchdog_epoch = at.ticks();
        self.watchdog_reset = false;
        self.loopback_pending = None;
        self.crc = 0;
        self.crc_data = 0;
        self.set_word(CRCINIRES, self.crc);
        self.set_word(CRC16DI, self.crc_data);
        for signal in self.timer_irq_signals {
            self.set_signal(signal, 0, 1, at);
        }
        self.set_signal(self.port1_irq_signal, 0, 1, at);
        self.set_signal(self.watchdog_reset_signal, 0, 1, at);
        let _ = self.refresh_ports(at);
    }

    fn poll_timer(&mut self, timer: usize, now: SimTime, vectors: &mut Vec<u16>) {
        let control_address = timer_register(timer, TIMER_CTL_OFFSET);
        let counter_address = timer_register(timer, TIMER_COUNTER_OFFSET);
        let control = self.word(control_address);
        let mode = (control >> 4) & 0x3;
        let elapsed = now.ticks().saturating_sub(self.timer_epoch[timer]);
        self.timer_epoch[timer] = now.ticks();
        if mode == 0 || elapsed == 0 {
            return;
        }

        let ccr0 = u64::from(self.word(timer_register(timer, TIMER_CCR0_OFFSET)));
        let top = match mode {
            1 | 3 => ccr0.saturating_add(1).max(1),
            _ => 1 << 16,
        };
        let previous = u64::from(self.word(counter_address));
        let total = previous.saturating_add(elapsed);
        let (next, wrapped) = match mode {
            1 | 2 => (total % top, total >= top),
            // The exact up/down phase is not cycle-accurate here, but the
            // counter and compare edges remain deterministic and observable.
            3 => {
                let cycle = top.saturating_mul(2).max(2);
                let phase = total % cycle;
                let count = if phase <= top { phase } else { cycle - phase };
                (count, total >= cycle)
            }
            _ => (previous, false),
        };
        self.set_word(counter_address, next as u16);
        if wrapped {
            self.set_word(control_address, control | TAIFG);
        }

        for channel in 0..TIMER_CHANNELS[timer] {
            let cctl_address = timer_register(timer, TIMER_CCTL0_OFFSET + channel * 2);
            let ccr_address = timer_register(timer, TIMER_CCR0_OFFSET + channel * 2);
            let compare = u64::from(self.word(ccr_address));
            if compare == 0 {
                continue;
            }
            let hit = match mode {
                1 | 2 => compare < top && crossed_up(previous, elapsed, compare, top),
                3 => next == compare && next != previous,
                _ => false,
            };
            if hit {
                self.set_word(cctl_address, self.word(cctl_address) | CCIFG);
            }
        }

        let cctl0 = timer_register(timer, TIMER_CCTL0_OFFSET);
        if self.word(cctl0) & (CCIE | CCIFG) == (CCIE | CCIFG) {
            self.set_signal(self.timer_irq_signals[timer], 1, 1, now);
            self.set_word(cctl0, self.word(cctl0) & !CCIFG);
            vectors.push(MSP430_TIMER_A0_VECTORS[timer]);
        }

        let combined_pending = (1..TIMER_CHANNELS[timer]).any(|channel| {
            self.word(timer_register(timer, TIMER_CCTL0_OFFSET + channel * 2)) & CCIFG != 0
        }) || self.word(control_address) & (TAIE | TAIFG) == (TAIE | TAIFG);
        if combined_pending && !self.timer_a1_delivered[timer] {
            self.set_signal(self.timer_irq_signals[timer], 1, 1, now);
            self.timer_a1_delivered[timer] = true;
            vectors.push(MSP430_TIMER_A1_VECTORS[timer]);
        }
    }
}

fn crossed_up(previous: u64, elapsed: u64, compare: u64, period: u64) -> bool {
    if elapsed == 0 || period == 0 {
        return false;
    }
    let end = previous.saturating_add(elapsed);
    let first = if previous < compare {
        compare
    } else {
        compare.saturating_add(period)
    };
    first <= end
}

/// Host-facing state for the MSP430FR2433 peripheral window.
#[derive(Clone)]
pub struct Msp430PeripheralsHandle(Arc<Mutex<Msp430State>>);

impl Msp430PeripheralsHandle {
    /// Captured eUSCI_A0 transmit bytes.
    pub fn uart_bytes(&self) -> Vec<u8> {
        self.0
            .lock()
            .expect("MSP430 peripheral lock poisoned")
            .uart
            .clone()
    }

    /// Advances functional timers and edge detection, returning pending vector addresses.
    pub fn poll(&self, now: SimTime) -> Vec<u16> {
        let mut state = self.0.lock().expect("MSP430 peripheral lock poisoned");
        state.update_inputs();
        let mut vectors = Vec::new();

        // Interrupt request signals describe the instantaneous request. Clear
        // last poll's assertion before evaluating the current register state.
        state.set_signal(state.port1_irq_signal, 0, 1, now);
        for signal in state.timer_irq_signals {
            state.set_signal(signal, 0, 1, now);
        }

        if state
            .loopback_pending
            .is_some_and(|(_, due)| now.ticks() >= due)
        {
            let (byte, _) = state
                .loopback_pending
                .take()
                .expect("pending eUSCI loopback was just checked");
            state.set_word(UCA0RXBUF, u16::from(byte));
            let flags = state.word(UCA0IFG) | UCRXIFG;
            state.set_word(UCA0IFG, flags);
        }

        let p1 = state.registers[PAIN];
        let changed = p1 ^ state.previous_p1;
        let falling = state.previous_p1 & !p1;
        let rising = !state.previous_p1 & p1;
        state.previous_p1 = p1;
        let edge_select = state.registers[PAIES];
        let edge_matches = (falling & edge_select) | (rising & !edge_select);
        state.registers[PAIFG] |= changed & edge_matches;
        if state.registers[PAIFG] & state.registers[PAIE] != 0 {
            state.set_signal(state.port1_irq_signal, 1, 1, now);
            vectors.push(MSP430_PORT1_VECTOR);
        }

        for timer in 0..TIMER_BASES.len() {
            state.poll_timer(timer, now, &mut vectors);
        }

        if state.word(UCA0IE) & state.word(UCA0IFG) & (UCRXIFG | UCTXIFG) != 0 {
            vectors.push(MSP430_USCI_A0_VECTOR);
        }

        let watchdog = state.word(WDTCTL);
        if watchdog & WDTHOLD == 0 && now.ticks().saturating_sub(state.watchdog_epoch) >= 65_536 {
            state.watchdog_reset = true;
            state.watchdog_epoch = now.ticks();
            state.set_signal(state.watchdog_reset_signal, 1, 1, now);
        }
        vectors
    }

    /// Consumes a pending WDT_A reset request.
    pub fn take_watchdog_reset(&self) -> bool {
        std::mem::take(
            &mut self
                .0
                .lock()
                .expect("MSP430 peripheral lock poisoned")
                .watchdog_reset,
        )
    }
}

/// Unified MSP430FR2433 peripheral register window.
pub struct Msp430Peripherals {
    name: String,
    state: Arc<Mutex<Msp430State>>,
}

impl Msp430Peripherals {
    /// Creates the exact FR2433 startup/peripheral slice and its three package ports.
    pub fn new(
        name: impl Into<String>,
        hub: SignalHub,
    ) -> Result<(Self, Msp430PeripheralsHandle, [GpioHandle; 3]), remu_signals::SignalError> {
        let (p1, p1_signals, p1_handle) = vendor_gpio(8, "board.msp430fr2433.port1", &hub)?;
        let (p2, p2_signals, p2_handle) = vendor_gpio(8, "board.msp430fr2433.port2", &hub)?;
        let (p3, p3_signals, p3_handle) = vendor_gpio(3, "board.msp430fr2433.port3", &hub)?;
        let uart_byte_signal = hub.declare(
            "board.msp430fr2433.uart0.tx_byte",
            SignalValue::from_u64(0, 8)?,
            Some("eUSCI_A0 transmitted byte".to_owned()),
        )?;
        let uart_strobe_signal = hub.declare(
            "board.msp430fr2433.uart0.tx_strobe",
            SignalValue::from_u64(0, 1)?,
            Some("eUSCI_A0 transmit event".to_owned()),
        )?;
        let timer_irq_signals = [
            hub.declare(
                "board.msp430fr2433.timer_a0.ccr0_irq",
                SignalValue::from_u64(0, 1)?,
                Some("Timer_A0 CCR0/A1 interrupt request".to_owned()),
            )?,
            hub.declare(
                "board.msp430fr2433.timer_a1.irq",
                SignalValue::from_u64(0, 1)?,
                Some("Timer_A1 CCR0/A1 interrupt request".to_owned()),
            )?,
            hub.declare(
                "board.msp430fr2433.timer_a2.irq",
                SignalValue::from_u64(0, 1)?,
                Some("Timer_A2 CCR0/A1 interrupt request".to_owned()),
            )?,
            hub.declare(
                "board.msp430fr2433.timer_a3.irq",
                SignalValue::from_u64(0, 1)?,
                Some("Timer_A3 CCR0/A1 interrupt request".to_owned()),
            )?,
        ];
        let port1_irq_signal = hub.declare(
            "board.msp430fr2433.interrupt.port1",
            SignalValue::from_u64(0, 1)?,
            Some("Port 1 interrupt request".to_owned()),
        )?;
        let watchdog_reset_signal = hub.declare(
            "board.msp430fr2433.watchdog.reset",
            SignalValue::from_u64(0, 1)?,
            Some("WDT_A reset request".to_owned()),
        )?;
        let state = Arc::new(Mutex::new(Msp430State {
            registers: [0; REGISTER_BYTES],
            ports: [p1, p2, p3],
            port_signals: [p1_signals, p2_signals, p3_signals],
            hub,
            uart: Vec::new(),
            previous_p1: 0,
            timer_epoch: [0; 4],
            timer_a1_delivered: [false; 4],
            watchdog_epoch: 0,
            watchdog_reset: false,
            loopback_pending: None,
            crc: 0,
            crc_data: 0,
            uart_strobe: false,
            uart_byte_signal,
            uart_strobe_signal,
            timer_irq_signals,
            port1_irq_signal,
            watchdog_reset_signal,
        }));
        state
            .lock()
            .expect("MSP430 peripheral lock poisoned")
            .reset_registers(SimTime::ZERO);
        let handle = Msp430PeripheralsHandle(state.clone());
        Ok((
            Self {
                name: name.into(),
                state,
            },
            handle,
            [p1_handle, p2_handle, p3_handle],
        ))
    }
}

fn transfer_bytes(width: AccessWidth) -> usize {
    match width {
        AccessWidth::Byte => 1,
        AccessWidth::HalfWord => 2,
        AccessWidth::Word => 4,
        AccessWidth::DoubleWord => 8,
    }
}

fn overlaps(start: usize, length: usize, register: usize, register_length: usize) -> bool {
    start < register.saturating_add(register_length) && register < start.saturating_add(length)
}

impl Device for Msp430Peripherals {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        let start = usize::try_from(offset)
            .map_err(|_| DeviceError::new("MSP430 peripheral address does not fit usize"))?;
        let length = transfer_bytes(width);
        let end = start
            .checked_add(length)
            .ok_or_else(|| DeviceError::new("MSP430 peripheral read overflow"))?;
        if end > REGISTER_BYTES {
            return Err(DeviceError::new(format!(
                "MSP430 peripheral read outside modeled window at {offset:#x}"
            )));
        }
        let mut state = self.state.lock().expect("MSP430 peripheral lock poisoned");
        state.update_inputs();
        if start == FRCTL0 && length >= 2 {
            let low = state.word(FRCTL0) & 0x00ff;
            state.set_word(FRCTL0, 0x9600 | low);
        }
        if start == P1IV && length >= 2 {
            let flags = state.registers[PAIFG];
            let vector = flags
                .trailing_zeros()
                .checked_add(1)
                .filter(|_| flags != 0)
                .map_or(0, |index| (index * 2) as u16);
            if flags != 0 {
                state.registers[PAIFG] &= !(1 << flags.trailing_zeros());
            }
            state.set_word(P1IV, vector);
            if state.registers[PAIFG] == 0 {
                state.set_signal(state.port1_irq_signal, 0, 1, at);
            }
        }
        if start == UCA0IV && length >= 2 {
            let flags = state.word(UCA0IFG);
            let vector = if flags & UCRXIFG != 0 {
                2
            } else if flags & UCTXIFG != 0 {
                4
            } else {
                0
            };
            state.set_word(UCA0IV, vector);
        }
        if start == CRCINIRES && length >= 2 {
            let crc = state.crc;
            state.set_word(CRCINIRES, crc);
        }
        if start == CRCRESR && length >= 2 {
            let reversed = state.crc.reverse_bits();
            state.set_word(CRCRESR, reversed);
        }
        if start == CRC16DI && length >= 2 {
            let data = state.crc_data;
            state.set_word(CRC16DI, data);
        }
        for timer in 0..TIMER_BASES.len() {
            if start == timer_register(timer, TIMER_IV_OFFSET) && length >= 2 {
                let mut vector = 0_u16;
                for channel in 1..TIMER_CHANNELS[timer] {
                    let cctl = timer_register(timer, TIMER_CCTL0_OFFSET + channel * 2);
                    if state.word(cctl) & CCIFG != 0 {
                        vector = u16::try_from(channel * 2).expect("Timer_A IV value fits");
                        let value = state.word(cctl) & !CCIFG;
                        state.set_word(cctl, value);
                        break;
                    }
                }
                if vector == 0 {
                    let ctl = timer_register(timer, TIMER_CTL_OFFSET);
                    if state.word(ctl) & TAIFG != 0 {
                        vector = 10;
                        let value = state.word(ctl) & !TAIFG;
                        state.set_word(ctl, value);
                    }
                }
                state.timer_a1_delivered[timer] = false;
                state.set_word(timer_register(timer, TIMER_IV_OFFSET), vector);
            }
        }
        if overlaps(start, length, UCA0RXBUF, 2) {
            let flags = state.word(UCA0IFG) & !UCRXIFG;
            state.set_word(UCA0IFG, flags);
        }
        let mut value = 0_u64;
        for index in 0..length {
            value |= u64::from(state.registers[start + index]) << (index * 8);
        }
        Ok(value)
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        let start = usize::try_from(offset)
            .map_err(|_| DeviceError::new("MSP430 peripheral address does not fit usize"))?;
        let length = transfer_bytes(width);
        let end = start
            .checked_add(length)
            .ok_or_else(|| DeviceError::new("MSP430 peripheral write overflow"))?;
        if end > REGISTER_BYTES {
            return Err(DeviceError::new(format!(
                "MSP430 peripheral write outside modeled window at {offset:#x}"
            )));
        }
        let mut state = self.state.lock().expect("MSP430 peripheral lock poisoned");
        let input_value = value as u16;
        for index in 0..length {
            state.registers[start + index] = (value >> (index * 8)) as u8;
        }
        if start == CRCINIRES && length >= 2 {
            state.crc = input_value;
            state.set_word(CRCINIRES, input_value);
        }
        if start == CRC16DI {
            state.crc_update(input_value, length.min(2), false);
        }
        if start == CRCDIRB {
            state.crc_update(input_value, length.min(2), true);
        }
        if overlaps(start, length, WDTCTL, 2) {
            let written = state.word(WDTCTL);
            if written & 0xff00 != WDTPW {
                state.watchdog_reset = true;
                state.set_signal(state.watchdog_reset_signal, 1, 1, at);
            } else {
                state.set_word(WDTCTL, 0x6900 | (written & 0x00ff));
                state.watchdog_epoch = at.ticks();
            }
        }
        for timer in 0..TIMER_BASES.len() {
            if overlaps(start, length, timer_register(timer, TIMER_CTL_OFFSET), 2)
                || overlaps(start, length, timer_register(timer, TIMER_CCR0_OFFSET), 2)
            {
                state.timer_epoch[timer] = at.ticks();
            }
        }
        if overlaps(start, length, UCA0TXBUF, 2) && state.word(UCA0CTLW0) & UCSWRST == 0 {
            let byte = state.registers[UCA0TXBUF];
            state.uart.push(byte);
            let flags = state.word(UCA0IFG) | UCTXIFG;
            // UCLISTEN is the eUSCI's documented internal loopback facility.
            // It lets the official external-loopback example run under a
            // deterministic board harness without special UART injection.
            if state.registers[UCA0STATW] & UCLISTEN != 0 {
                // A short functional delay preserves the peripheral ordering
                // that real serialisation provides: firmware can enter LPM
                // after TX before the looped-back RX interrupt wakes it.
                state.loopback_pending = Some((byte, at.ticks().saturating_add(8)));
            }
            state.set_word(UCA0IFG, flags);
            state.set_signal(state.uart_byte_signal, u64::from(byte), 8, at);
            state.uart_strobe = !state.uart_strobe;
            state.set_signal(
                state.uart_strobe_signal,
                u64::from(state.uart_strobe),
                1,
                at,
            );
        }
        if overlaps(start, length, UCA0RXBUF, 2) {
            let flags = state.word(UCA0IFG) & !UCRXIFG;
            state.set_word(UCA0IFG, flags);
        }
        if overlaps(start, length, PM5CTL0, 2)
            || overlaps(start, length, PAOUT, 4)
            || overlaps(start, length, PADIR, 4)
            || overlaps(start, length, PBOUT, 2)
            || overlaps(start, length, PBDIR, 2)
        {
            state.refresh_ports(at)?;
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.lock().expect("MSP430 peripheral lock poisoned");
        state.reset_registers(SimTime::ZERO);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpio_is_locked_until_pm5ctl0_is_cleared() {
        let hub = SignalHub::new();
        let (mut device, _handle, gpio) =
            Msp430Peripherals::new("fr2433", hub).expect("signals should construct");
        device
            .write(PADIR as u64, AccessWidth::Byte, 1, SimTime::ZERO)
            .unwrap();
        device
            .write(PAOUT as u64, AccessWidth::Byte, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(gpio[0].direction(), 0);
        device
            .write(PM5CTL0 as u64, AccessWidth::HalfWord, 0, SimTime::ZERO)
            .unwrap();
        assert_eq!(gpio[0].direction(), 1);
        assert_eq!(gpio[0].resolved(0).unwrap(), Logic::One);
    }

    #[test]
    fn eusci_captures_a_transmitted_byte() {
        let hub = SignalHub::new();
        let (mut device, handle, _gpio) =
            Msp430Peripherals::new("fr2433", hub).expect("signals should construct");
        device
            .write(UCA0CTLW0 as u64, AccessWidth::HalfWord, 0, SimTime::ZERO)
            .unwrap();
        device
            .write(
                UCA0TXBUF as u64,
                AccessWidth::HalfWord,
                b'R'.into(),
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(handle.uart_bytes(), b"R");
    }

    #[test]
    fn eusci_listen_mode_loops_tx_back_to_rx() {
        let hub = SignalHub::new();
        let (mut device, handle, _gpio) =
            Msp430Peripherals::new("fr2433", hub).expect("signals should construct");
        device
            .write(UCA0CTLW0 as u64, AccessWidth::HalfWord, 0, SimTime::ZERO)
            .unwrap();
        device
            .write(
                UCA0STATW as u64,
                AccessWidth::Byte,
                UCLISTEN.into(),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(UCA0TXBUF as u64, AccessWidth::HalfWord, 0x5a, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.uart_bytes(), [0x5a]);
        assert!(handle.poll(SimTime::from_ticks(7)).is_empty());
        assert!(handle.poll(SimTime::from_ticks(8)).is_empty());
        assert_eq!(
            device.read(
                UCA0RXBUF as u64,
                AccessWidth::HalfWord,
                SimTime::from_ticks(8)
            ),
            Ok(0x5a)
        );
        assert_eq!(
            device.read(UCA0IFG as u64, AccessWidth::HalfWord, SimTime::ZERO),
            Ok(UCTXIFG.into())
        );
    }

    #[test]
    fn crc16_registers_accumulate_normal_and_bit_reversed_data() {
        let hub = SignalHub::new();
        let (mut device, _handle, _gpio) =
            Msp430Peripherals::new("fr2433", hub).expect("signals should construct");
        device
            .write(
                CRCINIRES as u64,
                AccessWidth::HalfWord,
                0xffff,
                SimTime::ZERO,
            )
            .unwrap();
        for byte in b"123456789" {
            device
                .write(
                    CRC16DI as u64,
                    AccessWidth::Byte,
                    u64::from(*byte),
                    SimTime::ZERO,
                )
                .unwrap();
        }
        assert_eq!(
            device.read(CRCINIRES as u64, AccessWidth::HalfWord, SimTime::ZERO),
            Ok(0x29b1)
        );
        assert_eq!(
            device.read(CRCRESR as u64, AccessWidth::HalfWord, SimTime::ZERO),
            Ok(0x8d94)
        );
        device
            .write(CRCINIRES as u64, AccessWidth::HalfWord, 0, SimTime::ZERO)
            .unwrap();
        device
            .write(
                CRCDIRB as u64,
                AccessWidth::Byte,
                u64::from(b'1'),
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            device.read(CRC16DI as u64, AccessWidth::HalfWord, SimTime::ZERO),
            Ok(u64::from(b'1'.reverse_bits()))
        );
    }

    #[test]
    fn timer_a_instances_route_compare_and_overflow_vectors() {
        let hub = SignalHub::new();
        let (mut device, handle, _gpio) =
            Msp430Peripherals::new("fr2433", hub).expect("signals should construct");
        for timer in 0..TIMER_BASES.len() {
            device
                .write(
                    timer_register(timer, TIMER_CTL_OFFSET) as u64,
                    AccessWidth::HalfWord,
                    u64::from(0x12_u16),
                    SimTime::ZERO,
                )
                .unwrap();
            device
                .write(
                    timer_register(timer, TIMER_CCR0_OFFSET) as u64,
                    AccessWidth::HalfWord,
                    3,
                    SimTime::ZERO,
                )
                .unwrap();
            device
                .write(
                    timer_register(timer, TIMER_CCTL0_OFFSET + 2) as u64,
                    AccessWidth::HalfWord,
                    u64::from(CCIE),
                    SimTime::ZERO,
                )
                .unwrap();
            device
                .write(
                    timer_register(timer, TIMER_CCR0_OFFSET + 2) as u64,
                    AccessWidth::HalfWord,
                    2,
                    SimTime::ZERO,
                )
                .unwrap();
        }
        let vectors = handle.poll(SimTime::from_ticks(2));
        for vector in MSP430_TIMER_A1_VECTORS {
            assert!(
                vectors.contains(&vector),
                "missing Timer_A vector {vector:#x}"
            );
        }
        for timer in 0..TIMER_BASES.len() {
            assert_eq!(
                device.read(
                    timer_register(timer, TIMER_IV_OFFSET) as u64,
                    AccessWidth::HalfWord,
                    SimTime::from_ticks(2),
                ),
                Ok(2)
            );
        }
        let vectors = handle.poll(SimTime::from_ticks(4));
        for vector in MSP430_TIMER_A1_VECTORS {
            assert!(
                vectors.contains(&vector),
                "missing overflow vector {vector:#x}"
            );
        }
        for timer in 0..TIMER_BASES.len() {
            assert_eq!(
                device.read(
                    timer_register(timer, TIMER_IV_OFFSET) as u64,
                    AccessWidth::HalfWord,
                    SimTime::from_ticks(4),
                ),
                Ok(10)
            );
        }
    }
}
