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
const PCA0CN: usize = 0xd8;
const PCA0MD: usize = 0xd9;
const PCA0CPM0: usize = 0xda;
const PCA0CPM1: usize = 0xdb;
const PCA0CPM2: usize = 0xdc;
const EIE1: usize = 0xe6;
const XBR0: usize = 0xe1;
const XBR2: usize = 0xe3;
const EIP1: usize = 0x10bb;
const EIP1H: usize = 0x10ee;
const PCA0POL: usize = 0x96;
const PCA0PWM: usize = 0xf7;
const PCA0CENT: usize = 0xf8;
const PCA0L: usize = 0xf9;
const PCA0H: usize = 0xfa;
const PCA0CPL0: usize = 0xfb;
const PCA0CPH0: usize = 0xfc;
const PCA0CPL1: usize = 0xe9;
const PCA0CPH1: usize = 0xea;
const PCA0CPL2: usize = 0xeb;
const PCA0CPH2: usize = 0xec;
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
const PCA0_CPM: [usize; 3] = [PCA0CPM0, PCA0CPM1, PCA0CPM2];
const PCA0_CPL: [usize; 3] = [PCA0CPL0, PCA0CPL1, PCA0CPL2];
const PCA0_CPH: [usize; 3] = [PCA0CPH0, PCA0CPH1, PCA0CPH2];
const PCA0_CCF: [u8; 3] = [PCA0CN_CCF0, PCA0CN_CCF1, PCA0CN_CCF2];

const IE_EA: u8 = 0x80;
const IE_ET0: u8 = 0x02;
const IE_ES0: u8 = 0x10;
const IE_ET2: u8 = 0x20;
const EIE1_EPCA0: u8 = 0x10;
const EIP1_PPCA0: u8 = 0x10;
const EIP1H_PHPCA0: u8 = 0x10;
const TCON_TR0: u8 = 0x10;
const TCON_TF0: u8 = 0x20;
const TMR2_TR2: u8 = 0x04;
const TMR2_TF2H: u8 = 0x80;
const SCON0_RI: u8 = 0x01;
const SCON0_TI: u8 = 0x02;
const XBR0_URT0E: u8 = 0x01;
const XBR2_XBARE: u8 = 0x40;
const PCA0CN_CF: u8 = 0x80;
const PCA0CN_CR: u8 = 0x40;
const PCA0CN_CCF0: u8 = 0x01;
const PCA0CN_CCF1: u8 = 0x02;
const PCA0CN_CCF2: u8 = 0x04;
const PCA0MD_ECF: u8 = 0x01;
const PCA0PWM_ECOV: u8 = 0x40;
const PCA0PWM_COVF: u8 = 0x20;
const PCA0PWM_CLSEL_MASK: u8 = 0x07;
const PCA0CPM_PWM16: u8 = 0x80;
const PCA0CPM_ECOM: u8 = 0x40;
const PCA0CPM_CAPP: u8 = 0x20;
const PCA0CPM_CAPN: u8 = 0x10;
const PCA0CPM_MAT: u8 = 0x08;
const PCA0CPM_TOG: u8 = 0x04;
const PCA0CPM_PWM: u8 = 0x02;
const PCA0CPM_ECCF: u8 = 0x01;

