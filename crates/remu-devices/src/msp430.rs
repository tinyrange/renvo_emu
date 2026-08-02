use super::{GpioHandle, GpioState, SignalHub, refresh_gpio, vendor_gpio};
use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use remu_signals::{Logic, SignalId, SignalValue};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

const REGISTER_BYTES: usize = 0x1000;

const PM5CTL0: usize = 0x0130;
const FRCTL0: usize = 0x01a0;
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

const TA0CTL: usize = 0x0380;
const TA0CCTL0: usize = 0x0382;
const TA0R: usize = 0x0390;
const TA0CCR0: usize = 0x0392;

const UCA0CTLW0: usize = 0x0500;
const UCA0STATW: usize = 0x050a;
const UCA0RXBUF: usize = 0x050c;
const UCA0TXBUF: usize = 0x050e;
const UCA0IE: usize = 0x051a;
const UCA0IFG: usize = 0x051c;
const UCA0IV: usize = 0x051e;

/// FR2433 eUSCI_B0 register identities from the TI device table.
///
/// Keeping the addresses behind a named type makes register-level tests and
/// host integrations harder to accidentally couple to an unlabelled integer.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(usize)]
pub enum Msp430EusciB0Register {
    /// Control word 0.
    Ctlw0 = 0x0540,
    /// Control word 1.
    Ctlw1 = 0x0542,
    /// Bit-rate control word (UCB0BR0/UCB0BR1).
    Brw = 0x0546,
    /// Read-only status word.
    Statw = 0x0548,
    /// Automatic-stop byte-counter threshold.
    TbCnt = 0x054a,
    /// Receive buffer.
    RxBuf = 0x054c,
    /// Transmit buffer.
    TxBuf = 0x054e,
    /// Own address 0.
    I2cOa0 = 0x0554,
    /// Own address 1.
    I2cOa1 = 0x0556,
    /// Own address 2.
    I2cOa2 = 0x0558,
    /// Own address 3.
    I2cOa3 = 0x055a,
    /// Received address.
    AddrX = 0x055c,
    /// Address mask.
    AddMask = 0x055e,
    /// Slave address used by master transactions.
    I2cSa = 0x0560,
    /// Interrupt enables.
    Ie = 0x056a,
    /// Interrupt flags.
    Ifg = 0x056c,
    /// Interrupt vector.
    Iv = 0x056e,
}

impl Msp430EusciB0Register {
    /// Returns the unified FR2433 peripheral-window address.
    pub const fn address(self) -> usize {
        self as usize
    }

    /// Resolves an exact register address to its named identity.
    pub const fn from_address(address: usize) -> Option<Self> {
        match address {
            0x0540 => Some(Self::Ctlw0),
            0x0542 => Some(Self::Ctlw1),
            0x0546 => Some(Self::Brw),
            0x0548 => Some(Self::Statw),
            0x054a => Some(Self::TbCnt),
            0x054c => Some(Self::RxBuf),
            0x054e => Some(Self::TxBuf),
            0x0554 => Some(Self::I2cOa0),
            0x0556 => Some(Self::I2cOa1),
            0x0558 => Some(Self::I2cOa2),
            0x055a => Some(Self::I2cOa3),
            0x055c => Some(Self::AddrX),
            0x055e => Some(Self::AddMask),
            0x0560 => Some(Self::I2cSa),
            0x056a => Some(Self::Ie),
            0x056c => Some(Self::Ifg),
            0x056e => Some(Self::Iv),
            _ => None,
        }
    }
}

