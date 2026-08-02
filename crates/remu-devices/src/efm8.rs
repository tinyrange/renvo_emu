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
const ADC0CN0: usize = 0xe8;
const ADC0CN1: usize = 0xb2;
const ADC0CN2: usize = 0xb3;
const ADC0CF1: usize = 0xb9;
const ADC0CF2: usize = 0xdf;
const ADC0L: usize = 0xbd;
const ADC0H: usize = 0xbe;
const ADC0GTH: usize = 0xc4;
const ADC0GTL: usize = 0xc3;
const ADC0LTH: usize = 0xc6;
const ADC0LTL: usize = 0xc5;
const ADC0MX: usize = 0xbb;
const EIE1: usize = 0xe6;
const EIP1: usize = (0x10 << 8) | 0xbb;
const EIP1H: usize = (0x10 << 8) | 0xee;
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
const ADC0_ADEN: u8 = 0x80;
const ADC0_ADINT: u8 = 0x20;
const ADC0_ADBUSY: u8 = 0x10;
const ADC0_ADWINT: u8 = 0x08;
const ADC0_EADC0: u8 = 0x08;
const ADC0_EWADC0: u8 = 0x04;
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
    adc_inputs: [u16; 32],
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
    adc_result_signal: SignalId,
    adc_eoc_signal: SignalId,
    adc_window_signal: SignalId,
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
        self.adc_inputs.fill(0);
        self.registers[ADC0MX] = 0x1f;
        self.registers[ADC0CF2] = 0x1f;
        self.registers[ADC0GTH] = 0xff;
        self.registers[ADC0GTL] = 0xff;
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
            self.adc_eoc_signal,
            self.adc_window_signal,
            self.interrupt_signal,
            self.watchdog_reset_signal,
        ] {
            self.set_signal(signal, 0, 1, at);
        }
        self.set_signal(self.adc_result_signal, 0, 16, at);
        for port in 0..4 {
            let _ = self.refresh_port(port, at);
        }
    }

    fn complete_adc_conversion(&mut self, at: SimTime) {
        let control = self.registers[ADC0CN0];
        if control & ADC0_ADEN == 0 {
            self.registers[ADC0CN0] &= !ADC0_ADBUSY;
            return;
        }
        let channel = usize::from(self.registers[ADC0MX] & 0x1f);
        let mut sample = self.adc_inputs[channel.min(self.adc_inputs.len() - 1)];
        match (self.registers[ADC0CN1] >> 5) & 0x03 {
            0x01 => sample >>= 2,
            0x02 => sample >>= 4,
            _ => {}
        }
        let repeat = match self.registers[ADC0CN1] & 0x07 {
            0x01 => 4,
            0x02 => 8,
            0x03 => 16,
            0x04 => 32,
            _ => 1,
        };
        let mut result = sample.saturating_mul(repeat);
        result >>= (self.registers[ADC0CN1] >> 3) & 0x03;
        let [low, high] = result.to_le_bytes();
        self.registers[ADC0L] = low;
        self.registers[ADC0H] = high;
        self.registers[ADC0CN0] &= !ADC0_ADBUSY;
        self.registers[ADC0CN0] |= ADC0_ADINT;
        let greater = u16::from_be_bytes([self.registers[ADC0GTH], self.registers[ADC0GTL]]);
        let less = u16::from_be_bytes([self.registers[ADC0LTH], self.registers[ADC0LTL]]);
        if result > greater || result < less {
            self.registers[ADC0CN0] |= ADC0_ADWINT;
        }
        self.set_signal(self.adc_result_signal, u64::from(result), 16, at);
        self.set_signal(self.adc_eoc_signal, 1, 1, at);
        self.set_signal(
            self.adc_window_signal,
            u64::from(self.registers[ADC0CN0] & ADC0_ADWINT != 0),
            1,
            at,
        );
    }

    fn canonical(raw: usize) -> usize {
        let page = raw >> 8;
        let address = raw & 0xff;
        if raw == EIP1 || raw == EIP1H {
            return raw;
        }
        if page == 0x30
            && matches!(
                address,
                ADC0CN1
                    | ADC0CN2
                    | ADC0CF1
                    | ADC0MX
                    | ADC0L..=ADC0H
                    | ADC0GTL..=ADC0LTH
                    | ADC0CF2
                    | ADC0CN0
            )
        {
            // The ADC control/result block is mirrored on pages 0x00 and
            // 0x30; autoscan-only registers retain their page-30 address.
            return address;
        }
        match address {
            0x80
            | 0x88..=0x8e
            | 0x90
            | ADC0CN1..=ADC0CN2
            | ADC0CF1
            | ADC0MX
            | ADC0L..=ADC0H
            | ADC0GTL..=ADC0LTH
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
            | ADC0CN0
            | ADC0CF2
            | 0xef
            | 0xf1..=0xf3 => address,
            0x9c | 0xf4 if page == PAGE3 => (PAGE3 << 8) | address,
            _ => raw,
        }
    }

    fn interrupt_levels(&self) -> [bool; 18] {
        let enabled = self.registers[IE];
        if enabled & IE_EA == 0 {
            return [false; 18];
        }
        let active = [
            enabled & IE_ET0 != 0 && self.registers[TCON] & TCON_TF0 != 0,
            enabled & IE_ES0 != 0 && self.registers[SCON0] & (SCON0_RI | SCON0_TI) != 0,
            enabled & IE_ET2 != 0 && self.registers[TMR2CN0] & TMR2_TF2H != 0,
        ];
        let priorities = [IE_ET0, IE_ES0, IE_ET2];
        let mut levels = [false; 18];
        for source in 0..3 {
            if active[source] {
                let high = self.registers[IP] & priorities[source] != 0;
                levels[source + if high { 3 } else { 0 }] = true;
            }
        }
        let adc_window =
            self.registers[EIE1] & ADC0_EWADC0 != 0 && self.registers[ADC0CN0] & ADC0_ADWINT != 0;
        let adc_complete =
            self.registers[EIE1] & ADC0_EADC0 != 0 && self.registers[ADC0CN0] & ADC0_ADINT != 0;
        let adc_window_high = self.registers[EIP1] & 0x04 != 0 || self.registers[EIP1H] & 0x04 != 0;
        let adc_complete_high =
            self.registers[EIP1] & 0x08 != 0 || self.registers[EIP1H] & 0x08 != 0;
        if adc_window {
            levels[14 + usize::from(adc_window_high)] = true;
        }
        if adc_complete {
            levels[16 + usize::from(adc_complete_high)] = true;
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
            self.adc_eoc_signal,
            u64::from(self.registers[ADC0CN0] & ADC0_ADINT != 0),
            1,
            at,
        );
        self.set_signal(
            self.adc_window_signal,
            u64::from(self.registers[ADC0CN0] & ADC0_ADWINT != 0),
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

    /// Supplies one received UART0 byte and raises RI.
    pub fn inject_uart_rx(&self, value: u8, at: SimTime) {
        let mut state = self.0.lock().expect("EFM8 lock poisoned");
        state.registers[SBUF0] = value;
        state.registers[SCON0] |= SCON0_RI;
        state.update_interrupt_signals(at);
    }

    /// Sets one deterministic analog input code for the ADC multiplexer.
    pub fn set_adc_input(&self, channel: u8, value: u16) -> Result<(), DeviceError> {
        let channel = usize::from(channel);
        let mut state = self.0.lock().expect("EFM8 lock poisoned");
        let input = state.adc_inputs.get_mut(channel).ok_or_else(|| {
            DeviceError::new(format!("EFM8 ADC channel {channel} is outside 0..31"))
        })?;
        *input = value.min(0x0fff);
        Ok(())
    }

    /// Advances functional timers/watchdog and returns low/high CPU interrupt inputs.
    pub fn poll(&self, now: SimTime) -> [bool; 18] {
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
        let adc_result_signal = hub.declare(
            "board.efm8bb52f32g.adc0.result",
            SignalValue::from_u64(0, 16)?,
            Some("last ADC0 conversion result".to_owned()),
        )?;
        let adc_eoc_signal = hub.declare(
            "board.efm8bb52f32g.adc0.end_of_conversion",
            SignalValue::from_u64(0, 1)?,
            Some("ADC0 conversion-complete flag".to_owned()),
        )?;
        let adc_window_signal = hub.declare(
            "board.efm8bb52f32g.adc0.window",
            SignalValue::from_u64(0, 1)?,
            Some("ADC0 window-comparison flag".to_owned()),
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
            adc_inputs: [0; 32],
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
            adc_result_signal,
            adc_eoc_signal,
            adc_window_signal,
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
        } else if address == ADC0CN0 && value & ADC0_ADBUSY != 0 {
            if value & ADC0_ADEN != 0 && state.registers[ADC0CN2] & 0x0f == 0 {
                state.complete_adc_conversion(at);
            } else {
                // Only the documented software-trigger path is modeled;
                // unsupported trigger sources never leave the ADC stuck busy.
                state.registers[ADC0CN0] &= !ADC0_ADBUSY;
            }
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
        ADC0_ADBUSY, ADC0_ADEN, ADC0_ADINT, ADC0_ADWINT, ADC0_EADC0, ADC0_EWADC0, ADC0CN0, ADC0GTH,
        ADC0GTL, ADC0H, ADC0L, ADC0LTH, ADC0LTL, ADC0MX, AccessWidth, EIE1, Efm8Peripherals, IE,
        IE_EA, IE_ET0, P0, P0MDOUT, SBUF0, SimTime, TCON, TCON_TR0, TMOD, XBR0, XBR0_URT0E, XBR2,
        XBR2_XBARE,
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
    fn adc_channel_conversion_window_and_interrupts_are_functional() {
        let hub = super::SignalHub::new();
        let trace_hub = hub.clone();
        let (mut device, handle, _) = Efm8Peripherals::new("efm8.sfr", hub).unwrap();
        handle.set_adc_input(3, 0x0abc).unwrap();

        device
            .write(ADC0MX as u64, AccessWidth::Byte, 3, SimTime::ZERO)
            .unwrap();
        device
            .write(ADC0GTH as u64, AccessWidth::Byte, 0x0b, SimTime::ZERO)
            .unwrap();
        device
            .write(ADC0GTL as u64, AccessWidth::Byte, 0xff, SimTime::ZERO)
            .unwrap();
        device
            .write(ADC0LTH as u64, AccessWidth::Byte, 0x08, SimTime::ZERO)
            .unwrap();
        device
            .write(ADC0LTL as u64, AccessWidth::Byte, 0x00, SimTime::ZERO)
            .unwrap();
        // The page-0x30 aliases are the names used by the Silicon Labs SDK.
        device
            .write(0x30b2, AccessWidth::Byte, 0, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            device
                .read(0x30b2, AccessWidth::Byte, SimTime::ZERO)
                .unwrap(),
            0
        );
        device
            .write(
                EIE1 as u64,
                AccessWidth::Byte,
                u64::from(ADC0_EADC0 | ADC0_EWADC0),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                IE as u64,
                AccessWidth::Byte,
                u64::from(IE_EA),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                ADC0CN0 as u64,
                AccessWidth::Byte,
                u64::from(ADC0_ADEN | ADC0_ADBUSY),
                SimTime::from_ticks(1),
            )
            .unwrap();

        assert_eq!(
            device
                .read(ADC0L as u64, AccessWidth::Byte, SimTime::ZERO)
                .unwrap(),
            0xbc
        );
        assert_eq!(
            device
                .read(ADC0H as u64, AccessWidth::Byte, SimTime::ZERO)
                .unwrap(),
            0x0a
        );
        let control = device
            .read(ADC0CN0 as u64, AccessWidth::Byte, SimTime::ZERO)
            .unwrap();
        assert_ne!(control & u64::from(ADC0_ADINT), 0);
        assert_eq!(control & u64::from(ADC0_ADWINT), 0);
        let levels = handle.poll(SimTime::from_ticks(1));
        assert!(!levels[14]);
        assert!(levels[16]);

        let result_id = trace_hub
            .with_registry(|registry| registry.find("board.efm8bb52f32g.adc0.result"))
            .unwrap();
        assert_eq!(
            trace_hub
                .with_registry(|registry| { registry.value(result_id).unwrap().to_vcd_binary() }),
            "0000101010111100"
        );

        // Clear the latched EOC/window flags, then produce an out-of-window
        // result and verify that the independent window interrupt is raised.
        device
            .write(
                ADC0CN0 as u64,
                AccessWidth::Byte,
                u64::from(ADC0_ADEN),
                SimTime::from_ticks(2),
            )
            .unwrap();
        handle.set_adc_input(3, 0x0fff).unwrap();
        device
            .write(
                ADC0CN0 as u64,
                AccessWidth::Byte,
                u64::from(ADC0_ADEN | ADC0_ADBUSY),
                SimTime::from_ticks(3),
            )
            .unwrap();
        let control = device
            .read(ADC0CN0 as u64, AccessWidth::Byte, SimTime::ZERO)
            .unwrap();
        assert_ne!(control & u64::from(ADC0_ADWINT), 0);
        let levels = handle.poll(SimTime::from_ticks(3));
        assert!(levels[14]);
        assert!(levels[16]);
    }
}
