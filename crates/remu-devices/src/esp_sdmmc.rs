use super::*;

/// Native ESP32-S3 SD/MMC register identifiers from Espressif's
/// `sdmmc_reg.h`.  `Dbaddru` and `Idsts` are aliases at the same hardware
/// offset; the associated `IDSTS` constant provides the second datasheet
/// name without introducing an integer register identifier.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u16)]
#[allow(missing_docs)]
pub enum Esp32S3SdmmcRegister {
    Ctrl = 0x00,
    Pwren = 0x04,
    Clkdiv = 0x08,
    Clksrc = 0x0c,
    Clkena = 0x10,
    Tmout = 0x14,
    Ctype = 0x18,
    Blksiz = 0x1c,
    Bytcnt = 0x20,
    Intmask = 0x24,
    Cmdarg = 0x28,
    Cmd = 0x2c,
    Resp0 = 0x30,
    Resp1 = 0x34,
    Resp2 = 0x38,
    Resp3 = 0x3c,
    Mintsts = 0x40,
    Rintsts = 0x44,
    Status = 0x48,
    Fifoth = 0x4c,
    Cdetect = 0x50,
    Wrtprt = 0x54,
    Gpio = 0x58,
    Tcbcnt = 0x5c,
    Tbbcnt = 0x60,
    Debnce = 0x64,
    Usrid = 0x68,
    Verid = 0x6c,
    Hcon = 0x70,
    Uhs = 0x74,
    RstN = 0x78,
    Bmod = 0x80,
    Pldmnd = 0x84,
    Dbaddr = 0x88,
    Dbaddru = 0x8c,
    Idinten = 0x90,
    Dscaddr = 0x94,
    Dscaddrl = 0x98,
    Dscaddru = 0x9c,
    Bufaddrl = 0xa0,
    Bufaddru = 0xa4,
    Cardthrctl = 0x100,
    BackendPower = 0x104,
    UhsExt = 0x108,
    EmmcDdr = 0x10c,
    EnableShift = 0x110,
    Clock = 0x800,
    /// Functional host FIFO endpoint used by the emulator's card model.
    Fifo = 0x200,
}

impl Esp32S3SdmmcRegister {
    /// Datasheet spelling for the DBADDRU/IDSTS register alias.
    pub const IDSTS: Self = Self::Dbaddru;

    /// Returns the byte offset of this register in the SD/MMC page.
    pub const fn offset(self) -> u64 {
        self as u64
    }

    /// Resolves a byte offset in the native SD/MMC page to its named ID.
    /// Reserved holes return `None`.
    pub const fn from_offset(offset: u64) -> Option<Self> {
        Some(match offset {
            0x00 => Self::Ctrl,
            0x04 => Self::Pwren,
            0x08 => Self::Clkdiv,
            0x0c => Self::Clksrc,
            0x10 => Self::Clkena,
            0x14 => Self::Tmout,
            0x18 => Self::Ctype,
            0x1c => Self::Blksiz,
            0x20 => Self::Bytcnt,
            0x24 => Self::Intmask,
            0x28 => Self::Cmdarg,
            0x2c => Self::Cmd,
            0x30 => Self::Resp0,
            0x34 => Self::Resp1,
            0x38 => Self::Resp2,
            0x3c => Self::Resp3,
            0x40 => Self::Mintsts,
            0x44 => Self::Rintsts,
            0x48 => Self::Status,
            0x4c => Self::Fifoth,
            0x50 => Self::Cdetect,
            0x54 => Self::Wrtprt,
            0x58 => Self::Gpio,
            0x5c => Self::Tcbcnt,
            0x60 => Self::Tbbcnt,
            0x64 => Self::Debnce,
            0x68 => Self::Usrid,
            0x6c => Self::Verid,
            0x70 => Self::Hcon,
            0x74 => Self::Uhs,
            0x78 => Self::RstN,
            0x80 => Self::Bmod,
            0x84 => Self::Pldmnd,
            0x88 => Self::Dbaddr,
            0x8c => Self::Dbaddru,
            0x90 => Self::Idinten,
            0x94 => Self::Dscaddr,
            0x98 => Self::Dscaddrl,
            0x9c => Self::Dscaddru,
            0xa0 => Self::Bufaddrl,
            0xa4 => Self::Bufaddru,
            0x100 => Self::Cardthrctl,
            0x104 => Self::BackendPower,
            0x108 => Self::UhsExt,
            0x10c => Self::EmmcDdr,
            0x110 => Self::EnableShift,
            0x200 => Self::Fifo,
            0x800 => Self::Clock,
            _ => return None,
        })
    }

