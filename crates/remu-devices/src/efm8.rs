use super::{GpioHandle, GpioState, SignalHub, refresh_gpio, vendor_gpio};
use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use remu_signals::{Logic, SignalId, SignalValue};
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
const TMR3CN0: usize = 0x91;
const TMR3RLL: usize = 0x92;
const TMR3RLH: usize = 0x93;
const TMR3L: usize = 0x94;
const TMR3H: usize = 0x95;
const TMR3CN1: usize = (0x10 << 8) | 0xfe;
const TMR4RLL: usize = (0x10 << 8) | 0xa2;
const TMR4RLH: usize = (0x10 << 8) | 0xa3;
const TMR4L: usize = (0x10 << 8) | 0xa4;
const TMR4H: usize = (0x10 << 8) | 0xa5;
const TMR4CN0: usize = (0x10 << 8) | 0x98;
const TMR4CN1: usize = (0x10 << 8) | 0xff;
const TMR5RLL: usize = (0x10 << 8) | 0xd2;
const TMR5RLH: usize = (0x10 << 8) | 0xd3;
const TMR5L: usize = (0x10 << 8) | 0xd4;
const TMR5H: usize = (0x10 << 8) | 0xd5;
const TMR5CN0: usize = (0x10 << 8) | 0xc0;
const TMR5CN1: usize = (0x10 << 8) | 0xf1;
const EIE1: usize = 0xe6;
const EIE1_PAGE10: usize = (0x10 << 8) | 0xe6;
const EIP1: usize = (0x10 << 8) | 0xbb;
const EIP1H: usize = (0x10 << 8) | 0xee;
const EIE2: usize = 0xf3;
const EIP2: usize = (0x10 << 8) | 0xed;
const EIP2H: usize = (0x10 << 8) | 0xf6;
const XBR0: usize = 0xe1;
const XBR2: usize = 0xe3;
const RSTSRC: usize = 0xef;
const P0MDIN: usize = 0xf1;
const P1MDIN: usize = 0xf2;
const P2MDIN: usize = 0xf3;
const P3MDIN: usize = (PAGE3 << 8) | 0xf4;

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
const TMR3_TR3: u8 = 0x04;
const TMR3_TF3L: u8 = 0x40;
const TMR3_TF3H: u8 = 0x80;
const TMR3_TF3LEN: u8 = 0x20;
const TMR3_TF3CEN: u8 = 0x10;
const TMR4_TR4: u8 = 0x04;
const TMR4_TF4L: u8 = 0x40;
const TMR4_TF4H: u8 = 0x80;
const TMR4_TF4LEN: u8 = 0x20;
const TMR4_TF4CEN: u8 = 0x10;
const TMR5_TR5: u8 = 0x04;
const TMR5_TF5L: u8 = 0x40;
const TMR5_TF5H: u8 = 0x80;
const TMR5_TF5LEN: u8 = 0x20;
const TMR5_TF5CEN: u8 = 0x10;
const EIE1_ET3: u8 = 0x80;
const EIE2_ET4: u8 = 0x04;
const EIE2_ET5: u8 = 0x08;
const SCON0_RI: u8 = 0x01;
const SCON0_TI: u8 = 0x02;
const XBR0_URT0E: u8 = 0x01;
const XBR2_XBARE: u8 = 0x40;

