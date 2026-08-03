use super::*;

const REGISTER_BYTES: usize = 0x2b0;
const IN_INT_RAW: u64 = 0x00;
const IN_INT_STATUS: u64 = 0x04;
const IN_INT_ENABLE: u64 = 0x08;
const IN_INT_CLEAR: u64 = 0x0c;
const OUT_INT_RAW: u64 = 0x30;
const OUT_INT_STATUS: u64 = 0x34;
const OUT_INT_ENABLE: u64 = 0x38;
const OUT_INT_CLEAR: u64 = 0x3c;
const MISC_CONFIG: u64 = 0x64;
const DATE: u64 = 0x68;
const IN_CONFIG0: u64 = 0x70;
const IN_CONFIG1: u64 = 0x74;
const IN_FIFO_STATUS: u64 = 0x78;
const IN_POP: u64 = 0x7c;
const IN_LINK: u64 = 0x80;
const IN_PRIORITY: u64 = 0x9c;
const IN_PERIPHERAL: u64 = 0xa0;
const OUT_CONFIG0: u64 = 0xd0;
const OUT_CONFIG1: u64 = 0xd4;
const OUT_FIFO_STATUS: u64 = 0xd8;
const OUT_PUSH: u64 = 0xdc;
const OUT_LINK: u64 = 0xe0;
const OUT_PRIORITY: u64 = 0xfc;
const OUT_PERIPHERAL: u64 = 0x100;

const IN_DONE: u32 = 1 << 0;
const IN_SUC_EOF: u32 = 1 << 1;
const IN_FIFO_OVF: u32 = 1 << 5;
const OUT_DONE: u32 = 1 << 0;
const OUT_EOF: u32 = 1 << 1;
const OUT_FIFO_OVF: u32 = 1 << 4;
const IN_INT_MASK: u32 = 0x7f;
const OUT_INT_MASK: u32 = 0x3f;
const MISC_CONFIG_MASK: u32 = (1 << 0) | (1 << 2) | (1 << 3);
const IN_CONFIG0_MASK: u32 = 0x3f;
const OUT_CONFIG0_MASK: u32 = 0x7f;
const CONFIG1_MASK: u32 = 1 << 12;
const LINK_ADDRESS_MASK: u32 = 0x000f_ffff;
const IN_LINK_RW_MASK: u32 = LINK_ADDRESS_MASK | (1 << 20);
const OUT_LINK_RW_MASK: u32 = LINK_ADDRESS_MASK;
const IN_LINK_PARK: u32 = 1 << 24;
const OUT_LINK_PARK: u32 = 1 << 23;
const PRIORITY_MASK: u32 = 0x0f;
const PERIPHERAL_MASK: u32 = 0x3f;
const IN_POP_DATA_MASK: u32 = 0x0fff;
const IN_POP_TRIGGER: u32 = 1 << 12;
const OUT_PUSH_DATA_MASK: u32 = 0x01ff;
const OUT_PUSH_TRIGGER: u32 = 1 << 9;
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
        registers[DATE as usize / 4] = 35_660_368;
        registers[IN_PERIPHERAL as usize / 4] = 63;
        registers[OUT_PERIPHERAL as usize / 4] = 63;
        registers[IN_POP as usize / 4] = 0x800;
        registers[IN_LINK as usize / 4] = (1 << 20) | IN_LINK_PARK;
        registers[OUT_CONFIG0 as usize / 4] = 1 << 3;
        registers[OUT_LINK as usize / 4] = OUT_LINK_PARK;
        let mut state = Self {
            registers,
            ..Self::default()
        };
        state.refresh();
        state
    }

    fn refresh(&mut self) {
        self.registers[IN_INT_STATUS as usize / 4] =
            self.registers[IN_INT_RAW as usize / 4] & self.registers[IN_INT_ENABLE as usize / 4];
        self.registers[OUT_INT_STATUS as usize / 4] =
            self.registers[OUT_INT_RAW as usize / 4] & self.registers[OUT_INT_ENABLE as usize / 4];
        let mut input_status = ((self.input.len().min(63) as u32) << 2) | (1 << 1);
        if self.input.len() >= FIFO_LIMIT {
            input_status |= 1;
            input_status &= !(1 << 1);
        }
        self.registers[IN_FIFO_STATUS as usize / 4] = input_status;
        let mut output_status = ((self.output.len().min(63) as u32) << 2) | (1 << 1);
        if self.output.len() >= FIFO_LIMIT {
            output_status |= 1;
            output_status &= !(1 << 1);
        }
        self.registers[OUT_FIFO_STATUS as usize / 4] = output_status;
    }
}

/// Host-facing ESP32-C6 GDMA channel-0 fixture handle.
#[derive(Clone)]
pub struct EspGdmaHandle {
    state: Arc<Mutex<EspGdmaState>>,
}

