use super::*;
use remu_signals::{SignalId, SignalValue};
use std::collections::VecDeque;

const BLOCK_B: u64 = 0x20;
const CR1: u64 = 0x00;
const CR2: u64 = 0x04;
const FRCR: u64 = 0x08;
const SLOTR: u64 = 0x0c;
const IM: u64 = 0x10;
const SR: u64 = 0x14;
const CLRFR: u64 = 0x18;
const DR: u64 = 0x1c;

const CR1_SAIEN: u32 = 1;
const CR2_FFLUSH: u32 = 1 << 3;
const SR_OVRUDR: u32 = 1;
const SR_FRE: u32 = 1 << 3;
const SR_CNRDY: u32 = 1 << 4;
const FLAG_MASK: u32 = SR_OVRUDR | SR_FRE | SR_CNRDY | (1 << 5) | (1 << 6);

#[derive(Default)]
struct SaiBlock {
    cr1: u32,
    cr2: u32,
    frcr: u32,
    slotr: u32,
    im: u32,
    sr: u32,
    tx: VecDeque<u32>,
    rx: VecDeque<u32>,
}

#[derive(Default)]
struct SaiState {
    blocks: [SaiBlock; 2],
    tx_strobe: bool,
}

/// Host-facing STM32 SAI1 stream handle.
#[derive(Clone)]
pub struct Stm32Sai1Handle {
    state: Arc<Mutex<SaiState>>,
    hub: SignalHub,
    interrupt_signal: SignalId,
    tx_word_signal: SignalId,
    tx_strobe_signal: SignalId,
    rx_word_signal: SignalId,
}

impl Stm32Sai1Handle {
    /// Advances SAI1 and reports an enabled block interrupt request.
    pub fn poll(&self, now: SimTime) -> bool {
        let pending = {
            let state = self.state.lock().expect("STM32 SAI1 lock poisoned");
            state
                .blocks
                .iter()
                .any(|block| block.sr & block.im & FLAG_MASK != 0)
        };
        self.publish_irq(pending, now);
        pending
    }

    /// Supplies one or more host-side receive samples to block A or B.
    pub fn push_rx(&self, block: u8, samples: &[u32], at: SimTime) -> Result<(), String> {
        let index = usize::from(block);
        if index >= 2 || samples.len() > 256 {
            return Err("SAI1 block or receive batch is out of range".to_owned());
        }
        let mut state = self.state.lock().expect("STM32 SAI1 lock poisoned");
        let fifo = &mut state.blocks[index].rx;
        fifo.extend(samples.iter().copied());
        state.blocks[index].sr |= SR_FRE;
        let word = samples.first().copied().unwrap_or(0);
        drop(state);
        self.hub
            .set(
                self.rx_word_signal,
                SignalValue::from_u64(u64::from(word), 32).expect("SAI1 RX signal width is valid"),
                at,
            )
            .expect("SAI1 RX signal is declared");
        Ok(())
    }

    /// Takes all samples the firmware has written to a transmit block.
    pub fn take_tx(&self, block: u8) -> Result<Vec<u32>, String> {
        let index = usize::from(block);
        if index >= 2 {
            return Err("SAI1 block is out of range".to_owned());
        }
        let mut state = self.state.lock().expect("STM32 SAI1 lock poisoned");
        Ok(state.blocks[index].tx.drain(..).collect())
    }

    /// Returns whether a block is enabled.
    pub fn enabled(&self, block: u8) -> bool {
        let index = usize::from(block);
        self.state
            .lock()
            .expect("STM32 SAI1 lock poisoned")
            .blocks
            .get(index)
            .is_some_and(|block| block.cr1 & CR1_SAIEN != 0)
    }

    fn publish_irq(&self, pending: bool, at: SimTime) {
        self.hub
            .set(
                self.interrupt_signal,
                SignalValue::from_u64(u64::from(pending), 1)
                    .expect("SAI1 interrupt signal width is valid"),
                at,
            )
            .expect("SAI1 interrupt signal is declared");
    }

    fn publish_tx(&self, word: u32, at: SimTime) {
        let mut state = self.state.lock().expect("STM32 SAI1 lock poisoned");
        state.tx_strobe = !state.tx_strobe;
        let strobe = state.tx_strobe;
        drop(state);
        self.hub
            .set(
                self.tx_word_signal,
                SignalValue::from_u64(u64::from(word), 32).expect("SAI1 TX signal width is valid"),
                at,
            )
            .expect("SAI1 TX signal is declared");
        self.hub
            .set(
                self.tx_strobe_signal,
                SignalValue::from_u64(u64::from(strobe), 1).expect("SAI1 TX strobe width is valid"),
                at,
            )
            .expect("SAI1 TX strobe signal is declared");
    }
}

/// Functional STM32L432 SAI1 audio serial interface.
pub struct Stm32Sai1 {
    name: String,
    state: Arc<Mutex<SaiState>>,
    handle: Stm32Sai1Handle,
}