const UCB0CTLW0: usize = Msp430EusciB0Register::Ctlw0.address();
const UCB0CTLW1: usize = Msp430EusciB0Register::Ctlw1.address();
const UCB0BRW: usize = Msp430EusciB0Register::Brw.address();
const UCB0STATW: usize = Msp430EusciB0Register::Statw.address();
const UCB0TBCNT: usize = Msp430EusciB0Register::TbCnt.address();
const UCB0RXBUF: usize = Msp430EusciB0Register::RxBuf.address();
const UCB0TXBUF: usize = Msp430EusciB0Register::TxBuf.address();
const UCB0I2COA0: usize = Msp430EusciB0Register::I2cOa0.address();
const UCB0I2COA1: usize = Msp430EusciB0Register::I2cOa1.address();
const UCB0I2COA2: usize = Msp430EusciB0Register::I2cOa2.address();
const UCB0I2COA3: usize = Msp430EusciB0Register::I2cOa3.address();
const UCB0ADDRX: usize = Msp430EusciB0Register::AddrX.address();
const UCB0ADDMASK: usize = Msp430EusciB0Register::AddMask.address();
const UCB0I2CSA: usize = Msp430EusciB0Register::I2cSa.address();
const UCB0IE: usize = Msp430EusciB0Register::Ie.address();
const UCB0IFG: usize = Msp430EusciB0Register::Ifg.address();
const UCB0IV: usize = Msp430EusciB0Register::Iv.address();

const LOCKLPM5: u16 = 0x0001;
const WDTHOLD: u16 = 0x0080;
const WDTPW: u16 = 0x5a00;
const UCSWRST: u16 = 0x0001;
const UCLISTEN: u8 = 0x80;
const UCTR: u16 = 1 << 4;
const UCTXSTT: u16 = 1 << 1;
const UCTXSTP: u16 = 1 << 2;
const UCTXNACK: u16 = 1 << 3;
const UCMODE_I2C: u16 = 0x0600;
const UCMODE_MASK: u16 = 0x0600;
const UCMST: u16 = 0x0800;
const UCSYNC: u16 = 0x0100;
const UCSSEL_MASK: u16 = 0x00c0;
const UCA10: u16 = 1 << 15;
const UCSLA10: u16 = 1 << 14;
const UCMM: u16 = 1 << 13;
const UCTXACK: u16 = 1 << 5;
const UCB0_CONFIG_MASK: u16 = UCA10 | UCSLA10 | UCMM | UCMST | UCMODE_MASK | UCSYNC | UCSSEL_MASK;
const UCB0_RUNTIME_CONTROL_MASK: u16 = UCTXACK | UCTR | UCTXNACK | UCTXSTP | UCTXSTT | UCSWRST;
const UCB0_IFG_MASK: u16 = 0x7fff;
const UCB0_IE_MASK: u16 = 0x7fff;
const UCBBUSY: u16 = 1 << 4;
const UCASTP_MASK: u16 = 0x000c;
const UCASTP_STOP: u16 = 0x0008;
const UCBCNTIFG: u16 = 1 << 6;
const UCCLTOIFG: u16 = 1 << 7;
const UCSTTIFG: u16 = 1 << 2;
const UCSTPIFG: u16 = 1 << 3;
const UCNACKIFG: u16 = 1 << 5;
const UCALIFG: u16 = 1 << 4;
const UCBIT9IFG: u16 = 1 << 14;
const UCB0_INTERRUPT_FLAGS: u16 = UCRXIFG
    | UCTXIFG
    | UCSTTIFG
    | UCSTPIFG
    | UCNACKIFG
    | UCALIFG
    | UCBCNTIFG
    | UCCLTOIFG
    | UCBIT9IFG;
const CCIE: u16 = 0x0010;
const CCIFG: u16 = 0x0001;
const UCRXIFG: u16 = 0x0001;
const UCTXIFG: u16 = 0x0002;