    /// Bits that are architecturally visible when the register is read.
    /// The Espressif header publishes offsets and interrupt bits, while the
    /// remaining DesignWare fields are intentionally modelled conservatively.
    const fn read_mask(self) -> u32 {
        match self {
            Self::Mintsts | Self::Rintsts => SDMMC_INTERRUPT_MASK,
            Self::Cdetect | Self::Wrtprt => 1,
            Self::Resp0 | Self::Resp1 | Self::Resp2 | Self::Resp3 => u32::MAX,
            Self::Verid | Self::Hcon | Self::Tcbcnt | Self::Tbbcnt => u32::MAX,
            _ => u32::MAX,
        }
    }

    /// Bits accepted by a normal register write.  Read-only registers are
    /// ignored by the device write path; RINTSTS is a functional W1C latch.
    const fn write_mask(self) -> u32 {
        match self {
            Self::Resp0
            | Self::Resp1
            | Self::Resp2
            | Self::Resp3
            | Self::Mintsts
            | Self::Status
            | Self::Cdetect
            | Self::Wrtprt
            | Self::Tcbcnt
            | Self::Tbbcnt
            | Self::Verid
            | Self::Hcon
            | Self::Dbaddru => 0,
            Self::Intmask => SDMMC_INTERRUPT_MASK,
            Self::Rintsts => SDMMC_INTERRUPT_MASK,
            Self::Pwren | Self::Clksrc | Self::Ctype => 0x3,
            Self::Blksiz => 0xffff,
            Self::Fifoth => 0x03ff_03ff,
            Self::RstN => 0x7,
            Self::Idinten => SDMMC_IDMAC_INTERRUPT_MASK,
            Self::Pldmnd => 1,
            _ => u32::MAX,
        }
    }
}

const CTRL: Esp32S3SdmmcRegister = Esp32S3SdmmcRegister::Ctrl;
const BLKSIZ: Esp32S3SdmmcRegister = Esp32S3SdmmcRegister::Blksiz;
const BYTCNT: Esp32S3SdmmcRegister = Esp32S3SdmmcRegister::Bytcnt;
const INTMASK: Esp32S3SdmmcRegister = Esp32S3SdmmcRegister::Intmask;
const CMDARG: Esp32S3SdmmcRegister = Esp32S3SdmmcRegister::Cmdarg;
const CMD: Esp32S3SdmmcRegister = Esp32S3SdmmcRegister::Cmd;
const RESP0: Esp32S3SdmmcRegister = Esp32S3SdmmcRegister::Resp0;
const RESP1: Esp32S3SdmmcRegister = Esp32S3SdmmcRegister::Resp1;
const RESP2: Esp32S3SdmmcRegister = Esp32S3SdmmcRegister::Resp2;
const RESP3: Esp32S3SdmmcRegister = Esp32S3SdmmcRegister::Resp3;
const MINTSTS: Esp32S3SdmmcRegister = Esp32S3SdmmcRegister::Mintsts;
const RINTSTS: Esp32S3SdmmcRegister = Esp32S3SdmmcRegister::Rintsts;
const STATUS: Esp32S3SdmmcRegister = Esp32S3SdmmcRegister::Status;
const CDETECT: Esp32S3SdmmcRegister = Esp32S3SdmmcRegister::Cdetect;
const TCBCNT: Esp32S3SdmmcRegister = Esp32S3SdmmcRegister::Tcbcnt;
const TBBCNT: Esp32S3SdmmcRegister = Esp32S3SdmmcRegister::Tbbcnt;
const VERID: Esp32S3SdmmcRegister = Esp32S3SdmmcRegister::Verid;
const HCON: Esp32S3SdmmcRegister = Esp32S3SdmmcRegister::Hcon;
const RST_N: Esp32S3SdmmcRegister = Esp32S3SdmmcRegister::RstN;
const FIFO: Esp32S3SdmmcRegister = Esp32S3SdmmcRegister::Fifo;