struct Efm8State {
    registers: Box<[u8]>,
    ports: [Arc<Mutex<GpioState>>; 4],
    port_signals: [Vec<SignalId>; 4],
    hub: SignalHub,
    uart: Vec<u8>,
    timer0_epoch: u64,
    timer2_epoch: u64,
    timer3_epoch: u64,
    timer4_epoch: u64,
    timer5_epoch: u64,
    watchdog_epoch: u64,
    watchdog_key: u8,
    watchdog_enabled: bool,
    watchdog_reset: bool,
    uart_byte_signal: SignalId,
    uart_strobe_signal: SignalId,
    timer0_irq_signal: SignalId,
    timer2_irq_signal: SignalId,
    timer3_irq_signal: SignalId,
    timer4_irq_signal: SignalId,
    timer5_irq_signal: SignalId,
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
        self.timer0_epoch = at.ticks();
        self.timer2_epoch = at.ticks();
        self.timer3_epoch = at.ticks();
        self.timer4_epoch = at.ticks();
        self.timer5_epoch = at.ticks();
        self.watchdog_epoch = at.ticks();
        self.watchdog_key = 0;
        self.watchdog_enabled = true;
        self.watchdog_reset = false;
        for signal in [
            self.uart_strobe_signal,
            self.timer0_irq_signal,
            self.timer2_irq_signal,
            self.timer3_irq_signal,
            self.timer4_irq_signal,
            self.timer5_irq_signal,
            self.interrupt_signal,
            self.watchdog_reset_signal,
        ] {
            self.set_signal(signal, 0, 1, at);
        }
        for port in 0..4 {
            let _ = self.refresh_port(port, at);
        }
    }

    fn canonical(raw: usize) -> usize {
        let page = raw >> 8;
        let address = raw & 0xff;
        if page == 0x10 {
            if (0x91..=0x95).contains(&address) || raw == EIE1_PAGE10 {
                // Timer3 and EIE1 are mirrored on SFR page 0x10.
                return address;
            }
            if matches!(
                raw,
                TMR3CN1
                    | TMR4CN0
                    | TMR4RLL
                    | TMR4RLH
                    | TMR4L
                    | TMR4H
                    | TMR4CN1
                    | TMR5RLL
                    | TMR5RLH
                    | TMR5L
                    | TMR5H
                    | TMR5CN0
                    | TMR5CN1
            ) {
                // Timer4/5 and their control-1 registers exist only on page
                // 0x10 and must not alias page-0 GPIO/SFR names.
                return raw;
            }
        }
        match address {
            0x80
            | 0x88..=0x8e
            | 0x90
            | 0x91..=0x95
            | 0x97..=0x99
            | 0xa0
            | 0xa4..=0xa6
            | 0xa8..=0xa9
            | 0xb0
            | 0xb8
            | 0xc8
            | 0xca..=0xcf
            | 0xd4..=0xd5
            | 0xe1..=0xe3
            | 0xef
            | 0xf1..=0xf3 => address,
            0x9c | 0xf4 if page == PAGE3 => (PAGE3 << 8) | address,
            _ => raw,
        }
    }

    fn interrupt_levels(&self) -> [bool; 14] {
        let enabled = self.registers[IE];
        if enabled & IE_EA == 0 {
            return [false; 14];
        }
        let active = [
            enabled & IE_ET0 != 0 && self.registers[TCON] & TCON_TF0 != 0,
            enabled & IE_ES0 != 0 && self.registers[SCON0] & (SCON0_RI | SCON0_TI) != 0,
            enabled & IE_ET2 != 0 && self.registers[TMR2CN0] & TMR2_TF2H != 0,
        ];
        let priorities = [IE_ET0, IE_ES0, IE_ET2];
        let mut levels = [false; 14];
        for source in 0..3 {
            if active[source] {
                let high = self.registers[IP] & priorities[source] != 0;
                levels[source + if high { 3 } else { 0 }] = true;
            }
        }
        let timer3 = self.registers[EIE1] & EIE1_ET3 != 0
            && ((self.registers[TMR3CN0] & TMR3_TF3H != 0
                && self.registers[TMR3CN0] & TMR3_TF3CEN != 0)
                || (self.registers[TMR3CN0] & TMR3_TF3L != 0
                    && self.registers[TMR3CN0] & TMR3_TF3LEN != 0));
        let timer4 = self.registers[EIE2] & EIE2_ET4 != 0
            && ((self.registers[TMR4CN0] & TMR4_TF4H != 0
                && self.registers[TMR4CN0] & TMR4_TF4CEN != 0)
                || (self.registers[TMR4CN0] & TMR4_TF4L != 0
                    && self.registers[TMR4CN0] & TMR4_TF4LEN != 0));
        let timer5 = self.registers[EIE2] & EIE2_ET5 != 0
            && ((self.registers[TMR5CN0] & TMR5_TF5H != 0
                && self.registers[TMR5CN0] & TMR5_TF5CEN != 0)
                || (self.registers[TMR5CN0] & TMR5_TF5L != 0
                    && self.registers[TMR5CN0] & TMR5_TF5LEN != 0));
        let timer3_high = self.registers[EIP1] & 0x80 != 0 || self.registers[EIP1H] & 0x80 != 0;
        let timer4_high = self.registers[EIP2] & 0x04 != 0 || self.registers[EIP2H] & 0x04 != 0;
        let timer5_high = self.registers[EIP2] & 0x08 != 0 || self.registers[EIP2H] & 0x08 != 0;
        if timer3 {
            levels[8 + usize::from(timer3_high)] = true;
        }
        if timer4 {
            levels[10 + usize::from(timer4_high)] = true;
        }
        if timer5 {
            levels[12 + usize::from(timer5_high)] = true;
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
            self.timer3_irq_signal,
            u64::from(self.registers[TMR3CN0] & (TMR3_TF3L | TMR3_TF3H) != 0),
            1,
            at,
        );
        self.set_signal(
            self.timer4_irq_signal,
            u64::from(self.registers[TMR4CN0] & (TMR4_TF4L | TMR4_TF4H) != 0),
            1,
            at,
        );
        self.set_signal(
            self.timer5_irq_signal,
            u64::from(self.registers[TMR5CN0] & (TMR5_TF5L | TMR5_TF5H) != 0),
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

fn advance_16bit_timer(
    state: &mut Efm8State,
    now: u64,
    epoch: u64,
    control: usize,
    current_low: usize,
    current_high: usize,
    reload_low: usize,
    reload_high: usize,
    run_bit: u8,
    low_flag: u8,
    high_flag: u8,
) -> u64 {
    if state.registers[control] & run_bit == 0 {
        return epoch;
    }
    let initial = u16::from_le_bytes([state.registers[current_low], state.registers[current_high]]);
    let elapsed = now.saturating_sub(epoch);
    let low_until_overflow = u64::from(0x100_u16 - (initial & 0xff));
    let until_overflow = u64::from(u16::MAX - initial) + 1;
    if elapsed >= until_overflow {
        state.registers[control] |= high_flag | low_flag;
        state.registers[current_low] = state.registers[reload_low];
        state.registers[current_high] = state.registers[reload_high];
    } else {
        if elapsed >= low_until_overflow {
            state.registers[control] |= low_flag;
        }
        let value = initial.wrapping_add((elapsed & u64::from(u16::MAX)) as u16);
        let [low, high] = value.to_le_bytes();
        state.registers[current_low] = low;
        state.registers[current_high] = high;
    }
    now
}

/// Machine-facing EFM8BB52F32G peripheral state.
#[derive(Clone)]
pub struct Efm8PeripheralsHandle(Arc<Mutex<Efm8State>>);

impl Efm8PeripheralsHandle {
    /// Captured UART0 transmit bytes.
    pub fn uart_bytes(&self) -> Vec<u8> {
        self.0.lock().expect("EFM8 lock poisoned").uart.clone()
    }

    /// Supplies one received UART0 byte and raises RI.
    pub fn inject_uart_rx(&self, value: u8, at: SimTime) {
        let mut state = self.0.lock().expect("EFM8 lock poisoned");
        state.registers[SBUF0] = value;
        state.registers[SCON0] |= SCON0_RI;
        state.update_interrupt_signals(at);
    }

    /// Advances functional timers/watchdog and returns low/high CPU interrupt inputs.
    pub fn poll(&self, now: SimTime) -> [bool; 14] {
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
        let timer3_epoch = state.timer3_epoch;
        state.timer3_epoch = advance_16bit_timer(
            &mut state,
            now.ticks(),
            timer3_epoch,
            TMR3CN0,
            TMR3L,
            TMR3H,
            TMR3RLL,
            TMR3RLH,
            TMR3_TR3,
            TMR3_TF3L,
            TMR3_TF3H,
        );
        let timer4_epoch = state.timer4_epoch;
        state.timer4_epoch = advance_16bit_timer(
            &mut state,
            now.ticks(),
            timer4_epoch,
            TMR4CN0,
            TMR4L,
            TMR4H,
            TMR4RLL,
            TMR4RLH,
            TMR4_TR4,
            TMR4_TF4L,
            TMR4_TF4H,
        );
        let timer5_epoch = state.timer5_epoch;
        state.timer5_epoch = advance_16bit_timer(
            &mut state,
            now.ticks(),
            timer5_epoch,
            TMR5CN0,
            TMR5L,
            TMR5H,
            TMR5RLL,
            TMR5RLH,
            TMR5_TR5,
            TMR5_TF5L,
            TMR5_TF5H,
        );
        if state.watchdog_enabled && now.ticks().saturating_sub(state.watchdog_epoch) >= 65_536 {
            state.watchdog_reset = true;
            state.set_signal(state.watchdog_reset_signal, 1, 1, now);
        }
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
        let timer3_irq_signal = hub.declare(
            "board.efm8bb52f32g.timer3.irq",
            SignalValue::from_u64(0, 1)?,
            Some("Timer3 overflow request".to_owned()),
        )?;
        let timer4_irq_signal = hub.declare(
            "board.efm8bb52f32g.timer4.irq",
            SignalValue::from_u64(0, 1)?,
            Some("Timer4 overflow request".to_owned()),
        )?;
        let timer5_irq_signal = hub.declare(
            "board.efm8bb52f32g.timer5.irq",
            SignalValue::from_u64(0, 1)?,
            Some("Timer5 overflow request".to_owned()),
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
            timer0_epoch: 0,
            timer2_epoch: 0,
            timer3_epoch: 0,
            timer4_epoch: 0,
            timer5_epoch: 0,
            watchdog_epoch: 0,
            watchdog_key: 0,
            watchdog_enabled: true,
            watchdog_reset: false,
            uart_byte_signal,
            uart_strobe_signal,
            timer0_irq_signal,
            timer2_irq_signal,
            timer3_irq_signal,
            timer4_irq_signal,
            timer5_irq_signal,
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
        let mut value = *state
            .registers
            .get(address)
            .ok_or_else(|| DeviceError::new(format!("EFM8 read outside SFR space: {raw:#x}")))?;
        if address == CLKSEL {
            value |= 0x80;
        }
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
        state.registers[address] = value;
        if let Some(port) = Self::port_index(address) {
            state.registers[address] &= PORT_MASKS[port];
            state.refresh_port(port, at)?;
        } else if let Some(port) = PORT_MDOUT.iter().position(|item| *item == address) {
            state.registers[address] &= PORT_MASKS[port];
            state.refresh_port(port, at)?;
        } else if address == SBUF0 {
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
            if state.watchdog_key == 0xde && value == 0xad {
                state.watchdog_enabled = false;
            }
            state.watchdog_key = value;
            state.watchdog_epoch = at.ticks();
        } else if address == TCON && value & TCON_TR0 != 0 {
            state.timer0_epoch = at.ticks();
        } else if address == TMR2CN0 && value & TMR2_TR2 != 0 {
            state.timer2_epoch = at.ticks();
        } else if address == TMR3CN0 && value & TMR3_TR3 != 0 {
            state.timer3_epoch = at.ticks();
        } else if address == TMR4CN0 && value & TMR4_TR4 != 0 {
            state.timer4_epoch = at.ticks();
        } else if address == TMR5CN0 && value & TMR5_TR5 != 0 {
            state.timer5_epoch = at.ticks();
        }
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
        AccessWidth, Efm8Peripherals, IE, IE_EA, IE_ET0, P0, P0MDOUT, SBUF0, SimTime, TCON,
        TCON_TR0, TMOD, XBR0, XBR0_URT0E, XBR2, XBR2_XBARE,
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
    fn timer345_reload_flags_and_interrupts_are_functional() {
        let hub = super::SignalHub::new();
        let (mut device, handle, _) = Efm8Peripherals::new("efm8.sfr", hub).unwrap();
        device
            .write(
                super::EIE1_PAGE10 as u64,
                AccessWidth::Byte,
                super::EIE1_ET3.into(),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                super::EIE2 as u64,
                AccessWidth::Byte,
                (super::EIE2_ET4 | super::EIE2_ET5).into(),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(IE as u64, AccessWidth::Byte, IE_EA.into(), SimTime::ZERO)
            .unwrap();

        for (reload_low, reload_high, current_low, current_high, control, run, cen) in [
            (
                super::TMR3RLL,
                super::TMR3RLH,
                super::TMR3L,
                super::TMR3H,
                super::TMR3CN0,
                super::TMR3_TR3,
                super::TMR3_TF3CEN,
            ),
            (
                super::TMR4RLL,
                super::TMR4RLH,
                super::TMR4L,
                super::TMR4H,
                super::TMR4CN0,
                super::TMR4_TR4,
                super::TMR4_TF4CEN,
            ),
            (
                super::TMR5RLL,
                super::TMR5RLH,
                super::TMR5L,
                super::TMR5H,
                super::TMR5CN0,
                super::TMR5_TR5,
                super::TMR5_TF5CEN,
            ),
        ] {
            device
                .write(reload_low as u64, AccessWidth::Byte, 0xfc, SimTime::ZERO)
                .unwrap();
            device
                .write(reload_high as u64, AccessWidth::Byte, 0xff, SimTime::ZERO)
                .unwrap();
            device
                .write(current_low as u64, AccessWidth::Byte, 0xfc, SimTime::ZERO)
                .unwrap();
            device
                .write(current_high as u64, AccessWidth::Byte, 0xff, SimTime::ZERO)
                .unwrap();
            device
                .write(
                    control as u64,
                    AccessWidth::Byte,
                    (run | cen).into(),
                    SimTime::ZERO,
                )
                .unwrap();
        }

        // The page-10 control-1 aliases are addressable without colliding with
        // the page-0 GPIO/SFR names.
        for address in [super::TMR3CN1, super::TMR4CN1, super::TMR5CN1] {
            device
                .write(address as u64, AccessWidth::Byte, 0, SimTime::ZERO)
                .unwrap();
        }
        let levels = handle.poll(SimTime::from_ticks(4));
        assert!(levels[8]);
        assert!(levels[10]);
        assert!(levels[12]);
        assert_ne!(
            device
                .read(super::TMR3CN0 as u64, AccessWidth::Byte, SimTime::ZERO)
                .unwrap()
                & u64::from(super::TMR3_TF3H),
            0
        );
        assert_ne!(
            device
                .read(super::TMR4CN0 as u64, AccessWidth::Byte, SimTime::ZERO)
                .unwrap()
                & u64::from(super::TMR4_TF4H),
            0
        );
        assert_ne!(
            device
                .read(super::TMR5CN0 as u64, AccessWidth::Byte, SimTime::ZERO)
                .unwrap()
                & u64::from(super::TMR5_TF5H),
            0
        );
    }
}
