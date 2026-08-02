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
const CLEN0: usize = (PAGE3 << 8) | 0xc6;
const CLIE0: usize = (PAGE3 << 8) | 0xc7;
const CLIF0: usize = (PAGE3 << 8) | 0xe8;
const CLOUT0: usize = (PAGE3 << 8) | 0xd1;
const CLU_MX: [usize; 4] = [
    (PAGE3 << 8) | 0x84,
    (PAGE3 << 8) | 0x85,
    (PAGE3 << 8) | 0x91,
    (PAGE3 << 8) | 0xae,
];
const CLU_FN: [usize; 4] = [
    (PAGE3 << 8) | 0xaf,
    (PAGE3 << 8) | 0xb2,
    (PAGE3 << 8) | 0xb5,
    (PAGE3 << 8) | 0xbe,
];
const CLU_CF: [usize; 4] = [
    (PAGE3 << 8) | 0xb1,
    (PAGE3 << 8) | 0xb3,
    (PAGE3 << 8) | 0xb6,
    (PAGE3 << 8) | 0xbf,
];
const EIE2: usize = 0xf3;
const EIP2: usize = (0x10 << 8) | 0xed;
const EIP2H: usize = (0x10 << 8) | 0xf6;

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
const EIE2_CL0: u8 = 0x10;
const CLU_CF_OUTSEL: u8 = 0x80;
const CLU_CF_OEN: u8 = 0x40;
const CLU_CF_RST: u8 = 0x08;
const CLU_CF_CLKINV: u8 = 0x04;
const CLU_CF_CLKSEL: u8 = 0x03;

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
    clu_input_overrides: [Option<[bool; 2]>; 4],
    clu_ff: [bool; 4],
    clu_lut_outputs: [bool; 4],
    uart_byte_signal: SignalId,
    uart_strobe_signal: SignalId,
    timer0_irq_signal: SignalId,
    timer2_irq_signal: SignalId,
    clu_output_signals: [SignalId; 4],
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

    fn clu_pin(unit: usize, input: usize, selector: u8) -> Option<(usize, u8)> {
        // The EFM8BB52 manual's CLUnMX tables enumerate the external pin
        // choices as selectors 8..15. Internal timer/PWM/serial sources are
        // intentionally left low until their peripheral models are present.
        const A: [[(usize, u8); 8]; 4] = [
            [
                (0, 0),
                (0, 2),
                (0, 4),
                (0, 6),
                (1, 0),
                (1, 2),
                (1, 4),
                (1, 6),
            ],
            [
                (0, 4),
                (0, 5),
                (1, 0),
                (1, 2),
                (1, 4),
                (1, 5),
                (2, 0),
                (2, 2),
            ],
            [
                (0, 0),
                (0, 1),
                (1, 0),
                (1, 1),
                (1, 6),
                (1, 7),
                (2, 0),
                (2, 1),
            ],
            [
                (0, 2),
                (0, 3),
                (0, 6),
                (0, 7),
                (1, 2),
                (1, 3),
                (2, 2),
                (2, 3),
            ],
        ];
        const B: [[(usize, u8); 8]; 4] = [
            [
                (0, 1),
                (0, 3),
                (0, 5),
                (0, 7),
                (1, 1),
                (1, 3),
                (1, 5),
                (1, 7),
            ],
            [
                (0, 6),
                (0, 7),
                (1, 1),
                (1, 3),
                (1, 6),
                (1, 7),
                (2, 1),
                (2, 3),
            ],
            [
                (0, 2),
                (0, 3),
                (1, 2),
                (1, 3),
                (1, 4),
                (1, 5),
                (2, 2),
                (2, 3),
            ],
            [
                (0, 0),
                (0, 1),
                (0, 4),
                (0, 5),
                (1, 0),
                (1, 1),
                (2, 0),
                (2, 1),
            ],
        ];
        let table = if input == 0 { &A } else { &B };
        table
            .get(unit)
            .and_then(|unit_table| unit_table.get(usize::from(selector.saturating_sub(8))))
            .copied()
    }

    fn clu_input(&self, unit: usize, input: usize, lut: &[bool; 4]) -> bool {
        if let Some(override_inputs) = self.clu_input_overrides[unit] {
            return override_inputs[input];
        }
        let selector = if input == 0 {
            self.registers[CLU_MX[unit]] >> 4
        } else {
            self.registers[CLU_MX[unit]] & 0x0f
        };
        match selector {
            0..=3 => lut[usize::from(selector)],
            8..=15 => Self::clu_pin(unit, input, selector).map_or(false, |(port, pin)| {
                self.resolved_port(port) & (1 << pin) != 0
            }),
            _ => false,
        }
    }

    fn clu_enabled(&self, unit: usize) -> bool {
        self.registers[CLEN0] & (1 << unit) != 0
    }

    fn clu_output(&self, unit: usize) -> bool {
        self.registers[CLOUT0] & (1 << unit) != 0
    }

    fn refresh_clu(&mut self, at: SimTime) {
        let previous = [
            self.clu_output(0),
            self.clu_output(1),
            self.clu_output(2),
            self.clu_output(3),
        ];
        let mut lut = self.clu_lut_outputs;
        // A CLU can consume the preceding CLU's output and CLU0 wraps from
        // CLU3. Iterate the ring to a deterministic fixed point so simple
        // cascades settle without a clock-accurate event simulator.
        for _ in 0..4 {
            let old = lut;
            for unit in 0..4 {
                if !self.clu_enabled(unit) {
                    lut[unit] = false;
                    continue;
                }
                let a = self.clu_input(unit, 0, &old);
                let b = self.clu_input(unit, 1, &old);
                let carry = old[if unit == 0 { 3 } else { unit - 1 }];
                let index = usize::from(u8::from(carry) | (u8::from(b) << 1) | (u8::from(a) << 2));
                lut[unit] = self.registers[CLU_FN[unit]] & (1 << index) != 0;
            }
        }
        self.clu_lut_outputs = lut;
        for unit in 0..4 {
            let config = self.registers[CLU_CF[unit]];
            if self.clu_enabled(unit) && config & CLU_CF_OUTSEL == 0 {
                // The functional scheduler treats each refresh as one
                // SYSCLK opportunity for the D flip-flop. CLKSEL and CLKINV
                // remain metadata until timer/clock sources are modelled.
                self.clu_ff[unit] = lut[unit];
            }
            let output = if !self.clu_enabled(unit) {
                false
            } else if config & CLU_CF_OUTSEL != 0 {
                lut[unit]
            } else {
                self.clu_ff[unit]
            };
            self.registers[CLOUT0] =
                (self.registers[CLOUT0] & !(1 << unit)) | (u8::from(output) << unit);
            if output != previous[unit] {
                let rising = 1 << (unit * 2 + 1);
                let falling = 1 << (unit * 2);
                self.registers[CLIF0] |= if output { rising } else { falling };
            }
            self.set_signal(self.clu_output_signals[unit], u64::from(output), 1, at);
        }
    }

    fn write_clu_register(&mut self, address: usize, value: u8, at: SimTime) -> bool {
        if address == CLEN0 {
            self.registers[address] = value & 0x0f;
        } else if address == CLIE0 {
            self.registers[address] = value;
        } else if address == CLIF0 {
            self.registers[address] = value;
        } else if address == CLOUT0 {
            return true;
        } else if let Some(unit) = CLU_MX.iter().position(|register| *register == address) {
            self.registers[address] = value;
            let _ = unit;
        } else if let Some(unit) = CLU_FN.iter().position(|register| *register == address) {
            self.registers[address] = value;
            let _ = unit;
        } else if let Some(unit) = CLU_CF.iter().position(|register| *register == address) {
            if value & CLU_CF_RST != 0 {
                self.clu_ff[unit] = false;
            }
            self.registers[address] =
                value & (CLU_CF_OUTSEL | CLU_CF_OEN | CLU_CF_CLKINV | CLU_CF_CLKSEL);
        } else {
            return false;
        }
        self.refresh_clu(at);
        true
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
        self.clu_input_overrides = [None; 4];
        self.clu_ff = [false; 4];
        self.clu_lut_outputs = [false; 4];
        for signal in [
            self.uart_strobe_signal,
            self.timer0_irq_signal,
            self.timer2_irq_signal,
            self.clu_output_signals[0],
            self.clu_output_signals[1],
            self.clu_output_signals[2],
            self.clu_output_signals[3],
            self.interrupt_signal,
            self.watchdog_reset_signal,
        ] {
            self.set_signal(signal, 0, 1, at);
        }
        for port in 0..4 {
            let _ = self.refresh_port(port, at);
        }
        self.refresh_clu(at);
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
            0xed | 0xf6 if page == 0x10 => (0x10 << 8) | address,
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
            self.registers[EIE2] & EIE2_CL0 != 0
                && self.registers[CLIE0] & self.registers[CLIF0] != 0,
        ];
        let priorities = [IE_ET0, IE_ES0, IE_ET2, EIE2_CL0];
        let mut levels = [false; 8];
        for source in 0..3 {
            if active[source] {
                let high = self.registers[IP] & priorities[source] != 0;
                levels[source + if high { 3 } else { 0 }] = true;
            }
        }
        if active[3] {
            let high =
                self.registers[EIP2] & EIE2_CL0 != 0 || self.registers[EIP2H] & EIE2_CL0 != 0;
            levels[6 + usize::from(high)] = true;
        }
        levels
    }

    fn update_interrupt_signals(&mut self, at: SimTime) {
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
        self.refresh_clu(at);
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

    /// Supplies the resolved A and B logic inputs for one CLU.
    ///
    /// This is a deterministic host oracle for internal timer, PWM, serial,
    /// and analog-derived sources that are not yet modelled. While an
    /// override is present it takes precedence over the CLUnMX pin selection.
    pub fn set_clu_inputs(
        &self,
        clu: u8,
        a: bool,
        b: bool,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        let mut state = self.0.lock().expect("EFM8 lock poisoned");
        let index = usize::from(clu);
        let inputs = state
            .clu_input_overrides
            .get_mut(index)
            .ok_or_else(|| DeviceError::new(format!("EFM8 CLU index {clu} is outside 0..3")))?;
        *inputs = Some([a, b]);
        state.refresh_clu(at);
        state.update_interrupt_signals(at);
        Ok(())
    }

    /// Releases a CLU host-input override and returns to CLUnMX resolution.
    pub fn clear_clu_inputs(&self, clu: u8, at: SimTime) -> Result<(), DeviceError> {
        let mut state = self.0.lock().expect("EFM8 lock poisoned");
        let index = usize::from(clu);
        let inputs = state
            .clu_input_overrides
            .get_mut(index)
            .ok_or_else(|| DeviceError::new(format!("EFM8 CLU index {clu} is outside 0..3")))?;
        *inputs = None;
        state.refresh_clu(at);
        state.update_interrupt_signals(at);
        Ok(())
    }

    /// Returns the current selected output of a CLU.
    pub fn clu_output(&self, clu: u8) -> Result<bool, DeviceError> {
        let state = self.0.lock().expect("EFM8 lock poisoned");
        let index = usize::from(clu);
        if index >= 4 {
            return Err(DeviceError::new(format!(
                "EFM8 CLU index {clu} is outside 0..3"
            )));
        }
        Ok(state.clu_output(index))
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
        let clu_output_signals = [
            hub.declare(
                "board.efm8bb52f32g.clu0.output",
                SignalValue::from_u64(0, 1)?,
                Some("CLU0 selected logic output".to_owned()),
            )?,
            hub.declare(
                "board.efm8bb52f32g.clu1.output",
                SignalValue::from_u64(0, 1)?,
                Some("CLU1 selected logic output".to_owned()),
            )?,
            hub.declare(
                "board.efm8bb52f32g.clu2.output",
                SignalValue::from_u64(0, 1)?,
                Some("CLU2 selected logic output".to_owned()),
            )?,
            hub.declare(
                "board.efm8bb52f32g.clu3.output",
                SignalValue::from_u64(0, 1)?,
                Some("CLU3 selected logic output".to_owned()),
            )?,
        ];
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
            watchdog_epoch: 0,
            watchdog_key: 0,
            watchdog_enabled: true,
            watchdog_reset: false,
            clu_input_overrides: [None; 4],
            clu_ff: [false; 4],
            clu_lut_outputs: [false; 4],
            uart_byte_signal,
            uart_strobe_signal,
            timer0_irq_signal,
            timer2_irq_signal,
            clu_output_signals,
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
        let clu_write = state.write_clu_register(address, value, at);
        if !clu_write && address != CLOUT0 {
            state.registers[address] = value;
        }
        if clu_write {
            // CLU register writes already refresh their combinatorial and
            // synchronous outputs below.
        } else if let Some(port) = Self::port_index(address) {
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
        AccessWidth, CLEN0, CLIE0, CLIF0, CLOUT0, CLU_CF, CLU_FN, EIE2, EIE2_CL0, EIP2,
        Efm8Peripherals, IE, IE_EA, IE_ET0, P0, P0MDOUT, SBUF0, SimTime, TCON, TCON_TR0, TMOD,
        XBR0, XBR0_URT0E, XBR2, XBR2_XBARE,
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
    fn configurable_logic_lut_edges_and_interrupts_are_functional() {
        let hub = super::SignalHub::new();
        let (mut device, handle, _) = Efm8Peripherals::new("efm8.sfr", hub).unwrap();
        // FNSEL=0xc0 is the documented A AND B truth table. Select the LUT
        // output, enable CLU0, and enable both edge flags.
        device
            .write(CLU_FN[0] as u64, AccessWidth::Byte, 0xc0, SimTime::ZERO)
            .unwrap();
        device
            .write(CLU_CF[0] as u64, AccessWidth::Byte, 0x80, SimTime::ZERO)
            .unwrap();
        device
            .write(CLEN0 as u64, AccessWidth::Byte, 1, SimTime::ZERO)
            .unwrap();
        device
            .write(CLIE0 as u64, AccessWidth::Byte, 0x03, SimTime::ZERO)
            .unwrap();
        device
            .write(
                EIE2 as u64,
                AccessWidth::Byte,
                EIE2_CL0.into(),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(IE as u64, AccessWidth::Byte, IE_EA.into(), SimTime::ZERO)
            .unwrap();
        assert_eq!(
            device.read(CLU_FN[0] as u64, AccessWidth::Byte, SimTime::ZERO),
            Ok(0xc0)
        );
        assert_eq!(
            device.read(CLU_CF[0] as u64, AccessWidth::Byte, SimTime::ZERO),
            Ok(0x80)
        );
        assert_eq!(
            device.read(CLEN0 as u64, AccessWidth::Byte, SimTime::ZERO),
            Ok(1)
        );
        handle
            .set_clu_inputs(0, false, true, SimTime::from_ticks(1))
            .unwrap();
        assert!(!handle.clu_output(0).unwrap());
        handle
            .set_clu_inputs(0, true, true, SimTime::from_ticks(2))
            .unwrap();
        assert!(handle.clu_output(0).unwrap());
        assert_eq!(
            device.read(CLOUT0 as u64, AccessWidth::Byte, SimTime::ZERO),
            Ok(1)
        );
        assert_eq!(
            device.read(CLIF0 as u64, AccessWidth::Byte, SimTime::ZERO),
            Ok(0x02)
        );
        assert!(handle.poll(SimTime::from_ticks(2))[6]);

        device
            .write(
                EIP2 as u64,
                AccessWidth::Byte,
                EIE2_CL0.into(),
                SimTime::ZERO,
            )
            .unwrap();
        assert!(handle.poll(SimTime::from_ticks(2))[7]);
        device
            .write(CLIF0 as u64, AccessWidth::Byte, 0, SimTime::ZERO)
            .unwrap();
        handle
            .set_clu_inputs(0, false, true, SimTime::from_ticks(3))
            .unwrap();
        assert_eq!(
            device.read(CLIF0 as u64, AccessWidth::Byte, SimTime::ZERO),
            Ok(0x01)
        );
    }
}
