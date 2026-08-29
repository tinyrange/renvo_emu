use super::{GpioHandle, GpioState, SignalHub, UartHandle, refresh_gpio, vendor_gpio_wide};
use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use remu_signals::{Logic, SignalId};
use std::sync::{Arc, Mutex};

/// nRF52840 P0/P1 digital GPIO register slice.
pub struct Nrf52840Gpio {
    name: String,
    state: Arc<Mutex<GpioState>>,
    signals: Vec<SignalId>,
    hub: SignalHub,
    pin_cnf: [u32; 48],
}

impl Nrf52840Gpio {
    /// Constructs the combined 48-pin GPIO peripheral and host pin handle.
    pub fn new(
        name: impl Into<String>,
        path: &str,
        hub: SignalHub,
    ) -> Result<(Self, GpioHandle), remu_signals::SignalError> {
        let (state, signals, handle) = vendor_gpio_wide(48, path, &hub)?;
        Ok((
            Self {
                name: name.into(),
                state,
                signals,
                hub,
                pin_cnf: [2; 48],
            },
            handle,
        ))
    }

    fn decode_port(offset: u64) -> Option<(usize, u64)> {
        match offset {
            0x500..=0x77f => Some((0, offset - 0x500)),
            0x800..=0xa7f => Some((1, offset - 0x800)),
            _ => None,
        }
    }

    fn port_mask(port: usize) -> u32 {
        if port == 0 { u32::MAX } else { 0xffff }
    }

    fn resolved_input(&self, port: usize) -> u32 {
        let state = self.state.lock().expect("GPIO lock poisoned");
        let start = port * 32;
        let end = (start + 32).min(48);
        (start..end).fold(0_u32, |value, pin| {
            value | (u32::from(state.nets[pin].resolved() == Logic::One) << (pin - start))
        })
    }

    fn update_port(
        &mut self,
        port: usize,
        register: u64,
        value: u32,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        let mask = Self::port_mask(port);
        let value = value & mask;
        {
            let mut state = self.state.lock().expect("GPIO lock poisoned");
            match (port, register) {
                (0, 0x04) => state.output = value,
                (0, 0x08) => state.output |= value,
                (0, 0x0c) => state.output &= !value,
                (0, 0x14) => state.direction = value,
                (0, 0x18) => state.direction |= value,
                (0, 0x1c) => state.direction &= !value,
                (1, 0x04) => state.output_high = value,
                (1, 0x08) => state.output_high |= value,
                (1, 0x0c) => state.output_high &= !value,
                (1, 0x14) => state.direction_high = value,
                (1, 0x18) => state.direction_high |= value,
                (1, 0x1c) => state.direction_high &= !value,
                (_, 0x200..=0x27c) if register & 3 == 0 => {
                    let local =
                        usize::try_from((register - 0x200) / 4).expect("GPIO pin index fits usize");
                    let pin = port * 32 + local;
                    if pin >= self.pin_cnf.len() {
                        return Err(DeviceError::new("nRF52840 GPIO pin is outside the package"));
                    }
                    self.pin_cnf[pin] = value & 0x0007_030f;
                    let output_enabled = value & 1 != 0;
                    let bit = 1_u32 << local;
                    let direction = if port == 0 {
                        &mut state.direction
                    } else {
                        &mut state.direction_high
                    };
                    if output_enabled {
                        *direction |= bit;
                    } else {
                        *direction &= !bit;
                    }
                }
                _ => {
                    return Err(DeviceError::new(format!(
                        "unmodeled nRF52840 GPIO write at {register:#x}"
                    )));
                }
            }
        }
        refresh_gpio(&self.state, &self.signals, &self.hub, 48, at)
    }
}