impl EspGdmaHandle {
    /// Queues words for a firmware IN-channel FIFO pop sequence.
    pub fn queue_input_words(&self, words: &[u32]) {
        let mut state = self.state.lock().expect("ESP GDMA lock poisoned");
        let mut accepted = false;
        let mut overflow = false;
        for word in words {
            if state.input.len() < FIFO_LIMIT {
                state.input.push_back(*word & 0xfff);
                accepted = true;
            } else {
                overflow = true;
            }
        }
        if accepted {
            state.registers[IN_INT_RAW as usize / 4] |= IN_DONE | IN_SUC_EOF;
        }
        if overflow {
            state.registers[IN_INT_RAW as usize / 4] |= IN_FIFO_OVF;
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

/// Functional ESP32-C6 GDMA channel-0 register block.
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

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("ESP GDMA requires aligned word access"));
        }
        let mut state = self.state.lock().expect("ESP GDMA lock poisoned");
        let value = if offset == IN_FIFO_STATUS || offset == OUT_FIFO_STATUS {
            state.refresh();
            state.registers[offset as usize / 4]
        } else if offset == IN_INT_STATUS || offset == OUT_INT_STATUS {
            state.refresh();
            state.registers[offset as usize / 4]
        } else if offset == IN_POP {
            // RDATA is a read-only latch.  Firmware first writes INFIFO_POP,
            // then reads this register, matching gdma_ll_rx_pop_data().
            state.registers[IN_POP as usize / 4] & IN_POP_DATA_MASK
        } else if offset == OUT_PUSH {
            state.registers[OUT_PUSH as usize / 4] & OUT_PUSH_DATA_MASK
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
        let mut input_word = None;
        let mut output_word = None;
        if offset == IN_POP {
            if value & IN_POP_TRIGGER != 0 {
                let word = state.input.pop_front().unwrap_or_default() & IN_POP_DATA_MASK;
                state.registers[IN_POP as usize / 4] = word;
                input_word = Some(word);
            }
        } else if offset == OUT_PUSH {
            state.registers[OUT_PUSH as usize / 4] = value & OUT_PUSH_DATA_MASK;
            if value & OUT_PUSH_TRIGGER != 0 {
                let word = value & OUT_PUSH_DATA_MASK;
                if state.output.len() < FIFO_LIMIT {
                    state.output.push_back(word);
                    state.registers[OUT_INT_RAW as usize / 4] |= OUT_DONE | OUT_EOF;
                    output_word = Some(word);
                } else {
                    state.registers[OUT_INT_RAW as usize / 4] |= OUT_FIFO_OVF;
                }
            }
        } else {
            match offset {
                IN_INT_ENABLE => state.registers[IN_INT_ENABLE as usize / 4] = value & IN_INT_MASK,
                OUT_INT_ENABLE => {
                    state.registers[OUT_INT_ENABLE as usize / 4] = value & OUT_INT_MASK
                }
                IN_INT_RAW => {
                    // R/WTC/SS: writing one clears a raw interrupt bit.
                    state.registers[IN_INT_RAW as usize / 4] &= !(value & IN_INT_MASK)
                }
                OUT_INT_RAW => state.registers[OUT_INT_RAW as usize / 4] &= !(value & OUT_INT_MASK),
                IN_INT_CLEAR => state.registers[IN_INT_RAW as usize / 4] &= !(value & IN_INT_MASK),
                OUT_INT_CLEAR => {
                    state.registers[OUT_INT_RAW as usize / 4] &= !(value & OUT_INT_MASK)
                }
                IN_CONFIG0 => state.registers[IN_CONFIG0 as usize / 4] = value & IN_CONFIG0_MASK,
                IN_CONFIG1 => state.registers[IN_CONFIG1 as usize / 4] = value & CONFIG1_MASK,
                OUT_CONFIG0 => state.registers[OUT_CONFIG0 as usize / 4] = value & OUT_CONFIG0_MASK,
                OUT_CONFIG1 => state.registers[OUT_CONFIG1 as usize / 4] = value & CONFIG1_MASK,
                IN_LINK => {
                    let park = state.registers[IN_LINK as usize / 4] & IN_LINK_PARK;
                    state.registers[IN_LINK as usize / 4] = (value & IN_LINK_RW_MASK) | park;
                }
                OUT_LINK => {
                    let park = state.registers[OUT_LINK as usize / 4] & OUT_LINK_PARK;
                    state.registers[OUT_LINK as usize / 4] = (value & OUT_LINK_RW_MASK) | park;
                }
                IN_PRIORITY => state.registers[IN_PRIORITY as usize / 4] = value & PRIORITY_MASK,
                OUT_PRIORITY => state.registers[OUT_PRIORITY as usize / 4] = value & PRIORITY_MASK,
                IN_PERIPHERAL | OUT_PERIPHERAL => {
                    state.registers[offset as usize / 4] = value & PERIPHERAL_MASK
                }
                MISC_CONFIG => state.registers[MISC_CONFIG as usize / 4] = value & MISC_CONFIG_MASK,
                DATE => state.registers[DATE as usize / 4] = value,
                // Interrupt status and FIFO/status registers are read-only;
                // reserved and unimplemented channel registers are ignored.
                _ => {}
            }
        }
        state.refresh();
        drop(state);
        if let Some(word) = input_word {
            self.emit(self.input_signal, word, 12, at)?;
        }
        if let Some(word) = output_word {
            self.emit(self.output_signal, word, 9, at)?;
        }
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
