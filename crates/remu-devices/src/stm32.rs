use super::{GpioHandle, GpioState, SignalHub, refresh_gpio, vendor_gpio};
use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use remu_signals::{Logic, SignalId};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// STM32L4 GPIO port with mode, input/output, bit-set/reset and alternate-function registers.
pub struct Stm32Gpio {
    name: String,
    state: Arc<Mutex<GpioState>>,
    signals: Vec<SignalId>,
    hub: SignalHub,
    moder: u32,
    otyper: u32,
    ospeedr: u32,
    pupdr: u32,
    lckr: u32,
    afr: [u32; 2],
}

impl Stm32Gpio {
    /// Constructs one 16-pin STM32 GPIO port and external pin handle.
    pub fn new(
        name: impl Into<String>,
        path: &str,
        hub: SignalHub,
    ) -> Result<(Self, GpioHandle), remu_signals::SignalError> {
        let (state, signals, handle) = vendor_gpio(16, path, &hub)?;
        Ok((
            Self {
                name: name.into(),
                state,
                signals,
                hub,
                moder: 0xffff_ffff,
                otyper: 0,
                ospeedr: 0,
                pupdr: 0,
                lckr: 0,
                afr: [0; 2],
            },
            handle,
        ))
    }

    fn apply_modes(&self, at: SimTime) -> Result<(), DeviceError> {
        {
            let mut state = self.state.lock().expect("GPIO lock poisoned");
            state.direction = (0..16_u32).fold(0_u32, |direction, pin| {
                direction | (u32::from((self.moder >> (pin * 2)) & 3 == 1) << pin)
            });
        }
        refresh_gpio(&self.state, &self.signals, &self.hub, 16, at)
    }

    fn input(&self) -> u32 {
        self.state
            .lock()
            .expect("GPIO lock poisoned")
            .nets
            .iter()
            .enumerate()
            .fold(0_u32, |value, (pin, net)| {
                value | (u32::from(net.resolved() == Logic::One) << pin)
            })
    }
}

impl Device for Stm32Gpio {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("STM32 GPIO requires word accesses"));
        }
        let value = match offset {
            0x00 => self.moder,
            0x04 => self.otyper,
            0x08 => self.ospeedr,
            0x0c => self.pupdr,
            0x10 => self.input(),
            0x14 => self.state.lock().expect("GPIO lock poisoned").output,
            0x1c => self.lckr,
            0x20 => self.afr[0],
            0x24 => self.afr[1],
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
            return Err(DeviceError::new("STM32 GPIO requires word accesses"));
        }
        let value = value as u32;
        match offset {
            0x00 => {
                self.moder = value;
                return self.apply_modes(at);
            }
            0x04 => self.otyper = value & 0xffff,
            0x08 => self.ospeedr = value,
            0x0c => self.pupdr = value,
            0x14 => self.state.lock().expect("GPIO lock poisoned").output = value & 0xffff,
            0x18 => {
                let mut state = self.state.lock().expect("GPIO lock poisoned");
                state.output = (state.output | (value & 0xffff)) & !(value >> 16);
            }
            0x1c => self.lckr = value,
            0x20 => self.afr[0] = value,
            0x24 => self.afr[1] = value,
            0x28 => self.state.lock().expect("GPIO lock poisoned").output &= !(value & 0xffff),
            _ => {}
        }
        refresh_gpio(&self.state, &self.signals, &self.hub, 16, at)
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.moder = 0xffff_ffff;
        self.otyper = 0;
        self.ospeedr = 0;
        self.pupdr = 0;
        self.lckr = 0;
        self.afr = [0; 2];
        let mut state = self.state.lock().expect("GPIO lock poisoned");
        state.direction = 0;
        state.output = 0;
    }
}

#[derive(Default)]
struct TimerState {
    enabled: bool,
    update_interrupt: bool,
    pending: bool,
    started: u64,
    prescaler: u16,
    reload: u32,
}

/// Machine handle for STM32 TIM2 update events.
#[derive(Clone)]
pub struct Stm32TimerHandle(Arc<Mutex<TimerState>>);

impl Stm32TimerHandle {
    /// Advances the counter and returns the update-interrupt level.
    pub fn poll(&self, now: SimTime) -> bool {
        let mut state = self.0.lock().expect("TIM lock poisoned");
        let period = u64::from(state.reload.saturating_add(1))
            .saturating_mul(u64::from(state.prescaler) + 1);
        if state.enabled && period != 0 && now.ticks().saturating_sub(state.started) >= period {
            state.pending = true;
            state.started = now.ticks();
        }
        state.pending && state.update_interrupt
    }
}