struct Efm8State {
    registers: Box<[u8]>,
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
    pca_epoch: u64,
    pca_outputs: [Logic; 3],
    pca_inputs: [Logic; 3],
    pca_output_signals: [SignalId; 3],
    pca_interrupt_signal: SignalId,
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
        self.watchdog_epoch = at.ticks();
        self.watchdog_key = 0;
        self.watchdog_enabled = true;
        self.watchdog_reset = false;
        self.pca_epoch = at.ticks();
        self.pca_outputs = [Logic::Zero; 3];
        self.pca_inputs = [Logic::Zero; 3];
        for signal in [
            self.uart_strobe_signal,
            self.timer0_irq_signal,
            self.timer2_irq_signal,
            self.interrupt_signal,
            self.watchdog_reset_signal,
            self.pca_output_signals[0],
            self.pca_output_signals[1],
            self.pca_output_signals[2],
            self.pca_interrupt_signal,
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
            | 0xd8..=0xdc
            | 0xd4..=0xd5
            | 0xe1..=0xe3
            | 0xe6
            | 0xe9..=0xec
            | 0xef
            | 0xf1..=0xf3
            | 0xf7..=0xfc => address,
            0x9c | 0xf4 if page == PAGE3 => (PAGE3 << 8) | address,
            _ => raw,
        }
    }

    fn interrupt_levels(&self) -> [bool; 8] {
        let enabled = self.registers[IE];
        if enabled & IE_EA == 0 {
            return [false; 8];
        }
        let active = [
            enabled & IE_ET0 != 0 && self.registers[TCON] & TCON_TF0 != 0,
            enabled & IE_ES0 != 0 && self.registers[SCON0] & (SCON0_RI | SCON0_TI) != 0,
            enabled & IE_ET2 != 0 && self.registers[TMR2CN0] & TMR2_TF2H != 0,
        ];
        let priorities = [IE_ET0, IE_ES0, IE_ET2];
        let mut levels = [false; 8];
        for source in 0..3 {
            if active[source] {
                let high = self.registers[IP] & priorities[source] != 0;
                levels[source + if high { 3 } else { 0 }] = true;
            }
        }
        if self.pca_interrupt_pending() {
            levels[if self.pca_high_priority() { 7 } else { 6 }] = true;
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
        self.set_signal(
            self.pca_interrupt_signal,
            u64::from(self.pca_interrupt_pending()),
            1,
            at,
        );
    }

    fn pca_counter(&self) -> u16 {
        u16::from_le_bytes([self.registers[PCA0L], self.registers[PCA0H]])
    }

    fn set_pca_counter(&mut self, value: u16) {
        let [low, high] = value.to_le_bytes();
        self.registers[PCA0L] = low;
        self.registers[PCA0H] = high;
    }

    fn pca_divider(&self) -> u64 {
        match (self.registers[PCA0MD] >> 1) & 0x07 {
            0 => 12,
            1 => 4,
            2 | 3 => 1,
            4 => 1,
            5 | 6 => 8,
            _ => 1,
        }
    }

    fn pca_crossed(start: u16, ticks: u64, target: u16, modulus: u64) -> bool {
        if ticks == 0 {
            return false;
        }
        if ticks >= modulus {
            return true;
        }
        let end = u64::from(start) + ticks;
        if end < modulus {
            u64::from(target) > u64::from(start) && u64::from(target) <= end
        } else {
            let wrapped = end % modulus;
            u64::from(target) > u64::from(start) || u64::from(target) <= wrapped
        }
    }

    fn pca_width(&self, channel: usize, mode: u8) -> u8 {
        if mode & PCA0CPM_PWM16 != 0 {
            16
        } else {
            8 + (self.registers[PCA0PWM] & PCA0PWM_CLSEL_MASK).min(3)
        }
        .min(if channel < 3 { 16 } else { 8 })
    }

    fn pca_interrupt_pending(&self) -> bool {
        if self.registers[EIE1] & EIE1_EPCA0 == 0 {
            return false;
        }
        let cn = self.registers[PCA0CN];
        let pwm = self.registers[PCA0PWM];
        (cn & PCA0CN_CF != 0 && self.registers[PCA0MD] & PCA0MD_ECF != 0)
            || (pwm & PCA0PWM_COVF != 0 && pwm & PCA0PWM_ECOV != 0)
            || (0..3).any(|channel| {
                cn & PCA0_CCF[channel] != 0 && self.registers[PCA0_CPM[channel]] & PCA0CPM_ECCF != 0
            })
    }

    fn pca_high_priority(&self) -> bool {
        self.registers[EIP1H] & EIP1H_PHPCA0 != 0 || self.registers[EIP1] & EIP1_PPCA0 != 0
    }

    fn update_pca_output(&mut self, channel: usize, value: Logic, at: SimTime) {
        if self.pca_outputs[channel] == value {
            return;
        }
        self.pca_outputs[channel] = value;
        self.set_signal(
            self.pca_output_signals[channel],
            u64::from(value == Logic::One),
            1,
            at,
        );
    }

    fn advance_pca(&mut self, now: SimTime) -> Result<(), DeviceError> {
        let elapsed = now.ticks().saturating_sub(self.pca_epoch);
        if self.registers[PCA0CN] & PCA0CN_CR == 0 {
            self.pca_epoch = now.ticks();
            self.update_interrupt_signals(now);
            return Ok(());
        }
        let divider = self.pca_divider();
        let ticks = elapsed / divider;
        if ticks == 0 {
            self.update_interrupt_signals(now);
            return Ok(());
        }
        let start = self.pca_counter();
        let end = start.wrapping_add(ticks as u16);
        let overflow = u64::from(start) + ticks >= 0x1_0000;
        if overflow {
            self.registers[PCA0CN] |= PCA0CN_CF;
        }
        let cycle_bits = 8 + (self.registers[PCA0PWM] & PCA0PWM_CLSEL_MASK).min(3);
        let cycle_modulus = 1_u64 << cycle_bits;
        if u64::from(start) + ticks >= cycle_modulus {
            self.registers[PCA0PWM] |= PCA0PWM_COVF;
        }
        for channel in 0..3 {
            let mode = self.registers[PCA0_CPM[channel]];
            let compare = u16::from_le_bytes([
                self.registers[PCA0_CPL[channel]],
                self.registers[PCA0_CPH[channel]],
            ]);
            let matched =
                mode & PCA0CPM_ECOM != 0 && Self::pca_crossed(start, ticks, compare, 0x1_0000);
            if matched && mode & PCA0CPM_MAT != 0 {
                self.registers[PCA0CN] |= PCA0_CCF[channel];
            }
            if matched && mode & PCA0CPM_TOG != 0 {
                let value = if self.pca_outputs[channel] == Logic::One {
                    Logic::Zero
                } else {
                    Logic::One
                };
                self.update_pca_output(channel, value, now);
            } else if mode & PCA0CPM_PWM != 0 && mode & PCA0CPM_TOG == 0 {
                let width = self.pca_width(channel, mode);
                let mask = (1_u32 << width) - 1;
                let duty = u32::from(compare) & mask;
                let count = u32::from(end) & mask;
                let mut high = count >= duty;
                if self.registers[PCA0POL] & (1 << channel) != 0 {
                    high = !high;
                }
                self.update_pca_output(channel, if high { Logic::One } else { Logic::Zero }, now);
            } else if mode & PCA0CPM_PWM == 0 && mode & PCA0CPM_TOG == 0 {
                self.update_pca_output(channel, Logic::Zero, now);
            }
        }
        self.set_pca_counter(end);
        self.pca_epoch = now.ticks().saturating_sub(elapsed % divider);
        self.update_interrupt_signals(now);
        Ok(())
    }

    fn capture_pca_input(
        &mut self,
        channel: usize,
        value: Logic,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        let Some(cpm_address) = PCA0_CPM.get(channel).copied() else {
            return Err(DeviceError::new(format!(
                "EFM8 PCA channel {channel} is outside 0..2"
            )));
        };
        self.advance_pca(at)?;
        let previous = self.pca_inputs[channel];
        self.pca_inputs[channel] = value;
        let rising = previous != Logic::One && value == Logic::One;
        let falling = previous != Logic::Zero && value == Logic::Zero;
        let mode = self.registers[cpm_address];
        if (rising && mode & PCA0CPM_CAPP != 0) || (falling && mode & PCA0CPM_CAPN != 0) {
            let counter = self.pca_counter();
            let [low, high] = counter.to_le_bytes();
            self.registers[PCA0_CPL[channel]] = low;
            self.registers[PCA0_CPH[channel]] = high;
            self.registers[PCA0CN] |= PCA0_CCF[channel];
        }
        self.update_interrupt_signals(at);
        Ok(())
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

    /// Returns the resolved PCA CEX output for a channel.
    pub fn pca_output(&self, channel: usize) -> Logic {
        self.0
            .lock()
            .expect("EFM8 lock poisoned")
            .pca_outputs
            .get(channel)
            .copied()
            .unwrap_or(Logic::X)
    }

    /// Returns the current 16-bit PCA counter.
    pub fn pca_counter(&self) -> u16 {
        self.0.lock().expect("EFM8 lock poisoned").pca_counter()
    }

    /// Supplies a sampled CEX input edge for a capture channel.
    pub fn set_pca_input(
        &self,
        channel: usize,
        value: Logic,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        self.0
            .lock()
            .expect("EFM8 lock poisoned")
            .capture_pca_input(channel, value, at)
    }

    /// Returns the currently asserted PCA interrupt request.
    pub fn pca_interrupt_pending(&self) -> bool {
        self.0
            .lock()
            .expect("EFM8 lock poisoned")
            .pca_interrupt_pending()
    }

    /// Supplies one received UART0 byte and raises RI.
    pub fn inject_uart_rx(&self, value: u8, at: SimTime) {
        let mut state = self.0.lock().expect("EFM8 lock poisoned");
        state.registers[SBUF0] = value;
        state.registers[SCON0] |= SCON0_RI;
        state.update_interrupt_signals(at);
    }

    /// Advances functional timers/watchdog and returns low/high CPU interrupt inputs.
    pub fn poll(&self, now: SimTime) -> [bool; 8] {
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
        let _ = state.advance_pca(now);
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
        let pca_output_signals = [
            hub.declare(
                "board.efm8bb52f32g.pca0.cex0",
                SignalValue::from_u64(0, 1)?,
                Some("PCA channel 0 CEX output".to_owned()),
            )?,
            hub.declare(
                "board.efm8bb52f32g.pca0.cex1",
                SignalValue::from_u64(0, 1)?,
                Some("PCA channel 1 CEX output".to_owned()),
            )?,
            hub.declare(
                "board.efm8bb52f32g.pca0.cex2",
                SignalValue::from_u64(0, 1)?,
                Some("PCA channel 2 CEX output".to_owned()),
            )?,
        ];
        let pca_interrupt_signal = hub.declare(
            "board.efm8bb52f32g.pca0.interrupt",
            SignalValue::from_u64(0, 1)?,
            Some("PCA capture/compare interrupt request".to_owned()),
        )?;
        let state = Arc::new(Mutex::new(Efm8State {
            registers: vec![0; SFR_BYTES].into_boxed_slice(),
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
            pca_epoch: 0,
            pca_outputs: [Logic::Zero; 3],
            pca_inputs: [Logic::Zero; 3],
            pca_output_signals,
            pca_interrupt_signal,
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
        if matches!(
            address,
            PCA0CN
                | PCA0MD
                | PCA0CPM0
                | PCA0CPM1
                | PCA0CPM2
                | PCA0PWM
                | PCA0CENT
                | PCA0L
                | PCA0H
                | PCA0CPL0
                | PCA0CPH0
                | PCA0CPL1
                | PCA0CPH1
                | PCA0CPL2
                | PCA0CPH2
                | PCA0POL
        ) {
            state.advance_pca(at)?;
        }
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
        let pca_register = matches!(
            address,
            PCA0CN
                | PCA0MD
                | PCA0CPM0
                | PCA0CPM1
                | PCA0CPM2
                | PCA0PWM
                | PCA0CENT
                | PCA0L
                | PCA0H
                | PCA0CPL0
                | PCA0CPH0
                | PCA0CPL1
                | PCA0CPH1
                | PCA0CPL2
                | PCA0CPH2
                | PCA0POL
        );
        if pca_register {
            state.advance_pca(at)?;
        }
        state.registers[address] = value;
        if let Some(port) = Self::port_index(address) {
            state.registers[address] &= PORT_MASKS[port];
            state.refresh_port(port, at)?;
        } else if let Some(port) = PORT_MDOUT.iter().position(|item| *item == address) {
            state.registers[address] &= PORT_MASKS[port];
            state.refresh_port(port, at)?;
        } else if address == PCA0CN {
            state.registers[address] &=
                PCA0CN_CF | PCA0CN_CR | PCA0CN_CCF0 | PCA0CN_CCF1 | PCA0CN_CCF2;
            state.pca_epoch = at.ticks();
        } else if address == PCA0L || address == PCA0H {
            state.pca_epoch = at.ticks();
        } else if let Some(channel) = PCA0_CPL.iter().position(|item| *item == address) {
            state.registers[PCA0_CPM[channel]] &= !PCA0CPM_ECOM;
        } else if let Some(channel) = PCA0_CPH.iter().position(|item| *item == address) {
            state.registers[PCA0_CPM[channel]] |= PCA0CPM_ECOM;
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
        AccessWidth, EIE1, EIE1_EPCA0, Efm8Peripherals, IE, IE_EA, IE_ET0, P0, P0MDOUT, PCA0CN,
        PCA0CN_CR, PCA0CPH0, PCA0CPH1, PCA0CPL0, PCA0CPL1, PCA0CPM0, PCA0CPM1, PCA0MD, PCA0PWM,
        SBUF0, SimTime, TCON, TCON_TR0, TMOD, XBR0, XBR0_URT0E, XBR2, XBR2_XBARE,
    };
    use remu_bus::Device;
    use remu_signals::Logic;

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
    fn pca_pwm_capture_and_interrupt_slice_is_functional() {
        let hub = super::SignalHub::new();
        let (mut device, handle, _) = Efm8Peripherals::new("efm8.sfr", hub).unwrap();
        // Select SYSCLK as the abstract PCA timebase and configure an 8-bit PWM.
        device
            .write(PCA0MD as u64, AccessWidth::Byte, 0x08, SimTime::ZERO)
            .unwrap();
        device
            .write(PCA0PWM as u64, AccessWidth::Byte, 0, SimTime::ZERO)
            .unwrap();
        device
            .write(PCA0CPM0 as u64, AccessWidth::Byte, 0x02, SimTime::ZERO)
            .unwrap();
        device
            .write(PCA0CPL0 as u64, AccessWidth::Byte, 0x40, SimTime::ZERO)
            .unwrap();
        device
            .write(PCA0CPH0 as u64, AccessWidth::Byte, 0, SimTime::ZERO)
            .unwrap();
        device
            .write(
                PCA0CN as u64,
                AccessWidth::Byte,
                PCA0CN_CR.into(),
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(handle.poll(SimTime::from_ticks(0))[0], false);
        assert_eq!(handle.poll(SimTime::from_ticks(0x40))[0], false);
        assert_eq!(handle.pca_output(0), Logic::One);
        assert_eq!(handle.pca_counter(), 0x40);
        assert_eq!(handle.poll(SimTime::from_ticks(0x100))[0], false);
        assert_eq!(handle.pca_output(0), Logic::Zero);

        // A channel compare and an input capture share the PCA request line.
        device
            .write(
                PCA0CPM0 as u64,
                AccessWidth::Byte,
                0x49,
                SimTime::from_ticks(0x100),
            )
            .unwrap();
        device
            .write(
                PCA0CPL0 as u64,
                AccessWidth::Byte,
                2,
                SimTime::from_ticks(0x100),
            )
            .unwrap();
        device
            .write(
                PCA0CPH0 as u64,
                AccessWidth::Byte,
                1,
                SimTime::from_ticks(0x100),
            )
            .unwrap();
        device
            .write(
                EIE1 as u64,
                AccessWidth::Byte,
                EIE1_EPCA0.into(),
                SimTime::from_ticks(0x100),
            )
            .unwrap();
        device
            .write(
                IE as u64,
                AccessWidth::Byte,
                IE_EA.into(),
                SimTime::from_ticks(0x100),
            )
            .unwrap();
        let levels = handle.poll(SimTime::from_ticks(0x104));
        assert!(levels[6]);
        assert!(handle.pca_interrupt_pending());

        device
            .write(
                PCA0CPM1 as u64,
                AccessWidth::Byte,
                0x21,
                SimTime::from_ticks(0x104),
            )
            .unwrap();
        handle
            .set_pca_input(1, Logic::One, SimTime::from_ticks(0x108))
            .unwrap();
        assert_eq!(
            device
                .read(
                    PCA0CPL1 as u64,
                    AccessWidth::Byte,
                    SimTime::from_ticks(0x108)
                )
                .unwrap(),
            0x08
        );
        assert_eq!(
            device
                .read(
                    PCA0CPH1 as u64,
                    AccessWidth::Byte,
                    SimTime::from_ticks(0x108)
                )
                .unwrap(),
            1
        );
    }
}
