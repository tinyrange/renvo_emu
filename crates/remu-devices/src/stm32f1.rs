use super::{GpioHandle, GpioState, SignalHub, refresh_gpio, vendor_gpio};
use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use remu_signals::{Logic, SignalId};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

const GPIO_CONFIG_RESET: u32 = 0x4444_4444;

/// STM32F1 GPIO port using the CRL/CRH configuration layout from RM0008.
pub struct Stm32F1Gpio {
    name: String,
    state: Arc<Mutex<GpioState>>,
    signals: Vec<SignalId>,
    hub: SignalHub,
    crl: u32,
    crh: u32,
    lckr: u32,
}

impl Stm32F1Gpio {
    /// Creates one 16-pin F1 GPIO port and its host-facing digital-pin handle.
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
                crl: GPIO_CONFIG_RESET,
                crh: GPIO_CONFIG_RESET,
                lckr: 0,
            },
            handle,
        ))
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

    fn apply_configuration(&self, at: SimTime) -> Result<(), DeviceError> {
        let direction = (0..16_u32).fold(0_u32, |mask, pin| {
            let config = if pin < 8 { self.crl } else { self.crh };
            let shift = (pin & 7) * 4;
            mask | (u32::from((config >> shift) & 3 != 0) << pin)
        });
        self.state.lock().expect("GPIO lock poisoned").direction = direction;
        refresh_gpio(&self.state, &self.signals, &self.hub, 16, at)
    }

    fn refresh(&self, at: SimTime) -> Result<(), DeviceError> {
        refresh_gpio(&self.state, &self.signals, &self.hub, 16, at)
    }
}

impl Device for Stm32F1Gpio {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("STM32F1 GPIO requires word accesses"));
        }
        let value = match offset {
            0x00 => self.crl,
            0x04 => self.crh,
            0x08 => self.input(),
            0x0c => self.state.lock().expect("GPIO lock poisoned").output,
            0x18 => self.lckr,
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled STM32F1 GPIO read at offset {offset:#x}"
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
            return Err(DeviceError::new("STM32F1 GPIO requires word accesses"));
        }
        let value = value as u32;
        match offset {
            0x00 => {
                self.crl = value;
                return self.apply_configuration(at);
            }
            0x04 => {
                self.crh = value;
                return self.apply_configuration(at);
            }
            0x0c => self.state.lock().expect("GPIO lock poisoned").output = value & 0xffff,
            0x10 => {
                let mut state = self.state.lock().expect("GPIO lock poisoned");
                state.output = (state.output | (value & 0xffff)) & !(value >> 16);
            }
            0x14 => self.state.lock().expect("GPIO lock poisoned").output &= !(value & 0xffff),
            0x18 => self.lckr = value & 0x0001_ffff,
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled STM32F1 GPIO write at offset {offset:#x}"
                )));
            }
        }
        self.refresh(at)
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.crl = GPIO_CONFIG_RESET;
        self.crh = GPIO_CONFIG_RESET;
        self.lckr = 0;
        let mut state = self.state.lock().expect("GPIO lock poisoned");
        state.direction = 0;
        state.output = 0;
    }
}

const USART_SR_RXNE: u32 = 1 << 5;
const USART_SR_TC: u32 = 1 << 6;
const USART_SR_TXE: u32 = 1 << 7;
const USART_CR1_RE: u32 = 1 << 2;
const USART_CR1_TE: u32 = 1 << 3;
const USART_CR1_RXNEIE: u32 = 1 << 5;
const USART_CR1_TCIE: u32 = 1 << 6;
const USART_CR1_TXEIE: u32 = 1 << 7;
const USART_CR1_UE: u32 = 1 << 13;

struct UsartState {
    sr: u32,
    brr: u32,
    cr1: u32,
    cr2: u32,
    cr3: u32,
    gtpr: u32,
    transmitted: Vec<u8>,
    received: VecDeque<u8>,
}

impl Default for UsartState {
    fn default() -> Self {
        Self {
            sr: USART_SR_TXE | USART_SR_TC,
            brr: 0,
            cr1: 0,
            cr2: 0,
            cr3: 0,
            gtpr: 0,
            transmitted: Vec::new(),
            received: VecDeque::new(),
        }
    }
}

/// Host-facing state for an STM32F1 USART instance.
#[derive(Clone)]
pub struct Stm32F1UsartHandle(Arc<Mutex<UsartState>>);

impl Stm32F1UsartHandle {
    /// Returns bytes accepted through the enabled transmitter.
    pub fn bytes(&self) -> Vec<u8> {
        self.0
            .lock()
            .expect("STM32F1 USART lock poisoned")
            .transmitted
            .clone()
    }

