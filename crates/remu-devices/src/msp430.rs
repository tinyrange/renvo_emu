use super::{GpioHandle, GpioState, SignalHub, refresh_gpio, vendor_gpio};
use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use remu_signals::{Logic, SignalId, SignalValue};
use std::collections::{BTreeMap, VecDeque};
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

const RTCCTL: usize = 0x0300;
const RTCIV: usize = 0x0304;
const RTCMOD: usize = 0x0308;
const RTCCNT: usize = 0x030c;

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
const UCA1CTLW0: usize = 0x0520;
const UCA1STATW: usize = 0x052a;
const UCA1RXBUF: usize = 0x052c;
const UCA1TXBUF: usize = 0x052e;
const UCA1IE: usize = 0x053a;
const UCA1IFG: usize = 0x053c;
const UCA1IV: usize = 0x053e;
const UCB0CTLW0: usize = 0x0540;
const UCB0RXBUF: usize = 0x054c;
const UCB0TXBUF: usize = 0x054e;
const UCB0IE: usize = 0x056a;
const UCB0IFG: usize = 0x056c;
const UCB0IV: usize = 0x056e;
const ADCCTL0: usize = 0x0700;
const ADCCTL1: usize = 0x0702;
const ADCCTL2: usize = 0x0704;
const ADCMCTL0: usize = 0x070a;
const ADCMEM0: usize = 0x0712;
const ADCIE: usize = 0x071a;
const ADCIFG: usize = 0x071c;
const ADCIV: usize = 0x071e;
const UCB0I2CSA: usize = 0x0560;

const LOCKLPM5: u16 = 0x0001;
const WDTHOLD: u16 = 0x0080;
const WDTPW: u16 = 0x5a00;
const UCSWRST: u16 = 0x0001;
const UCLISTEN: u8 = 0x80;
const UCSYNC: u16 = 0x0100;
const UCMODE_MASK: u16 = 0x0600;
const UCMST: u16 = 0x0800;
const ADCSC: u16 = 0x0001;
const ADCENC: u16 = 0x0002;
const ADCON: u16 = 0x0010;
const ADCBUSY: u16 = 0x0001;
const ADCIFG0: u16 = 0x0001;
const ADCINCH_MASK: u16 = 0x000f;
const UCTR: u16 = 1 << 4;
const UCTXSTT: u16 = 1 << 1;
const UCTXSTP: u16 = 1 << 2;
const UCTXNACK: u16 = 1 << 3;
const UCMODE_I2C: u16 = 0x0600;
const CCIE: u16 = 0x0010;
const CCIFG: u16 = 0x0001;
const TAIFG: u16 = 0x0001;
const TAIE: u16 = 0x0002;
const RTCIF: u16 = 0x0001;
const RTCIE: u16 = 0x0002;
const RTCSR: u16 = 1 << 6;
const RTCSS_MASK: u16 = 0x7000;
const UCRXIFG: u16 = 0x0001;
const UCTXIFG: u16 = 0x0002;

/// FR2433 interrupt vector addresses consumed by the MSP430 CPU adapter.
pub const MSP430_PORT1_VECTOR: u16 = 0xffdc;
/// eUSCI_A0 receive/transmit vector address.
pub const MSP430_USCI_A0_VECTOR: u16 = 0xffe4;
/// RTC counter overflow interrupt vector address.
pub const MSP430_RTC_VECTOR: u16 = 0xffe8;
/// eUSCI_A1 receive/transmit vector address.
pub const MSP430_USCI_A1_VECTOR: u16 = 0xffe2;
/// eUSCI_B0 receive/transmit vector address.
pub const MSP430_USCI_B0_VECTOR: u16 = 0xffe0;
/// ADC conversion-complete interrupt vector address.
pub const MSP430_ADC_VECTOR: u16 = 0xffde;
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

/// One functional eUSCI_B0 I²C host transaction observed by the test harness.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Msp430I2cEvent {
    /// START condition on the virtual bus.
    Start,
    /// Repeated START without releasing the virtual bus.
    RepeatedStart,
    /// A transmitted address or data byte.
    Write {
        /// Seven-bit target address selected in UCB0I2CSA.
        address: u16,
        /// Byte placed in UCB0TXBUF.
        value: u8,
    },
    /// A received byte supplied by the host fixture.
    Read {
        /// Seven-bit target address selected in UCB0I2CSA.
        address: u16,
        /// Byte supplied by the host fixture.
        value: u8,
    },
    /// STOP condition on the virtual bus.
    Stop,
}

