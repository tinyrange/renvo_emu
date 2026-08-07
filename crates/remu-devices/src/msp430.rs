use super::{GpioHandle, GpioState, SignalHub, refresh_gpio, vendor_gpio};
use remu_bus::{Device, DeviceError, SharedMemory};
use remu_core::{AccessWidth, ResetKind, SimTime};
use remu_signals::{Logic, SignalId, SignalValue};
use std::sync::{Arc, Mutex};

const REGISTER_BYTES: usize = 0x1000;

const PM5CTL0: usize = 0x0130;
const SYSCFG0: usize = 0x0160;
const FRCTL0: usize = 0x01a0;
const GCCTL0: usize = 0x01a4;
const GCCTL1: usize = 0x01a6;
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

const LOCKLPM5: u16 = 0x0001;
const WDTHOLD: u16 = 0x0080;
const WDTPW: u16 = 0x5a00;
const UCSWRST: u16 = 0x0001;
const UCLISTEN: u8 = 0x80;
const CCIE: u16 = 0x0010;
const CCIFG: u16 = 0x0001;
const UCRXIFG: u16 = 0x0001;
const UCTXIFG: u16 = 0x0002;

const FRAM_PASSWORD: u16 = 0xa500;
const REGISTER_READ_PASSWORD: u16 = 0x9600;
const SYSCFG0_VALUE_MASK: u16 = 0x0003;
const FRCTL0_VALUE_MASK: u16 = 0x0070;
const GCCTL0_VALUE_MASK: u16 = 0x00e6;
const GCCTL1_VALUE_MASK: u16 = 0x000e;
const GCCTL0_UBDRSTEN: u16 = 0x0080;
const GCCTL0_UBDIE: u16 = 0x0040;
const GCCTL0_FRPWR: u16 = 0x0004;
const GCCTL0_FRLPMPWR: u16 = 0x0002;

/// Main program FRAM as exposed by the compatibility memory map.
pub const MSP430_PROGRAM_FRAM_START: u64 = 0xc000;
/// Main program FRAM compatibility window size.
pub const MSP430_PROGRAM_FRAM_SIZE: usize = 16 * 1024;
/// Information FRAM start address on the FR2433.
pub const MSP430_INFO_FRAM_START: u64 = 0x1800;
/// Information FRAM size on the FR2433.
pub const MSP430_INFO_FRAM_SIZE: usize = 512;

