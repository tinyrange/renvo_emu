use super::{GpioHandle, GpioState, SignalHub, refresh_gpio, vendor_gpio};
use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use remu_signals::{Logic, SignalId, SignalValue};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

const SFR_BYTES: usize = 0x1_0000;
const PAGE3: usize = 0x20;
const P0: usize = 0x80;
const TCON: usize = 0x88;
const TMOD: usize = 0x89;
const TL0: usize = 0x8a;
const TH0: usize = 0x8c;
const P1: usize = 0x90;
const WDTCN: usize = 0x97;
const SCON0: usize = 0x98;
const SBUF0: usize = 0x99;
const SMB0CN0: usize = 0xc0;
const SMB0CF: usize = 0xc1;
const SMB0DAT: usize = 0xc2;
const SMB0TC: usize = 0xac;
const SMB0ADM: usize = 0xd6;
const SMB0ADR: usize = 0xd7;
const EIE1: usize = 0xe6;
const P3MDOUT: usize = (PAGE3 << 8) | 0x9c;
const P2: usize = 0xa0;
const P0MDOUT: usize = 0xa4;
const P1MDOUT: usize = 0xa5;
const P2MDOUT: usize = 0xa6;
const IE: usize = 0xa8;
const CLKSEL: usize = 0xa9;
const P3: usize = 0xb0;
const IP: usize = 0xb8;
const TMR2CN0: usize = 0xc8;
const TMR2RLL: usize = 0xca;
const TMR2RLH: usize = 0xcb;
const TMR2L: usize = 0xce;
const TMR2H: usize = 0xcf;
const XBR0: usize = 0xe1;
const XBR2: usize = 0xe3;
const RSTSRC: usize = 0xef;
const P0MDIN: usize = 0xf1;
const P1MDIN: usize = 0xf2;
const P2MDIN: usize = 0xf3;
const P3MDIN: usize = (PAGE3 << 8) | 0xf4;
const SMB0FCN0: usize = (PAGE3 << 8) | 0xc3;
const SMB0FCN1: usize = (PAGE3 << 8) | 0xc4;
const SMB0RXLN: usize = (PAGE3 << 8) | 0xc5;
const SMB0FCT: usize = (PAGE3 << 8) | 0xef;

const PORTS: [usize; 4] = [P0, P1, P2, P3];
const PORT_WIDTHS: [u8; 4] = [8, 8, 8, 5];
const PORT_MASKS: [u8; 4] = [0xff, 0xff, 0xff, 0x1f];
const PORT_MDOUT: [usize; 4] = [P0MDOUT, P1MDOUT, P2MDOUT, P3MDOUT];
const PORT_MDIN: [usize; 4] = [P0MDIN, P1MDIN, P2MDIN, P3MDIN];

const IE_EA: u8 = 0x80;
const IE_ET0: u8 = 0x02;
const IE_ES0: u8 = 0x10;
const IE_ET2: u8 = 0x20;
const TCON_TR0: u8 = 0x10;
const TCON_TF0: u8 = 0x20;
const TMR2_TR2: u8 = 0x04;
const TMR2_TF2H: u8 = 0x80;
const SCON0_RI: u8 = 0x01;
const SCON0_TI: u8 = 0x02;
const SMB0CN0_MASTER: u8 = 1 << 7;
const SMB0CN0_TXMODE: u8 = 1 << 6;
const SMB0CN0_STA: u8 = 1 << 5;
const SMB0CN0_STO: u8 = 1 << 4;
const SMB0CN0_ACKRQ: u8 = 1 << 3;
const SMB0CN0_ARBLOST: u8 = 1 << 2;
const SMB0CN0_ACK: u8 = 1 << 1;
const SMB0CN0_SI: u8 = 1;
const SMB0CF_ENSMB: u8 = 1 << 7;
const SMB0CF_BUSY: u8 = 1 << 5;
const EIE1_ESMB0: u8 = 1;
const XBR0_URT0E: u8 = 0x01;
const XBR2_XBARE: u8 = 0x40;