impl Device for Nrf52840Gpio {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("nRF52840 GPIO requires word accesses"));
        }
        let Some((port, register)) = Self::decode_port(offset) else {
            return Err(DeviceError::new(format!(
                "unmodeled nRF52840 GPIO read at {offset:#x}"
            )));
        };
        let state = self.state.lock().expect("GPIO lock poisoned");
        let (direction, output) = if port == 0 {
            (state.direction, state.output)
        } else {
            (state.direction_high, state.output_high)
        };
        drop(state);
        let value = match register {
            0x04 | 0x08 | 0x0c => output,
            0x10 => self.resolved_input(port),
            0x14 | 0x18 | 0x1c => direction,
            0x200..=0x27c if register & 3 == 0 => {
                let pin = port * 32
                    + usize::try_from((register - 0x200) / 4).expect("GPIO pin index fits usize");
                *self.pin_cnf.get(pin).unwrap_or(&0)
            }
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled nRF52840 GPIO read at {register:#x}"
                )));
            }
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
        if width != AccessWidth::Word {
            return Err(DeviceError::new("nRF52840 GPIO requires word accesses"));
        }
        let Some((port, register)) = Self::decode_port(offset) else {
            return Err(DeviceError::new(format!(
                "unmodeled nRF52840 GPIO write at {offset:#x}"
            )));
        };
        self.update_port(port, register, value as u32, at)
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.pin_cnf.fill(2);
        let mut state = self.state.lock().expect("GPIO lock poisoned");
        state.direction = 0;
        state.output = 0;
        state.direction_high = 0;
        state.output_high = 0;
    }
}

/// nRF52840 legacy UART task/event register slice at the UART0/UARTE0 base.
pub struct Nrf52840Uart {
    name: String,
    handle: UartHandle,
    enabled: bool,
    tx_running: bool,
    rx_running: bool,
    events_rxdrdy: bool,
    events_txdrdy: bool,
    baudrate: u32,
    config: u32,
    psel: [u32; 4],
}

impl Nrf52840Uart {
    /// Constructs UART0 and its host transport handle.
    pub fn new(name: impl Into<String>) -> (Self, UartHandle) {
        let handle = UartHandle::default();
        (
            Self {
                name: name.into(),
                handle: handle.clone(),
                enabled: false,
                tx_running: false,
                rx_running: false,
                events_rxdrdy: false,
                events_txdrdy: false,
                baudrate: 0x01d7_e000,
                config: 0,
                psel: [0xffff_ffff; 4],
            },
            handle,
        )
    }
}

impl Device for Nrf52840Uart {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("nRF52840 UART requires word accesses"));
        }
        let value = match offset {
            0x108 => u32::from(self.events_rxdrdy),
            0x11c => u32::from(self.events_txdrdy),
            0x518 if self.enabled && self.rx_running => {
                let byte = self.handle.receive().unwrap_or(0);
                self.events_rxdrdy = self.handle.rx_len() != 0;
                u32::from(byte)
            }
            0x500 => {
                if self.enabled {
                    4
                } else {
                    0
                }
            }
            0x508 => self.psel[0],
            0x50c => self.psel[1],
            0x510 => self.psel[2],
            0x514 => self.psel[3],
            0x524 => self.baudrate,
            0x56c => self.config,
            _ => 0,
        };
        Ok(u64::from(value))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("nRF52840 UART requires word accesses"));
        }
        let value = value as u32;
        match offset {
            0x000 if value & 1 != 0 => {
                self.rx_running = self.enabled;
                self.events_rxdrdy = self.rx_running && self.handle.rx_len() != 0;
            }
            0x004 if value & 1 != 0 => self.rx_running = false,
            0x008 if value & 1 != 0 => {
                self.tx_running = self.enabled;
                self.events_txdrdy = false;
            }
            0x00c if value & 1 != 0 => self.tx_running = false,
            0x108 => self.events_rxdrdy = value & 1 != 0,
            0x11c => self.events_txdrdy = value & 1 != 0,
            0x500 => {
                self.enabled = value == 4;
                if !self.enabled {
                    self.tx_running = false;
                    self.rx_running = false;
                }
            }
            0x508 => self.psel[0] = value,
            0x50c => self.psel[1] = value,
            0x510 => self.psel[2] = value,
            0x514 => self.psel[3] = value,
            0x51c if self.enabled && self.tx_running => {
                self.handle.transmit(&value.to_le_bytes()[..1]);
                self.events_txdrdy = true;
            }
            0x524 => self.baudrate = value,
            0x56c => self.config = value & 0x0000_010f,
            _ => {}
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.handle.clear();
        self.enabled = false;
        self.tx_running = false;
        self.rx_running = false;
        self.events_rxdrdy = false;
        self.events_txdrdy = false;
        self.baudrate = 0x01d7_e000;
        self.config = 0;
        self.psel = [0xffff_ffff; 4];
    }
}

