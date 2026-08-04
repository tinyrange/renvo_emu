use super::*;

const REGISTER_BYTES: usize = 0x410;

const IN_DONE: u32 = 1 << 0;
const IN_SUC_EOF: u32 = 1 << 1;
const IN_OVF_L1: u32 = 1 << 6;
const IN_UDF_L1: u32 = 1 << 7;
const OUT_DONE: u32 = 1 << 0;
const OUT_EOF: u32 = 1 << 1;
const OUT_OVF_L1: u32 = 1 << 4;
const OUT_PUSH_STROBE: u32 = 1 << 9;
const IN_POP_STROBE: u32 = 1 << 12;
const FIFO_LIMIT: usize = 64;

/// ESP32-S3 GDMA channel-zero register identifiers.
///
/// The offsets, masks and reset values are taken from Espressif's
/// `gdma_reg.h`.  Keeping the register identity as an enum prevents callers
/// from accidentally treating reserved offsets as valid registers.
#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Esp32s3GdmaRegister {
    InConfig0,
    InConfig1,
    InIntRaw,
    InIntStatus,
    InIntEnable,
    InIntClear,
    InFifoStatus,
    InPop,
    InLink,
    InState,
    InSucEofDescriptorAddress,
    InErrEofDescriptorAddress,
    InDescriptor,
    InDescriptorBefore0,
    InDescriptorBefore1,
    InWeight,
    InPriority,
    InPeripheral,
    OutConfig0,
    OutConfig1,
    OutIntRaw,
    OutIntStatus,
    OutIntEnable,
    OutIntClear,
    OutFifoStatus,
    OutPush,
    OutLink,
    OutState,
    OutEofDescriptorAddress,
    OutEofBeforeDescriptorAddress,
    OutDescriptor,
    OutDescriptorBefore0,
    OutDescriptorBefore1,
    OutWeight,
    OutPriority,
    OutPeripheral,
    MiscConfig,
    InSramSize,
    OutSramSize,
    ExternalMemoryRejectAddress,
    ExternalMemoryRejectStatus,
    ExternalMemoryRejectIntRaw,
    ExternalMemoryRejectIntStatus,
    ExternalMemoryRejectIntEnable,
    ExternalMemoryRejectIntClear,
    Date,
}

impl Esp32s3GdmaRegister {
    /// Returns the byte offset within the channel-zero GDMA block.
    pub const fn offset(self) -> u64 {
        match self {
            Self::InConfig0 => 0x00,
            Self::InConfig1 => 0x04,
            Self::InIntRaw => 0x08,
            Self::InIntStatus => 0x0c,
            Self::InIntEnable => 0x10,
            Self::InIntClear => 0x14,
            Self::InFifoStatus => 0x18,
            Self::InPop => 0x1c,
            Self::InLink => 0x20,
            Self::InState => 0x24,
            Self::InSucEofDescriptorAddress => 0x28,
            Self::InErrEofDescriptorAddress => 0x2c,
            Self::InDescriptor => 0x30,
            Self::InDescriptorBefore0 => 0x34,
            Self::InDescriptorBefore1 => 0x38,
            Self::InWeight => 0x3c,
            Self::InPriority => 0x44,
            Self::InPeripheral => 0x48,
            Self::OutConfig0 => 0x60,
            Self::OutConfig1 => 0x64,
            Self::OutIntRaw => 0x68,
            Self::OutIntStatus => 0x6c,
            Self::OutIntEnable => 0x70,
            Self::OutIntClear => 0x74,
            Self::OutFifoStatus => 0x78,
            Self::OutPush => 0x7c,
            Self::OutLink => 0x80,
            Self::OutState => 0x84,
            Self::OutEofDescriptorAddress => 0x88,
            Self::OutEofBeforeDescriptorAddress => 0x8c,
            Self::OutDescriptor => 0x90,
            Self::OutDescriptorBefore0 => 0x94,
            Self::OutDescriptorBefore1 => 0x98,
            Self::OutWeight => 0x9c,
            Self::OutPriority => 0xa4,
            Self::OutPeripheral => 0xa8,
            Self::MiscConfig => 0x3c8,
            Self::InSramSize => 0x3cc,
            Self::OutSramSize => 0x3d0,
            Self::ExternalMemoryRejectAddress => 0x3f4,
            Self::ExternalMemoryRejectStatus => 0x3f8,
            Self::ExternalMemoryRejectIntRaw => 0x3fc,
            Self::ExternalMemoryRejectIntStatus => 0x400,
            Self::ExternalMemoryRejectIntEnable => 0x404,
            Self::ExternalMemoryRejectIntClear => 0x408,
            Self::Date => 0x40c,
        }
    }