const CMD_INDEX_MASK: u32 = 0x3f;
const CMD_DATA_EXPECTED: u32 = 1 << 9;
const CMD_WRITE: u32 = 1 << 10;
const CMD_START: u32 = 1 << 31;

const INT_CD: u32 = 1 << 0;
const INT_CMD_DONE: u32 = 1 << 2;
const INT_DATA_OVER: u32 = 1 << 3;
const INT_TXDR: u32 = 1 << 4;
const INT_RXDR: u32 = 1 << 5;
const INT_RTO: u32 = 1 << 8;
const SDMMC_INTERRUPT_MASK: u32 = 0x0003_ffff;
const SDMMC_IDMAC_INTERRUPT_MASK: u32 = 0x0000_0337;

const BLOCK_BYTES: usize = 512;
const DEFAULT_CARD_BYTES: usize = 128 * BLOCK_BYTES;

/// Host-side endpoint for the functional ESP32-S3 SD/MMC card model.
#[derive(Clone)]
pub struct Esp32S3SdmmcHandle {
    state: Rc<RefCell<Esp32S3SdmmcState>>,
}

impl Esp32S3SdmmcHandle {
    /// Sets card-detect state. A change latches the native card-detect IRQ.
    pub fn set_card_present(&self, present: bool) {
        let mut state = self.state.borrow_mut();
        if state.card_present != present {
            state.card_present = present;
            state.registers.insert(CDETECT, if present { 0 } else { 1 });
            state.raw_interrupts |= INT_CD;
            state.refresh_interrupt_status();
        }
    }

    /// Loads bytes into the deterministic card backing store at a byte offset.
    pub fn load_card(&self, offset: usize, bytes: impl AsRef<[u8]>) {
        let bytes = bytes.as_ref();
        let mut state = self.state.borrow_mut();
        let end = offset.saturating_add(bytes.len());
        if state.card.len() < end {
            state.card.resize(end, 0);
        }
        state.card[offset..end].copy_from_slice(bytes);
    }

    /// Drains completed host-to-card block writes as `(LBA, bytes)` records.
    pub fn take_written_blocks(&self) -> Vec<(u32, Vec<u8>)> {
        std::mem::take(&mut self.state.borrow_mut().written_blocks)
    }

    /// Drains words emitted through the SDMMC data FIFO.
    pub fn take_data_words(&self) -> Vec<u32> {
        self.state.borrow_mut().data_trace.drain(..).collect()
    }
}

struct Esp32S3SdmmcState {
    registers: BTreeMap<Esp32S3SdmmcRegister, u32>,
    raw_interrupts: u32,
    card_present: bool,
    card: Vec<u8>,
    rx_fifo: VecDeque<u32>,
    tx_fifo: VecDeque<u32>,
    pending_write: Option<(u32, usize)>,
    written_blocks: Vec<(u32, Vec<u8>)>,
    data_trace: VecDeque<u32>,
    hub: SignalHub,
    card_signal: SignalId,
    data_signal: SignalId,
}

impl Esp32S3SdmmcState {
    fn new(hub: SignalHub, card_signal: SignalId, data_signal: SignalId) -> Self {
        let mut registers = BTreeMap::new();
        registers.insert(BLKSIZ, BLOCK_BYTES as u32);
        registers.insert(CDETECT, 0);
        registers.insert(VERID, 0x3430_322a);
        registers.insert(HCON, 0x0000_0001);
        Self {
            registers,
            raw_interrupts: 0,
            card_present: true,
            card: vec![0; DEFAULT_CARD_BYTES],
            rx_fifo: VecDeque::new(),
            tx_fifo: VecDeque::new(),
            pending_write: None,
            written_blocks: Vec::new(),
            data_trace: VecDeque::new(),
            hub,
            card_signal,
            data_signal,
        }
    }