#[derive(Default)]
struct TimerState {
    enabled: bool,
    started: u64,
    base: u32,
    cc: [u32; 4],
    event0: bool,
    compare0_fired: bool,
    interrupt0: bool,
    shorts: u32,
    prescaler: u8,
    mode: u8,
    bitmode: u8,
}

/// Host handle used by the machine to advance TIMER0 and route COMPARE0.
#[derive(Clone)]
pub struct Nrf52840TimerHandle(Arc<Mutex<TimerState>>);

impl Nrf52840TimerHandle {
    fn counter(state: &TimerState, now: SimTime) -> u32 {
        let value = if !state.enabled || state.mode != 0 {
            state.base
        } else {
            state
                .base
                .wrapping_add((now.ticks().saturating_sub(state.started) >> state.prescaler) as u32)
        };
        value & [0xffff, 0xff, 0x00ff_ffff, u32::MAX][usize::from(state.bitmode)]
    }

    /// Advances the functional timer and reports the COMPARE0 interrupt level.
    pub fn poll(&self, now: SimTime) -> bool {
        let mut state = self.0.lock().expect("nRF TIMER lock poisoned");
        let counter = Self::counter(&state, now);
        if state.enabled && !state.compare0_fired && counter >= state.cc[0] {
            state.event0 = true;
            state.compare0_fired = true;
            if state.shorts & 1 != 0 {
                state.base = 0;
                state.started = now.ticks();
                state.compare0_fired = false;
            }
            if state.shorts & (1 << 8) != 0 {
                state.base = counter;
                state.enabled = false;
            }
        }
        state.event0 && state.interrupt0
    }
}

/// nRF52840 TIMER0 task/event/counter slice.
pub struct Nrf52840Timer {
    name: String,
    state: Arc<Mutex<TimerState>>,
}