/// FR2433 interrupt vector addresses consumed by the MSP430 CPU adapter.
pub const MSP430_PORT1_VECTOR: u16 = 0xffdc;
/// eUSCI_A0 receive/transmit vector address.
pub const MSP430_USCI_A0_VECTOR: u16 = 0xffe4;
/// eUSCI_B0 receive/transmit vector address.
pub const MSP430_USCI_B0_VECTOR: u16 = 0xffe0;
/// Timer0_A0 capture/compare vector address.
pub const MSP430_TIMER0_A0_VECTOR: u16 = 0xfff8;

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
    /// A target did not acknowledge its address.
    Nack {
        /// Seven-bit target address selected in UCB0I2CSA.
        address: u16,
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
    previous_p1: u8,
    timer_epoch: u64,
    watchdog_epoch: u64,
    watchdog_reset: bool,
    loopback_pending: Option<(u8, u64)>,
    i2c_events: Vec<Msp430I2cEvent>,
    i2c_responses: BTreeMap<u16, VecDeque<u8>>,
    i2c_acknowledgements: BTreeMap<u16, bool>,
    i2c_active_address: Option<u16>,
    i2c_read: bool,
    i2c_byte_count: u8,
    i2c_strobe: bool,
    uart_strobe: bool,
    uart_byte_signal: SignalId,
    uart_strobe_signal: SignalId,
    i2c_byte_signal: SignalId,
    i2c_strobe_signal: SignalId,
    timer_irq_signal: SignalId,
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

    fn mask_word(&mut self, address: usize, mask: u16) {
        let value = self.word(address) & mask;
        self.set_word(address, value);
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

    fn set_i2c_status(&mut self, busy: bool) {
        let status = (u16::from(self.i2c_byte_count) << 8) | u16::from(busy) * UCBBUSY;
        self.set_word(UCB0STATW, status);
    }

    fn set_i2c_ifg(&mut self, mask: u16, enabled: bool) {
        let flags = self.word(UCB0IFG);
        self.set_word(UCB0IFG, if enabled { flags | mask } else { flags & !mask });
    }

    fn i2c_target_acknowledges(&self, address: u16) -> bool {
        self.i2c_acknowledgements
            .get(&address)
            .copied()
            .unwrap_or(true)
    }

    fn i2c_count_byte(&mut self, at: SimTime) {
        self.i2c_byte_count = self.i2c_byte_count.saturating_add(1);
        self.set_i2c_status(true);
        let threshold = self.registers[UCB0TBCNT];
        let automatic_stop = self.word(UCB0CTLW1) & UCASTP_MASK;
        if threshold != 0 && self.i2c_byte_count == threshold && automatic_stop != 0 {
            self.set_i2c_ifg(UCBCNTIFG, true);
            if automatic_stop == UCASTP_STOP {
                self.i2c_stop(at);
            }
        }
    }

    fn i2c_stop(&mut self, _at: SimTime) {
        if self.i2c_active_address.take().is_some() {
            self.i2c_events.push(Msp430I2cEvent::Stop);
        }
        self.i2c_read = false;
        self.set_i2c_status(false);
        self.set_i2c_ifg(UCSTPIFG | UCTXIFG, true);
        self.set_word(UCB0CTLW0, self.word(UCB0CTLW0) & !UCTXSTP);
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
        self.i2c_byte_count = 0;
        self.set_i2c_status(true);
        self.set_i2c_ifg(UCRXIFG | UCNACKIFG, false);
        self.set_word(UCB0CTLW0, self.word(UCB0CTLW0) & !UCTXSTT);
        if !self.i2c_target_acknowledges(address) {
            self.i2c_events.push(Msp430I2cEvent::Nack { address });
            self.set_i2c_ifg(UCNACKIFG | UCTXIFG, true);
            return;
        }
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
            self.i2c_count_byte(at);
        } else {
            self.set_word(UCB0IFG, self.word(UCB0IFG) | UCTXIFG);
        }
    }

    fn i2c_control_write(&mut self, previous: u16, at: SimTime) {
        let requested = self.word(UCB0CTLW0);
        let was_reset = previous & UCSWRST != 0;
        let control = if was_reset {
            requested & (UCB0_CONFIG_MASK | UCB0_RUNTIME_CONTROL_MASK)
        } else {
            (previous & UCB0_CONFIG_MASK) | (requested & UCB0_RUNTIME_CONTROL_MASK)
        } | UCSYNC;
        self.set_word(UCB0CTLW0, control);
        if control & UCSWRST != 0 {
            if !was_reset {
                self.i2c_active_address = None;
                self.i2c_read = false;
                self.i2c_byte_count = 0;
                self.set_i2c_status(false);
                self.set_word(UCB0IFG, UCTXIFG);
            }
            return;
        }
        if !self.eusci_b0_i2c_master() {
            return;
        }
        if control & UCTXSTT != 0 {
            self.i2c_start(control, at);
        }
        if control & UCTXSTP != 0 {
            self.i2c_stop(at);
        }
        // START, STOP, and NACK are command bits in the real peripheral and
        // clear once accepted. Keeping that behavior makes polling loops in
        // vendor drivers deterministic without adding bus-level timing.
        self.set_word(UCB0CTLW0, control & !(UCTXSTT | UCTXSTP | UCTXNACK));
    }

    fn i2c_tx_write(&mut self, value: u8, at: SimTime) {
        self.set_i2c_ifg(UCTXIFG, false);
        let Some(address) = self.i2c_active_address else {
            return;
        };
        if self.i2c_read {
            return;
        }
        self.i2c_events
            .push(Msp430I2cEvent::Write { address, value });
        self.emit_i2c_byte(value, at);
        self.i2c_count_byte(at);
        self.set_i2c_ifg(UCTXIFG, true);
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
        self.i2c_count_byte(at);
    }

    fn i2c_vector(flags: u16) -> (u16, u16) {
        [
            (UCALIFG, 0x02),
            (UCNACKIFG, 0x04),
            (UCSTTIFG, 0x06),
            (UCSTPIFG, 0x08),
            (1 << 12, 0x0a),
            (1 << 13, 0x0c),
            (1 << 10, 0x0e),
            (1 << 11, 0x10),
            (1 << 8, 0x12),
            (1 << 9, 0x14),
            (UCRXIFG, 0x16),
            (UCTXIFG, 0x18),
            (UCBCNTIFG, 0x1a),
            (UCCLTOIFG, 0x1c),
            (UCBIT9IFG, 0x1e),
        ]
        .into_iter()
        .find(|(mask, _)| flags & mask != 0)
        .map_or((0, 0), |(mask, vector)| (mask, vector))
    }

    fn reset_registers(&mut self, at: SimTime) {
        self.registers.fill(0);
        self.set_word(PM5CTL0, LOCKLPM5);
        self.set_word(WDTCTL, 0x6900);
        self.set_word(UCA0CTLW0, UCSWRST);
        self.set_word(UCA0IFG, UCTXIFG);
        self.set_word(UCB0CTLW0, UCSYNC | UCSSEL_MASK | UCSWRST);
        self.set_word(UCB0IFG, UCTXIFG);
        self.previous_p1 = 0;
        self.timer_epoch = at.ticks();
        self.watchdog_epoch = at.ticks();
        self.watchdog_reset = false;
        self.loopback_pending = None;
        self.i2c_events.clear();
        self.i2c_responses.clear();
        self.i2c_acknowledgements.clear();
        self.i2c_active_address = None;
        self.i2c_read = false;
        self.i2c_byte_count = 0;
        self.i2c_strobe = false;
        self.set_signal(self.timer_irq_signal, 0, 1, at);
        self.set_signal(self.port1_irq_signal, 0, 1, at);
        self.set_signal(self.watchdog_reset_signal, 0, 1, at);
        self.set_signal(self.i2c_byte_signal, 0, 8, at);
        self.set_signal(self.i2c_strobe_signal, 0, 1, at);
        self.set_i2c_status(false);
        let _ = self.refresh_ports(at);
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

    /// Configures whether a queued 7-bit target acknowledges its address.
    ///
    /// Targets acknowledge by default, which keeps simple functional fixtures
    /// useful without an extra setup call. Set this to `false` to exercise the
    /// UCNACKIFG/error path described by TI's eUSCI_B I²C state machine.
    pub fn set_i2c_ack(&self, address: u16, acknowledge: bool) {
        self.0
            .lock()
            .expect("MSP430 peripheral lock poisoned")
            .i2c_acknowledgements
            .insert(address & 0x007f, acknowledge);
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
        state.set_signal(state.timer_irq_signal, 0, 1, now);

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

        let timer_control = state.word(TA0CTL);
        if timer_control & 0x0030 != 0 {
            let period = u64::from(state.word(TA0CCR0)).saturating_add(1).max(1);
            let elapsed = now.ticks().saturating_sub(state.timer_epoch);
            state.set_word(TA0R, (elapsed % period) as u16);
            if elapsed >= period {
                state.timer_epoch = now.ticks();
                let control = state.word(TA0CCTL0) | CCIFG;
                state.set_word(TA0CCTL0, control);
            }
        }
        if state.word(TA0CCTL0) & (CCIE | CCIFG) == (CCIE | CCIFG) {
            state.set_signal(state.timer_irq_signal, 1, 1, now);
            let control = state.word(TA0CCTL0) & !CCIFG;
            state.set_word(TA0CCTL0, control);
            vectors.push(MSP430_TIMER0_A0_VECTOR);
        }

        if state.word(UCA0IE) & state.word(UCA0IFG) & (UCRXIFG | UCTXIFG) != 0 {
            vectors.push(MSP430_USCI_A0_VECTOR);
        }
        if state.word(UCB0IE) & state.word(UCB0IFG) & UCB0_INTERRUPT_FLAGS != 0 {
            vectors.push(MSP430_USCI_B0_VECTOR);
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
        let timer_irq_signal = hub.declare(
            "board.msp430fr2433.timer_a0.ccr0_irq",
            SignalValue::from_u64(0, 1)?,
            Some("Timer_A0 CCR0 interrupt request".to_owned()),
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
            previous_p1: 0,
            timer_epoch: 0,
            watchdog_epoch: 0,
            watchdog_reset: false,
            loopback_pending: None,
            i2c_events: Vec::new(),
            i2c_responses: BTreeMap::new(),
            i2c_acknowledgements: BTreeMap::new(),
            i2c_active_address: None,
            i2c_read: false,
            i2c_byte_count: 0,
            i2c_strobe: false,
            uart_strobe: false,
            uart_byte_signal,
            uart_strobe_signal,
            i2c_byte_signal,
            i2c_strobe_signal,
            timer_irq_signal,
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
        if overlaps(start, length, UCB0IV, 2) {
            let pending = state.word(UCB0IFG) & state.word(UCB0IE) & UCB0_INTERRUPT_FLAGS;
            let (flag, vector) = Msp430State::i2c_vector(pending);
            state.set_word(UCB0IV, vector);
            if flag != 0 {
                state.mask_word(UCB0IFG, !flag);
            }
        }
        if overlaps(start, length, UCA0RXBUF, 2) {
            let flags = state.word(UCA0IFG) & !UCRXIFG;
            state.set_word(UCA0IFG, flags);
        }
        let i2c_rx_read = overlaps(start, length, UCB0RXBUF, 2)
            && state.i2c_active_address.is_some()
            && state.i2c_read;
        if overlaps(start, length, UCB0RXBUF, 2) {
            let flags = state.word(UCB0IFG) & !UCRXIFG;
            state.set_word(UCB0IFG, flags);
            state.mask_word(UCB0RXBUF, 0x00ff);
        }
        if overlaps(start, length, UCB0STATW, 2) {
            let busy = state.i2c_active_address.is_some();
            state.set_i2c_status(busy);
        }
        if overlaps(start, length, UCB0IFG, 2) {
            state.mask_word(UCB0IFG, UCB0_IFG_MASK);
        }
        if overlaps(start, length, UCB0IE, 2) {
            state.mask_word(UCB0IE, UCB0_IE_MASK);
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
        let previous_control = state.word(UCB0CTLW0);
        let previous_ctlw1 = state.word(UCB0CTLW1);
        let previous_brw = state.word(UCB0BRW);
        let previous_tbcnt = state.word(UCB0TBCNT);
        let previous_rxbuf = state.word(UCB0RXBUF);
        let previous_statw = state.word(UCB0STATW);
        let previous_addrx = state.word(UCB0ADDRX);
        let previous_i2coa = [
            state.word(UCB0I2COA0),
            state.word(UCB0I2COA1),
            state.word(UCB0I2COA2),
            state.word(UCB0I2COA3),
        ];
        for index in 0..length {
            state.registers[start + index] = (value >> (index * 8)) as u8;
        }
        if overlaps(start, length, UCB0CTLW0, 2) {
            state.i2c_control_write(previous_control, at);
        }
        let was_reset = previous_control & UCSWRST != 0;
        if overlaps(start, length, UCB0CTLW1, 2) {
            if was_reset {
                state.mask_word(UCB0CTLW1, 0x01ff);
            } else {
                state.set_word(UCB0CTLW1, previous_ctlw1);
            }
        }
        if overlaps(start, length, UCB0BRW, 2) && !was_reset {
            state.set_word(UCB0BRW, previous_brw);
        }
        if overlaps(start, length, UCB0TBCNT, 2) {
            if was_reset {
                state.mask_word(UCB0TBCNT, 0x00ff);
            } else {
                state.set_word(UCB0TBCNT, previous_tbcnt);
            }
        }
        if overlaps(start, length, UCB0RXBUF, 2) {
            state.set_word(UCB0RXBUF, previous_rxbuf & 0x00ff);
        }
        if overlaps(start, length, UCB0STATW, 2) {
            state.set_word(UCB0STATW, previous_statw);
        }
        if overlaps(start, length, UCB0ADDRX, 2) {
            state.set_word(UCB0ADDRX, previous_addrx & 0x03ff);
        }
        for (register, previous) in [
            (UCB0I2COA0, previous_i2coa[0]),
            (UCB0I2COA1, previous_i2coa[1]),
            (UCB0I2COA2, previous_i2coa[2]),
            (UCB0I2COA3, previous_i2coa[3]),
        ] {
            if overlaps(start, length, register, 2) {
                if was_reset {
                    state.mask_word(register, 0x87ff);
                } else {
                    state.set_word(register, previous);
                }
            }
        }
        if overlaps(start, length, UCB0ADDMASK, 2) && was_reset {
            state.mask_word(UCB0ADDMASK, 0x03ff);
        }
        if overlaps(start, length, UCB0I2CSA, 2) {
            state.mask_word(UCB0I2CSA, 0x03ff);
        }
        if overlaps(start, length, UCB0IE, 2) {
            state.mask_word(UCB0IE, UCB0_IE_MASK);
        }
        if overlaps(start, length, UCB0IFG, 2) {
            state.mask_word(UCB0IFG, UCB0_IFG_MASK);
        }
        if overlaps(start, length, UCB0IV, 2) {
            state.set_word(UCB0IFG, 0);
            state.set_word(UCB0IV, 0);
        }
        if overlaps(start, length, UCB0TXBUF, 2) {
            state.mask_word(UCB0TXBUF, 0x00ff);
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
        if overlaps(start, length, TA0CTL, 2) || overlaps(start, length, TA0CCR0, 2) {
            state.timer_epoch = at.ticks();
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
    fn eusci_b0_register_ids_match_ti_map() {
        assert_eq!(Msp430EusciB0Register::Ctlw0.address(), 0x0540);
        assert_eq!(Msp430EusciB0Register::TbCnt.address(), 0x054a);
        assert_eq!(Msp430EusciB0Register::I2cSa.address(), 0x0560);
        assert_eq!(
            Msp430EusciB0Register::from_address(0x056e),
            Some(Msp430EusciB0Register::Iv)
        );
        assert_eq!(Msp430EusciB0Register::from_address(0x0544), None);
    }
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

    #[test]
    fn eusci_b0_honors_reset_write_protection_and_reserved_bits() {
        let hub = SignalHub::new();
        let (mut device, _handle, _gpio) =
            Msp430Peripherals::new("fr2433", hub).expect("signals should construct");
        device
            .write(
                UCB0CTLW1 as u64,
                AccessWidth::HalfWord,
                0x01ff,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                UCB0TBCNT as u64,
                AccessWidth::HalfWord,
                0x01ff,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(UCB0BRW as u64, AccessWidth::HalfWord, 0x1234, SimTime::ZERO)
            .unwrap();
        device
            .write(
                UCB0CTLW0 as u64,
                AccessWidth::HalfWord,
                u64::from(UCMODE_I2C | UCMST | UCSYNC),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                UCB0CTLW1 as u64,
                AccessWidth::HalfWord,
                0,
                SimTime::from_ticks(1),
            )
            .unwrap();
        device
            .write(
                UCB0TBCNT as u64,
                AccessWidth::HalfWord,
                0,
                SimTime::from_ticks(1),
            )
            .unwrap();
        device
            .write(
                UCB0BRW as u64,
                AccessWidth::HalfWord,
                0,
                SimTime::from_ticks(1),
            )
            .unwrap();
        assert_eq!(
            device.read(
                UCB0CTLW0 as u64,
                AccessWidth::HalfWord,
                SimTime::from_ticks(2)
            ),
            Ok(u64::from(UCMODE_I2C | UCMST | UCSYNC))
        );
        assert_eq!(
            device.read(
                UCB0CTLW1 as u64,
                AccessWidth::HalfWord,
                SimTime::from_ticks(2)
            ),
            Ok(0x01ff)
        );
        assert_eq!(
            device.read(
                UCB0TBCNT as u64,
                AccessWidth::HalfWord,
                SimTime::from_ticks(2)
            ),
            Ok(0x00ff)
        );
        assert_eq!(
            device.read(
                UCB0BRW as u64,
                AccessWidth::HalfWord,
                SimTime::from_ticks(2)
            ),
            Ok(0x1234)
        );
    }

    #[test]
    fn eusci_b0_nack_sets_error_and_iv_clears_it() {
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
            .write(UCB0I2CSA as u64, AccessWidth::HalfWord, 0x52, SimTime::ZERO)
            .unwrap();
        handle.set_i2c_ack(0x52, false);
        device
            .write(
                UCB0IE as u64,
                AccessWidth::HalfWord,
                u64::from(UCNACKIFG),
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
            device.read(UCB0IV as u64, AccessWidth::HalfWord, SimTime::from_ticks(2)),
            Ok(0x04)
        );
        assert_eq!(
            device.read(
                UCB0IFG as u64,
                AccessWidth::HalfWord,
                SimTime::from_ticks(2)
            ),
            Ok(UCTXIFG.into())
        );
        assert_eq!(
            handle.i2c_events(),
            [
                Msp430I2cEvent::Start,
                Msp430I2cEvent::Nack { address: 0x52 },
            ]
        );
    }

    #[test]
    fn eusci_b0_byte_counter_can_generate_an_automatic_stop() {
        let hub = SignalHub::new();
        let (mut device, handle, _gpio) =
            Msp430Peripherals::new("fr2433", hub).expect("signals should construct");
        let base = UCSYNC | UCMODE_I2C | UCMST;
        device
            .write(
                UCB0CTLW1 as u64,
                AccessWidth::HalfWord,
                u64::from(UCASTP_STOP),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(UCB0TBCNT as u64, AccessWidth::HalfWord, 2, SimTime::ZERO)
            .unwrap();
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
                UCB0TXBUF as u64,
                AccessWidth::HalfWord,
                0x20,
                SimTime::from_ticks(3),
            )
            .unwrap();
        assert_eq!(
            device.read(
                UCB0STATW as u64,
                AccessWidth::HalfWord,
                SimTime::from_ticks(4)
            ),
            Ok(0x0200)
        );
        assert_eq!(
            device.read(
                UCB0IFG as u64,
                AccessWidth::HalfWord,
                SimTime::from_ticks(4)
            ),
            Ok((UCBCNTIFG | UCSTPIFG | UCTXIFG).into())
        );
        assert_eq!(
            handle.i2c_events(),
            [
                Msp430I2cEvent::Start,
                Msp430I2cEvent::Write {
                    address: 0x50,
                    value: 0x10,
                },
                Msp430I2cEvent::Write {
                    address: 0x50,
                    value: 0x20,
                },
                Msp430I2cEvent::Stop,
            ]
        );
    }
}