    /// Resolves an aligned byte offset to a channel-zero register.
    pub const fn from_offset(offset: u64) -> Option<Self> {
        match offset {
            0x00 => Some(Self::InConfig0),
            0x04 => Some(Self::InConfig1),
            0x08 => Some(Self::InIntRaw),
            0x0c => Some(Self::InIntStatus),
            0x10 => Some(Self::InIntEnable),
            0x14 => Some(Self::InIntClear),
            0x18 => Some(Self::InFifoStatus),
            0x1c => Some(Self::InPop),
            0x20 => Some(Self::InLink),
            0x24 => Some(Self::InState),
            0x28 => Some(Self::InSucEofDescriptorAddress),
            0x2c => Some(Self::InErrEofDescriptorAddress),
            0x30 => Some(Self::InDescriptor),
            0x34 => Some(Self::InDescriptorBefore0),
            0x38 => Some(Self::InDescriptorBefore1),
            0x3c => Some(Self::InWeight),
            0x44 => Some(Self::InPriority),
            0x48 => Some(Self::InPeripheral),
            0x60 => Some(Self::OutConfig0),
            0x64 => Some(Self::OutConfig1),
            0x68 => Some(Self::OutIntRaw),
            0x6c => Some(Self::OutIntStatus),
            0x70 => Some(Self::OutIntEnable),
            0x74 => Some(Self::OutIntClear),
            0x78 => Some(Self::OutFifoStatus),
            0x7c => Some(Self::OutPush),
            0x80 => Some(Self::OutLink),
            0x84 => Some(Self::OutState),
            0x88 => Some(Self::OutEofDescriptorAddress),
            0x8c => Some(Self::OutEofBeforeDescriptorAddress),
            0x90 => Some(Self::OutDescriptor),
            0x94 => Some(Self::OutDescriptorBefore0),
            0x98 => Some(Self::OutDescriptorBefore1),
            0x9c => Some(Self::OutWeight),
            0xa4 => Some(Self::OutPriority),
            0xa8 => Some(Self::OutPeripheral),
            0x3c8 => Some(Self::MiscConfig),
            0x3cc => Some(Self::InSramSize),
            0x3d0 => Some(Self::OutSramSize),
            0x3f4 => Some(Self::ExternalMemoryRejectAddress),
            0x3f8 => Some(Self::ExternalMemoryRejectStatus),
            0x3fc => Some(Self::ExternalMemoryRejectIntRaw),
            0x400 => Some(Self::ExternalMemoryRejectIntStatus),
            0x404 => Some(Self::ExternalMemoryRejectIntEnable),
            0x408 => Some(Self::ExternalMemoryRejectIntClear),
            0x40c => Some(Self::Date),
            _ => None,
        }
    }

    const fn read_mask(self) -> u32 {
        match self {
            Self::InConfig0 => 0x1f,
            Self::InConfig1 => 0x7fff,
            Self::InIntRaw | Self::InIntStatus | Self::InIntEnable | Self::InIntClear => 0x3ff,
            Self::InFifoStatus => 0x1fff_ffff,
            Self::InPop => 0x0fff,
            Self::InLink => 0x01ff_ffff,
            Self::InState => 0x007f_ffff,
            Self::InSucEofDescriptorAddress
            | Self::InErrEofDescriptorAddress
            | Self::InDescriptor
            | Self::InDescriptorBefore0
            | Self::InDescriptorBefore1 => u32::MAX,
            Self::InWeight => 0x0f00,
            Self::InPriority => 0x000f,
            Self::InPeripheral => 0x003f,
            Self::OutConfig0 => 0x3f,
            Self::OutConfig1 => 0x7000,
            Self::OutIntRaw | Self::OutIntStatus | Self::OutIntEnable | Self::OutIntClear => 0xff,
            Self::OutFifoStatus => 0x07ff_ffff,
            Self::OutPush => 0x01ff,
            Self::OutLink => 0x00ff_ffff,
            Self::OutState => 0x007f_ffff,
            Self::OutEofDescriptorAddress
            | Self::OutEofBeforeDescriptorAddress
            | Self::OutDescriptor
            | Self::OutDescriptorBefore0
            | Self::OutDescriptorBefore1 => u32::MAX,
            Self::OutWeight => 0x0f00,
            Self::OutPriority => 0x000f,
            Self::OutPeripheral => 0x003f,
            Self::MiscConfig => 0x17,
            Self::InSramSize | Self::OutSramSize => 0x7f,
            Self::ExternalMemoryRejectAddress => u32::MAX,
            Self::ExternalMemoryRejectStatus => 0x0fff,
            Self::ExternalMemoryRejectIntRaw
            | Self::ExternalMemoryRejectIntStatus
            | Self::ExternalMemoryRejectIntEnable
            | Self::ExternalMemoryRejectIntClear => 1,
            Self::Date => u32::MAX,
        }
    }

