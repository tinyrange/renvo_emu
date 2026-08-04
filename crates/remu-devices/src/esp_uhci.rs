use super::{
    AccessWidth, Arc, Device, DeviceError, EspGdmaHandle, Mutex, ResetKind, SimTime, UartHandle,
    VecDeque,
};

const UHCI_REGISTER_WORDS: usize = 0x88 / 4;
const UHCI_INTERRUPT_MASK: u32 = 0x01ff;
const UHCI_GDMA_TRIGGER: u8 = 2;
const UHCI_CONF0_RESET: u32 = 0x06e0;
const UHCI_CONF1_RESET: u32 = 0x0033;
const UHCI_ESCAPE_CONF_RESET: u32 = 0x0033;
const UHCI_HUNG_CONF_RESET: u32 = 0x0081_0810;
const UHCI_DATE_RESET: u32 = 0x0201_0090;

const RX_START_INTERRUPT: u32 = 1 << 0;
const TX_START_INTERRUPT: u32 = 1 << 1;
const SEND_SINGLE_INTERRUPT: u32 = 1 << 4;
const SEND_ALWAYS_INTERRUPT: u32 = 1 << 5;
const OUTLINK_ERROR_INTERRUPT: u32 = 1 << 6;

/// Native ESP32-S3 UHCI register identifiers.
#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Esp32S3UhciRegister {
    Conf0,
    IntRaw,
    IntStatus,
    IntEnable,
    IntClear,
    AppIntSet,
    Conf1,
    State0,
    State1,
    EscapeConf,
    HungConf,
    AckNum,
    RxHead,
    QuickSent,
    QuickData { slot: u8, word: u8 },
    EscapeSequence(u8),
    PacketThreshold,
    Date,
}

impl Esp32S3UhciRegister {
    /// Returns the byte offset within a UHCI register page.
    pub const fn offset(self) -> u64 {
        match self {
            Self::Conf0 => 0x00,
            Self::IntRaw => 0x04,
            Self::IntStatus => 0x08,
            Self::IntEnable => 0x0c,
            Self::IntClear => 0x10,
            Self::AppIntSet => 0x14,
            Self::Conf1 => 0x18,
            Self::State0 => 0x1c,
            Self::State1 => 0x20,
            Self::EscapeConf => 0x24,
            Self::HungConf => 0x28,
            Self::AckNum => 0x2c,
            Self::RxHead => 0x30,
            Self::QuickSent => 0x34,
            Self::QuickData { slot, word } => 0x38 + slot as u64 * 8 + word as u64 * 4,
            Self::EscapeSequence(sequence) => 0x70 + sequence as u64 * 4,
            Self::PacketThreshold => 0x80,
            Self::Date => 0x84,
        }
    }

    /// Resolves one aligned vendor register offset.
    pub fn from_offset(offset: u64) -> Option<Self> {
        if !offset.is_multiple_of(4) {
            return None;
        }
        match offset {
            0x00 => Some(Self::Conf0),
            0x04 => Some(Self::IntRaw),
            0x08 => Some(Self::IntStatus),
            0x0c => Some(Self::IntEnable),
            0x10 => Some(Self::IntClear),
            0x14 => Some(Self::AppIntSet),
            0x18 => Some(Self::Conf1),
            0x1c => Some(Self::State0),
            0x20 => Some(Self::State1),
            0x24 => Some(Self::EscapeConf),
            0x28 => Some(Self::HungConf),
            0x2c => Some(Self::AckNum),
            0x30 => Some(Self::RxHead),
            0x34 => Some(Self::QuickSent),
            0x38..=0x6c => {
                let index =
                    u8::try_from((offset - 0x38) / 4).expect("UHCI quick-data range fits u8");
                Some(Self::QuickData {
                    slot: index / 2,
                    word: index % 2,
                })
            }
            0x70..=0x7c => Some(Self::EscapeSequence(
                u8::try_from((offset - 0x70) / 4).expect("four UHCI escape sequences fit u8"),
            )),
            0x80 => Some(Self::PacketThreshold),
            0x84 => Some(Self::Date),
            _ => None,
        }
    }

