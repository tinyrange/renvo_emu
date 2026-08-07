use super::{GpioHandle, GpioState, SignalHub, refresh_gpio, vendor_gpio};
use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use remu_signals::{Logic, SignalId, SignalValue};
use std::sync::{Arc, Mutex};

const DATA_BYTES: usize = 0x2000;
const INTCON: usize = 0x00b;
const PORT_BASE: usize = 0x00c;
const TRIS_BASE: usize = 0x012;
const LAT_BASE: usize = 0x018;
const RC1REG: usize = 0x119;
const TX1REG: usize = 0x11a;
const RC1STA: usize = 0x11d;
const TX1STA: usize = 0x11e;
const TMR1L: usize = 0x20c;
const TMR1H: usize = 0x20d;
const T1CON: usize = 0x20e;
const TMR0L: usize = 0x59c;
const TMR0H: usize = 0x59d;
const T0CON0: usize = 0x59e;
const PIR0: usize = 0x70c;
const PIR3: usize = 0x70f;
const PIR4: usize = 0x710;
const PIE0: usize = 0x716;
const PIE3: usize = 0x719;
const PIE4: usize = 0x71a;
const WDTCON0: usize = 0x80c;
const OSCSTAT: usize = 0x890;
// PIC16F15376 data-sheet register summary, bank 17 (DS40001866A §4.3).
const CLKRCON: usize = 0x895;
const CLKRCLK: usize = 0x896;
const PPSLOCK: usize = 0x1e8f;
const PPS_OUTPUT_BASES: [usize; 5] = [0x1f10, 0x1f18, 0x1f20, 0x1f28, 0x1f30];
const ANSEL: [usize; 5] = [0x1f38, 0x1f43, 0x1f4e, 0x1f59, 0x1f64];

const PORT_WIDTHS: [u8; 5] = [8, 8, 8, 8, 4];
const PORT_MASKS: [u8; 5] = [0xff, 0xff, 0xff, 0xff, 0x0f];
const INTCON_GIE: u8 = 1 << 7;
const INTCON_PEIE: u8 = 1 << 6;
const TMR0IF: u8 = 1 << 5;
const TMR1IF: u8 = 1;
const TX1IF: u8 = 1 << 4;
const RC1IF: u8 = 1 << 5;
const TXEN: u8 = 1 << 5;
const SPEN: u8 = 1 << 7;
const CLKRCON_ENABLE: u8 = 1 << 7;
const CLKRCON_WRITABLE_MASK: u8 = 0x9f;
const CLKRCLK_WRITABLE_MASK: u8 = 0x0f;

struct Pic16State {
    registers: Vec<u8>,
    ports: [Arc<Mutex<GpioState>>; 5],
    port_signals: [Vec<SignalId>; 5],
    hub: SignalHub,
    uart: Vec<u8>,
    timer0_epoch: u64,
    timer1_epoch: u64,
    watchdog_epoch: u64,
    clock_reference_epoch: u64,
    watchdog_reset: bool,
    uart_byte_signal: SignalId,
    uart_strobe_signal: SignalId,
    timer0_irq_signal: SignalId,
    timer1_irq_signal: SignalId,
    interrupt_signal: SignalId,
    watchdog_reset_signal: SignalId,
    clock_reference_signal: SignalId,
}

impl Pic16State {
    fn set_signal(&self, signal: SignalId, value: u64, width: u16, at: SimTime) {
        self.hub
            .set(
                signal,
                SignalValue::from_u64(value, width).expect("fixed PIC16 signal width is valid"),
                at,
            )
            .expect("PIC16 signal identity is fixed at construction");
    }