impl Stm32Sai1 {
    /// Creates disabled SAI1 blocks A and B with empty FIFOs.
    pub fn new(
        name: impl Into<String>,
        hub: SignalHub,
    ) -> Result<(Self, Stm32Sai1Handle), remu_signals::SignalError> {
        let name = name.into();
        let interrupt_signal = hub.declare(
            format!("{name}.irq"),
            SignalValue::from_u64(0, 1)?,
            Some("SAI1 block interrupt request".to_owned()),
        )?;
        let tx_word_signal = hub.declare(
            format!("{name}.tx_word"),
            SignalValue::from_u64(0, 32)?,
            Some("SAI1 transmitted sample".to_owned()),
        )?;
        let tx_strobe_signal = hub.declare(
            format!("{name}.tx_strobe"),
            SignalValue::from_u64(0, 1)?,
            Some("SAI1 transmitted sample strobe".to_owned()),
        )?;
        let rx_word_signal = hub.declare(
            format!("{name}.rx_word"),
            SignalValue::from_u64(0, 32)?,
            Some("SAI1 received sample".to_owned()),
        )?;
        let state = Arc::new(Mutex::new(SaiState::default()));
        let handle = Stm32Sai1Handle {
            state: state.clone(),
            hub,
            interrupt_signal,
            tx_word_signal,
            tx_strobe_signal,
            rx_word_signal,
        };
        Ok((
            Self {
                name,
                state,
                handle: handle.clone(),
            },
            handle,
        ))
    }

    fn block_offset(offset: u64) -> Option<(usize, u64)> {
        if offset < BLOCK_B {
            Some((0, offset))
        } else if offset < 2 * BLOCK_B {
            Some((1, offset - BLOCK_B))
        } else {
            None
        }
    }

    fn read_register(&self, offset: u64) -> u32 {
        let mut state = self.state.lock().expect("STM32 SAI1 lock poisoned");
        let (index, register) = Self::block_offset(offset).unwrap_or((0, offset));
        let block = &mut state.blocks[index];
        match register {
            CR1 => block.cr1,
            CR2 => block.cr2,
            FRCR => block.frcr,
            SLOTR => block.slotr,
            IM => block.im,
            SR => block.sr,
            DR => block.rx.pop_front().unwrap_or(0),
            _ => 0,
        }
    }

    fn write_register(&mut self, offset: u64, value: u32, at: SimTime) {
        let mut tx_word = None;
        let pending = {
            let mut state = self.state.lock().expect("STM32 SAI1 lock poisoned");
            let (index, register) = Self::block_offset(offset).unwrap_or((0, offset));
            let block = &mut state.blocks[index];
            match register {
                CR1 => block.cr1 = value,
                CR2 => {
                    block.cr2 = value & !CR2_FFLUSH;
                    if value & CR2_FFLUSH != 0 {
                        block.tx.clear();
                        block.rx.clear();
                    }
                }
                FRCR => block.frcr = value,
                SLOTR => block.slotr = value,
                IM => block.im = value & FLAG_MASK,
                CLRFR => block.sr &= !(value & FLAG_MASK),
                DR => {
                    if block.cr1 & CR1_SAIEN != 0 {
                        if block.tx.len() >= 8 {
                            block.sr |= SR_OVRUDR;
                        } else {
                            block.tx.push_back(value);
                            tx_word = Some(value);
                        }
                    } else {
                        block.sr |= SR_CNRDY;
                    }
                }
                _ => {}
            }
            block.sr & block.im & FLAG_MASK != 0
        };
        if let Some(word) = tx_word {
            self.handle.publish_tx(word, at);
        }
        self.handle.publish_irq(pending, at);
    }
}

impl Device for Stm32Sai1 {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 || offset >= 0x400 {
            return Err(DeviceError::new(format!(
                "STM32 SAI1 access at {offset:#x}"
            )));
        }
        Ok(u64::from(self.read_register(offset)))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 || offset >= 0x400 {
            return Err(DeviceError::new(format!(
                "STM32 SAI1 access at {offset:#x}"
            )));
        }
        self.write_register(offset, value as u32, at);
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("STM32 SAI1 lock poisoned") = SaiState::default();
        self.handle.publish_irq(false, SimTime::ZERO);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sai1_block_a_transmits_samples_and_strobes_vcd() {
        let hub = SignalHub::new();
        let (mut sai, handle) = Stm32Sai1::new("sai1", hub).unwrap();
        sai.write(CR1, AccessWidth::Word, u64::from(CR1_SAIEN), SimTime::ZERO)
            .unwrap();
        sai.write(DR, AccessWidth::Word, 0x1234, SimTime::from_ticks(1))
            .unwrap();
        sai.write(DR, AccessWidth::Word, 0x5678, SimTime::from_ticks(2))
            .unwrap();
        assert!(handle.enabled(0));
        assert_eq!(handle.take_tx(0).unwrap(), vec![0x1234, 0x5678]);
    }

    #[test]
    fn sai1_block_b_receives_samples_and_flushes() {
        let hub = SignalHub::new();
        let (mut sai, handle) = Stm32Sai1::new("sai1", hub).unwrap();
        handle.push_rx(1, &[0xa5a5, 0x5a5a], SimTime::ZERO).unwrap();
        assert_eq!(
            sai.read(BLOCK_B + DR, AccessWidth::Word, SimTime::ZERO),
            Ok(0xa5a5)
        );
        sai.write(
            BLOCK_B + CR2,
            AccessWidth::Word,
            u64::from(CR2_FFLUSH),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            sai.read(BLOCK_B + DR, AccessWidth::Word, SimTime::ZERO),
            Ok(0)
        );
    }
}