    fn refresh_interrupt_status(&mut self) {
        self.registers
            .insert(MINTSTS, self.raw_interrupts & self.register(INTMASK));
    }

    fn register(&self, register: Esp32S3SdmmcRegister) -> u32 {
        self.registers.get(&register).copied().unwrap_or_default()
    }

    fn publish(&self, signal: SignalId, value: u32, at: SimTime) -> Result<(), DeviceError> {
        self.hub
            .set(
                signal,
                SignalValue::from_u64(u64::from(value), 32)
                    .expect("fixed SDMMC signal width is valid"),
                at,
            )
            .map_err(|error| DeviceError::new(error.to_string()))
    }

    fn status(&self) -> u32 {
        let mut value = 0;
        if !self.rx_fifo.is_empty() {
            value |= 1 << 0;
        }
        if self.tx_fifo.len() < 128 {
            value |= 1 << 1;
        }
        if self.rx_fifo.is_empty() {
            value |= 1 << 2;
        }
        if self.rx_fifo.len() >= 128 {
            value |= 1 << 3;
        }
        if self.card_present {
            value |= 1 << 8;
        }
        if self.pending_write.is_some() {
            value |= 1 << 10;
        }
        value | ((self.rx_fifo.len().min(0x1fff) as u32) << 17)
    }

    fn clear_fifos(&mut self) {
        self.rx_fifo.clear();
        self.tx_fifo.clear();
        self.pending_write = None;
        self.raw_interrupts &= !(INT_RXDR | INT_TXDR | INT_DATA_OVER);
        self.refresh_interrupt_status();
    }

    fn card_bytes(&self, block: u32, count: usize) -> Vec<u8> {
        let start = usize::try_from(block)
            .unwrap_or(usize::MAX)
            .saturating_mul(BLOCK_BYTES);
        let mut bytes = vec![0; count];
        if start < self.card.len() {
            let available = count.min(self.card.len() - start);
            bytes[..available].copy_from_slice(&self.card[start..start + available]);
        }
        bytes
    }

    fn start_read(&mut self, block: u32, count: usize, at: SimTime) -> Result<(), DeviceError> {
        self.rx_fifo.clear();
        for chunk in self.card_bytes(block, count).chunks(4) {
            let mut word = [0_u8; 4];
            word[..chunk.len()].copy_from_slice(chunk);
            self.rx_fifo.push_back(u32::from_le_bytes(word));
        }
        self.raw_interrupts |= INT_RXDR | INT_DATA_OVER;
        self.refresh_interrupt_status();
        self.publish(self.card_signal, block, at)
    }