    /// Publishes the functional CLKR waveform.
    ///
    /// The emulator's timeline is an abstract instruction/action tick, so the
    /// oscillator sources use deterministic relative periods rather than
    /// claiming a silicon clock frequency. FOSC/HFINTOSC use one base period;
    /// the low- and medium-frequency sources use fixed coarse ratios. NCO/CLC
    /// sources are retained in CLKRCLK but remain low until those generators
    /// are modelled.
    fn refresh_clock_reference(&self, at: SimTime) {
        let control = self.registers[CLKRCON];
        let source = self.registers[CLKRCLK] & CLKRCLK_WRITABLE_MASK;
        let output = if control & CLKRCON_ENABLE == 0 {
            false
        } else if let Some(source_period) = match source {
            0 | 1 => Some(1_u64), // FOSC, HFINTOSC
            2 => Some(512_u64),   // LFINTOSC
            3 => Some(32_u64),    // MFINTOSC (500 kHz)
            4 => Some(512_u64),   // MFINTOSC (31.25 kHz)
            5 => Some(1024_u64),  // SOSC
            6..=10 => None,       // NCO1/CLC1..4: not modelled in this slice
            _ => None,            // reserved source encodings
        } {
            let divider = 1_u64 << u32::from(control & 0x07);
            // Four sub-ticks per base period let the functional model express
            // the documented 25/50/75% duty-cycle choices.
            let period = 4_u64
                .saturating_mul(source_period)
                .saturating_mul(divider)
                .max(1);
            let duty = u64::from((control >> 3) & 0x03);
            let high_ticks = period.saturating_mul(duty) / 4;
            let phase = at.ticks().saturating_sub(self.clock_reference_epoch) % period;
            phase < high_ticks
        } else {
            false
        };
        self.set_signal(self.clock_reference_signal, u64::from(output), 1, at);
    }

    fn resolved_port(&self, port: usize) -> u8 {
        self.ports[port]
            .lock()
            .expect("PIC16 GPIO lock poisoned")
            .nets
            .iter()
            .enumerate()
            .fold(0_u8, |value, (pin, net)| {
                value | (u8::from(net.resolved() == Logic::One) << pin)
            })
            & PORT_MASKS[port]
    }

    fn refresh_port(&mut self, port: usize, at: SimTime) -> Result<(), DeviceError> {
        let direction = (!self.registers[TRIS_BASE + port]) & PORT_MASKS[port];
        let output = self.registers[LAT_BASE + port] & PORT_MASKS[port];
        {
            let mut gpio = self.ports[port].lock().expect("PIC16 GPIO lock poisoned");
            gpio.direction = u32::from(direction);
            gpio.output = u32::from(output);
        }
        refresh_gpio(
            &self.ports[port],
            &self.port_signals[port],
            &self.hub,
            PORT_WIDTHS[port],
            at,
        )?;
        let digital = !self.registers[ANSEL[port]];
        self.registers[PORT_BASE + port] = self.resolved_port(port) & digital & PORT_MASKS[port];
        Ok(())
    }

    fn reset_registers(&mut self, at: SimTime) {
        self.registers.fill(0);
        for port in 0..5 {
            self.registers[TRIS_BASE + port] = PORT_MASKS[port];
            self.registers[ANSEL[port]] = PORT_MASKS[port];
        }
        self.registers[PIR3] = TX1IF;
        self.registers[TX1STA] = 1 << 1; // TRMT
        self.registers[OSCSTAT] = 1 << 6; // internal HF oscillator ready
        // CLKRDC1 resets high on this family, yielding the documented 50%
        // default duty selection while the module itself remains disabled.
        self.registers[CLKRCON] = 0x08;
        self.registers[PPSLOCK] = 1;
        self.uart.clear();
        self.timer0_epoch = at.ticks();
        self.timer1_epoch = at.ticks();
        self.watchdog_epoch = at.ticks();
        self.clock_reference_epoch = at.ticks();
        self.watchdog_reset = false;
        self.set_signal(self.uart_strobe_signal, 0, 1, at);
        self.set_signal(self.timer0_irq_signal, 0, 1, at);
        self.set_signal(self.timer1_irq_signal, 0, 1, at);
        self.set_signal(self.interrupt_signal, 0, 1, at);
        self.set_signal(self.watchdog_reset_signal, 0, 1, at);
        self.set_signal(self.clock_reference_signal, 0, 1, at);
        for port in 0..5 {
            let _ = self.refresh_port(port, at);
        }
    }

    fn interrupt_pending(&self) -> bool {
        let peripheral = self.registers[INTCON] & INTCON_PEIE != 0
            && ((self.registers[PIR0] & self.registers[PIE0] & TMR0IF != 0)
                || (self.registers[PIR4] & self.registers[PIE4] & TMR1IF != 0)
                || (self.registers[PIR3] & self.registers[PIE3] & (TX1IF | RC1IF) != 0));
        self.registers[INTCON] & INTCON_GIE != 0 && peripheral
    }
}

