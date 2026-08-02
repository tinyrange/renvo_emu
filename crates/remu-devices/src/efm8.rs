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
const XBR0: usize = 0xe1;
const XBR2: usize = 0xe3;
const RSTSRC: usize = 0xef;
const P0MDIN: usize = 0xf1;
const P1MDIN: usize = 0xf2;
const P2MDIN: usize = 0xf3;
const P3MDIN: usize = (PAGE3 << 8) | 0xf4;
const P2SKIP: usize = 0xcc;
const P0SKIP: usize = 0xd4;
const P1SKIP: usize = 0xd5;
const XBR1: usize = 0xe2;

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
const XBR0_URT0E: u8 = 0x01;
const XBR2_XBARE: u8 = 0x40;

/// Crossbar functions that can be assigned to a QFN32 port pin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Efm8CrossbarFunction {
    /// UART0 transmit output (fixed P0.4).
    Uart0Tx,
    /// UART0 receive input (fixed P0.5).
    Uart0Rx,
    /// SPI0 serial clock.
    Spi0Sck,
    /// SPI0 master-in/slave-out.
    Spi0Miso,
    /// SPI0 master-out/slave-in.
    Spi0Mosi,
    /// SPI0 four-wire slave-select.
    Spi0Nss,
    /// SMBus0 data.
    Smb0Sda,
    /// SMBus0 clock.
    Smb0Scl,
    /// Comparator 0 synchronous output.
    Cmp0,
    /// Comparator 0 asynchronous output.
    Cmp0a,
    /// Comparator 1 synchronous output.
    Cmp1,
    /// Comparator 1 asynchronous output.
    Cmp1a,
    /// SYSCLK output.
    Sysclk,
    /// PCA CEX0 output.
    PcaCex0,
    /// PCA CEX1 output.
    PcaCex1,
    /// PCA CEX2 output.
    PcaCex2,
    /// PCA external counter input.
    PcaEci,
    /// Timer 0 external input.
    Timer0,
    /// Timer 1 external input.
    Timer1,
    /// Timer 2/3/4/5 external input.
    Timer2345,
    /// SMBus1 data.
    Smb1Sda,
    /// SMBus1 clock.
    Smb1Scl,
    /// UART1 transmit output.
    Uart1Tx,
    /// UART1 receive input.
    Uart1Rx,
    /// UART1 RTS output.
    Uart1Rts,
    /// UART1 CTS input.
    Uart1Cts,
    /// PWM channel 0 output.
    Pwm0,
    /// PWM channel 1 output.
    Pwm1,
    /// PWM channel 2 output.
    Pwm2,
}

impl Efm8CrossbarFunction {
    const COUNT: usize = 29;

    const fn index(self) -> usize {
        match self {
            Self::Uart0Tx => 0,
            Self::Uart0Rx => 1,
            Self::Spi0Sck => 2,
            Self::Spi0Miso => 3,
            Self::Spi0Mosi => 4,
            Self::Spi0Nss => 5,
            Self::Smb0Sda => 6,
            Self::Smb0Scl => 7,
            Self::Cmp0 => 8,
            Self::Cmp0a => 9,
            Self::Cmp1 => 10,
            Self::Cmp1a => 11,
            Self::Sysclk => 12,
            Self::PcaCex0 => 13,
            Self::PcaCex1 => 14,
            Self::PcaCex2 => 15,
            Self::PcaEci => 16,
            Self::Timer0 => 17,
            Self::Timer1 => 18,
            Self::Timer2345 => 19,
            Self::Smb1Sda => 20,
            Self::Smb1Scl => 21,
            Self::Uart1Tx => 22,
            Self::Uart1Rx => 23,
            Self::Uart1Rts => 24,
            Self::Uart1Cts => 25,
            Self::Pwm0 => 26,
            Self::Pwm1 => 27,
            Self::Pwm2 => 28,
        }
    }
}

/// A physical QFN32 pin selected by the EFM8 priority crossbar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Efm8CrossbarPin {
    /// Port number (0, 1, or 2).
    pub port: u8,
    /// Bit number within the port.
    pub pin: u8,
}