    const fn write_mask(self) -> u32 {
        match self {
            Self::InConfig0 | Self::InConfig1 | Self::InIntRaw | Self::InIntEnable => {
                self.read_mask()
            }
            Self::InIntClear => 0x3ff,
            Self::InPop => IN_POP_STROBE,
            Self::InLink => 0x00ff_ffff,
            Self::InWeight | Self::InPriority | Self::InPeripheral => self.read_mask(),
            Self::OutConfig0 | Self::OutConfig1 | Self::OutIntRaw | Self::OutIntEnable => {
                self.read_mask()
            }
            Self::OutIntClear => 0xff,
            Self::OutPush => OUT_PUSH_STROBE | 0x01ff,
            Self::OutLink => 0x007f_ffff,
            Self::OutWeight | Self::OutPriority | Self::OutPeripheral => self.read_mask(),
            Self::MiscConfig | Self::InSramSize | Self::OutSramSize | Self::Date => {
                self.read_mask()
            }
            Self::ExternalMemoryRejectIntRaw | Self::ExternalMemoryRejectIntEnable => 1,
            Self::ExternalMemoryRejectIntClear => 1,
            Self::InFifoStatus
            | Self::InIntStatus
            | Self::InState
            | Self::InSucEofDescriptorAddress
            | Self::InErrEofDescriptorAddress
            | Self::InDescriptor
            | Self::InDescriptorBefore0
            | Self::InDescriptorBefore1
            | Self::OutFifoStatus
            | Self::OutIntStatus
            | Self::OutState
            | Self::OutEofDescriptorAddress
            | Self::OutEofBeforeDescriptorAddress
            | Self::OutDescriptor
            | Self::OutDescriptorBefore0
            | Self::OutDescriptorBefore1
            | Self::ExternalMemoryRejectAddress
            | Self::ExternalMemoryRejectStatus
            | Self::ExternalMemoryRejectIntStatus => 0,
        }
    }
}

#[derive(Default)]
struct EspGdmaState {
    registers: Vec<u32>,
    input: VecDeque<u32>,
    output: VecDeque<u32>,
}

impl EspGdmaState {
    fn new() -> Self {
        let mut registers = vec![0; REGISTER_BYTES / 4];
        registers[index(Esp32s3GdmaRegister::Date)] = 0x0210_1180;
        registers[index(Esp32s3GdmaRegister::InConfig1)] = 0x000c;
        registers[index(Esp32s3GdmaRegister::InLink)] = 1 << 24;
        registers[index(Esp32s3GdmaRegister::InPop)] = 0x800;
        registers[index(Esp32s3GdmaRegister::InWeight)] = 0x0f00;
        registers[index(Esp32s3GdmaRegister::InPeripheral)] = 0x3f;
        registers[index(Esp32s3GdmaRegister::OutConfig0)] = 1 << 3;
        registers[index(Esp32s3GdmaRegister::OutLink)] = 1 << 23;
        registers[index(Esp32s3GdmaRegister::OutWeight)] = 0x0f00;
        registers[index(Esp32s3GdmaRegister::OutPeripheral)] = 0x3f;
        registers[index(Esp32s3GdmaRegister::InSramSize)] = 14;
        registers[index(Esp32s3GdmaRegister::OutSramSize)] = 14;
        let mut state = Self {
            registers,
            ..Self::default()
        };
        state.refresh();
        state
    }