    /// Bits returned by native reads.
    pub const fn read_mask(self) -> u32 {
        match self {
            Self::Conf0 | Self::PacketThreshold => 0x1fff,
            Self::IntRaw | Self::IntStatus | Self::IntEnable => UHCI_INTERRUPT_MASK,
            Self::IntClear | Self::AppIntSet => 0,
            Self::Conf1 => 0x01bf,
            Self::State0 => 0x003f,
            Self::State1 | Self::AckNum => 0x0007,
            Self::EscapeConf | Self::QuickSent => 0x00ff,
            Self::HungConf | Self::EscapeSequence(_) => 0x00ff_ffff,
            Self::RxHead | Self::QuickData { .. } | Self::Date => u32::MAX,
        }
    }

    /// Bits accepted by native writes.
    pub const fn write_mask(self) -> u32 {
        match self {
            Self::Conf0 | Self::PacketThreshold => 0x1fff,
            Self::IntRaw => 0x0180,
            Self::IntStatus | Self::State0 | Self::State1 | Self::RxHead => 0,
            Self::IntEnable | Self::IntClear => UHCI_INTERRUPT_MASK,
            Self::AppIntSet => 0x0003,
            Self::Conf1 => 0x01bf,
            Self::EscapeConf | Self::QuickSent => 0x00ff,
            Self::HungConf | Self::EscapeSequence(_) => 0x00ff_ffff,
            Self::AckNum => 0x000f,
            Self::QuickData { .. } | Self::Date => u32::MAX,
        }
    }
}

fn register_index(register: Esp32S3UhciRegister) -> usize {
    (register.offset() / 4) as usize
}

struct Esp32S3UhciState {
    registers: [u32; UHCI_REGISTER_WORDS],
    uarts: [UartHandle; 3],
    received_packets: VecDeque<Vec<u8>>,
}

impl Esp32S3UhciState {
    fn new(uarts: [UartHandle; 3]) -> Self {
        let mut registers = [0; UHCI_REGISTER_WORDS];
        registers[register_index(Esp32S3UhciRegister::Conf0)] = UHCI_CONF0_RESET;
        registers[register_index(Esp32S3UhciRegister::Conf1)] = UHCI_CONF1_RESET;
        registers[register_index(Esp32S3UhciRegister::EscapeConf)] = UHCI_ESCAPE_CONF_RESET;
        registers[register_index(Esp32S3UhciRegister::HungConf)] = UHCI_HUNG_CONF_RESET;
        registers[register_index(Esp32S3UhciRegister::EscapeSequence(0))] = 0x00dc_dbc0;
        registers[register_index(Esp32S3UhciRegister::EscapeSequence(1))] = 0x00dd_dbdb;
        registers[register_index(Esp32S3UhciRegister::EscapeSequence(2))] = 0x00de_db11;
        registers[register_index(Esp32S3UhciRegister::EscapeSequence(3))] = 0x00df_db13;
        registers[register_index(Esp32S3UhciRegister::PacketThreshold)] = 0x80;
        registers[register_index(Esp32S3UhciRegister::Date)] = UHCI_DATE_RESET;
        let mut state = Self {
            registers,
            uarts,
            received_packets: VecDeque::new(),
        };
        state.refresh_interrupt_status();
        state
    }

    fn register(&self, register: Esp32S3UhciRegister) -> u32 {
        self.registers[register_index(register)]
    }

    fn set_register(&mut self, register: Esp32S3UhciRegister, value: u32) {
        self.registers[register_index(register)] = value;
    }

    fn refresh_interrupt_status(&mut self) {
        let status = self.register(Esp32S3UhciRegister::IntRaw)
            & self.register(Esp32S3UhciRegister::IntEnable)
            & UHCI_INTERRUPT_MASK;
        self.set_register(Esp32S3UhciRegister::IntStatus, status);
    }

    fn set_raw_interrupt(&mut self, mask: u32) {
        let raw = self.register(Esp32S3UhciRegister::IntRaw) | mask;
        self.set_register(Esp32S3UhciRegister::IntRaw, raw & UHCI_INTERRUPT_MASK);
        self.refresh_interrupt_status();
    }