struct Efm8State {
    registers: Box<[u8]>,
    ports: [Arc<Mutex<GpioState>>; 4],
    port_signals: [Vec<SignalId>; 4],
    hub: SignalHub,
    uart: Vec<u8>,
    smbus0_tx: Vec<u8>,
    smbus0_rx: VecDeque<u8>,
    timer0_epoch: u64,
    timer2_epoch: u64,
    watchdog_epoch: u64,
    watchdog_key: u8,
    watchdog_enabled: bool,
    watchdog_reset: bool,
    uart_byte_signal: SignalId,
    uart_strobe_signal: SignalId,
    smbus0_tx_byte_signal: SignalId,
    smbus0_tx_strobe_signal: SignalId,
    smbus0_busy_signal: SignalId,
    smbus0_interrupt_signal: SignalId,
    timer0_irq_signal: SignalId,
    timer2_irq_signal: SignalId,
    interrupt_signal: SignalId,
    watchdog_reset_signal: SignalId,
}

impl Efm8State {
    fn set_signal(&self, signal: SignalId, value: u64, width: u16, at: SimTime) {
        self.hub
            .set(
                signal,
                SignalValue::from_u64(value, width).expect("fixed EFM8 signal width is valid"),
                at,
            )
            .expect("EFM8 signal identity is fixed at construction");
    }

    fn resolved_port(&self, port: usize) -> u8 {
        self.ports[port]
            .lock()
            .expect("EFM8 GPIO lock poisoned")
            .nets
            .iter()
            .enumerate()
            .fold(0_u8, |value, (pin, net)| {
                value | (u8::from(net.resolved() == Logic::One) << pin)
            })
            & PORT_MASKS[port]
    }

    fn update_smbus0_signals(&self, at: SimTime) {
        let enabled = self.registers[SMB0CF] & SMB0CF_ENSMB != 0;
        let busy = enabled && self.registers[SMB0CF] & SMB0CF_BUSY != 0;
        let interrupt = enabled
            && self.registers[EIE1] & EIE1_ESMB0 != 0
            && self.registers[SMB0CN0] & SMB0CN0_SI != 0;
        self.set_signal(self.smbus0_busy_signal, u64::from(busy), 1, at);
        self.set_signal(self.smbus0_interrupt_signal, u64::from(interrupt), 1, at);
    }

    fn smbus0_start(&mut self) {
        self.registers[SMB0CF] |= SMB0CF_BUSY;
        self.registers[SMB0CN0] |= SMB0CN0_MASTER | SMB0CN0_STA | SMB0CN0_SI;
        self.registers[SMB0CN0] &= !SMB0CN0_STO;
    }

    fn smbus0_stop(&mut self) {
        self.registers[SMB0CF] &= !SMB0CF_BUSY;
        self.registers[SMB0CN0] &= !(SMB0CN0_MASTER
            | SMB0CN0_TXMODE
            | SMB0CN0_STA
            | SMB0CN0_STO
            | SMB0CN0_ACKRQ
            | SMB0CN0_SI);
    }

    fn refresh_port(&mut self, port: usize, at: SimTime) -> Result<(), DeviceError> {
        let latch = self.registers[PORTS[port]] & PORT_MASKS[port];
        let push_pull = self.registers[PORT_MDOUT[port]] & PORT_MASKS[port];
        let direction = push_pull | ((!latch) & PORT_MASKS[port]);
        {
            let mut gpio = self.ports[port].lock().expect("EFM8 GPIO lock poisoned");
            gpio.direction = u32::from(direction);
            gpio.output = u32::from(latch);
        }
        refresh_gpio(
            &self.ports[port],
            &self.port_signals[port],
            &self.hub,
            PORT_WIDTHS[port],
            at,
        )
    }