struct Efm8State {
    registers: Box<[u8]>,
    crossbar_routes: [Option<Efm8CrossbarPin>; Efm8CrossbarFunction::COUNT],
    ports: [Arc<Mutex<GpioState>>; 4],
    port_signals: [Vec<SignalId>; 4],
    hub: SignalHub,
    uart: Vec<u8>,
    timer0_epoch: u64,
    timer2_epoch: u64,
    watchdog_epoch: u64,
    watchdog_key: u8,
    watchdog_enabled: bool,
    watchdog_reset: bool,
    uart_byte_signal: SignalId,
    uart_strobe_signal: SignalId,
    timer0_irq_signal: SignalId,
    timer2_irq_signal: SignalId,
    interrupt_signal: SignalId,
    watchdog_reset_signal: SignalId,
    crossbar_enabled_signal: SignalId,
    crossbar_assigned_signal: SignalId,
    crossbar_uart0_tx_signal: SignalId,
    crossbar_uart0_rx_signal: SignalId,
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

    fn crossbar_pin(index: usize) -> Efm8CrossbarPin {
        Efm8CrossbarPin {
            port: u8::try_from(index / 8).expect("EFM8 crossbar port fits in u8"),
            pin: u8::try_from(index % 8).expect("EFM8 crossbar pin fits in u8"),
        }
    }

    fn crossbar_skip_mask(&self) -> u32 {
        u32::from(self.registers[P0SKIP])
            | (u32::from(self.registers[P1SKIP]) << 8)
            | (u32::from(self.registers[P2SKIP]) << 16)
    }

    fn assign_crossbar_pin(
        &mut self,
        function: Efm8CrossbarFunction,
        occupied: &mut u32,
        count: usize,
    ) {
        let skip = self.crossbar_skip_mask();
        let mut assigned = 0;
        for index in 0..24 {
            let bit = 1_u32 << index;
            if skip & bit == 0 && *occupied & bit == 0 {
                self.crossbar_routes[function.index()] = Some(Self::crossbar_pin(index));
                *occupied |= bit;
                assigned += 1;
                if assigned == count {
                    return;
                }
            }
        }
        self.crossbar_routes[function.index()] = None;
    }

    fn assign_crossbar_fixed(
        &mut self,
        function: Efm8CrossbarFunction,
        occupied: &mut u32,
        index: usize,
    ) {
        self.crossbar_routes[function.index()] = Some(Self::crossbar_pin(index));
        *occupied |= 1_u32 << index;
    }