    fn selected_uart(&self) -> Option<UartHandle> {
        let conf0 = self.register(Esp32S3UhciRegister::Conf0);
        (0..3)
            .find(|uart| conf0 & (1 << (uart + 2)) != 0)
            .map(|uart| self.uarts[uart].clone())
    }

    fn quick_packet(&self, slot: u8) -> Vec<u8> {
        if slot >= 7 {
            return Vec::new();
        }
        let mut bytes = Vec::with_capacity(8);
        for word in 0..2 {
            bytes.extend_from_slice(
                &self
                    .register(Esp32S3UhciRegister::QuickData { slot, word })
                    .to_le_bytes(),
            );
        }
        bytes
    }

    fn substitution(&self, sequence: u8) -> (u8, [u8; 2]) {
        let bytes = self
            .register(Esp32S3UhciRegister::EscapeSequence(sequence))
            .to_le_bytes();
        (bytes[0], [bytes[1], bytes[2]])
    }

    fn encode(&self, payload: &[u8]) -> Vec<u8> {
        let conf0 = self.register(Esp32S3UhciRegister::Conf0);
        let escape_conf = self.register(Esp32S3UhciRegister::EscapeConf);
        let separator = self.substitution(0).0;
        let mut encoded = Vec::with_capacity(payload.len() + 2);
        if conf0 & (1 << 5) != 0 {
            encoded.push(separator);
        }
        for byte in payload {
            let replacement = (0..4).find_map(|sequence| {
                let (source, replacement) = self.substitution(sequence);
                (escape_conf & (1 << (sequence + 4)) != 0 && *byte == source).then_some(replacement)
            });
            if let Some(replacement) = replacement {
                encoded.extend_from_slice(&replacement);
            } else {
                encoded.push(*byte);
            }
        }
        if conf0 & (1 << 5) != 0 {
            encoded.push(separator);
        }
        encoded
    }

    fn decode(&self, frame: &[u8]) -> Vec<u8> {
        let conf0 = self.register(Esp32S3UhciRegister::Conf0);
        let escape_conf = self.register(Esp32S3UhciRegister::EscapeConf);
        let separator = self.substitution(0).0;
        let mut start = 0;
        let mut end = frame.len();
        if conf0 & (1 << 5) != 0 {
            while start < end && frame[start] == separator {
                start += 1;
            }
            while end > start && frame[end - 1] == separator {
                end -= 1;
            }
        }
        let mut decoded = Vec::with_capacity(end - start);
        let mut index = start;
        while index < end {
            let mut matched = false;
            if index + 1 < end {
                for sequence in 0..4 {
                    let (source, replacement) = self.substitution(sequence);
                    if escape_conf & (1 << sequence) != 0 && frame[index..index + 2] == replacement
                    {
                        decoded.push(source);
                        index += 2;
                        matched = true;
                        break;
                    }
                }
            }
            if !matched {
                decoded.push(frame[index]);
                index += 1;
            }
        }
        if conf0 & (1 << 9) != 0 {
            let threshold = self.register(Esp32S3UhciRegister::PacketThreshold) as usize;
            if threshold != 0 {
                decoded.truncate(threshold);
            }
        }
        decoded
    }

    fn transmit(&mut self, payload: &[u8], interrupt: u32) -> Option<(UartHandle, Vec<u8>)> {
        let uart = self.selected_uart();
        if uart.is_none() {
            self.set_raw_interrupt(OUTLINK_ERROR_INTERRUPT);
            return None;
        }
        let mut framed = Vec::new();
        let quick = self.register(Esp32S3UhciRegister::QuickSent);
        if quick & (1 << 7) != 0 {
            let slot = u8::try_from((quick >> 4) & 0x7).expect("three-bit UHCI slot fits u8");
            framed.extend(self.quick_packet(slot));
            self.set_raw_interrupt(SEND_ALWAYS_INTERRUPT);
        }
        framed.extend(self.encode(payload));
        self.set_raw_interrupt(interrupt);
        Some((uart.expect("selected UART checked"), framed))
    }
}