/// Host-facing PIC16F15376 peripheral state.
#[derive(Clone)]
pub struct Pic16PeripheralsHandle(Arc<Mutex<Pic16State>>);

impl Pic16PeripheralsHandle {
    /// Captured EUSART1 transmit bytes.
    pub fn uart_bytes(&self) -> Vec<u8> {
        self.0
            .lock()
            .expect("PIC16 peripheral lock poisoned")
            .uart
            .clone()
    }

    /// Advances functional timers and returns the combined interrupt request.
    pub fn poll(&self, now: SimTime) -> bool {
        let mut state = self.0.lock().expect("PIC16 peripheral lock poisoned");
        for port in 0..5 {
            let _ = state.refresh_port(port, now);
        }
        state.refresh_clock_reference(now);
        if state.registers[T0CON0] & 0x80 != 0 {
            let period = u64::from(state.registers[TMR0H]).saturating_add(1).max(1);
            let elapsed = now.ticks().saturating_sub(state.timer0_epoch);
            state.registers[TMR0L] = (elapsed % period) as u8;
            if elapsed >= period {
                state.timer0_epoch = now.ticks();
                state.registers[PIR0] |= TMR0IF;
                state.set_signal(state.timer0_irq_signal, 1, 1, now);
            }
        }
        if state.registers[T1CON] & 1 != 0 {
            let initial =
                u16::from(state.registers[TMR1L]) | (u16::from(state.registers[TMR1H]) << 8);
            let elapsed = now.ticks().saturating_sub(state.timer1_epoch);
            let total = u64::from(initial).saturating_add(elapsed);
            let value = total as u16;
            state.registers[TMR1L] = value as u8;
            state.registers[TMR1H] = (value >> 8) as u8;
            state.timer1_epoch = now.ticks();
            if total > u64::from(u16::MAX) {
                state.registers[PIR4] |= TMR1IF;
                state.set_signal(state.timer1_irq_signal, 1, 1, now);
            }
        }
        if state.registers[WDTCON0] & 1 != 0 {
            let exponent = u32::from((state.registers[WDTCON0] >> 1) & 0x1f).min(20);
            let period = 32_u64.checked_shl(exponent).unwrap_or(u64::MAX);
            if now.ticks().saturating_sub(state.watchdog_epoch) >= period {
                state.watchdog_reset = true;
                state.set_signal(state.watchdog_reset_signal, 1, 1, now);
            }
        }
        let pending = state.interrupt_pending();
        state.set_signal(state.interrupt_signal, u64::from(pending), 1, now);
        pending
    }

    /// Restarts the functional watchdog interval after CLRWDT.
    pub fn clear_watchdog(&self, now: SimTime) {
        self.0
            .lock()
            .expect("PIC16 peripheral lock poisoned")
            .watchdog_epoch = now.ticks();
    }

    /// Consumes a watchdog reset request.
    pub fn take_watchdog_reset(&self) -> bool {
        std::mem::take(
            &mut self
                .0
                .lock()
                .expect("PIC16 peripheral lock poisoned")
                .watchdog_reset,
        )
    }
}

/// PIC16F15376 banked data and peripheral window.
pub struct Pic16Peripherals {
    name: String,
    state: Arc<Mutex<Pic16State>>,
}