    fn refresh(&mut self) {
        let in_raw = self.registers[index(Esp32s3GdmaRegister::InIntRaw)] & 0x3ff;
        let in_enable = self.registers[index(Esp32s3GdmaRegister::InIntEnable)] & 0x3ff;
        self.registers[index(Esp32s3GdmaRegister::InIntStatus)] = in_raw & in_enable;
        let out_raw = self.registers[index(Esp32s3GdmaRegister::OutIntRaw)] & 0xff;
        let out_enable = self.registers[index(Esp32s3GdmaRegister::OutIntEnable)] & 0xff;
        self.registers[index(Esp32s3GdmaRegister::OutIntStatus)] = out_raw & out_enable;
        let external_raw =
            self.registers[index(Esp32s3GdmaRegister::ExternalMemoryRejectIntRaw)] & 1;
        let external_enable =
            self.registers[index(Esp32s3GdmaRegister::ExternalMemoryRejectIntEnable)] & 1;
        self.registers[index(Esp32s3GdmaRegister::ExternalMemoryRejectIntStatus)] =
            external_raw & external_enable;
        self.registers[index(Esp32s3GdmaRegister::InFifoStatus)] = fifo_status_in(self.input.len());
        self.registers[index(Esp32s3GdmaRegister::OutFifoStatus)] =
            fifo_status_out(self.output.len());
    }
}

fn index(register: Esp32s3GdmaRegister) -> usize {
    (register.offset() / 4) as usize
}

fn fifo_status_in(len: usize) -> u32 {
    // The fixture models the host-fed queue as GDMA's L1 FIFO.  L2/L3 are
    // empty, as they are until a descriptor engine is attached.
    let mut status = 0x0f00_0000 | (1 << 5) | (1 << 3);
    status |= (len.min(0x3f) as u32) << 6;
    if len == 0 {
        status |= 1 << 1;
    }
    if len >= FIFO_LIMIT {
        status |= 1;
    }
    status
}