    fn port_read(&self, port: usize) -> u8 {
        let latch = self.registers[PORTS[port]];
        let push_pull = self.registers[PORT_MDOUT[port]];
        let input = self.resolved_port(port) & self.registers[PORT_MDIN[port]];
        ((latch & push_pull) | (input & !push_pull)) & PORT_MASKS[port]
    }

    fn reset_registers(&mut self, at: SimTime, kind: ResetKind) {
        self.registers.fill(0);
        for port in 0..4 {
            self.registers[PORTS[port]] = PORT_MASKS[port];
            self.registers[PORT_MDIN[port]] = PORT_MASKS[port];
        }
        self.registers[CLKSEL] = 0x80;
        self.registers[RSTSRC] = match kind {
            ResetKind::PowerOn => 0x02,
            ResetKind::External => 0x01,
            ResetKind::Software => 0x10,
            ResetKind::Watchdog => 0x08,
        };
        self.uart.clear();
        self.smbus0_tx.clear();
        self.smbus0_rx.clear();
        self.registers[SMB0ADM] = 0x7f;
        self.timer0_epoch = at.ticks();
        self.timer2_epoch = at.ticks();
        self.watchdog_epoch = at.ticks();
        self.watchdog_key = 0;
        self.watchdog_enabled = true;
        self.watchdog_reset = false;
        for signal in [
            self.uart_strobe_signal,
            self.smbus0_tx_strobe_signal,
            self.timer0_irq_signal,
            self.timer2_irq_signal,
            self.interrupt_signal,
            self.watchdog_reset_signal,
        ] {
            self.set_signal(signal, 0, 1, at);
        }
        self.set_signal(self.smbus0_tx_byte_signal, 0, 8, at);
        self.update_smbus0_signals(at);
        for port in 0..4 {
            let _ = self.refresh_port(port, at);
        }
    }

    fn canonical(raw: usize) -> usize {
        let page = raw >> 8;
        let address = raw & 0xff;
        match address {
            0x80
            | 0x88..=0x8e
            | 0x90
            | 0x97..=0x99
            | 0xa0
            | 0xa4..=0xa6
            | 0xa8..=0xa9
            | 0xac
            | 0xb0
            | 0xb8
            | 0xc0..=0xc2
            | 0xc8
            | 0xca..=0xcf
            | 0xd4..=0xd7
            | 0xe1..=0xe3
            | 0xe6
            | 0xf1..=0xf3 => address,
            0x9c | 0xc3..=0xc5 | 0xef | 0xf4 if page == PAGE3 => (PAGE3 << 8) | address,
            _ => raw,
        }
    }

    fn interrupt_levels(&self) -> [bool; 6] {
        let enabled = self.registers[IE];
        if enabled & IE_EA == 0 {
            return [false; 6];
        }
        let active = [
            enabled & IE_ET0 != 0 && self.registers[TCON] & TCON_TF0 != 0,
            enabled & IE_ES0 != 0 && self.registers[SCON0] & (SCON0_RI | SCON0_TI) != 0,
            enabled & IE_ET2 != 0 && self.registers[TMR2CN0] & TMR2_TF2H != 0,
        ];
        let priorities = [IE_ET0, IE_ES0, IE_ET2];
        let mut levels = [false; 6];
        for source in 0..3 {
            if active[source] {
                let high = self.registers[IP] & priorities[source] != 0;
                levels[source + if high { 3 } else { 0 }] = true;
            }
        }
        levels
    }

    fn update_interrupt_signals(&self, at: SimTime) {
        self.set_signal(
            self.timer0_irq_signal,
            u64::from(self.registers[TCON] & TCON_TF0 != 0),
            1,
            at,
        );
        self.set_signal(
            self.timer2_irq_signal,
            u64::from(self.registers[TMR2CN0] & TMR2_TF2H != 0),
            1,
            at,
        );
        self.set_signal(
            self.interrupt_signal,
            u64::from(self.interrupt_levels().iter().any(|level| *level)),
            1,
            at,
        );
    }
}