/// FR2433 interrupt vector addresses consumed by the MSP430 CPU adapter.
pub const MSP430_PORT1_VECTOR: u16 = 0xffdc;
/// eUSCI_A0 receive/transmit vector address.
pub const MSP430_USCI_A0_VECTOR: u16 = 0xffe4;
/// Timer0_A0 capture/compare vector address.
pub const MSP430_TIMER0_A0_VECTOR: u16 = 0xfff8;

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
    frctl_reset: bool,
    frctl_unlocked: bool,
    fram_write_ignored: u64,
    loopback_pending: Option<(u8, u64)>,
    uart_strobe: bool,
    uart_byte_signal: SignalId,
    uart_strobe_signal: SignalId,
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

    fn gpio_unlocked(&self) -> bool {
        self.word(PM5CTL0) & LOCKLPM5 == 0
    }

    fn fram_program_write_protected(&self) -> bool {
        self.word(SYSCFG0) & 0x0001 != 0
    }

    fn fram_info_write_protected(&self) -> bool {
        self.word(SYSCFG0) & 0x0002 != 0
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

    fn request_frctl_reset(&mut self) {
        self.frctl_reset = true;
    }

    fn write_frctl0(&mut self, value: u16) {
        if value & 0xff00 != FRAM_PASSWORD {
            self.frctl_unlocked = false;
            self.request_frctl_reset();
            return;
        }
        self.frctl_unlocked = true;
        self.set_word(FRCTL0, value & FRCTL0_VALUE_MASK);
    }

    fn write_gcctl0(&mut self, value: u16) {
        if !self.frctl_unlocked {
            self.request_frctl_reset();
            return;
        }
        let mut value = value & GCCTL0_VALUE_MASK;
        // UBDRSTEN and UBDIE are mutually exclusive. Prefer the reset action
        // if firmware accidentally requests both in one write.
        if value & GCCTL0_UBDRSTEN != 0 && value & GCCTL0_UBDIE != 0 {
            value &= !GCCTL0_UBDIE;
        }
        self.set_word(GCCTL0, value);
    }

    fn write_gcctl1(&mut self, value: u16) {
        if !self.frctl_unlocked {
            self.request_frctl_reset();
            return;
        }
        // ACCTEIFG/UBDIFG/CBDIFG are write-zero-to-clear flags.
        let current = self.word(GCCTL1) & GCCTL1_VALUE_MASK;
        self.set_word(GCCTL1, current & (value & GCCTL1_VALUE_MASK));
    }

    fn write_syscfg0(&mut self, value: u16) {
        // SYSCFG0's password and protection bits are written as one word. An
        // invalid password leaves the existing protection state untouched.
        if value & 0xff00 == FRAM_PASSWORD {
            self.set_word(
                SYSCFG0,
                REGISTER_READ_PASSWORD | (value & SYSCFG0_VALUE_MASK),
            );
        }
    }

    fn reset_registers(&mut self, at: SimTime) {
        self.registers.fill(0);
        self.set_word(PM5CTL0, LOCKLPM5);
        self.set_word(SYSCFG0, REGISTER_READ_PASSWORD | SYSCFG0_VALUE_MASK);
        self.set_word(FRCTL0, REGISTER_READ_PASSWORD);
        self.set_word(GCCTL0, GCCTL0_FRPWR | GCCTL0_FRLPMPWR);
        self.set_word(GCCTL1, 0);
        self.set_word(WDTCTL, 0x6900);
        self.set_word(UCA0CTLW0, UCSWRST);
        self.set_word(UCA0IFG, UCTXIFG);
        self.previous_p1 = 0;
        self.timer_epoch = at.ticks();
        self.watchdog_epoch = at.ticks();
        self.watchdog_reset = false;
        self.frctl_reset = false;
        self.frctl_unlocked = false;
        self.fram_write_ignored = 0;
        self.loopback_pending = None;
        self.set_signal(self.timer_irq_signal, 0, 1, at);
        self.set_signal(self.port1_irq_signal, 0, 1, at);
        self.set_signal(self.watchdog_reset_signal, 0, 1, at);
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

    /// Returns whether protected FRAM controller registers may currently be written.
    pub fn frctl_unlocked(&self) -> bool {
        self.0
            .lock()
            .expect("MSP430 peripheral lock poisoned")
            .frctl_unlocked
    }

    /// Returns whether program or information FRAM writes are protected.
    pub fn fram_write_protected(&self, information: bool) -> bool {
        let state = self.0.lock().expect("MSP430 peripheral lock poisoned");
        if information {
            state.fram_info_write_protected()
        } else {
            state.fram_program_write_protected()
        }
    }

    /// Current FRAM wait-state setting (FRCTL0.NWAITS).
    pub fn fram_wait_states(&self) -> u8 {
        let state = self.0.lock().expect("MSP430 peripheral lock poisoned");
        ((state.word(FRCTL0) & FRCTL0_VALUE_MASK) >> 4) as u8
    }

    /// Returns whether the FRAM array power bit is enabled.
    pub fn fram_powered(&self) -> bool {
        self.0
            .lock()
            .expect("MSP430 peripheral lock poisoned")
            .word(GCCTL0)
            & GCCTL0_FRPWR
            != 0
    }

    /// Models the device's automatic FRAM array wake-up on an access.
    pub fn power_fram(&self) {
        let mut state = self.0.lock().expect("MSP430 peripheral lock poisoned");
        let value = state.word(GCCTL0) | GCCTL0_FRPWR;
        state.set_word(GCCTL0, value & GCCTL0_VALUE_MASK);
    }

    /// Number of runtime writes ignored by FRAM write protection.
    pub fn fram_write_ignored(&self) -> u64 {
        self.0
            .lock()
            .expect("MSP430 peripheral lock poisoned")
            .fram_write_ignored
    }

    fn note_ignored_fram_write(&self) {
        let mut state = self.0.lock().expect("MSP430 peripheral lock poisoned");
        state.fram_write_ignored = state.fram_write_ignored.saturating_add(1);
    }

    /// Consumes a FRAM-controller protection fault, which is a PUC-like reset.
    pub fn take_frctl_reset(&self) -> bool {
        std::mem::take(
            &mut self
                .0
                .lock()
                .expect("MSP430 peripheral lock poisoned")
                .frctl_reset,
        )
    }
}

/// Functional, persistent FRAM window used by the FR2433 machine.
///
/// The controller owns protection and power state while this device owns only
/// the bytes. This keeps firmware loading and reset persistence independent of
/// the peripheral register window.
pub struct Msp430Fram {
    name: String,
    storage: SharedMemory,
    peripherals: Msp430PeripheralsHandle,
    information: bool,
}

impl Msp430Fram {
    /// Creates a program or information FRAM window over shared backing bytes.
    pub fn new(
        name: impl Into<String>,
        storage: SharedMemory,
        peripherals: Msp430PeripheralsHandle,
        information: bool,
    ) -> Self {
        Self {
            name: name.into(),
            storage,
            peripherals,
            information,
        }
    }

    fn range(&self, offset: u64, width: AccessWidth) -> Result<(usize, usize), DeviceError> {
        let start = usize::try_from(offset)
            .map_err(|_| DeviceError::new("MSP430 FRAM address does not fit usize"))?;
        let length = transfer_bytes(width);
        let end = start
            .checked_add(length)
            .ok_or_else(|| DeviceError::new("MSP430 FRAM access overflow"))?;
        if end > self.storage.len() {
            return Err(DeviceError::new(format!(
                "MSP430 FRAM access outside mapped window at {offset:#x}"
            )));
        }
        Ok((start, length))
    }
}

impl Device for Msp430Fram {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let (start, length) = self.range(offset, width)?;
        self.peripherals.power_fram();
        let bytes = self
            .storage
            .read_range(start, length)
            .expect("checked MSP430 FRAM read range");
        Ok(bytes
            .iter()
            .enumerate()
            .fold(0_u64, |value, (index, byte)| {
                value | (u64::from(*byte) << (index * 8))
            }))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        let (start, length) = self.range(offset, width)?;
        self.peripherals.power_fram();
        if self.peripherals.fram_write_protected(self.information) {
            // TI specifies a protected FRAM write as an ignored/invalid write,
            // not as a bus fault or reset. Keep a deterministic diagnostic
            // counter so tests can prove that the protection was exercised.
            self.peripherals.note_ignored_fram_write();
            return Ok(());
        }
        let bytes = (0..length)
            .map(|index| (value >> (index * 8)) as u8)
            .collect::<Vec<_>>();
        if self.storage.write_range(start, &bytes) {
            Ok(())
        } else {
            Err(DeviceError::new("MSP430 FRAM write backing range failed"))
        }
    }

    fn reset(&mut self, _kind: ResetKind) {
        // FRAM contents persist across PUC, watchdog and software resets.
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
            frctl_reset: false,
            frctl_unlocked: false,
            fram_write_ignored: 0,
            loopback_pending: None,
            uart_strobe: false,
            uart_byte_signal,
            uart_strobe_signal,
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
        if start == SYSCFG0 && length >= 2 {
            let low = state.word(SYSCFG0) & SYSCFG0_VALUE_MASK;
            state.set_word(SYSCFG0, REGISTER_READ_PASSWORD | low);
        }
        if start == FRCTL0 && length >= 2 {
            let low = state.word(FRCTL0) & FRCTL0_VALUE_MASK;
            state.set_word(FRCTL0, REGISTER_READ_PASSWORD | low);
        }
        if start == GCCTL0 && length >= 2 {
            let value = state.word(GCCTL0) & GCCTL0_VALUE_MASK;
            state.set_word(GCCTL0, value);
        }
        if start == GCCTL1 && length >= 2 {
            let value = state.word(GCCTL1) & GCCTL1_VALUE_MASK;
            state.set_word(GCCTL1, value);
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

        // Protected FRAM/SYS control registers have write semantics that are
        // not representable by a plain byte array. Keep these handlers ahead
        // of the generic register write path so passwords and write-one/zero
        // behaviours are applied atomically.
        if overlaps(start, length, SYSCFG0, 2) {
            if start == SYSCFG0 && length == 2 {
                state.write_syscfg0(value as u16);
            }
            return Ok(());
        }
        if start == FRCTL0 && length == 2 {
            state.write_frctl0(value as u16);
            return Ok(());
        }
        if start == FRCTL0 + 1 && length == 1 {
            if value as u8 == (FRAM_PASSWORD >> 8) as u8 {
                state.frctl_unlocked = true;
            } else {
                // In byte mode an invalid password disables access but does
                // not itself generate the word-write PUC.
                state.frctl_unlocked = false;
            }
            return Ok(());
        }
        if start == FRCTL0 && length == 1 {
            if state.frctl_unlocked {
                state.set_word(FRCTL0, value as u16 & FRCTL0_VALUE_MASK);
            } else {
                state.request_frctl_reset();
            }
            return Ok(());
        }
        if overlaps(start, length, FRCTL0, 2) {
            return Err(DeviceError::new(
                "MSP430 FRCTL0 requires an aligned byte or half-word access",
            ));
        }

        if start == GCCTL0 && length == 2 {
            state.write_gcctl0(value as u16);
            return Ok(());
        }
        if start == GCCTL0 && length == 1 {
            let value = (state.word(GCCTL0) & 0xff00) | (value as u16 & 0x00ff);
            state.write_gcctl0(value);
            return Ok(());
        }
        if start == GCCTL0 + 1 && length == 1 {
            let value = (state.word(GCCTL0) & 0x00ff) | ((value as u16) << 8);
            state.write_gcctl0(value);
            return Ok(());
        }
        if overlaps(start, length, GCCTL0, 2) {
            return Err(DeviceError::new(
                "MSP430 GCCTL0 requires an aligned byte or half-word access",
            ));
        }

        if start == GCCTL1 && length == 2 {
            state.write_gcctl1(value as u16);
            return Ok(());
        }
        if start == GCCTL1 && length == 1 {
            let value = (state.word(GCCTL1) & 0xff00) | (value as u16 & 0x00ff);
            state.write_gcctl1(value);
            return Ok(());
        }
        if start == GCCTL1 + 1 && length == 1 {
            let value = (state.word(GCCTL1) & 0x00ff) | ((value as u16) << 8);
            state.write_gcctl1(value);
            return Ok(());
        }
        if overlaps(start, length, GCCTL1, 2) {
            return Err(DeviceError::new(
                "MSP430 GCCTL1 requires an aligned byte or half-word access",
            ));
        }

        for index in 0..length {
            state.registers[start + index] = (value >> (index * 8)) as u8;
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
    fn fram_controller_exposes_passwords_masks_and_puc_on_locked_write() {
        let hub = SignalHub::new();
        let (mut device, handle, _gpio) =
            Msp430Peripherals::new("fr2433", hub).expect("signals should construct");
        assert_eq!(
            device.read(SYSCFG0 as u64, AccessWidth::HalfWord, SimTime::ZERO),
            Ok(0x9603)
        );
        assert_eq!(
            device.read(FRCTL0 as u64, AccessWidth::HalfWord, SimTime::ZERO),
            Ok(0x9600)
        );
        assert_eq!(
            device.read(GCCTL0 as u64, AccessWidth::HalfWord, SimTime::ZERO),
            Ok(0x0006)
        );
        device
            .write(FRCTL0 as u64, AccessWidth::HalfWord, 0x0030, SimTime::ZERO)
            .unwrap();
        assert!(handle.take_frctl_reset());
        assert_eq!(handle.fram_wait_states(), 0);

        device
            .write(FRCTL0 as u64, AccessWidth::HalfWord, 0xa5f3, SimTime::ZERO)
            .unwrap();
        assert!(handle.frctl_unlocked());
        assert_eq!(handle.fram_wait_states(), 7);
        assert_eq!(
            device.read(FRCTL0 as u64, AccessWidth::HalfWord, SimTime::ZERO),
            Ok(0x9670)
        );
        device
            .write(GCCTL0 as u64, AccessWidth::HalfWord, 0x00c6, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            device.read(GCCTL0 as u64, AccessWidth::HalfWord, SimTime::ZERO),
            Ok(0x0086)
        );
    }

    #[test]
    fn fram_write_protection_is_per_region_and_unlockable() {
        let hub = SignalHub::new();
        let (mut registers, handle, _gpio) =
            Msp430Peripherals::new("fr2433", hub).expect("signals should construct");
        let program = SharedMemory::zeroed(4);
        let information = SharedMemory::zeroed(4);
        let mut program_device = Msp430Fram::new("program", program.clone(), handle.clone(), false);
        let mut info_device = Msp430Fram::new("info", information.clone(), handle.clone(), true);

        program_device
            .write(0, AccessWidth::Byte, 0x5a, SimTime::ZERO)
            .unwrap();
        info_device
            .write(0, AccessWidth::Byte, 0xa5, SimTime::ZERO)
            .unwrap();
        assert_eq!(program.to_vec(), [0, 0, 0, 0]);
        assert_eq!(information.to_vec(), [0, 0, 0, 0]);
        assert_eq!(handle.fram_write_ignored(), 2);

        // PFWP/DFWP are changed only with the SYSCFG0 password in the same
        // half-word write. Clear both bits, then writes reach their backing.
        registers
            .write(SYSCFG0 as u64, AccessWidth::HalfWord, 0xa500, SimTime::ZERO)
            .unwrap();
        program_device
            .write(0, AccessWidth::Byte, 0x5a, SimTime::ZERO)
            .unwrap();
        info_device
            .write(0, AccessWidth::Byte, 0xa5, SimTime::ZERO)
            .unwrap();
        assert_eq!(program.to_vec(), [0x5a, 0, 0, 0]);
        assert_eq!(information.to_vec(), [0xa5, 0, 0, 0]);

        registers
            .write(SYSCFG0 as u64, AccessWidth::HalfWord, 0xa503, SimTime::ZERO)
            .unwrap();
        program_device
            .write(0, AccessWidth::Byte, 0xff, SimTime::ZERO)
            .unwrap();
        assert_eq!(program.to_vec()[0], 0x5a);
    }

    #[test]
    fn fram_access_powers_array_after_fram_power_bit_is_cleared() {
        let hub = SignalHub::new();
        let (mut registers, handle, _gpio) =
            Msp430Peripherals::new("fr2433", hub).expect("signals should construct");
        let storage = SharedMemory::from_bytes(vec![0x42]);
        let mut fram = Msp430Fram::new("program", storage, handle.clone(), false);
        registers
            .write(FRCTL0 as u64, AccessWidth::HalfWord, 0xa500, SimTime::ZERO)
            .unwrap();
        registers
            .write(GCCTL0 as u64, AccessWidth::HalfWord, 0, SimTime::ZERO)
            .unwrap();
        assert!(!handle.fram_powered());
        assert_eq!(fram.read(0, AccessWidth::Byte, SimTime::ZERO), Ok(0x42));
        assert!(handle.fram_powered());
    }
}