/// Functional STM32 TIM2 counter/update slice.
pub struct Stm32Timer {
    name: String,
    state: Arc<Mutex<TimerState>>,
    registers: [u32; 18],
}

impl Stm32Timer {
    /// Constructs TIM2 and its interrupt handle.
    pub fn new(name: impl Into<String>) -> (Self, Stm32TimerHandle) {
        let state = Arc::new(Mutex::new(TimerState {
            reload: u32::MAX,
            ..TimerState::default()
        }));
        (
            Self {
                name: name.into(),
                state: state.clone(),
                registers: [0; 18],
            },
            Stm32TimerHandle(state),
        )
    }
}

impl Device for Stm32Timer {
    fn name(&self) -> &str {
        &self.name
    }
    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("STM32 TIM requires word accesses"));
        }
        let state = self.state.lock().expect("TIM lock poisoned");
        match offset {
            0x00 => Ok(u64::from(state.enabled)),
            0x0c => Ok(u64::from(state.update_interrupt)),
            0x10 => Ok(u64::from(state.pending)),
            0x24 => Ok((at.ticks().saturating_sub(state.started)
                / (u64::from(state.prescaler) + 1))
                & u64::from(u32::MAX)),
            0x28 => Ok(u64::from(state.prescaler)),
            0x2c => Ok(u64::from(state.reload)),
            _ => Ok(u64::from(
                self.registers[usize::try_from(offset / 4).unwrap_or(0).min(17)],
            )),
        }
    }
    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("STM32 TIM requires word accesses"));
        }
        let mut state = self.state.lock().expect("TIM lock poisoned");
        match offset {
            0x00 => {
                state.enabled = value & 1 != 0;
                state.started = at.ticks();
            }
            0x0c => state.update_interrupt = value & 1 != 0,
            0x10 => state.pending &= value & 1 != 0,
            0x14 => {
                if value & 1 != 0 {
                    state.pending = true;
                    state.started = at.ticks();
                }
            }
            0x24 => {
                state.started = at
                    .ticks()
                    .saturating_sub(value.saturating_mul(u64::from(state.prescaler) + 1))
            }
            0x28 => state.prescaler = value as u16,
            0x2c => state.reload = value as u32,
            _ => self.registers[usize::try_from(offset / 4).unwrap_or(0).min(17)] = value as u32,
        }
        Ok(())
    }
    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("TIM lock poisoned") = TimerState {
            reload: u32::MAX,
            ..TimerState::default()
        };
        self.registers = [0; 18];
    }
}

#[derive(Default)]
struct UsartState {
    control: u32,
    bytes: Vec<u8>,
    received: VecDeque<u8>,
    status: u32,
}

const USART_CR1_RXNEIE: u32 = 1 << 5;
const USART_CR1_TCIE: u32 = 1 << 6;
const USART_CR1_TXEIE: u32 = 1 << 7;
const USART_ISR_RXNE: u32 = 1 << 5;
const USART_ISR_TC: u32 = 1 << 6;
const USART_ISR_TXE: u32 = 1 << 7;

/// Machine handle for STM32 USART output and TX-empty interrupt state.
#[derive(Clone)]
pub struct Stm32UsartHandle(Arc<Mutex<UsartState>>);

impl Stm32UsartHandle {
    /// Bytes written to TDR.
    pub fn bytes(&self) -> Vec<u8> {
        self.0.lock().expect("USART lock poisoned").bytes.clone()
    }

    /// Queues one byte for firmware reception and raises RXNE/RXFNE.
    pub fn inject_rx(&self, value: u8) {
        let mut state = self.0.lock().expect("USART lock poisoned");
        state.received.push_back(value);
        state.status |= USART_ISR_RXNE;
    }

    /// Whether TX-empty interrupt is enabled.
    pub fn interrupt_pending(&self) -> bool {
        let state = self.0.lock().expect("USART lock poisoned");
        (state.control & USART_CR1_RXNEIE != 0 && state.status & USART_ISR_RXNE != 0)
            || (state.control & USART_CR1_TCIE != 0 && state.status & USART_ISR_TC != 0)
            || (state.control & USART_CR1_TXEIE != 0 && state.status & USART_ISR_TXE != 0)
    }
}