    /// Queues one deterministic receive byte and raises RXNE when RX is enabled.
    pub fn inject_rx(&self, value: u8) {
        let mut state = self.0.lock().expect("STM32F1 USART lock poisoned");
        state.received.push_back(value);
        if state.cr1 & USART_CR1_UE != 0 && state.cr1 & USART_CR1_RE != 0 {
            state.sr |= USART_SR_RXNE;
        }
    }

    /// Returns the native USART1 interrupt request level.
    pub fn interrupt_pending(&self) -> bool {
        let state = self.0.lock().expect("STM32F1 USART lock poisoned");
        (state.cr1 & USART_CR1_RXNEIE != 0 && state.sr & USART_SR_RXNE != 0)
            || (state.cr1 & USART_CR1_TCIE != 0 && state.sr & USART_SR_TC != 0)
            || (state.cr1 & USART_CR1_TXEIE != 0 && state.sr & USART_SR_TXE != 0)
    }
}

/// STM32F1 USART register slice using SR/DR/BRR/CR1/CR2/CR3/GTPR offsets.
pub struct Stm32F1Usart {
    name: String,
    state: Arc<Mutex<UsartState>>,
}

impl Stm32F1Usart {
    /// Creates a reset USART and its machine-facing handle.
    pub fn new(name: impl Into<String>) -> (Self, Stm32F1UsartHandle) {
        let state = Arc::new(Mutex::new(UsartState::default()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Stm32F1UsartHandle(state),
        )
    }
}

impl Device for Stm32F1Usart {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("STM32F1 USART requires word accesses"));
        }
        let mut state = self.state.lock().expect("STM32F1 USART lock poisoned");
        let value = match offset {
            0x00 => state.sr,
            0x04 => {
                let value = state.received.pop_front().unwrap_or(0);
                if state.received.is_empty() {
                    state.sr &= !USART_SR_RXNE;
                }
                u32::from(value)
            }
            0x08 => state.brr,
            0x0c => state.cr1,
            0x10 => state.cr2,
            0x14 => state.cr3,
            0x18 => state.gtpr,
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled STM32F1 USART read at offset {offset:#x}"
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
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("STM32F1 USART requires word accesses"));
        }
        let mut state = self.state.lock().expect("STM32F1 USART lock poisoned");
        let value = value as u32;
        match offset {
            0x00 => state.sr &= value | !(USART_SR_RXNE | USART_SR_TC),
            0x04 => {
                if state.cr1 & (USART_CR1_UE | USART_CR1_TE) == (USART_CR1_UE | USART_CR1_TE) {
                    state.transmitted.push(value as u8);
                    state.sr |= USART_SR_TXE | USART_SR_TC;
                }
            }
            0x08 => state.brr = value & 0xffff,
            0x0c => {
                state.cr1 = value & 0x0000_3fff;
                if state.cr1 & (USART_CR1_UE | USART_CR1_RE) == (USART_CR1_UE | USART_CR1_RE)
                    && !state.received.is_empty()
                {
                    state.sr |= USART_SR_RXNE;
                }
            }
            0x10 => state.cr2 = value & 0x0000_7f7f,
            0x14 => state.cr3 = value & 0x0000_07ff,
            0x18 => state.gtpr = value & 0xffff,
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled STM32F1 USART write at offset {offset:#x}"
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("STM32F1 USART lock poisoned") = UsartState::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f1_gpio_uses_crl_modes_and_atomic_set_reset() {
        let hub = SignalHub::new();
        let (mut gpio, handle) =
            Stm32F1Gpio::new("gpioa", "board.gpioa", hub).expect("GPIO construction");
        gpio.write(0, AccessWidth::Word, 0x4444_4441, SimTime::ZERO)
            .expect("CRL write");
        gpio.write(0x10, AccessWidth::Word, 1, SimTime::ZERO)
            .expect("BSRR write");
        assert_eq!(handle.direction() & 1, 1);
        assert_eq!(handle.output() & 1, 1);
        gpio.write(0x10, AccessWidth::Word, 1 << 16, SimTime::ZERO)
            .expect("BSRR reset write");
        assert_eq!(handle.output() & 1, 0);
    }

    #[test]
    fn f1_usart_requires_enabled_transmitter() {
        let (mut usart, handle) = Stm32F1Usart::new("usart1");
        usart
            .write(0x04, AccessWidth::Word, b'X'.into(), SimTime::ZERO)
            .expect("disabled DR write");
        assert!(handle.bytes().is_empty());
        usart
            .write(
                0x0c,
                AccessWidth::Word,
                u64::from(USART_CR1_UE | USART_CR1_TE),
                SimTime::ZERO,
            )
            .expect("CR1 write");
        usart
            .write(0x04, AccessWidth::Word, b'Y'.into(), SimTime::ZERO)
            .expect("enabled DR write");
        assert_eq!(handle.bytes(), b"Y");
    }
}