struct Msp430State {
    registers: [u8; REGISTER_BYTES],
    ports: [Arc<Mutex<GpioState>>; 3],
    port_signals: [Vec<SignalId>; 3],
    hub: SignalHub,
    uart: Vec<u8>,
    uart1: Vec<u8>,
    spi0_tx: Vec<u8>,
    spi0_incoming: Vec<u8>,
    spi0_strobe: bool,
    adc_inputs: [u16; 16],
    adc_pending: Option<(u8, u64)>,
    previous_p1: u8,
    timer_epoch: [u64; 4],
    timer_a1_delivered: [bool; 4],
    rtc_epoch: u64,
    rtc_delivered: bool,
    watchdog_epoch: u64,
    watchdog_reset: bool,
    loopback_pending: Option<(u8, u64)>,
    crc: u16,
    crc_data: u16,
    loopback1_pending: Option<(u8, u64)>,
    i2c_events: Vec<Msp430I2cEvent>,
    i2c_responses: BTreeMap<u16, VecDeque<u8>>,
    i2c_active_address: Option<u16>,
    i2c_read: bool,
    i2c_strobe: bool,
    uart_strobe: bool,
    uart1_strobe: bool,
    uart_byte_signal: SignalId,
    uart_strobe_signal: SignalId,
    timer_irq_signals: [SignalId; 4],
    rtc_irq_signal: SignalId,
    uart1_byte_signal: SignalId,
    uart1_strobe_signal: SignalId,
    spi0_byte_signal: SignalId,
    spi0_rx_signal: SignalId,
    spi0_strobe_signal: SignalId,
    adc_sample_signal: SignalId,
    adc_strobe_signal: SignalId,
    i2c_byte_signal: SignalId,
    i2c_strobe_signal: SignalId,
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

    fn eusci_b0_i2c_master(&self) -> bool {
        let control = self.word(UCB0CTLW0);
        control & UCSWRST == 0
            && control & (UCSYNC | UCMODE_MASK | UCMST) == (UCSYNC | UCMODE_I2C | UCMST)
    }

    fn emit_i2c_byte(&mut self, value: u8, at: SimTime) {
        self.set_signal(self.i2c_byte_signal, u64::from(value), 8, at);
        self.i2c_strobe = !self.i2c_strobe;
        self.set_signal(self.i2c_strobe_signal, u64::from(self.i2c_strobe), 1, at);
    }

    fn i2c_start(&mut self, control: u16, at: SimTime) {
        let address = self.word(UCB0I2CSA) & 0x007f;
        if self.i2c_active_address.is_some() {
            self.i2c_events.push(Msp430I2cEvent::RepeatedStart);
        } else {
            self.i2c_events.push(Msp430I2cEvent::Start);
        }
        self.i2c_active_address = Some(address);
        self.i2c_read = control & UCTR == 0;
        if self.i2c_read {
            let value = self
                .i2c_responses
                .get_mut(&address)
                .and_then(VecDeque::pop_front)
                .unwrap_or(0xff);
            self.set_word(UCB0RXBUF, u16::from(value));
            self.set_word(UCB0IFG, self.word(UCB0IFG) | UCRXIFG);
            self.i2c_events
                .push(Msp430I2cEvent::Read { address, value });
            self.emit_i2c_byte(value, at);
        } else {
            self.set_word(UCB0IFG, self.word(UCB0IFG) | UCTXIFG);
        }
    }

    fn i2c_control_write(&mut self, at: SimTime) {
        if !self.eusci_b0_i2c_master() {
            return;
        }
        let control = self.word(UCB0CTLW0);
        if control & UCTXSTT != 0 {
            self.i2c_start(control, at);
        }
        if control & UCTXSTP != 0 {
            if self.i2c_active_address.take().is_some() {
                self.i2c_events.push(Msp430I2cEvent::Stop);
            }
            self.i2c_read = false;
            self.set_word(UCB0IFG, self.word(UCB0IFG) | UCTXIFG);
        }
        // START, STOP, and NACK are command bits in the real peripheral and
        // clear once accepted. Keeping that behavior makes polling loops in
        // vendor drivers deterministic without adding bus-level timing.
        self.set_word(UCB0CTLW0, control & !(UCTXSTT | UCTXSTP | UCTXNACK));
    }