/// Host and scheduler-facing UHCI data-path handle.
#[derive(Clone)]
pub struct Esp32S3UhciHandle {
    state: Arc<Mutex<Esp32S3UhciState>>,
}

impl Esp32S3UhciHandle {
    /// Returns whether any enabled UHCI interrupt is pending.
    pub fn interrupt_pending(&self) -> bool {
        let mut state = self.state.lock().expect("ESP UHCI lock poisoned");
        state.refresh_interrupt_status();
        state.register(Esp32S3UhciRegister::IntStatus) != 0
    }

    /// Returns the masked UHCI interrupt status word.
    pub fn interrupt_status(&self) -> u32 {
        let mut state = self.state.lock().expect("ESP UHCI lock poisoned");
        state.refresh_interrupt_status();
        state.register(Esp32S3UhciRegister::IntStatus)
    }

    /// Drains GDMA channel-zero words addressed to UHCI0, frames them, and
    /// transmits them through the selected UART.
    pub fn poll_gdma(&self, gdma: &EspGdmaHandle) -> usize {
        let words = gdma.take_peripheral_output_words(UHCI_GDMA_TRIGGER);
        if words.is_empty() {
            return 0;
        }
        let payload: Vec<u8> = words
            .into_iter()
            .map(|word| word.to_le_bytes()[0])
            .collect();
        let action = self
            .state
            .lock()
            .expect("ESP UHCI lock poisoned")
            .transmit(&payload, RX_START_INTERRUPT);
        if let Some((uart, framed)) = action {
            uart.transmit(&framed);
        }
        payload.len()
    }

    /// Decodes one UART frame and offers the payload to a GDMA receive
    /// channel connected to UHCI0. Returns whether GDMA accepted it.
    pub fn receive_uart_frame(&self, gdma: &EspGdmaHandle, frame: &[u8]) -> bool {
        let decoded = {
            let mut state = self.state.lock().expect("ESP UHCI lock poisoned");
            let decoded = state.decode(frame);
            state.received_packets.push_back(decoded.clone());
            state.set_raw_interrupt(TX_START_INTERRUPT);
            decoded
        };
        let words: Vec<u32> = decoded.into_iter().map(u32::from).collect();
        gdma.queue_peripheral_input_words(UHCI_GDMA_TRIGGER, &words)
    }

    /// Takes all decoded packets retained for host inspection.
    pub fn take_received_packets(&self) -> Vec<Vec<u8>> {
        self.state
            .lock()
            .expect("ESP UHCI lock poisoned")
            .received_packets
            .drain(..)
            .collect()
    }
}

/// Functional ESP32-S3 UHCI register and framed GDMA/UART bridge.
pub struct Esp32S3Uhci {
    name: String,
    state: Arc<Mutex<Esp32S3UhciState>>,
}

impl Esp32S3Uhci {
    /// Creates a UHCI block connected to the three native UART endpoints.
    pub fn new(name: impl Into<String>, uarts: [UartHandle; 3]) -> (Self, Esp32S3UhciHandle) {
        let state = Arc::new(Mutex::new(Esp32S3UhciState::new(uarts)));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Esp32S3UhciHandle { state },
        )
    }
}

