use super::*;

const REGISTER_BYTES: usize = 0x410;
// ESP32-S3 GDMA channel 0 offsets from Espressif's gdma_reg.h.
const IN_CONFIG0: u64 = 0x00;
const IN_CONFIG1: u64 = 0x04;
const IN_INT_RAW: u64 = 0x08;
const IN_INT_STATUS: u64 = 0x0c;
const IN_INT_ENABLE: u64 = 0x10;
const IN_INT_CLEAR: u64 = 0x14;
const IN_FIFO_STATUS: u64 = 0x18;
const IN_POP: u64 = 0x1c;
const IN_LINK: u64 = 0x20;
const IN_PERIPHERAL: u64 = 0x48;
const OUT_CONFIG0: u64 = 0x60;
const OUT_CONFIG1: u64 = 0x64;
const OUT_INT_RAW: u64 = 0x68;
const OUT_INT_STATUS: u64 = 0x6c;
const OUT_INT_ENABLE: u64 = 0x70;
const OUT_INT_CLEAR: u64 = 0x74;
const OUT_FIFO_STATUS: u64 = 0x78;
const OUT_PUSH: u64 = 0x7c;
const OUT_LINK: u64 = 0x80;
const OUT_PERIPHERAL: u64 = 0xa8;
const MISC_CONFIG: u64 = 0x3c8;
const DATE: u64 = 0x40c;

const IN_DONE: u32 = 1 << 0;
const IN_SUC_EOF: u32 = 1 << 1;
const OUT_DONE: u32 = 1 << 0;
const OUT_EOF: u32 = 1 << 1;
const FIFO_LIMIT: usize = 64;

#[derive(Default)]
struct EspGdmaState {
    registers: Vec<u32>,
    input: VecDeque<u32>,
    output: VecDeque<u32>,
}

impl EspGdmaState {
    fn new() -> Self {
        let mut registers = vec![0; REGISTER_BYTES / 4];
        registers[DATE as usize / 4] = 0x0210_1180;
        registers[IN_PERIPHERAL as usize / 4] = 63;
        registers[OUT_PERIPHERAL as usize / 4] = 63;
        registers[IN_LINK as usize / 4] = 1 << 24;
        registers[OUT_CONFIG0 as usize / 4] = 1 << 3;
        registers[OUT_LINK as usize / 4] = 1 << 23;
        Self {
            registers,
            ..Self::default()
        }
    }

    fn refresh(&mut self) {
        self.registers[IN_INT_STATUS as usize / 4] =
            self.registers[IN_INT_RAW as usize / 4] & self.registers[IN_INT_ENABLE as usize / 4];
        self.registers[OUT_INT_STATUS as usize / 4] =
            self.registers[OUT_INT_RAW as usize / 4] & self.registers[OUT_INT_ENABLE as usize / 4];
        let mut input_status = ((self.input.len().min(31) as u32) << 2) | (1 << 1);
        if self.input.len() >= FIFO_LIMIT {
            input_status |= 1;
            input_status &= !(1 << 1);
        }
        self.registers[IN_FIFO_STATUS as usize / 4] = input_status;
        let mut output_status = ((self.output.len().min(31) as u32) << 6) | (1 << 1);
        if self.output.len() >= FIFO_LIMIT {
            output_status |= 1;
            output_status &= !(1 << 1);
        }
        self.registers[OUT_FIFO_STATUS as usize / 4] = output_status;
    }
}

/// Host-facing ESP32-S3 GDMA channel-0 fixture handle.
#[derive(Clone)]
pub struct EspGdmaHandle {
    state: Arc<Mutex<EspGdmaState>>,
}

impl EspGdmaHandle {
    /// Queues words for a firmware IN-channel FIFO pop sequence.
    pub fn queue_input_words(&self, words: &[u32]) {
        let mut state = self.state.lock().expect("ESP GDMA lock poisoned");
        for word in words {
            if state.input.len() < FIFO_LIMIT {
                state.input.push_back(*word & 0xfff);
            }
        }
        if !words.is_empty() {
            state.registers[IN_INT_RAW as usize / 4] |= IN_DONE | IN_SUC_EOF;
        }
        state.refresh();
    }

    /// Returns words written by firmware through the OUT-channel push port.
    pub fn take_output_words(&self) -> Vec<u32> {
        let mut state = self.state.lock().expect("ESP GDMA lock poisoned");
        let words = state.output.drain(..).collect();
        state.refresh();
        words
    }
}

/// Functional ESP32-S3 GDMA channel-0 register block.
pub struct EspGdma {
    name: String,
    state: Arc<Mutex<EspGdmaState>>,
    hub: SignalHub,
    input_signal: SignalId,
    output_signal: SignalId,
}

impl EspGdma {
    /// Creates a deterministic channel-0 GDMA model and host handle.
    pub fn new(
        name: impl Into<String>,
        signal_prefix: &str,
        hub: SignalHub,
    ) -> Result<(Self, EspGdmaHandle), SignalError> {
        let input_signal = hub.declare(
            format!("{signal_prefix}.in"),
            SignalValue::from_u64(0, 12)?,
            Some("last GDMA input word".to_string()),
        )?;
        let output_signal = hub.declare(
            format!("{signal_prefix}.out"),
            SignalValue::from_u64(0, 9)?,
            Some("last GDMA output word".to_string()),
        )?;
        let state = Arc::new(Mutex::new(EspGdmaState::new()));
        Ok((
            Self {
                name: name.into(),
                state: state.clone(),
                hub,
                input_signal,
                output_signal,
            },
            EspGdmaHandle { state },
        ))
    }