impl Pic16Peripherals {
    /// Creates the documented peripheral slice and five package port handles.
    pub fn new(
        name: impl Into<String>,
        hub: SignalHub,
    ) -> Result<(Self, Pic16PeripheralsHandle, [GpioHandle; 5]), remu_signals::SignalError> {
        let (porta, signals_a, handle_a) = vendor_gpio(8, "board.pic16f15376.porta", &hub)?;
        let (portb, signals_b, handle_b) = vendor_gpio(8, "board.pic16f15376.portb", &hub)?;
        let (portc, signals_c, handle_c) = vendor_gpio(8, "board.pic16f15376.portc", &hub)?;
        let (portd, signals_d, handle_d) = vendor_gpio(8, "board.pic16f15376.portd", &hub)?;
        let (porte, signals_e, handle_e) = vendor_gpio(4, "board.pic16f15376.porte", &hub)?;
        let uart_byte_signal = hub.declare(
            "board.pic16f15376.eusart1.tx_byte",
            SignalValue::from_u64(0, 8)?,
            Some("last byte written to EUSART1 TXREG".to_owned()),
        )?;
        let uart_strobe_signal = hub.declare(
            "board.pic16f15376.eusart1.tx_strobe",
            SignalValue::from_u64(0, 1)?,
            Some("toggles for each EUSART1 byte".to_owned()),
        )?;
        let timer0_irq_signal = hub.declare(
            "board.pic16f15376.timer0.irq",
            SignalValue::from_u64(0, 1)?,
            Some("functional Timer0 interrupt flag".to_owned()),
        )?;
        let timer1_irq_signal = hub.declare(
            "board.pic16f15376.timer1.irq",
            SignalValue::from_u64(0, 1)?,
            Some("functional Timer1 interrupt flag".to_owned()),
        )?;
        let interrupt_signal = hub.declare(
            "board.pic16f15376.interrupt.request",
            SignalValue::from_u64(0, 1)?,
            Some("combined enabled peripheral interrupt request".to_owned()),
        )?;
        let watchdog_reset_signal = hub.declare(
            "board.pic16f15376.watchdog.reset",
            SignalValue::from_u64(0, 1)?,
            Some("functional watchdog reset request".to_owned()),
        )?;
        let clock_reference_signal = hub.declare(
            "board.pic16f15376.clkr",
            SignalValue::from_u64(0, 1)?,
            Some("functional PIC16F15376 CLKR reference-clock output".to_owned()),
        )?;
        let state = Arc::new(Mutex::new(Pic16State {
            registers: vec![0; DATA_BYTES],
            ports: [porta, portb, portc, portd, porte],
            port_signals: [signals_a, signals_b, signals_c, signals_d, signals_e],
            hub,
            uart: Vec::new(),
            timer0_epoch: 0,
            timer1_epoch: 0,
            watchdog_epoch: 0,
            clock_reference_epoch: 0,
            watchdog_reset: false,
            uart_byte_signal,
            uart_strobe_signal,
            timer0_irq_signal,
            timer1_irq_signal,
            interrupt_signal,
            watchdog_reset_signal,
            clock_reference_signal,
        }));
        state
            .lock()
            .expect("new PIC16 peripheral lock poisoned")
            .reset_registers(SimTime::ZERO);
        Ok((
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Pic16PeripheralsHandle(state),
            [handle_a, handle_b, handle_c, handle_d, handle_e],
        ))
    }

    fn canonical_offset(offset: usize) -> usize {
        if offset & 0x7f >= 0x70 {
            offset & 0x7f
        } else {
            offset
        }
    }

    fn port_for(address: usize, bases: &[usize]) -> Option<usize> {
        bases
            .iter()
            .position(|base| (*base..*base + 5).contains(&address))
            .or_else(|| {
                if (PORT_BASE..PORT_BASE + 5).contains(&address) {
                    Some(address - PORT_BASE)
                } else if (TRIS_BASE..TRIS_BASE + 5).contains(&address) {
                    Some(address - TRIS_BASE)
                } else if (LAT_BASE..LAT_BASE + 5).contains(&address) {
                    Some(address - LAT_BASE)
                } else {
                    None
                }
            })
    }
}