/// Machine-facing EFM8BB52F32G peripheral state.
#[derive(Clone)]
pub struct Efm8PeripheralsHandle(Arc<Mutex<Efm8State>>);

impl Efm8PeripheralsHandle {
    /// Captured UART0 transmit bytes.
    pub fn uart_bytes(&self) -> Vec<u8> {
        self.0.lock().expect("EFM8 lock poisoned").uart.clone()
    }

    /// Captured SMBus 0 bytes written by the guest to the transmit FIFO.
    pub fn smbus0_tx_bytes(&self) -> Vec<u8> {
        self.0.lock().expect("EFM8 lock poisoned").smbus0_tx.clone()
    }

    /// Returns whether the functional SMBus 0 state machine owns the bus.
    pub fn smbus0_busy(&self) -> bool {
        let state = self.0.lock().expect("EFM8 lock poisoned");
        state.registers[SMB0CF] & SMB0CF_BUSY != 0
    }

    /// Returns whether SMBus 0 has an enabled service request pending.
    pub fn smbus0_interrupt(&self) -> bool {
        let state = self.0.lock().expect("EFM8 lock poisoned");
        state.registers[EIE1] & EIE1_ESMB0 != 0 && state.registers[SMB0CN0] & SMB0CN0_SI != 0
    }

    /// Queues bytes as a deterministic follower-side SMBus 0 receive event.
    pub fn inject_smbus0_rx(&self, bytes: &[u8], at: SimTime) {
        let mut state = self.0.lock().expect("EFM8 lock poisoned");
        state.smbus0_rx.extend(bytes.iter().copied());
        if let Some(&first) = state.smbus0_rx.front() {
            state.registers[SMB0DAT] = first;
            state.registers[SMB0CF] |= SMB0CF_BUSY;
            state.registers[SMB0CN0] &= !(SMB0CN0_MASTER | SMB0CN0_TXMODE);
            state.registers[SMB0CN0] |= SMB0CN0_ACKRQ | SMB0CN0_SI;
        }
        state.update_smbus0_signals(at);
    }

    /// Supplies one received UART0 byte and raises RI.
    pub fn inject_uart_rx(&self, value: u8, at: SimTime) {
        let mut state = self.0.lock().expect("EFM8 lock poisoned");
        state.registers[SBUF0] = value;
        state.registers[SCON0] |= SCON0_RI;
        state.update_interrupt_signals(at);
    }

    /// Advances functional timers/watchdog and returns low/high CPU interrupt inputs.
    pub fn poll(&self, now: SimTime) -> [bool; 6] {
        let mut state = self.0.lock().expect("EFM8 lock poisoned");
        for port in 0..4 {
            let _ = state.refresh_port(port, now);
        }
        if state.registers[TCON] & TCON_TR0 != 0 {
            let initial = u16::from_be_bytes([state.registers[TH0], state.registers[TL0]]);
            let elapsed = now.ticks().saturating_sub(state.timer0_epoch);
            let total = u64::from(initial).saturating_add(elapsed);
            let mode = state.registers[TMOD] & 3;
            if mode == 2 {
                let reload = state.registers[TH0];
                let period = u64::from(256_u16 - u16::from(reload)).max(1);
                state.registers[TL0] = reload.wrapping_add((elapsed % period).to_le_bytes()[0]);
                if elapsed >= period {
                    state.registers[TCON] |= TCON_TF0;
                    state.timer0_epoch = now.ticks();
                }
            } else {
                let bytes = total.to_le_bytes();
                state.registers[TL0] = bytes[0];
                state.registers[TH0] = bytes[1];
                state.timer0_epoch = now.ticks();
                if total > u64::from(u16::MAX) {
                    state.registers[TCON] |= TCON_TF0;
                }
            }
        }
        if state.registers[TMR2CN0] & TMR2_TR2 != 0 {
            let initial = u16::from_le_bytes([state.registers[TMR2L], state.registers[TMR2H]]);
            let elapsed = now.ticks().saturating_sub(state.timer2_epoch);
            let until_overflow = u64::from(u16::MAX - initial) + 1;
            if elapsed >= until_overflow {
                state.registers[TMR2CN0] |= TMR2_TF2H;
                state.registers[TMR2L] = state.registers[TMR2RLL];
                state.registers[TMR2H] = state.registers[TMR2RLH];
                state.timer2_epoch = now.ticks();
            } else {
                let elapsed = u16::try_from(elapsed)
                    .expect("non-overflowing Timer2 elapsed value fits in 16 bits");
                let value = initial.wrapping_add(elapsed);
                let [low, high] = value.to_le_bytes();
                state.registers[TMR2L] = low;
                state.registers[TMR2H] = high;
                state.timer2_epoch = now.ticks();
            }
        }
        if state.watchdog_enabled && now.ticks().saturating_sub(state.watchdog_epoch) >= 65_536 {
            state.watchdog_reset = true;
            state.set_signal(state.watchdog_reset_signal, 1, 1, now);
        }
        state.update_smbus0_signals(now);
        state.update_interrupt_signals(now);
        state.interrupt_levels()
    }