    fn emit(
        &self,
        signal: SignalId,
        value: u32,
        width: u16,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        let value = SignalValue::from_u64(u64::from(value), width)
            .map_err(|error| DeviceError::new(format!("{} signal value: {error}", self.name)))?;
        self.hub
            .set(signal, value, at)
            .map_err(|error| DeviceError::new(format!("{} signal update: {error}", self.name)))
    }
}

impl Device for EspGdma {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("ESP GDMA requires aligned word access"));
        }
        let mut state = self.state.lock().expect("ESP GDMA lock poisoned");
        let value = if offset == IN_POP {
            let word = state.input.pop_front().unwrap_or_default();
            state.refresh();
            drop(state);
            self.emit(self.input_signal, word, 12, at)?;
            return Ok(u64::from(word));
        } else if offset == IN_FIFO_STATUS || offset == OUT_FIFO_STATUS {
            state.refresh();
            state.registers[offset as usize / 4]
        } else if offset == IN_INT_STATUS || offset == OUT_INT_STATUS {
            state.refresh();
            state.registers[offset as usize / 4]
        } else {
            let index = usize::try_from(offset / 4).expect("GDMA register index fits");
            *state
                .registers
                .get(index)
                .ok_or_else(|| DeviceError::new(format!("{} read at {offset:#x}", self.name)))?
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
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("ESP GDMA requires aligned word access"));
        }
        let mut state = self.state.lock().expect("ESP GDMA lock poisoned");
        let value = value as u32;
        if offset == IN_POP {
            if value & (1 << 12) != 0 {
                state.input.pop_front();
            }
        } else if offset == OUT_PUSH {
            if value & (1 << 9) != 0 && state.output.len() < FIFO_LIMIT {
                let word = value & 0x1ff;
                state.output.push_back(word);
                state.registers[OUT_INT_RAW as usize / 4] |= OUT_DONE | OUT_EOF;
                state.refresh();
                drop(state);
                self.emit(self.output_signal, word, 9, at)?;
                return Ok(());
            }
        } else {
            match offset {
                IN_INT_ENABLE => state.registers[IN_INT_ENABLE as usize / 4] = value & 0x7f,
                OUT_INT_ENABLE => state.registers[OUT_INT_ENABLE as usize / 4] = value & 0x7f,
                IN_INT_RAW => state.registers[IN_INT_RAW as usize / 4] = value & 0x7f,
                OUT_INT_RAW => state.registers[OUT_INT_RAW as usize / 4] = value & 0x7f,
                IN_INT_CLEAR => state.registers[IN_INT_RAW as usize / 4] &= !value,
                OUT_INT_CLEAR => state.registers[OUT_INT_RAW as usize / 4] &= !value,
                IN_CONFIG0 | IN_CONFIG1 | IN_LINK | OUT_CONFIG0 | OUT_CONFIG1 | OUT_LINK
                | MISC_CONFIG | IN_PERIPHERAL | OUT_PERIPHERAL => {
                    state.registers[offset as usize / 4] = value
                }
                _ => {
                    let index = usize::try_from(offset / 4).expect("GDMA register index fits");
                    let register = state.registers.get_mut(index).ok_or_else(|| {
                        DeviceError::new(format!("{} write at {offset:#x}", self.name))
                    })?;
                    *register = value;
                }
            }
        }
        state.refresh();
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.lock().expect("ESP GDMA lock poisoned");
        *state = EspGdmaState::new();
        let zero_in = SignalValue::from_u64(0, 12).expect("12-bit signal");
        let zero_out = SignalValue::from_u64(0, 9).expect("9-bit signal");
        let _ = self.hub.set(self.input_signal, zero_in, SimTime::ZERO);
        let _ = self.hub.set(self.output_signal, zero_out, SimTime::ZERO);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_zero_fifo_paths_and_interrupts_are_deterministic() {
        let hub = SignalHub::new();
        let (mut gdma, handle) = EspGdma::new("gdma", "board.gdma", hub.clone()).unwrap();
        handle.queue_input_words(&[0xabc, 0x123]);
        assert_eq!(
            gdma.read(IN_FIFO_STATUS, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            2 | (2 << 2)
        );
        assert_eq!(
            gdma.read(IN_POP, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0xabc
        );
        assert_eq!(
            gdma.read(IN_INT_RAW, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            u64::from(IN_DONE | IN_SUC_EOF)
        );
        gdma.write(
            IN_INT_ENABLE,
            AccessWidth::Word,
            u64::from(IN_DONE | IN_SUC_EOF),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            gdma.read(IN_INT_STATUS, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            u64::from(IN_DONE | IN_SUC_EOF)
        );
        gdma.write(
            IN_INT_CLEAR,
            AccessWidth::Word,
            u64::from(IN_DONE | IN_SUC_EOF),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            gdma.read(IN_INT_STATUS, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            0
        );

        gdma.write(OUT_PUSH, AccessWidth::Word, 0x355, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.take_output_words(), vec![0x155]);
        assert!(
            hub.with_registry(|registry| registry.find("board.gdma.out"))
                .is_some()
        );
    }
}