impl Nrf52840Timer {
    /// Constructs TIMER0 and its interrupt handle.
    pub fn new(name: impl Into<String>) -> (Self, Nrf52840TimerHandle) {
        let state = Arc::new(Mutex::new(TimerState::default()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Nrf52840TimerHandle(state),
        )
    }
}

impl Device for Nrf52840Timer {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("nRF52840 TIMER requires word accesses"));
        }
        let state = self.state.lock().expect("nRF TIMER lock poisoned");
        let value = match offset {
            0x140 => u32::from(state.event0),
            0x200 => state.shorts,
            0x304 | 0x308 => u32::from(state.interrupt0) << 16,
            0x504 => u32::from(state.mode),
            0x508 => u32::from(state.bitmode),
            0x510 => u32::from(state.prescaler),
            0x540..=0x54c if offset & 3 == 0 => {
                state.cc[usize::try_from((offset - 0x540) / 4).expect("CC index fits")]
            }
            _ => 0,
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
        if width != AccessWidth::Word {
            return Err(DeviceError::new("nRF52840 TIMER requires word accesses"));
        }
        let value = value as u32;
        let mut state = self.state.lock().expect("nRF TIMER lock poisoned");
        match offset {
            0x000 if value & 1 != 0 => {
                state.enabled = true;
                state.started = at.ticks();
                state.compare0_fired = state.base >= state.cc[0];
            }
            0x004 if value & 1 != 0 => {
                state.base = Nrf52840TimerHandle::counter(&state, at);
                state.enabled = false;
            }
            0x008 if value & 1 != 0 && state.enabled && state.mode == 1 => {
                state.base = state.base.wrapping_add(1)
            }
            0x00c if value & 1 != 0 => {
                state.base = 0;
                state.started = at.ticks();
                state.compare0_fired = false;
            }
            0x040..=0x04c if offset & 3 == 0 && value & 1 != 0 => {
                let index = usize::try_from((offset - 0x040) / 4).expect("CC index fits");
                state.cc[index] = Nrf52840TimerHandle::counter(&state, at);
            }
            0x140 => state.event0 = value & 1 != 0,
            0x200 => state.shorts = value & 0x0000_0f0f,
            0x304 => state.interrupt0 |= value & (1 << 16) != 0,
            0x308 => state.interrupt0 &= value & (1 << 16) == 0,
            0x504 => state.mode = (value & 1) as u8,
            0x508 => state.bitmode = (value & 3) as u8,
            0x510 => state.prescaler = (value & 0x0f) as u8,
            0x540..=0x54c if offset & 3 == 0 => {
                let index = usize::try_from((offset - 0x540) / 4).expect("CC index fits");
                state.cc[index] = value;
                if index == 0 {
                    state.compare0_fired = false;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("nRF TIMER lock poisoned") = TimerState::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(device: &mut dyn Device, offset: u64, value: u32, at: u64) {
        device
            .write(
                offset,
                AccessWidth::Word,
                u64::from(value),
                SimTime::from_ticks(at),
            )
            .unwrap();
    }

    #[test]
    fn gpio_models_both_package_ports_and_rejects_absent_pins() {
        let hub = SignalHub::new();
        let (mut gpio, handle) = Nrf52840Gpio::new("gpio", "nrf.gpio", hub).unwrap();
        write(&mut gpio, 0x518, 1 << 3, 0);
        write(&mut gpio, 0x508, 1 << 3, 0);
        write(&mut gpio, 0x818, 1 << 15, 0);
        write(&mut gpio, 0x808, 1 << 15, 0);
        assert_eq!(handle.direction(), 1 << 3);
        assert_eq!(handle.output(), 1 << 3);
        assert_eq!(handle.direction_high(), 1 << 15);
        assert_eq!(handle.output_high(), 1 << 15);
        assert!(
            gpio.write(0xa40, AccessWidth::Word, 1, SimTime::ZERO)
                .is_err()
        );
    }

    #[test]
    fn uart_tasks_gate_bytes_and_raise_events() {
        let (mut uart, handle) = Nrf52840Uart::new("uart0");
        write(&mut uart, 0x51c, b'X'.into(), 0);
        assert!(handle.bytes().is_empty());
        write(&mut uart, 0x500, 4, 0);
        write(&mut uart, 0x008, 1, 0);
        write(&mut uart, 0x51c, b'N'.into(), 0);
        assert_eq!(handle.bytes(), b"N");
        assert_eq!(uart.read(0x11c, AccessWidth::Word, SimTime::ZERO), Ok(1));
        handle.feed_rx(b"R");
        write(&mut uart, 0x000, 1, 0);
        assert_eq!(uart.read(0x108, AccessWidth::Word, SimTime::ZERO), Ok(1));
        assert_eq!(
            uart.read(0x518, AccessWidth::Word, SimTime::ZERO),
            Ok(u64::from(b'R'))
        );
    }

    #[test]
    fn timer_compare_event_and_interrupt_are_deterministic() {
        let (mut timer, handle) = Nrf52840Timer::new("timer0");
        write(&mut timer, 0x540, 4, 0);
        write(&mut timer, 0x304, 1 << 16, 0);
        write(&mut timer, 0x000, 1, 0);
        assert!(!handle.poll(SimTime::from_ticks(3)));
        assert!(handle.poll(SimTime::from_ticks(4)));
        assert_eq!(timer.read(0x140, AccessWidth::Word, SimTime::ZERO), Ok(1));
        write(&mut timer, 0x140, 0, 4);
        assert!(!handle.poll(SimTime::from_ticks(4)));
    }
}