    /// Consumes a watchdog reset request.
    pub fn take_watchdog_reset(&self) -> bool {
        std::mem::take(&mut self.0.lock().expect("EFM8 lock poisoned").watchdog_reset)
    }
}

/// EFM8BB52F32G paged SFR peripheral window.
pub struct Efm8Peripherals {
    name: String,
    state: Arc<Mutex<Efm8State>>,
}

impl Efm8Peripherals {
    /// Creates the named functional slice and all 29 package GPIO handles.
    pub fn new(
        name: impl Into<String>,
        hub: SignalHub,
    ) -> Result<(Self, Efm8PeripheralsHandle, [GpioHandle; 4]), remu_signals::SignalError> {
        let (port0, signals0, handle0) = vendor_gpio(8, "board.efm8bb52f32g.port0", &hub)?;
        let (port1, signals1, handle1) = vendor_gpio(8, "board.efm8bb52f32g.port1", &hub)?;
        let (port2, signals2, handle2) = vendor_gpio(8, "board.efm8bb52f32g.port2", &hub)?;
        let (port3, signals3, handle3) = vendor_gpio(5, "board.efm8bb52f32g.port3", &hub)?;
        let uart_byte_signal = hub.declare(
            "board.efm8bb52f32g.uart0.tx_byte",
            SignalValue::from_u64(0, 8)?,
            Some("last UART0 transmit byte".to_owned()),
        )?;
        let uart_strobe_signal = hub.declare(
            "board.efm8bb52f32g.uart0.tx_strobe",
            SignalValue::from_u64(0, 1)?,
            Some("toggles for every UART0 transmit byte".to_owned()),
        )?;
        let smbus0_tx_byte_signal = hub.declare(
            "board.efm8bb52f32g.smb0.tx_byte",
            SignalValue::from_u64(0, 8)?,
            Some("last SMBus 0 byte written by the guest".to_owned()),
        )?;
        let smbus0_tx_strobe_signal = hub.declare(
            "board.efm8bb52f32g.smb0.tx_strobe",
            SignalValue::from_u64(0, 1)?,
            Some("toggles for every SMBus 0 transmit byte".to_owned()),
        )?;
        let smbus0_busy_signal = hub.declare(
            "board.efm8bb52f32g.smb0.busy",
            SignalValue::from_u64(0, 1)?,
            Some("functional SMBus 0 bus-busy state".to_owned()),
        )?;
        let smbus0_interrupt_signal = hub.declare(
            "board.efm8bb52f32g.smb0.interrupt",
            SignalValue::from_u64(0, 1)?,
            Some("enabled SMBus 0 service request".to_owned()),
        )?;
        let timer0_irq_signal = hub.declare(
            "board.efm8bb52f32g.timer0.irq",
            SignalValue::from_u64(0, 1)?,
            Some("Timer0 overflow request".to_owned()),
        )?;
        let timer2_irq_signal = hub.declare(
            "board.efm8bb52f32g.timer2.irq",
            SignalValue::from_u64(0, 1)?,
            Some("Timer2 high-byte overflow request".to_owned()),
        )?;
        let interrupt_signal = hub.declare(
            "board.efm8bb52f32g.interrupt.request",
            SignalValue::from_u64(0, 1)?,
            Some("combined enabled EFM8 interrupt request".to_owned()),
        )?;
        let watchdog_reset_signal = hub.declare(
            "board.efm8bb52f32g.watchdog.reset",
            SignalValue::from_u64(0, 1)?,
            Some("functional watchdog reset request".to_owned()),
        )?;
        let state = Arc::new(Mutex::new(Efm8State {
            registers: vec![0; SFR_BYTES].into_boxed_slice(),
            ports: [port0, port1, port2, port3],
            port_signals: [signals0, signals1, signals2, signals3],
            hub,
            uart: Vec::new(),
            smbus0_tx: Vec::new(),
            smbus0_rx: VecDeque::new(),
            timer0_epoch: 0,
            timer2_epoch: 0,
            watchdog_epoch: 0,
            watchdog_key: 0,
            watchdog_enabled: true,
            watchdog_reset: false,
            uart_byte_signal,
            uart_strobe_signal,
            smbus0_tx_byte_signal,
            smbus0_tx_strobe_signal,
            smbus0_busy_signal,
            smbus0_interrupt_signal,
            timer0_irq_signal,
            timer2_irq_signal,
            interrupt_signal,
            watchdog_reset_signal,
        }));
        state
            .lock()
            .expect("new EFM8 lock poisoned")
            .reset_registers(SimTime::ZERO, ResetKind::PowerOn);
        Ok((
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Efm8PeripheralsHandle(state),
            [handle0, handle1, handle2, handle3],
        ))
    }