    fn refresh_crossbar(&mut self, at: SimTime) {
        self.crossbar_routes.fill(None);
        let mut occupied = 0_u32;
        if self.registers[XBR0] & XBR0_URT0E != 0 {
            // UART0 is the only crossbar function with fixed pins on this
            // package and has priority over every assignable resource.
            self.assign_crossbar_fixed(Efm8CrossbarFunction::Uart0Tx, &mut occupied, 4);
            self.assign_crossbar_fixed(Efm8CrossbarFunction::Uart0Rx, &mut occupied, 5);
        }
        if self.registers[XBR0] & 0x02 != 0 {
            self.assign_crossbar_pin(Efm8CrossbarFunction::Spi0Sck, &mut occupied, 1);
            self.assign_crossbar_pin(Efm8CrossbarFunction::Spi0Miso, &mut occupied, 1);
            self.assign_crossbar_pin(Efm8CrossbarFunction::Spi0Mosi, &mut occupied, 1);
            self.assign_crossbar_pin(Efm8CrossbarFunction::Spi0Nss, &mut occupied, 1);
        }
        if self.registers[XBR0] & 0x04 != 0 {
            self.assign_crossbar_pin(Efm8CrossbarFunction::Smb0Sda, &mut occupied, 1);
            self.assign_crossbar_pin(Efm8CrossbarFunction::Smb0Scl, &mut occupied, 1);
        }
        for (function, mask) in [
            (Efm8CrossbarFunction::Cmp0, 0x08),
            (Efm8CrossbarFunction::Cmp0a, 0x10),
            (Efm8CrossbarFunction::Cmp1, 0x20),
            (Efm8CrossbarFunction::Cmp1a, 0x40),
            (Efm8CrossbarFunction::Sysclk, 0x80),
        ] {
            if self.registers[XBR0] & mask != 0 {
                self.assign_crossbar_pin(function, &mut occupied, 1);
            }
        }
        let xbr1 = self.registers[XBR1];
        let pca_count = usize::from(xbr1 & 0x03);
        for function in [
            Efm8CrossbarFunction::PcaCex0,
            Efm8CrossbarFunction::PcaCex1,
            Efm8CrossbarFunction::PcaCex2,
        ]
        .into_iter()
        .take(pca_count)
        {
            self.assign_crossbar_pin(function, &mut occupied, 1);
        }
        for (function, mask) in [
            (Efm8CrossbarFunction::PcaEci, 0x08),
            (Efm8CrossbarFunction::Timer0, 0x10),
            (Efm8CrossbarFunction::Timer1, 0x20),
            (Efm8CrossbarFunction::Timer2345, 0x40),
            (Efm8CrossbarFunction::Smb1Sda, 0x80),
        ] {
            if xbr1 & mask != 0 {
                self.assign_crossbar_pin(function, &mut occupied, 1);
            }
        }
        if xbr1 & 0x80 != 0 {
            self.assign_crossbar_pin(Efm8CrossbarFunction::Smb1Scl, &mut occupied, 1);
        }
        let xbr2 = self.registers[XBR2];
        if xbr2 & 0x01 != 0 {
            self.assign_crossbar_pin(Efm8CrossbarFunction::Uart1Tx, &mut occupied, 1);
            self.assign_crossbar_pin(Efm8CrossbarFunction::Uart1Rx, &mut occupied, 1);
        }
        if xbr2 & 0x02 != 0 {
            self.assign_crossbar_pin(Efm8CrossbarFunction::Uart1Rts, &mut occupied, 1);
        }
        if xbr2 & 0x04 != 0 {
            self.assign_crossbar_pin(Efm8CrossbarFunction::Uart1Cts, &mut occupied, 1);
        }
        let pwm_count = usize::from((xbr2 >> 3) & 0x03);
        for function in [
            Efm8CrossbarFunction::Pwm0,
            Efm8CrossbarFunction::Pwm1,
            Efm8CrossbarFunction::Pwm2,
        ]
        .into_iter()
        .take(pwm_count)
        {
            self.assign_crossbar_pin(function, &mut occupied, 1);
        }
        self.set_signal(
            self.crossbar_enabled_signal,
            u64::from(xbr2 & XBR2_XBARE != 0),
            1,
            at,
        );
        self.set_signal(
            self.crossbar_assigned_signal,
            u64::try_from(self.crossbar_routes.iter().flatten().count())
                .expect("EFM8 crossbar route count fits in u64"),
            8,
            at,
        );
        for (signal, function) in [
            (self.crossbar_uart0_tx_signal, Efm8CrossbarFunction::Uart0Tx),
            (self.crossbar_uart0_rx_signal, Efm8CrossbarFunction::Uart0Rx),
        ] {
            let value = self.crossbar_routes[function.index()]
                .map_or(0xff, |pin| u64::from(pin.port * 8 + pin.pin));
            self.set_signal(signal, value, 8, at);
        }
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
        self.watchdog_epoch = at.ticks();
        self.watchdog_key = 0;
        self.watchdog_enabled = true;
        self.watchdog_reset = false;
        for signal in [
            self.uart_strobe_signal,
            self.timer0_irq_signal,
            self.timer2_irq_signal,
            self.interrupt_signal,
            self.watchdog_reset_signal,
        ] {
            self.set_signal(signal, 0, 1, at);
        }
        for port in 0..4 {
            let _ = self.refresh_port(port, at);
        }
        self.refresh_crossbar(at);
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

    /// Returns the current physical pin assigned to a crossbar function.
    pub fn crossbar_pin(&self, function: Efm8CrossbarFunction) -> Option<Efm8CrossbarPin> {
        self.0.lock().expect("EFM8 lock poisoned").crossbar_routes[function.index()]
    }

    /// Reports whether the port crossbar output drivers are enabled.
    pub fn crossbar_enabled(&self) -> bool {
        self.0.lock().expect("EFM8 lock poisoned").registers[XBR2] & XBR2_XBARE != 0
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
        let crossbar_enabled_signal = hub.declare(
            "board.efm8bb52f32g.crossbar.enabled",
            SignalValue::from_u64(0, 1)?,
            Some("priority crossbar output-driver enable".to_owned()),
        )?;
        let crossbar_assigned_signal = hub.declare(
            "board.efm8bb52f32g.crossbar.assigned_count",
            SignalValue::from_u64(0, 8)?,
            Some("number of functions assigned to physical pins".to_owned()),
        )?;
        let crossbar_uart0_tx_signal = hub.declare(
            "board.efm8bb52f32g.crossbar.uart0.tx_pin",
            SignalValue::from_u64(0xff, 8)?,
            Some("UART0 TX physical pin index, or 0xff when disabled".to_owned()),
        )?;
        let crossbar_uart0_rx_signal = hub.declare(
            "board.efm8bb52f32g.crossbar.uart0.rx_pin",
            SignalValue::from_u64(0xff, 8)?,
            Some("UART0 RX physical pin index, or 0xff when disabled".to_owned()),
        )?;
        let state = Arc::new(Mutex::new(Efm8State {
            registers: vec![0; SFR_BYTES].into_boxed_slice(),
            crossbar_routes: [None; Efm8CrossbarFunction::COUNT],
            ports: [port0, port1, port2, port3],
            port_signals: [signals0, signals1, signals2, signals3],
            hub,
            uart: Vec::new(),
            timer0_epoch: 0,
            timer2_epoch: 0,
            watchdog_epoch: 0,
            watchdog_key: 0,
            watchdog_enabled: true,
            watchdog_reset: false,
            uart_byte_signal,
            uart_strobe_signal,
            timer0_irq_signal,
            timer2_irq_signal,
            interrupt_signal,
            watchdog_reset_signal,
            crossbar_enabled_signal,
            crossbar_assigned_signal,
            crossbar_uart0_tx_signal,
            crossbar_uart0_rx_signal,
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
        }
        if matches!(address, XBR0 | XBR1 | XBR2 | P0SKIP | P1SKIP | P2SKIP) {
            state.refresh_crossbar(at);
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
        AccessWidth, Efm8CrossbarFunction, Efm8CrossbarPin, Efm8Peripherals, IE, IE_EA, IE_ET0, P0,
        P0MDOUT, P0SKIP, SBUF0, SimTime, TCON, TCON_TR0, TMOD, XBR0, XBR0_URT0E, XBR2, XBR2_XBARE,
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
    fn crossbar_assigns_fixed_uart_and_priority_skips_pins() {
        let hub = super::SignalHub::new();
        let (mut device, handle, _) = Efm8Peripherals::new("efm8.sfr", hub).unwrap();
        device
            .write(P0SKIP as u64, AccessWidth::Byte, 0x01, SimTime::ZERO)
            .unwrap();
        device
            .write(
                XBR0 as u64,
                AccessWidth::Byte,
                (XBR0_URT0E | 0x02).into(),
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

        assert!(handle.crossbar_enabled());
        assert_eq!(
            handle.crossbar_pin(Efm8CrossbarFunction::Uart0Tx),
            Some(Efm8CrossbarPin { port: 0, pin: 4 })
        );
        assert_eq!(
            handle.crossbar_pin(Efm8CrossbarFunction::Uart0Rx),
            Some(Efm8CrossbarPin { port: 0, pin: 5 })
        );
        assert_eq!(
            handle.crossbar_pin(Efm8CrossbarFunction::Spi0Sck),
            Some(Efm8CrossbarPin { port: 0, pin: 1 })
        );
        assert_eq!(
            handle.crossbar_pin(Efm8CrossbarFunction::Spi0Nss),
            Some(Efm8CrossbarPin { port: 0, pin: 6 })
        );
    }
}