    fn finish_write(&mut self, block: u32, count: usize, at: SimTime) -> Result<(), DeviceError> {
        let mut bytes = Vec::with_capacity(self.tx_fifo.len() * 4);
        for word in self.tx_fifo.drain(..) {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        bytes.truncate(count);
        let start = usize::try_from(block)
            .unwrap_or(usize::MAX)
            .saturating_mul(BLOCK_BYTES);
        let end = start.saturating_add(bytes.len());
        if self.card.len() < end {
            self.card.resize(end, 0);
        }
        self.card[start..end].copy_from_slice(&bytes);
        self.written_blocks.push((block, bytes));
        self.pending_write = None;
        self.raw_interrupts = (self.raw_interrupts & !INT_TXDR) | INT_DATA_OVER;
        self.refresh_interrupt_status();
        self.publish(self.card_signal, block, at)
    }

    fn push_fifo(&mut self, word: u32, at: SimTime) -> Result<(), DeviceError> {
        self.tx_fifo.push_back(word);
        self.data_trace.push_back(word);
        self.publish(self.data_signal, word, at)?;
        if let Some((block, count)) = self.pending_write {
            if self.tx_fifo.len() * 4 >= count {
                self.finish_write(block, count, at)?;
            }
        }
        Ok(())
    }

    fn pop_fifo(&mut self, at: SimTime) -> Result<u32, DeviceError> {
        let word = self.rx_fifo.pop_front().unwrap_or_default();
        self.data_trace.push_back(word);
        self.publish(self.data_signal, word, at)?;
        if self.rx_fifo.is_empty() {
            self.raw_interrupts &= !INT_RXDR;
            self.refresh_interrupt_status();
        }
        Ok(word)
    }

    fn execute_command(&mut self, command: u32, at: SimTime) -> Result<(), DeviceError> {
        if command & CMD_START == 0 {
            return Ok(());
        }
        let index = command & CMD_INDEX_MASK;
        self.registers.insert(CMD, command & !CMD_START);
        self.registers.insert(TCBCNT, 0);
        self.registers.insert(TBBCNT, 0);
        if !self.card_present {
            self.raw_interrupts |= INT_RTO;
            self.refresh_interrupt_status();
            return Ok(());
        }

        let argument = self.register(CMDARG);
        let response = match index {
            0 => 0,
            8 => 0x0000_01aa,
            41 => 0xc0ff_8000,
            55 => 0,
            2 => 0x1234_5678,
            9 => 0x400e_0032,
            10 => 0x5245_4d55,
            _ => 0,
        };
        self.registers.insert(RESP0, response);
        self.registers
            .insert(RESP1, if index == 2 { 0x9abc_def0 } else { 0 });
        self.registers
            .insert(RESP2, if index == 2 { 0x1357_9bdf } else { 0 });
        self.registers
            .insert(RESP3, if index == 2 { 0x2468_ace0 } else { 0 });
        self.raw_interrupts |= INT_CMD_DONE;

        if command & CMD_DATA_EXPECTED != 0 {
            let block_size = usize::try_from(self.register(BLKSIZ)).unwrap_or(BLOCK_BYTES);
            let configured_bytes = usize::try_from(self.register(BYTCNT)).unwrap_or_default();
            let count = if configured_bytes == 0 {
                block_size.max(1)
            } else {
                configured_bytes
            };
            let blocks = argument;
            if command & CMD_WRITE != 0 {
                self.tx_fifo.clear();
                self.pending_write = Some((blocks, count));
                self.raw_interrupts |= INT_TXDR;
            } else {
                self.start_read(blocks, count, at)?;
            }
            self.registers.insert(TCBCNT, count as u32);
        }
        self.refresh_interrupt_status();
        Ok(())
    }

    fn reset(&mut self) {
        self.registers.clear();
        self.registers.insert(BLKSIZ, BLOCK_BYTES as u32);
        self.registers
            .insert(CDETECT, if self.card_present { 0 } else { 1 });
        self.registers.insert(VERID, 0x3430_322a);
        self.registers.insert(HCON, 0x0000_0001);
        self.raw_interrupts = 0;
        self.clear_fifos();
    }
}

/// Functional ESP32-S3 SD/MMC host controller.
///
/// The model follows Espressif's native `sdmmc_reg.h` offsets. It provides
/// deterministic card-detect, command/response, block read/write, FIFO and
/// interrupt behavior through a bounded host-backed card image. It does not
/// claim SD electrical signaling, CRC timing, multi-card arbitration, SDIO
/// function protocol, or descriptor-level IDMAC execution.
pub struct Esp32S3Sdmmc {
    name: String,
    state: Rc<RefCell<Esp32S3SdmmcState>>,
}

impl Esp32S3Sdmmc {
    /// Creates the native SDMMC register page and host card endpoint.
    pub fn new(
        name: impl Into<String>,
        hub: SignalHub,
    ) -> Result<(Self, Esp32S3SdmmcHandle), SignalError> {
        let card_signal = hub.declare(
            "board.esp32s3.sdmmc.card",
            SignalValue::from_u64(0, 32)?,
            Some("ESP32-S3 SD/MMC card block index".to_owned()),
        )?;
        let data_signal = hub.declare(
            "board.esp32s3.sdmmc.data",
            SignalValue::from_u64(0, 32)?,
            Some("ESP32-S3 SD/MMC FIFO word".to_owned()),
        )?;
        let state = Rc::new(RefCell::new(Esp32S3SdmmcState::new(
            hub,
            card_signal,
            data_signal,
        )));
        Ok((
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Esp32S3SdmmcHandle { state },
        ))
    }
}

impl Device for Esp32S3Sdmmc {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP32-S3 SDMMC requires aligned word access",
            ));
        }
        let register = Esp32S3SdmmcRegister::from_offset(offset).ok_or_else(|| {
            DeviceError::new(format!(
                "unsupported ESP32-S3 SDMMC register offset {offset:#x}"
            ))
        })?;
        let mut state = self.state.borrow_mut();
        let value = match register {
            MINTSTS => state.raw_interrupts & state.register(INTMASK),
            RINTSTS => state.raw_interrupts,
            STATUS => state.status(),
            CDETECT => {
                if state.card_present {
                    0
                } else {
                    1
                }
            }
            FIFO => state.pop_fifo(at)?,
            _ => state.register(register) & register.read_mask(),
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
            return Err(DeviceError::new(
                "ESP32-S3 SDMMC requires aligned word access",
            ));
        }
        let register = Esp32S3SdmmcRegister::from_offset(offset).ok_or_else(|| {
            DeviceError::new(format!(
                "unsupported ESP32-S3 SDMMC register offset {offset:#x}"
            ))
        })?;
        let value = u32::try_from(value).map_err(|_| {
            DeviceError::new(format!(
                "ESP32-S3 SDMMC word write exceeds 32 bits: {value:#x}"
            ))
        })?;
        let mut state = self.state.borrow_mut();
        match register {
            FIFO => state.push_fifo(value, at)?,
            CTRL => {
                if value & 1 != 0 {
                    state.reset();
                } else if value & 2 != 0 {
                    state.clear_fifos();
                }
                state.registers.insert(CTRL, value & CTRL.write_mask() & !7);
            }
            RINTSTS => {
                state.raw_interrupts &= !(value & RINTSTS.write_mask());
                state.refresh_interrupt_status();
            }
            INTMASK => {
                state
                    .registers
                    .insert(INTMASK, value & INTMASK.write_mask());
                state.refresh_interrupt_status();
            }
            CMD => {
                state.registers.insert(CMD, value & CMD.write_mask());
                state.execute_command(value, at)?;
            }
            RST_N if value == 0 => state.reset(),
            CDETECT
            | STATUS
            | MINTSTS
            | RESP0
            | RESP1
            | RESP2
            | RESP3
            | Esp32S3SdmmcRegister::Wrtprt
            | TCBCNT
            | TBBCNT
            | VERID
            | HCON
            | Esp32S3SdmmcRegister::Dbaddru => {}
            _ => {
                let mask = register.write_mask();
                state.registers.insert(register, value & mask);
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state.borrow_mut().reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_read_and_write_use_native_fifo_and_interrupts() {
        let hub = SignalHub::new();
        let (mut sdmmc, handle) = Esp32S3Sdmmc::new("sdmmc", hub).unwrap();
        handle.load_card(2 * BLOCK_BYTES, [0x10, 0x20, 0x30, 0x40]);
        sdmmc
            .write(
                INTMASK.offset(),
                AccessWidth::Word,
                (INT_CMD_DONE | INT_RXDR | INT_DATA_OVER) as u64,
                SimTime::ZERO,
            )
            .unwrap();
        sdmmc
            .write(CMDARG.offset(), AccessWidth::Word, 2, SimTime::ZERO)
            .unwrap();
        sdmmc
            .write(BYTCNT.offset(), AccessWidth::Word, 4, SimTime::ZERO)
            .unwrap();
        sdmmc
            .write(
                CMD.offset(),
                AccessWidth::Word,
                u64::from(CMD_START | CMD_DATA_EXPECTED | 17),
                SimTime::from_ticks(1),
            )
            .unwrap();
        assert_eq!(
            sdmmc
                .read(FIFO.offset(), AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            0x4030_2010
        );
        assert_eq!(
            sdmmc
                .read(MINTSTS.offset(), AccessWidth::Word, SimTime::ZERO)
                .unwrap()
                & u64::from(INT_CMD_DONE | INT_DATA_OVER),
            u64::from(INT_CMD_DONE | INT_DATA_OVER)
        );

        sdmmc
            .write(
                RINTSTS.offset(),
                AccessWidth::Word,
                u64::from(u32::MAX),
                SimTime::ZERO,
            )
            .unwrap();
        sdmmc
            .write(CMDARG.offset(), AccessWidth::Word, 3, SimTime::ZERO)
            .unwrap();
        sdmmc
            .write(BYTCNT.offset(), AccessWidth::Word, 4, SimTime::ZERO)
            .unwrap();
        sdmmc
            .write(
                CMD.offset(),
                AccessWidth::Word,
                u64::from(CMD_START | CMD_DATA_EXPECTED | CMD_WRITE | 24),
                SimTime::from_ticks(2),
            )
            .unwrap();
        sdmmc
            .write(
                FIFO.offset(),
                AccessWidth::Word,
                0xdead_beef,
                SimTime::from_ticks(3),
            )
            .unwrap();
        assert_eq!(
            handle.take_written_blocks(),
            vec![(3, vec![0xef, 0xbe, 0xad, 0xde])]
        );
    }

    #[test]
    fn card_detect_and_version_are_native() {
        let hub = SignalHub::new();
        let (mut sdmmc, handle) = Esp32S3Sdmmc::new("sdmmc", hub).unwrap();
        assert_eq!(
            sdmmc
                .read(CDETECT.offset(), AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            0
        );
        assert_eq!(
            sdmmc
                .read(VERID.offset(), AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            0x3430_322a
        );
        handle.set_card_present(false);
        assert_eq!(
            sdmmc
                .read(CDETECT.offset(), AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            1
        );
        sdmmc
            .write(
                CMD.offset(),
                AccessWidth::Word,
                u64::from(CMD_START | 0),
                SimTime::ZERO,
            )
            .unwrap();
        assert_ne!(
            sdmmc
                .read(RINTSTS.offset(), AccessWidth::Word, SimTime::ZERO)
                .unwrap()
                & u64::from(INT_RTO),
            0
        );
    }

    #[test]
    fn register_enum_covers_native_aliases_and_reset_semantics() {
        assert_eq!(Esp32S3SdmmcRegister::Ctrl.offset(), 0x00);
        assert_eq!(Esp32S3SdmmcRegister::Clock.offset(), 0x800);
        assert_eq!(Esp32S3SdmmcRegister::IDSTS, Esp32S3SdmmcRegister::Dbaddru);

        let hub = SignalHub::new();
        let (mut sdmmc, _) = Esp32S3Sdmmc::new("sdmmc", hub).unwrap();
        sdmmc
            .write(
                Esp32S3SdmmcRegister::Intmask.offset(),
                AccessWidth::Word,
                u64::from(u32::MAX),
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            sdmmc
                .read(
                    Esp32S3SdmmcRegister::Intmask.offset(),
                    AccessWidth::Word,
                    SimTime::ZERO,
                )
                .unwrap(),
            u64::from(SDMMC_INTERRUPT_MASK)
        );
        sdmmc
            .write(
                Esp32S3SdmmcRegister::Blksiz.offset(),
                AccessWidth::Word,
                u64::from(u32::MAX),
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            sdmmc
                .read(
                    Esp32S3SdmmcRegister::Blksiz.offset(),
                    AccessWidth::Word,
                    SimTime::ZERO,
                )
                .unwrap(),
            0xffff
        );
        sdmmc
            .write(
                Esp32S3SdmmcRegister::Ctrl.offset(),
                AccessWidth::Word,
                0x7,
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            sdmmc
                .read(
                    Esp32S3SdmmcRegister::Ctrl.offset(),
                    AccessWidth::Word,
                    SimTime::ZERO,
                )
                .unwrap(),
            0
        );
    }

    #[test]
    fn register_access_rejects_reserved_offsets_and_wide_words() {
        let hub = SignalHub::new();
        let (mut sdmmc, _) = Esp32S3Sdmmc::new("sdmmc", hub).unwrap();
        assert!(sdmmc.read(0x7c, AccessWidth::Word, SimTime::ZERO).is_err());
        assert!(
            sdmmc
                .write(
                    Esp32S3SdmmcRegister::Cmdarg.offset(),
                    AccessWidth::Word,
                    1_u64 << 32,
                    SimTime::ZERO,
                )
                .is_err()
        );
    }
}