    fn port_index(address: usize) -> Option<usize> {
        PORTS.iter().position(|candidate| *candidate == address)
    }
}

impl Device for Efm8Peripherals {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Byte {
            return Err(DeviceError::new("EFM8 SFR space requires byte accesses"));
        }
        let raw = usize::try_from(offset).map_err(|_| DeviceError::new("EFM8 offset overflow"))?;
        let address = Efm8State::canonical(raw);
        let mut state = self.state.lock().expect("EFM8 lock poisoned");
        if let Some(port) = Self::port_index(address) {
            state.refresh_port(port, at)?;
            return Ok(u64::from(state.port_read(port)));
        }
        let value = match address {
            SMB0DAT => {
                let value = state.registers[SMB0DAT];
                state.smbus0_rx.pop_front();
                if let Some(&next) = state.smbus0_rx.front() {
                    state.registers[SMB0DAT] = next;
                } else {
                    state.registers[SMB0CN0] &= !SMB0CN0_ACKRQ;
                }
                state.update_smbus0_signals(at);
                value
            }
            SMB0FCT => {
                let tx = u8::try_from(state.smbus0_tx.len().min(15)).unwrap_or(15);
                let rx = u8::try_from(state.smbus0_rx.len().min(15)).unwrap_or(15);
                (tx << 4) | rx
            }
            CLKSEL => state.registers[address] | 0x80,
            _ => *state.registers.get(address).ok_or_else(|| {
                DeviceError::new(format!("EFM8 read outside SFR space: {raw:#x}"))
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
            return Err(DeviceError::new("EFM8 SFR space requires byte accesses"));
        }
        let raw = usize::try_from(offset).map_err(|_| DeviceError::new("EFM8 offset overflow"))?;
        let address = Efm8State::canonical(raw);
        let value = value.to_le_bytes()[0];
        let mut state = self.state.lock().expect("EFM8 lock poisoned");
        if address >= SFR_BYTES {
            return Err(DeviceError::new(format!(
                "EFM8 write outside SFR space: {raw:#x}"
            )));
        }
        if address == SMB0CF {
            let busy = state.registers[SMB0CF] & SMB0CF_BUSY;
            state.registers[SMB0CF] = (value & !SMB0CF_BUSY) | busy;
            if value & SMB0CF_ENSMB == 0 {
                state.smbus0_stop();
                state.registers[SMB0CN0] &= !SMB0CN0_SI;
            }
        } else if address == SMB0TC {
            state.registers[address] = value & 0x83;
        } else if address == SMB0ADR {
            state.registers[address] = value & 0xfe;
        } else if address == SMB0ADM {
            state.registers[address] = value;
        } else if matches!(address, SMB0FCN0 | SMB0FCN1 | SMB0RXLN | SMB0FCT) {
            state.registers[address] = value;
        } else if address == SMB0CN0 {
            let request_start = value & SMB0CN0_STA != 0;
            let request_stop = value & SMB0CN0_STO != 0;
            let old_hardware = state.registers[SMB0CN0]
                & (SMB0CN0_MASTER | SMB0CN0_TXMODE | SMB0CN0_ACKRQ | SMB0CN0_ARBLOST);
            state.registers[SMB0CN0] = old_hardware | (value & SMB0CN0_ACK);
            if request_start {
                state.smbus0_start();
            } else if request_stop {
                state.smbus0_stop();
            } else if value & SMB0CN0_SI != 0 {
                state.registers[SMB0CN0] |= SMB0CN0_SI;
            }
        } else if address == SMB0DAT {
            state.registers[SMB0DAT] = value;
            if state.registers[SMB0CF] & SMB0CF_ENSMB != 0 {
                state.smbus0_tx.push(value);
                state.registers[SMB0CF] |= SMB0CF_BUSY;
                state.registers[SMB0CN0] |= SMB0CN0_MASTER | SMB0CN0_TXMODE | SMB0CN0_SI;
                state.registers[SMB0CN0] &= !SMB0CN0_ACKRQ;
                state.set_signal(state.smbus0_tx_byte_signal, u64::from(value), 8, at);
                let previous = state.hub.with_registry(|registry| {
                    registry
                        .value(state.smbus0_tx_strobe_signal)
                        .and_then(|signal| signal.bit(0))
                        .map_or(0, |logic| u64::from(logic == Logic::One))
                });
                state.set_signal(state.smbus0_tx_strobe_signal, previous ^ 1, 1, at);
            }
        } else if let Some(port) = Self::port_index(address) {
            state.registers[address] = value;
            state.registers[address] &= PORT_MASKS[port];
            state.refresh_port(port, at)?;
        } else if let Some(port) = PORT_MDOUT.iter().position(|item| *item == address) {
            state.registers[address] = value;
            state.registers[address] &= PORT_MASKS[port];
            state.refresh_port(port, at)?;
        } else if address == SBUF0 {
            state.registers[address] = value;
            if state.registers[XBR0] & XBR0_URT0E != 0 && state.registers[XBR2] & XBR2_XBARE != 0 {
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
            state.registers[SCON0] |= SCON0_TI;
        } else if address == WDTCN {
            state.registers[address] = value;
            if state.watchdog_key == 0xde && value == 0xad {
                state.watchdog_enabled = false;
            }
            state.watchdog_key = value;
            state.watchdog_epoch = at.ticks();
        } else if address == TCON && value & TCON_TR0 != 0 {
            state.registers[address] = value;
            state.timer0_epoch = at.ticks();
        } else if address == TMR2CN0 && value & TMR2_TR2 != 0 {
            state.registers[address] = value;
            state.timer2_epoch = at.ticks();
        } else {
            state.registers[address] = value;
        }
        state.update_smbus0_signals(at);
        state.update_interrupt_signals(at);
        Ok(())
    }

    fn reset(&mut self, kind: ResetKind) {
        self.state
            .lock()
            .expect("EFM8 lock poisoned")
            .reset_registers(SimTime::ZERO, kind);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AccessWidth, EIE1, Efm8Peripherals, IE, IE_EA, IE_ET0, P0, P0MDOUT, SBUF0, SMB0CF, SMB0CN0,
        SMB0DAT, SimTime, TCON, TCON_TR0, TMOD, XBR0, XBR0_URT0E, XBR2, XBR2_XBARE,
    };
    use remu_bus::Device;

    #[test]
    fn gpio_timer_uart_and_interrupt_slice_is_functional() {
        let hub = super::SignalHub::new();
        let (mut device, handle, ports) = Efm8Peripherals::new("efm8.sfr", hub).unwrap();
        device
            .write(P0MDOUT as u64, AccessWidth::Byte, 1, SimTime::ZERO)
            .unwrap();
        device
            .write(P0 as u64, AccessWidth::Byte, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(ports[0].output() & 1, 1);

        device
            .write(
                XBR0 as u64,
                AccessWidth::Byte,
                XBR0_URT0E.into(),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                XBR2 as u64,
                AccessWidth::Byte,
                XBR2_XBARE.into(),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(SBUF0 as u64, AccessWidth::Byte, b'E'.into(), SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.uart_bytes(), b"E");

        device
            .write(TMOD as u64, AccessWidth::Byte, 2, SimTime::ZERO)
            .unwrap();
        device
            .write(super::TH0 as u64, AccessWidth::Byte, 0xfc, SimTime::ZERO)
            .unwrap();
        device
            .write(
                IE as u64,
                AccessWidth::Byte,
                (IE_EA | IE_ET0).into(),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                TCON as u64,
                AccessWidth::Byte,
                TCON_TR0.into(),
                SimTime::ZERO,
            )
            .unwrap();
        assert!(handle.poll(SimTime::from_ticks(4))[0]);
    }

    #[test]
    fn smbus0_master_and_follower_byte_paths_are_observable() {
        let hub = super::SignalHub::new();
        let (mut device, handle, _ports) = Efm8Peripherals::new("efm8.sfr", hub).unwrap();
        device
            .write(SMB0CF as u64, AccessWidth::Byte, 0x80, SimTime::ZERO)
            .unwrap();
        device
            .write(EIE1 as u64, AccessWidth::Byte, 1, SimTime::ZERO)
            .unwrap();
        device
            .write(SMB0CN0 as u64, AccessWidth::Byte, 0x20, SimTime::ZERO)
            .unwrap();
        assert!(handle.smbus0_busy());
        assert!(handle.smbus0_interrupt());
        device
            .write(SMB0CN0 as u64, AccessWidth::Byte, 0, SimTime::from_ticks(1))
            .unwrap();
        device
            .write(
                SMB0DAT as u64,
                AccessWidth::Byte,
                0xa0,
                SimTime::from_ticks(2),
            )
            .unwrap();
        assert_eq!(handle.smbus0_tx_bytes(), vec![0xa0]);
        assert!(handle.smbus0_interrupt());

        device
            .write(SMB0CN0 as u64, AccessWidth::Byte, 0, SimTime::from_ticks(3))
            .unwrap();
        handle.inject_smbus0_rx(&[0x12, 0x34], SimTime::from_ticks(4));
        assert!(handle.smbus0_interrupt());
        assert_eq!(
            device.read(SMB0DAT as u64, AccessWidth::Byte, SimTime::from_ticks(5)),
            Ok(0x12)
        );
        assert_eq!(
            device.read(SMB0DAT as u64, AccessWidth::Byte, SimTime::from_ticks(6)),
            Ok(0x34)
        );
    }
}