/// Functional STM32 USART2 startup/transmit slice.
pub struct Stm32Usart {
    name: String,
    state: Arc<Mutex<UsartState>>,
    registers: [u32; 12],
}

impl Stm32Usart {
    /// Constructs USART2 and its machine handle.
    pub fn new(name: impl Into<String>) -> (Self, Stm32UsartHandle) {
        let state = Arc::new(Mutex::new(UsartState {
            status: USART_ISR_TXE | USART_ISR_TC,
            ..UsartState::default()
        }));
        (
            Self {
                name: name.into(),
                state: state.clone(),
                registers: [0; 12],
            },
            Stm32UsartHandle(state),
        )
    }
}

impl Device for Stm32Usart {
    fn name(&self) -> &str {
        &self.name
    }
    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("STM32 USART requires word accesses"));
        }
        if offset == 0x1c {
            return Ok(u64::from(
                self.state.lock().expect("USART lock poisoned").status
                    | USART_ISR_TXE
                    | USART_ISR_TC,
            ));
        }
        if offset == 0 {
            return Ok(u64::from(
                self.state.lock().expect("USART lock poisoned").control,
            ));
        }
        if offset == 0x24 {
            let mut state = self.state.lock().expect("USART lock poisoned");
            let value = state.received.pop_front().unwrap_or(0);
            if state.received.is_empty() {
                state.status &= !USART_ISR_RXNE;
            }
            return Ok(u64::from(value));
        }
        Ok(u64::from(
            self.registers[usize::try_from(offset / 4).unwrap_or(0).min(11)],
        ))
    }
    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("STM32 USART requires word accesses"));
        }
        match offset {
            0 => self.state.lock().expect("USART lock poisoned").control = value as u32,
            0x20 => self.state.lock().expect("USART lock poisoned").status &= !(value as u32),
            0x28 => {
                let mut state = self.state.lock().expect("USART lock poisoned");
                state.bytes.push(value as u8);
                state.status |= USART_ISR_TXE | USART_ISR_TC;
            }
            _ => self.registers[usize::try_from(offset / 4).unwrap_or(0).min(11)] = value as u32,
        }
        Ok(())
    }
    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("USART lock poisoned") = UsartState {
            status: USART_ISR_TXE | USART_ISR_TC,
            ..UsartState::default()
        };
        self.registers = [0; 12];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gpio_bsrr_sets_and_resets_output_bits() {
        let hub = SignalHub::new();
        let (mut gpio, handle) = Stm32Gpio::new("gpioa", "board.stm32.gpioa", hub).unwrap();
        gpio.write(0, AccessWidth::Word, 1, SimTime::ZERO).unwrap();
        gpio.write(0x18, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.output(), 1);
        gpio.write(0x18, AccessWidth::Word, 1 << 16, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.output(), 0);
    }
    #[test]
    fn tim2_update_and_usart_transmit_are_observable() {
        let (mut timer, timer_handle) = Stm32Timer::new("tim2");
        timer
            .write(0x2c, AccessWidth::Word, 3, SimTime::ZERO)
            .unwrap();
        timer
            .write(0x0c, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        timer.write(0, AccessWidth::Word, 1, SimTime::ZERO).unwrap();
        assert!(timer_handle.poll(SimTime::from_ticks(4)));
        let (mut usart, handle) = Stm32Usart::new("usart2");
        usart
            .write(0x28, AccessWidth::Word, u64::from(b'S'), SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.bytes(), b"S");
    }

    #[test]
    fn usart_receive_and_interrupt_flags_are_functional() {
        let (mut usart, handle) = Stm32Usart::new("usart1");
        handle.inject_rx(b'R');
        usart
            .write(
                0x00,
                AccessWidth::Word,
                u64::from(USART_CR1_RXNEIE),
                SimTime::ZERO,
            )
            .unwrap();
        assert!(handle.interrupt_pending());
        assert_eq!(
            usart.read(0x1c, AccessWidth::Word, SimTime::ZERO).unwrap() & u64::from(USART_ISR_RXNE),
            u64::from(USART_ISR_RXNE)
        );
        assert_eq!(
            usart.read(0x24, AccessWidth::Word, SimTime::ZERO).unwrap(),
            u64::from(b'R')
        );
        assert!(!handle.interrupt_pending());
    }
}