impl Device for Esp32S3Uhci {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP32-S3 UHCI requires aligned word access",
            ));
        }
        let register = Esp32S3UhciRegister::from_offset(offset).ok_or_else(|| {
            DeviceError::new(format!(
                "{} read at reserved UHCI offset {offset:#x}",
                self.name
            ))
        })?;
        let mut state = self.state.lock().expect("ESP UHCI lock poisoned");
        state.refresh_interrupt_status();
        Ok(u64::from(state.register(register) & register.read_mask()))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP32-S3 UHCI requires aligned word access",
            ));
        }
        let value = u32::try_from(value)
            .map_err(|_| DeviceError::new("ESP32-S3 UHCI rejects writes wider than 32 bits"))?;
        let register = Esp32S3UhciRegister::from_offset(offset).ok_or_else(|| {
            DeviceError::new(format!(
                "{} write at reserved UHCI offset {offset:#x}",
                self.name
            ))
        })?;
        if register.write_mask() == 0 {
            return Err(DeviceError::new(format!(
                "{} write to read-only UHCI register {register:?}",
                self.name
            )));
        }

        let mut action = None;
        let mut state = self.state.lock().expect("ESP UHCI lock poisoned");
        match register {
            Esp32S3UhciRegister::IntRaw => {
                let raw = (state.register(register) & !0x0180) | (value & 0x0180);
                state.set_register(register, raw);
            }
            Esp32S3UhciRegister::IntClear => {
                let raw =
                    state.register(Esp32S3UhciRegister::IntRaw) & !(value & UHCI_INTERRUPT_MASK);
                state.set_register(Esp32S3UhciRegister::IntRaw, raw);
            }
            Esp32S3UhciRegister::AppIntSet => {
                let raw = ((value & 1) << 7) | ((value & 2) << 7);
                state.set_raw_interrupt(raw);
            }
            Esp32S3UhciRegister::AckNum => {
                state.set_register(register, value & 0x7);
            }
            Esp32S3UhciRegister::QuickSent => {
                let stored = value & 0xf7;
                state.set_register(register, stored);
                if value & (1 << 3) != 0 {
                    let slot = u8::try_from(value & 0x7).expect("three-bit UHCI slot fits u8");
                    let packet = state.quick_packet(slot);
                    action = state.transmit(&packet, SEND_SINGLE_INTERRUPT);
                }
            }
            Esp32S3UhciRegister::Conf0 => {
                let masked = value & register.write_mask();
                state.set_register(register, masked);
                if masked & 0x3 != 0 {
                    state.received_packets.clear();
                    state.set_register(Esp32S3UhciRegister::State0, 0);
                    state.set_register(Esp32S3UhciRegister::State1, 0);
                }
            }
            _ => state.set_register(register, value & register.write_mask()),
        }
        state.refresh_interrupt_status();
        drop(state);
        if let Some((uart, bytes)) = action {
            uart.transmit(&bytes);
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let uarts = {
            let state = self.state.lock().expect("ESP UHCI lock poisoned");
            state.uarts.clone()
        };
        *self.state.lock().expect("ESP UHCI lock poisoned") = Esp32S3UhciState::new(uarts);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EspGdma, SignalHub};

    fn write(device: &mut impl Device, register: Esp32S3UhciRegister, value: u64) {
        device
            .write(register.offset(), AccessWidth::Word, value, SimTime::ZERO)
            .unwrap();
    }

    fn read(device: &mut impl Device, register: Esp32S3UhciRegister) -> u64 {
        device
            .read(register.offset(), AccessWidth::Word, SimTime::ZERO)
            .unwrap()
    }

    #[test]
    fn vendor_register_contract_has_exact_offsets_masks_and_resets() {
        for slot in 0..7 {
            for word in 0..2 {
                let register = Esp32S3UhciRegister::QuickData { slot, word };
                assert_eq!(
                    Esp32S3UhciRegister::from_offset(register.offset()),
                    Some(register)
                );
            }
        }
        assert_eq!(Esp32S3UhciRegister::from_offset(0x88), None);
        assert_eq!(Esp32S3UhciRegister::from_offset(0x39), None);
        assert_eq!(Esp32S3UhciRegister::IntRaw.write_mask(), 0x180);
        assert_eq!(Esp32S3UhciRegister::RxHead.write_mask(), 0);

        let (mut device, _) =
            Esp32S3Uhci::new("uhci", std::array::from_fn(|_| UartHandle::default()));
        assert_eq!(
            read(&mut device, Esp32S3UhciRegister::Conf0),
            UHCI_CONF0_RESET.into()
        );
        assert_eq!(
            read(&mut device, Esp32S3UhciRegister::Conf1),
            UHCI_CONF1_RESET.into()
        );
        assert_eq!(
            read(&mut device, Esp32S3UhciRegister::HungConf),
            UHCI_HUNG_CONF_RESET.into()
        );
        assert_eq!(
            read(&mut device, Esp32S3UhciRegister::Date),
            UHCI_DATE_RESET.into()
        );
    }

    #[test]
    fn software_interrupts_follow_raw_masked_and_write_one_clear_semantics() {
        let (mut device, handle) =
            Esp32S3Uhci::new("uhci", std::array::from_fn(|_| UartHandle::default()));
        write(&mut device, Esp32S3UhciRegister::IntEnable, 1 << 7);
        write(&mut device, Esp32S3UhciRegister::AppIntSet, 1);
        assert_eq!(handle.interrupt_status(), 1 << 7);
        assert!(handle.interrupt_pending());
        write(&mut device, Esp32S3UhciRegister::IntClear, 1 << 7);
        assert_eq!(handle.interrupt_status(), 0);
    }

    #[test]
    fn gdma_payloads_are_framed_escaped_and_routed_through_selected_uart() {
        let uarts = std::array::from_fn(|_| UartHandle::default());
        let terminal = uarts[1].clone();
        let (mut device, handle) = Esp32S3Uhci::new("uhci", uarts);
        write(
            &mut device,
            Esp32S3UhciRegister::Conf0,
            u64::from(UHCI_CONF0_RESET | (1 << 3)),
        );

        let hub = SignalHub::new();
        let (mut gdma, gdma_handle) = EspGdma::new("gdma", "gdma", hub).unwrap();
        gdma.write(0xa8, AccessWidth::Word, 2, SimTime::ZERO)
            .unwrap();
        for byte in [0x41_u64, 0xc0, 0xdb] {
            gdma.write(0x7c, AccessWidth::Word, (1 << 9) | byte, SimTime::ZERO)
                .unwrap();
        }
        assert_eq!(handle.poll_gdma(&gdma_handle), 3);
        assert_eq!(
            terminal.bytes(),
            vec![0xc0, 0x41, 0xdb, 0xdc, 0xdb, 0xdd, 0xc0]
        );

        gdma.write(0x48, AccessWidth::Word, 2, SimTime::ZERO)
            .unwrap();
        assert!(handle.receive_uart_frame(&gdma_handle, &terminal.bytes()));
        assert_eq!(handle.take_received_packets(), vec![vec![0x41, 0xc0, 0xdb]]);
        assert_eq!(
            gdma.read(0x1c, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0x41
        );
    }

    #[test]
    fn quick_send_uses_selected_slot_and_raises_completion_interrupt() {
        let uarts = std::array::from_fn(|_| UartHandle::default());
        let terminal = uarts[0].clone();
        let (mut device, handle) = Esp32S3Uhci::new("uhci", uarts);
        write(&mut device, Esp32S3UhciRegister::Conf0, 1 << 2);
        write(
            &mut device,
            Esp32S3UhciRegister::QuickData { slot: 2, word: 0 },
            0x4443_4241,
        );
        write(
            &mut device,
            Esp32S3UhciRegister::QuickData { slot: 2, word: 1 },
            0x4847_4645,
        );
        write(
            &mut device,
            Esp32S3UhciRegister::IntEnable,
            SEND_SINGLE_INTERRUPT.into(),
        );
        write(&mut device, Esp32S3UhciRegister::QuickSent, (1 << 3) | 2);
        assert_eq!(terminal.bytes(), b"ABCDEFGH");
        assert_eq!(handle.interrupt_status(), SEND_SINGLE_INTERRUPT);
        assert_eq!(read(&mut device, Esp32S3UhciRegister::QuickSent), 2);
    }

    #[test]
    fn rejects_reserved_unaligned_and_read_only_accesses() {
        let (mut device, _) =
            Esp32S3Uhci::new("uhci", std::array::from_fn(|_| UartHandle::default()));
        assert!(device.read(0x88, AccessWidth::Word, SimTime::ZERO).is_err());
        assert!(device.read(0x04, AccessWidth::Byte, SimTime::ZERO).is_err());
        assert!(
            device
                .write(0x30, AccessWidth::Word, 1, SimTime::ZERO)
                .is_err()
        );
    }
}