impl Device for Pic16Peripherals {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Byte {
            return Err(DeviceError::new("PIC16 data space requires byte accesses"));
        }
        let raw = usize::try_from(offset).map_err(|_| DeviceError::new("PIC16 offset overflow"))?;
        let address = Self::canonical_offset(raw);
        let mut state = self.state.lock().expect("PIC16 peripheral lock poisoned");
        if (PORT_BASE..PORT_BASE + 5).contains(&address) {
            state.refresh_port(address - PORT_BASE, at)?;
        }
        let value = match address {
            OSCSTAT => state.registers[address] | (1 << 6),
            CLKRCON => state.registers[address] & CLKRCON_WRITABLE_MASK,
            CLKRCLK => state.registers[address] & CLKRCLK_WRITABLE_MASK,
            TX1STA => state.registers[address] | (1 << 1),
            RC1REG => {
                state.registers[PIR3] &= !RC1IF;
                state.registers[address]
            }
            _ => *state.registers.get(address).ok_or_else(|| {
                DeviceError::new(format!("PIC16 read outside data space: {raw:#x}"))
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
            return Err(DeviceError::new("PIC16 data space requires byte accesses"));
        }
        let raw = usize::try_from(offset).map_err(|_| DeviceError::new("PIC16 offset overflow"))?;
        let address = Self::canonical_offset(raw);
        let value = value as u8;
        let mut state = self.state.lock().expect("PIC16 peripheral lock poisoned");
        if !(address < DATA_BYTES) {
            return Err(DeviceError::new(format!(
                "PIC16 write outside data space: {raw:#x}"
            )));
        }
        match address {
            PORT_BASE..=0x010 => {
                let port = address - PORT_BASE;
                state.registers[LAT_BASE + port] = value & PORT_MASKS[port];
                state.refresh_port(port, at)?;
            }
            TRIS_BASE..=0x016 => {
                let port = address - TRIS_BASE;
                state.registers[address] = value & PORT_MASKS[port];
                state.refresh_port(port, at)?;
            }
            LAT_BASE..=0x01c => {
                let port = address - LAT_BASE;
                state.registers[address] = value & PORT_MASKS[port];
                state.refresh_port(port, at)?;
            }
            TX1REG => {
                state.registers[address] = value;
                state.registers[PIR3] |= TX1IF;
                if state.registers[RC1STA] & SPEN != 0 && state.registers[TX1STA] & TXEN != 0 {
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
            }
            PIR0 => {
                state.registers[address] = value;
                state.set_signal(
                    state.timer0_irq_signal,
                    u64::from(value & TMR0IF != 0),
                    1,
                    at,
                );
            }
            PIR4 => {
                state.registers[address] = value;
                state.set_signal(
                    state.timer1_irq_signal,
                    u64::from(value & TMR1IF != 0),
                    1,
                    at,
                );
            }
            T0CON0 => {
                if state.registers[address] & 0x80 == 0 && value & 0x80 != 0 {
                    state.timer0_epoch = at.ticks();
                }
                state.registers[address] = value;
            }
            T1CON => {
                if state.registers[address] & 1 == 0 && value & 1 != 0 {
                    state.timer1_epoch = at.ticks();
                }
                state.registers[address] = value;
            }
            WDTCON0 => {
                state.registers[address] = value & 0x3f;
                state.watchdog_epoch = at.ticks();
            }
            CLKRCON => {
                state.registers[address] = value & CLKRCON_WRITABLE_MASK;
                state.clock_reference_epoch = at.ticks();
                state.refresh_clock_reference(at);
            }
            CLKRCLK => {
                state.registers[address] = value & CLKRCLK_WRITABLE_MASK;
                state.clock_reference_epoch = at.ticks();
                state.refresh_clock_reference(at);
            }
            _ => {
                state.registers[address] = value;
                if let Some(port) = Self::port_for(address, &ANSEL) {
                    state.refresh_port(port, at)?;
                }
                // PPS output registers are retained verbatim so firmware can read them back.
                let _is_output_pps = PPS_OUTPUT_BASES
                    .iter()
                    .any(|base| (*base..*base + 8).contains(&address));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state
            .lock()
            .expect("PIC16 peripheral lock poisoned")
            .reset_registers(SimTime::ZERO);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpio_uart_timer_and_watchdog_slice_is_functional() {
        let hub = SignalHub::new();
        let (mut device, handle, ports) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
        device
            .write(ANSEL[0] as u64, AccessWidth::Byte, 0, SimTime::ZERO)
            .unwrap();
        device
            .write(TRIS_BASE as u64, AccessWidth::Byte, 0xfe, SimTime::ZERO)
            .unwrap();
        device
            .write(LAT_BASE as u64, AccessWidth::Byte, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(ports[0].output() & 1, 1);

        device
            .write(RC1STA as u64, AccessWidth::Byte, SPEN.into(), SimTime::ZERO)
            .unwrap();
        device
            .write(TX1STA as u64, AccessWidth::Byte, TXEN.into(), SimTime::ZERO)
            .unwrap();
        device
            .write(TX1REG as u64, AccessWidth::Byte, b'P'.into(), SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.uart_bytes(), b"P");

        device
            .write(TMR0H as u64, AccessWidth::Byte, 3, SimTime::ZERO)
            .unwrap();
        device
            .write(PIE0 as u64, AccessWidth::Byte, TMR0IF.into(), SimTime::ZERO)
            .unwrap();
        device
            .write(
                INTCON as u64,
                AccessWidth::Byte,
                (INTCON_GIE | INTCON_PEIE).into(),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(T0CON0 as u64, AccessWidth::Byte, 0x80, SimTime::ZERO)
            .unwrap();
        assert!(handle.poll(SimTime::from_ticks(4)));
    }

    #[test]
    fn clock_reference_masks_registers_and_emits_deterministic_output() {
        let hub = SignalHub::new();
        let (mut device, handle, _) =
            Pic16Peripherals::new("pic16f15376.data", hub.clone()).unwrap();
        let clkr = hub
            .with_registry(|registry| registry.find("board.pic16f15376.clkr"))
            .expect("CLKR signal is declared");

        // The data sheet leaves POR duty-cycle bits device-dependent, while
        // CLKRDC1 is reset for the documented 50% default. Keep that default
        // deterministic and expose only implemented bits on readback.
        assert_eq!(
            device
                .read(CLKRCON as u64, AccessWidth::Byte, SimTime::ZERO)
                .unwrap(),
            0x08
        );
        assert_eq!(
            device
                .read(CLKRCLK as u64, AccessWidth::Byte, SimTime::ZERO)
                .unwrap(),
            0
        );
        device
            .write(CLKRCON as u64, AccessWidth::Byte, 0xff, SimTime::ZERO)
            .unwrap();
        device
            .write(CLKRCLK as u64, AccessWidth::Byte, 0xff, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            device
                .read(CLKRCON as u64, AccessWidth::Byte, SimTime::ZERO)
                .unwrap(),
            CLKRCON_WRITABLE_MASK.into()
        );
        assert_eq!(
            device
                .read(CLKRCLK as u64, AccessWidth::Byte, SimTime::ZERO)
                .unwrap(),
            CLKRCLK_WRITABLE_MASK.into()
        );
        // NCO/CLC and reserved source encodings are retained but do not drive
        // a waveform until those upstream generators are modelled.
        handle.poll(SimTime::ZERO);
        assert_eq!(
            hub.with_registry(|registry| registry.value(clkr).and_then(|value| value.bit(0))),
            Some(Logic::Zero)
        );
        hub.drain_changes();

        // FOSC, /2, 50% duty: the functional four-subtick period is eight
        // abstract ticks, with transitions at the half-period boundaries.
        device
            .write(CLKRCLK as u64, AccessWidth::Byte, 0, SimTime::ZERO)
            .unwrap();
        device
            .write(CLKRCON as u64, AccessWidth::Byte, 0x91, SimTime::ZERO)
            .unwrap();
        let changes = hub.drain_changes();
        assert_eq!(changes.last().map(|change| change.signal), Some(clkr));
        assert_eq!(
            changes.last().and_then(|change| change.value.bit(0)),
            Some(Logic::One)
        );

        handle.poll(SimTime::from_ticks(3));
        assert!(hub.drain_changes().is_empty());
        handle.poll(SimTime::from_ticks(4));
        let falling = hub.drain_changes();
        assert_eq!(
            falling.last().and_then(|change| change.value.bit(0)),
            Some(Logic::Zero)
        );
        handle.poll(SimTime::from_ticks(8));
        let rising = hub.drain_changes();
        assert_eq!(
            rising.last().and_then(|change| change.value.bit(0)),
            Some(Logic::One)
        );

        device
            .write(CLKRCON as u64, AccessWidth::Byte, 0, SimTime::from_ticks(9))
            .unwrap();
        let disabled = hub.drain_changes();
        assert_eq!(
            disabled.last().and_then(|change| change.value.bit(0)),
            Some(Logic::Zero)
        );
    }
}