fn fifo_status_out(len: usize) -> u32 {
    // The output status has a five-bit L1 count on ESP32-S3.  Keep the
    // fixture capacity at 64 words while exposing the architectural field.
    let mut status = 0x0780_0000 | (1 << 5) | (1 << 3);
    status |= (len.min(0x1f) as u32) << 6;
    if len == 0 {
        status |= 1 << 1;
    }
    if len >= FIFO_LIMIT {
        status |= 1;
    }
    status
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
            state.registers[index(Esp32s3GdmaRegister::InIntRaw)] |= IN_DONE | IN_SUC_EOF;
        }
        if overflow {
            state.registers[index(Esp32s3GdmaRegister::InIntRaw)] |= IN_OVF_L1;
        }
        state.refresh();
    }

    /// Queues peripheral-produced words when channel zero is connected to
    /// the requested native GDMA trigger ID. Returns whether the channel
    /// accepted the peripheral handshake.
    pub fn queue_peripheral_input_words(&self, peripheral: u8, words: &[u32]) -> bool {
        let selected = {
            let state = self.state.lock().expect("ESP GDMA lock poisoned");
            state.registers[index(Esp32s3GdmaRegister::InPeripheral)] & 0x3f
                == u32::from(peripheral)
        };
        if selected {
            self.queue_input_words(words);
        }
        selected
    }

    /// Returns words written by firmware through the OUT-channel push port.
    pub fn take_output_words(&self) -> Vec<u32> {
        let mut state = self.state.lock().expect("ESP GDMA lock poisoned");
        let words = state.output.drain(..).collect();
        state.refresh();
        words
    }

    /// Drains words destined for a peripheral when channel zero is connected
    /// to the requested native GDMA trigger ID.
    pub fn take_peripheral_output_words(&self, peripheral: u8) -> Vec<u32> {
        let selected = {
            let state = self.state.lock().expect("ESP GDMA lock poisoned");
            state.registers[index(Esp32s3GdmaRegister::OutPeripheral)] & 0x3f
                == u32::from(peripheral)
        };
        if selected {
            self.take_output_words()
        } else {
            Vec::new()
        }
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
        let register = Esp32s3GdmaRegister::from_offset(offset).ok_or_else(|| {
            DeviceError::new(format!(
                "{} read at unsupported GDMA offset {offset:#x}",
                self.name
            ))
        })?;
        let mut state = self.state.lock().expect("ESP GDMA lock poisoned");
        if matches!(
            register,
            Esp32s3GdmaRegister::InFifoStatus
                | Esp32s3GdmaRegister::InIntStatus
                | Esp32s3GdmaRegister::OutFifoStatus
                | Esp32s3GdmaRegister::OutIntStatus
        ) {
            state.refresh();
        }
        if register == Esp32s3GdmaRegister::InPop {
            // IN_POP is a data view.  The POP strobe is a write-only bit, so a
            // read must not consume the FIFO entry.
            let word = state.input.front().copied().unwrap_or(0x800) & 0xfff;
            state.registers[index(register)] = word;
            drop(state);
            self.emit(self.input_signal, word, 12, at)?;
            return Ok(u64::from(word));
        }
        let value = state.registers[index(register)] & register.read_mask();
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
        if value > u64::from(u32::MAX) {
            return Err(DeviceError::new(
                "ESP GDMA rejects values wider than 32 bits",
            ));
        }
        let register = Esp32s3GdmaRegister::from_offset(offset).ok_or_else(|| {
            DeviceError::new(format!(
                "{} write at unsupported GDMA offset {offset:#x}",
                self.name
            ))
        })?;
        let write_mask = register.write_mask();
        if write_mask == 0 {
            return Err(DeviceError::new(format!(
                "{} write to read-only GDMA register {register:?}",
                self.name
            )));
        }
        let mut state = self.state.lock().expect("ESP GDMA lock poisoned");
        let value = value as u32;
        let mut signal = None;
        match register {
            Esp32s3GdmaRegister::InPop => {
                if value & IN_POP_STROBE != 0 {
                    let word = state.input.pop_front().unwrap_or(0x800) & 0xfff;
                    state.registers[index(register)] = word;
                    signal = Some((self.input_signal, word, 12));
                    if word == 0x800 && state.input.is_empty() {
                        state.registers[index(Esp32s3GdmaRegister::InIntRaw)] |= IN_UDF_L1;
                    }
                }
            }
            Esp32s3GdmaRegister::OutPush => {
                state.registers[index(register)] = value & 0x01ff;
                if value & OUT_PUSH_STROBE != 0 {
                    if state.output.len() < FIFO_LIMIT {
                        let word = value & 0x01ff;
                        state.output.push_back(word);
                        state.registers[index(Esp32s3GdmaRegister::OutIntRaw)] |=
                            OUT_DONE | OUT_EOF;
                        signal = Some((self.output_signal, word, 9));
                    } else {
                        state.registers[index(Esp32s3GdmaRegister::OutIntRaw)] |= OUT_OVF_L1;
                    }
                }
            }
            Esp32s3GdmaRegister::InIntClear => {
                state.registers[index(Esp32s3GdmaRegister::InIntRaw)] &= !(value & write_mask);
            }
            Esp32s3GdmaRegister::OutIntClear => {
                state.registers[index(Esp32s3GdmaRegister::OutIntRaw)] &= !(value & write_mask);
            }
            Esp32s3GdmaRegister::ExternalMemoryRejectIntClear => {
                state.registers[index(Esp32s3GdmaRegister::ExternalMemoryRejectIntRaw)] &=
                    !(value & write_mask);
            }
            Esp32s3GdmaRegister::InConfig0 if value & 1 != 0 => {
                state.input.clear();
                state.registers[index(register)] = value & write_mask;
            }
            Esp32s3GdmaRegister::OutConfig0 if value & 1 != 0 => {
                state.output.clear();
                state.registers[index(register)] = value & write_mask;
            }
            _ => {
                let slot = &mut state.registers[index(register)];
                *slot = (*slot & !write_mask) | (value & write_mask);
            }
        }
        state.refresh();
        if let Some((signal, word, width)) = signal {
            drop(state);
            self.emit(signal, word, width, at)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_zero_fifo_paths_and_interrupts_are_deterministic() {
        let hub = SignalHub::new();
        let (mut gdma, handle) = EspGdma::new("gdma", "board.gdma", hub.clone()).unwrap();
        handle.queue_input_words(&[0xabc, 0x123]);
        let expected_status: u32 = 0x0f00_0000 | (1 << 5) | (1 << 3) | (2 << 6);
        assert_eq!(
            gdma.read(
                Esp32s3GdmaRegister::InFifoStatus.offset(),
                AccessWidth::Word,
                SimTime::ZERO
            )
            .unwrap(),
            u64::from(expected_status)
        );
        assert_eq!(
            gdma.read(
                Esp32s3GdmaRegister::InPop.offset(),
                AccessWidth::Word,
                SimTime::ZERO
            )
            .unwrap(),
            0xabc
        );
        // Reading the data view is non-destructive; the strobe is bit 12.
        gdma.write(
            Esp32s3GdmaRegister::InPop.offset(),
            AccessWidth::Word,
            u64::from(IN_POP_STROBE),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            gdma.read(
                Esp32s3GdmaRegister::InPop.offset(),
                AccessWidth::Word,
                SimTime::ZERO
            )
            .unwrap(),
            0x123
        );
        assert_eq!(
            gdma.read(
                Esp32s3GdmaRegister::InIntRaw.offset(),
                AccessWidth::Word,
                SimTime::ZERO
            )
            .unwrap(),
            u64::from(IN_DONE | IN_SUC_EOF)
        );
        gdma.write(
            Esp32s3GdmaRegister::InIntEnable.offset(),
            AccessWidth::Word,
            u64::from(IN_DONE | IN_SUC_EOF),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            gdma.read(
                Esp32s3GdmaRegister::InIntStatus.offset(),
                AccessWidth::Word,
                SimTime::ZERO
            )
            .unwrap(),
            u64::from(IN_DONE | IN_SUC_EOF)
        );
        gdma.write(
            Esp32s3GdmaRegister::InIntClear.offset(),
            AccessWidth::Word,
            u64::from(IN_DONE | IN_SUC_EOF),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            gdma.read(
                Esp32s3GdmaRegister::InIntStatus.offset(),
                AccessWidth::Word,
                SimTime::ZERO
            )
            .unwrap(),
            0
        );

        gdma.write(
            Esp32s3GdmaRegister::OutPush.offset(),
            AccessWidth::Word,
            0x355,
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(handle.take_output_words(), vec![0x155]);
        assert!(
            hub.with_registry(|registry| registry.find("board.gdma.out"))
                .is_some()
        );
    }

    #[test]
    fn register_enum_rejects_reserved_offsets_and_preserves_masks() {
        assert_eq!(
            Esp32s3GdmaRegister::from_offset(Esp32s3GdmaRegister::OutPush.offset()),
            Some(Esp32s3GdmaRegister::OutPush)
        );
        assert_eq!(Esp32s3GdmaRegister::from_offset(0x40), None);

        let hub = SignalHub::new();
        let (mut gdma, _) = EspGdma::new("gdma", "board.gdma", hub).unwrap();
        assert_eq!(
            gdma.read(
                Esp32s3GdmaRegister::InConfig1.offset(),
                AccessWidth::Word,
                SimTime::ZERO
            )
            .unwrap(),
            0x0c
        );
        gdma.write(
            Esp32s3GdmaRegister::InConfig1.offset(),
            AccessWidth::Word,
            u64::from(u32::MAX),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            gdma.read(
                Esp32s3GdmaRegister::InConfig1.offset(),
                AccessWidth::Word,
                SimTime::ZERO
            )
            .unwrap(),
            0x7fff
        );
        assert!(gdma.read(0x40, AccessWidth::Word, SimTime::ZERO).is_err());
        assert!(
            gdma.write(
                Esp32s3GdmaRegister::InIntStatus.offset(),
                AccessWidth::Word,
                1,
                SimTime::ZERO
            )
            .is_err()
        );
        gdma.write(
            Esp32s3GdmaRegister::ExternalMemoryRejectIntRaw.offset(),
            AccessWidth::Word,
            1,
            SimTime::ZERO,
        )
        .unwrap();
        gdma.write(
            Esp32s3GdmaRegister::ExternalMemoryRejectIntEnable.offset(),
            AccessWidth::Word,
            1,
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            gdma.read(
                Esp32s3GdmaRegister::ExternalMemoryRejectIntStatus.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap(),
            1
        );
        gdma.write(
            Esp32s3GdmaRegister::ExternalMemoryRejectIntClear.offset(),
            AccessWidth::Word,
            1,
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            gdma.read(
                Esp32s3GdmaRegister::ExternalMemoryRejectIntStatus.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap(),
            0
        );
        assert_eq!(
            gdma.read(
                Esp32s3GdmaRegister::ExternalMemoryRejectIntClear.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap(),
            0
        );
        assert!(
            gdma.write(
                Esp32s3GdmaRegister::Date.offset(),
                AccessWidth::Word,
                1 << 40,
                SimTime::ZERO
            )
            .is_err()
        );
    }
}