    fn i2c_tx_write(&mut self, value: u8, at: SimTime) {
        let Some(address) = self.i2c_active_address else {
            return;
        };
        if self.i2c_read {
            return;
        }
        self.i2c_events
            .push(Msp430I2cEvent::Write { address, value });
        self.emit_i2c_byte(value, at);
        self.set_word(UCB0IFG, self.word(UCB0IFG) | UCTXIFG);
    }

    fn i2c_prefetch_read(&mut self, at: SimTime) {
        let Some(address) = self.i2c_active_address else {
            return;
        };
        if !self.i2c_read {
            return;
        }
        let Some(value) = self
            .i2c_responses
            .get_mut(&address)
            .and_then(VecDeque::pop_front)
        else {
            return;
        };
        self.set_word(UCB0RXBUF, u16::from(value));
        self.set_word(UCB0IFG, self.word(UCB0IFG) | UCRXIFG);
        self.i2c_events
            .push(Msp430I2cEvent::Read { address, value });
        self.emit_i2c_byte(value, at);
    }

    fn reset_registers(&mut self, at: SimTime) {
        self.registers.fill(0);
        self.set_word(PM5CTL0, LOCKLPM5);
        self.set_word(WDTCTL, 0x6900);
        self.set_word(UCA0CTLW0, UCSWRST);
        self.set_word(UCA0IFG, UCTXIFG);
        self.set_word(UCA1CTLW0, UCSWRST);
        self.set_word(UCA1IFG, UCTXIFG);
        self.set_word(UCB0CTLW0, UCSWRST);
        self.set_word(UCB0IFG, UCTXIFG);
        self.previous_p1 = 0;
        self.timer_epoch = [at.ticks(); 4];
        self.timer_a1_delivered = [false; 4];
        self.spi0_tx.clear();
        self.spi0_incoming.clear();
        self.spi0_strobe = false;
        self.set_word(ADCCTL0, 0);
        self.set_word(ADCCTL1, 0);
        self.set_word(ADCCTL2, 0);
        self.set_word(ADCMEM0, 0);
        self.set_word(ADCIFG, 0);
        self.adc_inputs.fill(0);
        self.adc_pending = None;
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
        self.rtc_epoch = at.ticks();
        self.rtc_delivered = false;
        self.set_word(RTCCTL, 0);
        self.set_word(RTCMOD, u16::MAX);
        self.set_word(RTCCNT, 0);
        self.set_signal(self.rtc_irq_signal, 0, 1, at);
        self.loopback1_pending = None;
        self.uart_strobe = false;
        self.uart1_strobe = false;
        self.set_signal(self.port1_irq_signal, 0, 1, at);
        self.set_signal(self.watchdog_reset_signal, 0, 1, at);
        self.set_signal(self.spi0_strobe_signal, 0, 1, at);
        self.set_signal(self.spi0_byte_signal, 0, 8, at);
        self.set_signal(self.spi0_rx_signal, 0, 8, at);
        self.set_signal(self.adc_sample_signal, 0, 10, at);
        self.set_signal(self.adc_strobe_signal, 0, 1, at);
        self.i2c_events.clear();
        self.i2c_responses.clear();
        self.i2c_active_address = None;
        self.i2c_read = false;
        self.i2c_strobe = false;
        self.set_signal(self.i2c_byte_signal, 0, 8, at);
        self.set_signal(self.i2c_strobe_signal, 0, 1, at);
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

impl Msp430State {
    fn poll_rtc(&mut self, now: SimTime, vectors: &mut Vec<u16>) {
        let control = self.word(RTCCTL);
        let elapsed = now.ticks().saturating_sub(self.rtc_epoch);
        if control & RTCSS_MASK == 0 || elapsed == 0 {
            self.rtc_epoch = now.ticks();
            return;
        }
        let divider = match (control >> 8) & 7 {
            0 => 1,
            1 => 10,
            2 => 100,
            3 => 1000,
            4 => 16,
            5 => 64,
            6 => 256,
            _ => 1024,
        };
        let ticks = elapsed / divider;
        self.rtc_epoch = now.ticks().saturating_sub(elapsed % divider);
        if ticks == 0 {
            return;
        }
        let modulo = u64::from(self.word(RTCMOD));
        let top = modulo.saturating_add(1).max(1);
        let total = u64::from(self.word(RTCCNT)).saturating_add(ticks);
        if total >= top {
            self.set_word(RTCCTL, control | RTCIF);
        }
        self.set_word(RTCCNT, (total % top) as u16);
        if self.word(RTCCTL) & (RTCIE | RTCIF) == (RTCIE | RTCIF) && !self.rtc_delivered {
            self.set_signal(self.rtc_irq_signal, 1, 1, now);
            self.rtc_delivered = true;
            vectors.push(MSP430_RTC_VECTOR);
        }
    }
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

    /// Captured eUSCI_A1 transmit bytes.
    pub fn uart1_bytes(&self) -> Vec<u8> {
        self.0
            .lock()
            .expect("MSP430 peripheral lock poisoned")
            .uart1
            .clone()
    }

    /// Captured eUSCI_B0 MOSI bytes from completed functional SPI transfers.
    pub fn spi0_bytes(&self) -> Vec<u8> {
        self.0
            .lock()
            .expect("MSP430 peripheral lock poisoned")
            .spi0_tx
            .clone()
    }

    /// Supplies the MISO byte returned by the next eUSCI_B0 SPI transfer.
    pub fn inject_spi0_rx(&self, value: u8) {
        self.0
            .lock()
            .expect("MSP430 peripheral lock poisoned")
            .spi0_incoming
            .push(value);
    }

    /// Sets the deterministic analog value returned for one ADC input channel.
    pub fn set_adc_input(&self, channel: u8, value: u16) {
        if let Some(input) = self
            .0
            .lock()
            .expect("MSP430 peripheral lock poisoned")
            .adc_inputs
            .get_mut(usize::from(channel))
        {
            *input = value & 0x03ff;
        }
    }

    /// Queues bytes returned by the next 7-bit eUSCI_B0 I²C read at `address`.
    pub fn queue_i2c_read(&self, address: u16, bytes: impl IntoIterator<Item = u8>) {
        self.0
            .lock()
            .expect("MSP430 peripheral lock poisoned")
            .i2c_responses
            .entry(address & 0x007f)
            .or_default()
            .extend(bytes);
    }

    /// Returns the functional I²C host transactions observed since reset or clear.
    pub fn i2c_events(&self) -> Vec<Msp430I2cEvent> {
        self.0
            .lock()
            .expect("MSP430 peripheral lock poisoned")
            .i2c_events
            .clone()
    }

    /// Clears captured I²C events and queued host responses.
    pub fn clear_i2c(&self) {
        let mut state = self.0.lock().expect("MSP430 peripheral lock poisoned");
        state.i2c_events.clear();
        state.i2c_responses.clear();
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
        state.set_signal(state.rtc_irq_signal, 0, 1, now);

        if state.adc_pending.is_some_and(|(_, due)| now.ticks() >= due) {
            let (channel, _) = state
                .adc_pending
                .take()
                .expect("pending ADC conversion was just checked");
            let sample = state.adc_inputs[usize::from(channel)] & 0x03ff;
            state.set_word(ADCMEM0, sample);
            let flags = state.word(ADCIFG) | ADCIFG0;
            state.set_word(ADCIFG, flags);
            let control = state.word(ADCCTL0) & !ADCSC;
            state.set_word(ADCCTL0, control);
            let status = state.word(ADCCTL1) & !ADCBUSY;
            state.set_word(ADCCTL1, status);
            state.set_signal(state.adc_sample_signal, u64::from(sample), 10, now);
            state.set_signal(state.adc_strobe_signal, 1, 1, now);
        } else {
            state.set_signal(state.adc_strobe_signal, 0, 1, now);
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
        if state
            .loopback1_pending
            .is_some_and(|(_, due)| now.ticks() >= due)
        {
            let (byte, _) = state
                .loopback1_pending
                .take()
                .expect("pending eUSCI_A1 loopback was just checked");
            state.set_word(UCA1RXBUF, u16::from(byte));
            let flags = state.word(UCA1IFG) | UCRXIFG;
            state.set_word(UCA1IFG, flags);
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

        state.poll_rtc(now, &mut vectors);

        if state.word(UCA0IE) & state.word(UCA0IFG) & (UCRXIFG | UCTXIFG) != 0 {
            vectors.push(MSP430_USCI_A0_VECTOR);
        }
        if state.word(UCA1IE) & state.word(UCA1IFG) & (UCRXIFG | UCTXIFG) != 0 {
            vectors.push(MSP430_USCI_A1_VECTOR);
        }
        if state.word(UCB0IE) & state.word(UCB0IFG) & (UCRXIFG | UCTXIFG) != 0 {
            vectors.push(MSP430_USCI_B0_VECTOR);
        }
        if state.word(ADCIE) & state.word(ADCIFG) & ADCIFG0 != 0 {
            vectors.push(MSP430_ADC_VECTOR);
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
        let uart1_byte_signal = hub.declare(
            "board.msp430fr2433.uart1.tx_byte",
            SignalValue::from_u64(0, 8)?,
            Some("eUSCI_A1 transmitted byte".to_owned()),
        )?;
        let uart1_strobe_signal = hub.declare(
            "board.msp430fr2433.uart1.tx_strobe",
            SignalValue::from_u64(0, 1)?,
            Some("eUSCI_A1 transmit event".to_owned()),
        )?;
        let i2c_byte_signal = hub.declare(
            "board.msp430fr2433.i2c0.byte",
            SignalValue::from_u64(0, 8)?,
            Some("eUSCI_B0 I²C host byte".to_owned()),
        )?;
        let i2c_strobe_signal = hub.declare(
            "board.msp430fr2433.i2c0.strobe",
            SignalValue::from_u64(0, 1)?,
            Some("eUSCI_B0 I²C host byte event".to_owned()),
        )?;
        let adc_sample_signal = hub.declare(
            "board.msp430fr2433.adc0.sample",
            SignalValue::from_u64(0, 10)?,
            Some("ADC10 conversion result".to_owned()),
        )?;
        let adc_strobe_signal = hub.declare(
            "board.msp430fr2433.adc0.eoc",
            SignalValue::from_u64(0, 1)?,
            Some("ADC10 conversion complete event".to_owned()),
        )?;
        let spi0_byte_signal = hub.declare(
            "board.msp430fr2433.spi0.tx_byte",
            SignalValue::from_u64(0, 8)?,
            Some("eUSCI_B0 MOSI byte".to_owned()),
        )?;
        let spi0_rx_signal = hub.declare(
            "board.msp430fr2433.spi0.rx_byte",
            SignalValue::from_u64(0, 8)?,
            Some("eUSCI_B0 MISO byte".to_owned()),
        )?;
        let spi0_strobe_signal = hub.declare(
            "board.msp430fr2433.spi0.tx_strobe",
            SignalValue::from_u64(0, 1)?,
            Some("eUSCI_B0 transfer event".to_owned()),
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
        let rtc_irq_signal = hub.declare(
            "board.msp430fr2433.rtc.irq",
            SignalValue::from_u64(0, 1)?,
            Some("RTC counter overflow interrupt request".to_owned()),
        )?;
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
            uart1: Vec::new(),
            spi0_tx: Vec::new(),
            spi0_incoming: Vec::new(),
            spi0_strobe: false,
            adc_inputs: [0; 16],
            adc_pending: None,
            previous_p1: 0,
            timer_epoch: [0; 4],
            timer_a1_delivered: [false; 4],
            rtc_epoch: 0,
            rtc_delivered: false,
            watchdog_epoch: 0,
            watchdog_reset: false,
            loopback_pending: None,
            crc: 0,
            crc_data: 0,
            loopback1_pending: None,
            i2c_events: Vec::new(),
            i2c_responses: BTreeMap::new(),
            i2c_active_address: None,
            i2c_read: false,
            i2c_strobe: false,
            uart_strobe: false,
            uart1_strobe: false,
            uart_byte_signal,
            uart_strobe_signal,
            timer_irq_signals,
            rtc_irq_signal,
            uart1_byte_signal,
            uart1_strobe_signal,
            spi0_byte_signal,
            spi0_rx_signal,
            spi0_strobe_signal,
            adc_sample_signal,
            adc_strobe_signal,
            i2c_byte_signal,
            i2c_strobe_signal,
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
        if start == RTCIV && length >= 2 {
            let vector = if state.word(RTCCTL) & RTCIF != 0 {
                2
            } else {
                0
            };
            let value = state.word(RTCCTL) & !RTCIF;
            state.set_word(RTCCTL, value);
            state.rtc_delivered = false;
            state.set_word(RTCIV, vector);
            state.set_signal(state.rtc_irq_signal, 0, 1, at);
        }
        if start == UCA1IV && length >= 2 {
            let flags = state.word(UCA1IFG);
            let vector = if flags & UCRXIFG != 0 {
                2
            } else if flags & UCTXIFG != 0 {
                4
            } else {
                0
            };
            state.set_word(UCA1IV, vector);
        }
        if start == UCB0IV && length >= 2 {
            let flags = state.word(UCB0IFG);
            let vector = if flags & UCRXIFG != 0 {
                2
            } else if flags & UCTXIFG != 0 {
                4
            } else {
                0
            };
            state.set_word(UCB0IV, vector);
        }
        if start == ADCIV && length >= 2 {
            let flags = state.word(ADCIFG);
            let vector = if flags & ADCIFG0 != 0 { 2 } else { 0 };
            state.set_word(ADCIV, vector);
            state.set_word(ADCIFG, flags & !ADCIFG0);
        }
        if overlaps(start, length, UCA0RXBUF, 2) {
            let flags = state.word(UCA0IFG) & !UCRXIFG;
            state.set_word(UCA0IFG, flags);
        }
        if overlaps(start, length, UCA1RXBUF, 2) {
            let flags = state.word(UCA1IFG) & !UCRXIFG;
            state.set_word(UCA1IFG, flags);
        }
        let i2c_rx_read = overlaps(start, length, UCB0RXBUF, 2)
            && state.i2c_active_address.is_some()
            && state.i2c_read;
        if overlaps(start, length, UCB0RXBUF, 2) {
            let flags = state.word(UCB0IFG) & !UCRXIFG;
            state.set_word(UCB0IFG, flags);
        }
        let mut value = 0_u64;
        for index in 0..length {
            value |= u64::from(state.registers[start + index]) << (index * 8);
        }
        if i2c_rx_read {
            state.i2c_prefetch_read(at);
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
        if overlaps(start, length, UCB0CTLW0, 2) {
            state.i2c_control_write(at);
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
        if overlaps(start, length, RTCCTL, 2) {
            let control = state.word(RTCCTL);
            if control & RTCSR != 0 {
                state.set_word(RTCCTL, control & !RTCSR & !RTCIF);
                state.set_word(RTCCNT, 0);
                state.rtc_epoch = at.ticks();
                state.rtc_delivered = false;
            } else if control & RTCIF == 0 {
                state.rtc_delivered = false;
            }
        }
        if overlaps(start, length, ADCCTL0, 2) {
            let control = state.word(ADCCTL0);
            if control & (ADCON | ADCENC | ADCSC) == (ADCON | ADCENC | ADCSC)
                && state.adc_pending.is_none()
            {
                let channel = (state.word(ADCMCTL0) & ADCINCH_MASK) as u8;
                state.adc_pending = Some((channel, at.ticks().saturating_add(4)));
                let status = state.word(ADCCTL1) | ADCBUSY;
                state.set_word(ADCCTL1, status);
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
        if overlaps(start, length, UCA1TXBUF, 2) && state.word(UCA1CTLW0) & UCSWRST == 0 {
            let byte = state.registers[UCA1TXBUF];
            state.uart1.push(byte);
            let flags = state.word(UCA1IFG) | UCTXIFG;
            if state.registers[UCA1STATW] & UCLISTEN != 0 {
                state.loopback1_pending = Some((byte, at.ticks().saturating_add(8)));
            }
            state.set_word(UCA1IFG, flags);
            state.set_signal(state.uart1_byte_signal, u64::from(byte), 8, at);
            state.uart1_strobe = !state.uart1_strobe;
            state.set_signal(
                state.uart1_strobe_signal,
                u64::from(state.uart1_strobe),
                1,
                at,
            );
        }
        if overlaps(start, length, UCB0TXBUF, 2)
            && state.word(UCB0CTLW0) & (UCSWRST | UCSYNC | UCMODE_MASK) == UCSYNC
            && state.word(UCB0CTLW0) & UCMST != 0
        {
            let byte = state.registers[UCB0TXBUF];
            let received = state.spi0_incoming.first().copied().unwrap_or(byte);
            if !state.spi0_incoming.is_empty() {
                state.spi0_incoming.remove(0);
            }
            state.spi0_tx.push(byte);
            state.set_word(UCB0RXBUF, u16::from(received));
            let flags = state.word(UCB0IFG) | UCRXIFG | UCTXIFG;
            state.set_word(UCB0IFG, flags);
            state.set_signal(state.spi0_byte_signal, u64::from(byte), 8, at);
            state.set_signal(state.spi0_rx_signal, u64::from(received), 8, at);
            state.spi0_strobe = !state.spi0_strobe;
            state.set_signal(
                state.spi0_strobe_signal,
                u64::from(state.spi0_strobe),
                1,
                at,
            );
        }
        if overlaps(start, length, UCA0RXBUF, 2) {
            let flags = state.word(UCA0IFG) & !UCRXIFG;
            state.set_word(UCA0IFG, flags);
        }
        if overlaps(start, length, UCA1RXBUF, 2) {
            let flags = state.word(UCA1IFG) & !UCRXIFG;
            state.set_word(UCA1IFG, flags);
        }
        if overlaps(start, length, UCB0TXBUF, 2) && state.eusci_b0_i2c_master() {
            let byte = state.registers[UCB0TXBUF];
            state.i2c_tx_write(byte, at);
        }
        if overlaps(start, length, UCB0RXBUF, 2) {
            let flags = state.word(UCB0IFG) & !UCRXIFG;
            state.set_word(UCB0IFG, flags);
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
    fn eusci_a1_uart_transmit_and_loopback_are_observable() {
        let hub = SignalHub::new();
        let (mut device, handle, _gpio) =
            Msp430Peripherals::new("fr2433", hub).expect("signals should construct");
        device
            .write(UCA1CTLW0 as u64, AccessWidth::HalfWord, 0, SimTime::ZERO)
            .unwrap();
        device
            .write(
                UCA1STATW as u64,
                AccessWidth::Byte,
                UCLISTEN.into(),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                UCA1TXBUF as u64,
                AccessWidth::HalfWord,
                b'A'.into(),
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(handle.uart1_bytes(), b"A");
        assert!(handle.poll(SimTime::from_ticks(7)).is_empty());
        assert!(handle.poll(SimTime::from_ticks(8)).is_empty());
        assert_eq!(
            device.read(
                UCA1RXBUF as u64,
                AccessWidth::HalfWord,
                SimTime::from_ticks(8)
            ),
            Ok(u64::from(b'A'))
        );
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

    #[test]
    fn rtc_modulo_counter_sets_and_clears_overflow_interrupt() {
        let hub = SignalHub::new();
        let (mut device, handle, _gpio) =
            Msp430Peripherals::new("fr2433", hub).expect("signals should construct");
        device
            .write(RTCMOD as u64, AccessWidth::HalfWord, 3, SimTime::ZERO)
            .unwrap();
        device
            .write(
                RTCCTL as u64,
                AccessWidth::HalfWord,
                u64::from(RTCSS_MASK | RTCIE | RTCSR),
                SimTime::ZERO,
            )
            .unwrap();
        assert!(handle.poll(SimTime::from_ticks(3)).is_empty());
        assert_eq!(
            device.read(RTCCNT as u64, AccessWidth::HalfWord, SimTime::from_ticks(3)),
            Ok(3)
        );
        assert!(
            handle
                .poll(SimTime::from_ticks(4))
                .contains(&MSP430_RTC_VECTOR)
        );
        assert_eq!(
            device.read(RTCIV as u64, AccessWidth::HalfWord, SimTime::from_ticks(4)),
            Ok(2)
        );
        assert_eq!(
            device.read(RTCCTL as u64, AccessWidth::HalfWord, SimTime::from_ticks(4)),
            Ok((RTCSS_MASK | RTCIE).into())
        );
    }

    #[test]
    fn eusci_b0_spi_transfer_exposes_miso_and_interrupt_state() {
        let hub = SignalHub::new();
        let (mut device, handle, _gpio) =
            Msp430Peripherals::new("fr2433", hub).expect("signals should construct");
        device
            .write(
                UCB0CTLW0 as u64,
                AccessWidth::HalfWord,
                u64::from(UCSYNC | UCMST),
                SimTime::ZERO,
            )
            .unwrap();
        handle.inject_spi0_rx(0xa5);
        device
            .write(UCB0TXBUF as u64, AccessWidth::HalfWord, 0x3c, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.spi0_bytes(), [0x3c]);
        assert_eq!(
            device.read(UCB0RXBUF as u64, AccessWidth::HalfWord, SimTime::ZERO),
            Ok(0xa5)
        );
        device
            .write(
                UCB0IE as u64,
                AccessWidth::HalfWord,
                u64::from(UCRXIFG | UCTXIFG),
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(handle.poll(SimTime::ZERO), vec![MSP430_USCI_B0_VECTOR]);
        assert_eq!(
            device.read(UCB0IV as u64, AccessWidth::HalfWord, SimTime::ZERO),
            Ok(4)
        );
    }

    #[test]
    fn eusci_b0_i2c_host_records_write_start_and_stop() {
        let hub = SignalHub::new();
        let (mut device, handle, _gpio) =
            Msp430Peripherals::new("fr2433", hub).expect("signals should construct");
        let base = UCSYNC | UCMODE_I2C | UCMST;
        device
            .write(
                UCB0CTLW0 as u64,
                AccessWidth::HalfWord,
                u64::from(base | UCSWRST),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                UCB0CTLW0 as u64,
                AccessWidth::HalfWord,
                u64::from(base),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(UCB0I2CSA as u64, AccessWidth::HalfWord, 0x50, SimTime::ZERO)
            .unwrap();
        device
            .write(
                UCB0CTLW0 as u64,
                AccessWidth::HalfWord,
                u64::from(base | UCTR | UCTXSTT),
                SimTime::from_ticks(1),
            )
            .unwrap();
        device
            .write(
                UCB0TXBUF as u64,
                AccessWidth::HalfWord,
                0x10,
                SimTime::from_ticks(2),
            )
            .unwrap();
        device
            .write(
                UCB0CTLW0 as u64,
                AccessWidth::HalfWord,
                u64::from(base | UCTR | UCTXSTP),
                SimTime::from_ticks(3),
            )
            .unwrap();
        assert_eq!(
            handle.i2c_events(),
            [
                Msp430I2cEvent::Start,
                Msp430I2cEvent::Write {
                    address: 0x50,
                    value: 0x10,
                },
                Msp430I2cEvent::Stop,
            ]
        );
    }

    #[test]
    fn adc10_single_conversion_uses_injected_channel_and_interrupt() {
        let hub = SignalHub::new();
        let (mut device, handle, _gpio) =
            Msp430Peripherals::new("fr2433", hub).expect("signals should construct");
        handle.set_adc_input(3, 0x2aa);
        device
            .write(ADCMCTL0 as u64, AccessWidth::HalfWord, 3, SimTime::ZERO)
            .unwrap();
        device
            .write(
                ADCIE as u64,
                AccessWidth::HalfWord,
                u64::from(ADCIFG0),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                ADCCTL0 as u64,
                AccessWidth::HalfWord,
                u64::from(ADCON | ADCENC | ADCSC),
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            device.read(ADCCTL1 as u64, AccessWidth::HalfWord, SimTime::ZERO),
            Ok(u64::from(ADCBUSY))
        );
        assert!(handle.poll(SimTime::from_ticks(3)).is_empty());
        assert_eq!(handle.poll(SimTime::from_ticks(4)), vec![MSP430_ADC_VECTOR]);
        assert_eq!(
            device.read(
                ADCMEM0 as u64,
                AccessWidth::HalfWord,
                SimTime::from_ticks(4)
            ),
            Ok(0x2aa)
        );
        assert_eq!(
            device.read(ADCIV as u64, AccessWidth::HalfWord, SimTime::from_ticks(4)),
            Ok(2)
        );
        assert!(handle.poll(SimTime::from_ticks(5)).is_empty());
    }

    #[test]
    fn eusci_b0_i2c_host_supplies_queued_read_and_interrupt() {
        let hub = SignalHub::new();
        let (mut device, handle, _gpio) =
            Msp430Peripherals::new("fr2433", hub).expect("signals should construct");
        let base = UCSYNC | UCMODE_I2C | UCMST;
        device
            .write(
                UCB0CTLW0 as u64,
                AccessWidth::HalfWord,
                u64::from(base | UCSWRST),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                UCB0CTLW0 as u64,
                AccessWidth::HalfWord,
                u64::from(base),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(UCB0I2CSA as u64, AccessWidth::HalfWord, 0x44, SimTime::ZERO)
            .unwrap();
        handle.queue_i2c_read(0x44, [0x42, 0x43]);
        device
            .write(
                UCB0IE as u64,
                AccessWidth::HalfWord,
                u64::from(UCRXIFG),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                UCB0CTLW0 as u64,
                AccessWidth::HalfWord,
                u64::from(base | UCTXSTT),
                SimTime::from_ticks(1),
            )
            .unwrap();
        assert_eq!(handle.poll(SimTime::from_ticks(1)), [MSP430_USCI_B0_VECTOR]);
        assert_eq!(
            device.read(
                UCB0RXBUF as u64,
                AccessWidth::HalfWord,
                SimTime::from_ticks(2),
            ),
            Ok(0x42)
        );
        assert_eq!(
            device.read(
                UCB0RXBUF as u64,
                AccessWidth::HalfWord,
                SimTime::from_ticks(3),
            ),
            Ok(0x43)
        );
        assert_eq!(
            handle.i2c_events(),
            [
                Msp430I2cEvent::Start,
                Msp430I2cEvent::Read {
                    address: 0x44,
                    value: 0x42,
                },
                Msp430I2cEvent::Read {
                    address: 0x44,
                    value: 0x43,
                },
            ]
        );
    }
}
