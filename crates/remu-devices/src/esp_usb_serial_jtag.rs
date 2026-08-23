use super::*;

/// Host-facing state for the ESP USB Serial/JTAG CDC-ACM data endpoint.
#[derive(Clone)]
pub struct EspUsbSerialJtagHandle {
    state: Arc<Mutex<EspUsbSerialJtagState>>,
}

/// One software-visible transition on the USB PHY test interface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EspUsbPhyEvent {
    /// Simulation time of the TEST register write.
    pub at: SimTime,
    /// Whether the PHY output driver was enabled.
    pub output_enabled: bool,
    /// Driven D+ level. It is meaningful only while `output_enabled` is true.
    pub dp: bool,
    /// Driven D- level. It is meaningful only while `output_enabled` is true.
    pub dm: bool,
}

#[derive(Default)]
struct EspUsbSerialJtagState {
    rx: VecDeque<u8>,
    tx_packet: Vec<u8>,
    output: Vec<u8>,
    input_queued: bool,
    host_connected: bool,
    sof_epoch: SimTime,
    interrupt_raw: u32,
    interrupt_enable: u32,
    conf0: u32,
    test_control: u32,
    phy_input: Option<(bool, bool)>,
    phy_events: Vec<EspUsbPhyEvent>,
    registers: BTreeMap<u64, u32>,
}

const HOST_SCRIPT_COMPLETE_MARKER: &[u8] = b"__REMU_HOST_SCRIPT_COMPLETE__";

impl EspUsbSerialJtagHandle {
    /// Queues bytes sent by the deterministic host to the CDC-ACM OUT endpoint.
    pub fn queue_input(&self, bytes: &[u8]) {
        let mut state = self.state.lock().expect("USB Serial/JTAG lock poisoned");
        state.rx.extend(bytes.iter().copied());
        if !bytes.is_empty() {
            state.input_queued = true;
            state.interrupt_raw |= 1 << 2;
        }
    }

    /// Selects whether the deterministic USB host is attached.
    ///
    /// A connected host emits one start-of-frame indication every
    /// [`EspUsbSerialJtag::SOF_PERIOD_TICKS`] abstract ticks. The epoch is
    /// reset when the connection changes so tests can make the transition
    /// reproducible at a chosen simulation timestamp.
    pub fn set_host_connected(&self, connected: bool, at: SimTime) {
        let mut state = self.state.lock().expect("USB Serial/JTAG lock poisoned");
        state.host_connected = connected;
        state.sof_epoch = at;
        if !connected {
            state.interrupt_raw &= !EspUsbSerialJtag::SERIAL_SOF;
        }
    }

    /// Returns whether the deterministic host is currently attached.
    pub fn host_connected(&self) -> bool {
        self.state
            .lock()
            .expect("USB Serial/JTAG lock poisoned")
            .host_connected
    }

    /// Advances host USB scheduling and returns true on a newly asserted SOF.
    ///
    /// SOF is intentionally functional rather than clock accurate: one
    /// abstract tick is one completed architectural action, and the fixed
    /// period gives firmware a stable connected-host signal without tying the
    /// model to a particular CPU frequency.
    pub fn poll(&self, now: SimTime) -> bool {
        let mut state = self.state.lock().expect("USB Serial/JTAG lock poisoned");
        if !state.host_connected {
            state.interrupt_raw &= !EspUsbSerialJtag::SERIAL_SOF;
            return false;
        }

        let elapsed = now.ticks().saturating_sub(state.sof_epoch.ticks());
        if elapsed < EspUsbSerialJtag::SOF_PERIOD_TICKS {
            return false;
        }
        let periods = elapsed / EspUsbSerialJtag::SOF_PERIOD_TICKS;
        let advance = periods.saturating_mul(EspUsbSerialJtag::SOF_PERIOD_TICKS);
        state.sof_epoch = SimTime::from_ticks(state.sof_epoch.ticks().saturating_add(advance));
        let newly_asserted = state.interrupt_raw & EspUsbSerialJtag::SERIAL_SOF == 0;
        state.interrupt_raw |= EspUsbSerialJtag::SERIAL_SOF;
        newly_asserted
    }

    /// Returns all bytes transmitted to the deterministic CDC-ACM host.
    pub fn output(&self) -> Vec<u8> {
        self.state
            .lock()
            .expect("USB Serial/JTAG lock poisoned")
            .output
            .clone()
    }

    /// Reports that all queued raw-REPL input ran and its final prompt was flushed.
    pub fn input_complete(&self) -> bool {
        let state = self.state.lock().expect("USB Serial/JTAG lock poisoned");
        state.input_queued
            && state
                .output
                .windows(HOST_SCRIPT_COMPLETE_MARKER.len())
                .any(|window| window == HOST_SCRIPT_COMPLETE_MARKER)
            && state.output.ends_with(b"\x04\x04>")
    }

    /// Reports whether an enabled USB Serial/JTAG interrupt is pending.
    pub fn interrupt_pending(&self) -> bool {
        let state = self.state.lock().expect("USB Serial/JTAG lock poisoned");
        state.interrupt_raw & state.interrupt_enable != 0
    }

    /// Clears captured host output without changing endpoint configuration.
    pub fn clear_output(&self) {
        self.state
            .lock()
            .expect("USB Serial/JTAG lock poisoned")
            .output
            .clear();
    }

    /// Drives the receiver side of the raw USB PHY from a deterministic host.
    pub fn set_phy_input(&self, dp: bool, dm: bool) {
        self.state
            .lock()
            .expect("USB Serial/JTAG lock poisoned")
            .phy_input = Some((dp, dm));
    }

    /// Releases the deterministic host and lets output/pull state drive RX.
    pub fn release_phy_input(&self) {
        self.state
            .lock()
            .expect("USB Serial/JTAG lock poisoned")
            .phy_input = None;
    }

    /// Returns writes made through the raw USB PHY test interface.
    pub fn phy_events(&self) -> Vec<EspUsbPhyEvent> {
        self.state
            .lock()
            .expect("USB Serial/JTAG lock poisoned")
            .phy_events
            .clone()
    }

    /// Clears captured raw PHY transitions without changing PHY ownership.
    pub fn clear_phy_events(&self) {
        self.state
            .lock()
            .expect("USB Serial/JTAG lock poisoned")
            .phy_events
            .clear();
    }

    /// Decodes valid low-speed DATA packets written one bit-cell at a time.
    ///
    /// This is a packet oracle for software-SIE qualification. Malformed
    /// sync, stuffing, PID, CRC, or EOP sequences do not produce output.
    pub fn low_speed_output(&self) -> Vec<u8> {
        let state = self.state.lock().expect("USB Serial/JTAG lock poisoned");
        decode_low_speed_packets(&state.phy_events)
    }
}

fn usb_crc16(data: &[u8]) -> u16 {
    let mut crc = 0xffff_u16;
    for byte in data {
        let mut value = *byte;
        for _ in 0..8 {
            let mix = (crc ^ u16::from(value)) & 1;
            crc >>= 1;
            if mix != 0 {
                crc ^= 0xa001;
            }
            value >>= 1;
        }
    }
    !crc
}

fn decode_low_speed_packets(events: &[EspUsbPhyEvent]) -> Vec<u8> {
    // Low-speed J is D- high and K is D+ high.
    const J: (bool, bool) = (false, true);
    const K: (bool, bool) = (true, false);
    const SYNC: [(bool, bool); 8] = [K, J, K, J, K, J, K, K];
    let driven: Vec<&EspUsbPhyEvent> = events.iter().filter(|event| event.output_enabled).collect();
    let lines: Vec<(bool, bool)> = driven.iter().map(|event| (event.dp, event.dm)).collect();
    let mut output = Vec::new();
    let mut offset = 0;
    while offset + SYNC.len() <= lines.len() {
        if lines[offset..offset + SYNC.len()] != SYNC {
            offset += 1;
            continue;
        }
        let mut index = offset + SYNC.len();
        let mut previous = K;
        let mut ones = 0_u8;
        let mut bits = Vec::new();
        let mut valid = true;
        while index < lines.len() && lines[index] != (false, false) {
            let line = lines[index];
            if line != J && line != K {
                valid = false;
                break;
            }
            let bit = u8::from(line == previous);
            previous = line;
            index += 1;
            if ones == 6 {
                if bit != 0 {
                    valid = false;
                }
                ones = 0;
                if !valid {
                    break;
                }
                continue;
            }
            bits.push(bit);
            if bit == 1 {
                ones += 1;
            } else {
                ones = 0;
            }
        }
        if index + 2 >= lines.len()
            || lines[index] != (false, false)
            || lines[index + 1] != (false, false)
            || lines[index + 2] != J
        {
            valid = false;
        }
        if valid {
            // The ESP32-C6 HP-core qualification loop targets 160 MHz / 1.5
            // Mbit/s. Keep the packet oracle narrow enough to catch loop
            // jitter or accidentally reintroduced call overhead.
            for pair in driven[offset..=index + 2].windows(2) {
                let ticks = pair[1].at.ticks().saturating_sub(pair[0].at.ticks());
                if !(102..=111).contains(&ticks) {
                    valid = false;
                    break;
                }
            }
        }
        if valid && bits.len() % 8 == 0 {
            let mut bytes = Vec::with_capacity(bits.len() / 8);
            for byte_bits in bits.chunks_exact(8) {
                let mut byte = 0_u8;
                for (bit_index, bit) in byte_bits.iter().enumerate() {
                    byte |= bit << bit_index;
                }
                bytes.push(byte);
            }
            if bytes.len() >= 3
                && bytes[0] >> 4 == (!bytes[0] & 0x0f)
                && matches!(bytes[0] & 0x0f, 0x03 | 0x0b)
            {
                let payload_end = bytes.len() - 2;
                let expected = usb_crc16(&bytes[1..payload_end]);
                let actual = u16::from_le_bytes([bytes[payload_end], bytes[payload_end + 1]]);
                if actual == expected {
                    output.extend_from_slice(&bytes[1..payload_end]);
                }
            }
        }
        offset = index.saturating_add(3);
    }
    output
}

/// Functional ESP32-C6/S3 USB Serial/JTAG endpoint.
///
/// The model implements the software-visible CDC-ACM FIFO contract used by
/// ESP-IDF: EP1 byte access, endpoint availability, TX flush, and interrupt
/// raw/status/enable/clear registers. The deterministic host consumes every
/// flushed IN packet immediately.
pub struct EspUsbSerialJtag {
    name: String,
    state: Arc<Mutex<EspUsbSerialJtagState>>,
}

impl EspUsbSerialJtag {
    const EP1: u64 = 0x00;
    const EP1_CONF: u64 = 0x04;
    const INT_RAW: u64 = 0x08;
    const INT_ST: u64 = 0x0c;
    const INT_ENA: u64 = 0x10;
    const INT_CLR: u64 = 0x14;
    const CONF0: u64 = 0x18;
    const TEST: u64 = 0x1c;
    /// USB full-speed start-of-frame period in abstract simulation ticks.
    pub const SOF_PERIOD_TICKS: u64 = 1_000;
    const SERIAL_OUT_RECV_PKT: u32 = 1 << 2;
    const SERIAL_SOF: u32 = 1 << 1;
    const SERIAL_IN_EMPTY: u32 = 1 << 3;
    const INTERRUPT_MASK: u32 = 0x7ffff;
    const ENDPOINT_SIZE: usize = 64;
    const CONF0_DP_PULLUP: u32 = 1 << 9;
    const CONF0_DP_PULLDOWN: u32 = 1 << 10;
    const CONF0_DM_PULLUP: u32 = 1 << 11;
    const CONF0_DM_PULLDOWN: u32 = 1 << 12;
    const CONF0_USB_PAD_ENABLE: u32 = 1 << 14;
    const CONF0_RESET: u32 = Self::CONF0_DP_PULLUP | Self::CONF0_USB_PAD_ENABLE;
    const TEST_ENABLE: u32 = 1;
    const TEST_USB_OE: u32 = 1 << 1;
    const TEST_TX_DP: u32 = 1 << 2;
    const TEST_TX_DM: u32 = 1 << 3;
    const TEST_CONTROL_MASK: u32 = 0xf;
    const TEST_RX_RCV: u32 = 1 << 4;
    const TEST_RX_DP: u32 = 1 << 5;
    const TEST_RX_DM: u32 = 1 << 6;

    /// Creates the peripheral and its host-facing handle.
    pub fn new(name: impl Into<String>) -> (Self, EspUsbSerialJtagHandle) {
        let state = Arc::new(Mutex::new(EspUsbSerialJtagState {
            // Hardware reset state: an empty IN endpoint is writable and its
            // raw empty indication is asserted. The deterministic host starts
            // connected so existing console tests model a plugged-in USB
            // cable unless they explicitly select disconnected mode.
            host_connected: true,
            interrupt_raw: Self::SERIAL_IN_EMPTY,
            conf0: Self::CONF0_RESET,
            ..EspUsbSerialJtagState::default()
        }));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            EspUsbSerialJtagHandle { state },
        )
    }

    fn flush_tx(state: &mut EspUsbSerialJtagState) {
        state.output.append(&mut state.tx_packet);
        // The functional host takes the packet immediately, making EP1
        // writable again and producing the hardware's IN-empty indication.
        state.interrupt_raw |= Self::SERIAL_IN_EMPTY;
    }

    fn receive_lines(state: &EspUsbSerialJtagState) -> (bool, bool) {
        if state.conf0 & Self::CONF0_USB_PAD_ENABLE == 0 {
            return (false, false);
        }
        if let Some(lines) = state.phy_input {
            return lines;
        }
        if state.test_control & (Self::TEST_ENABLE | Self::TEST_USB_OE)
            == Self::TEST_ENABLE | Self::TEST_USB_OE
        {
            return (
                state.test_control & Self::TEST_TX_DP != 0,
                state.test_control & Self::TEST_TX_DM != 0,
            );
        }
        let dp =
            state.conf0 & Self::CONF0_DP_PULLUP != 0 && state.conf0 & Self::CONF0_DP_PULLDOWN == 0;
        let dm =
            state.conf0 & Self::CONF0_DM_PULLUP != 0 && state.conf0 & Self::CONF0_DM_PULLDOWN == 0;
        (dp, dm)
    }
}

impl Device for EspUsbSerialJtag {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, _width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let mut state = self.state.lock().expect("USB Serial/JTAG lock poisoned");
        match offset {
            Self::EP1 => {
                let byte = state.rx.pop_front().unwrap_or_default();
                if state.rx.is_empty() {
                    state.interrupt_raw &= !Self::SERIAL_OUT_RECV_PKT;
                }
                Ok(u64::from(byte))
            }
            Self::EP1_CONF => {
                let rx_available = u32::from(!state.rx.is_empty()) << 2;
                // The deterministic host drains packets immediately, so the
                // 64-byte IN FIFO is always available to firmware.
                Ok(u64::from((1 << 1) | rx_available))
            }
            Self::INT_RAW => Ok(u64::from(state.interrupt_raw)),
            Self::INT_ST => Ok(u64::from(state.interrupt_raw & state.interrupt_enable)),
            Self::INT_ENA => Ok(u64::from(state.interrupt_enable)),
            Self::INT_CLR => Ok(0),
            Self::CONF0 => Ok(u64::from(state.conf0)),
            Self::TEST => {
                let (dp, dm) = Self::receive_lines(&state);
                let receive = u32::from(dp) * Self::TEST_RX_RCV
                    | u32::from(dp) * Self::TEST_RX_DP
                    | u32::from(dm) * Self::TEST_RX_DM;
                Ok(u64::from(state.test_control | receive))
            }
            _ => Ok(u64::from(
                state.registers.get(&offset).copied().unwrap_or_default(),
            )),
        }
    }

    fn write(
        &mut self,
        offset: u64,
        _width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        let mut state = self.state.lock().expect("USB Serial/JTAG lock poisoned");
        let value = value as u32;
        match offset {
            Self::EP1 => {
                state.tx_packet.push(value as u8);
                if state.tx_packet.len() == Self::ENDPOINT_SIZE {
                    Self::flush_tx(&mut state);
                }
            }
            Self::EP1_CONF => {
                if value & 1 != 0 {
                    Self::flush_tx(&mut state);
                }
            }
            Self::INT_RAW => {
                // R/WTC/SS fields: writing one clears an asserted raw status.
                state.interrupt_raw &= !(value & Self::INTERRUPT_MASK);
            }
            Self::INT_ENA => state.interrupt_enable = value & Self::INTERRUPT_MASK,
            Self::INT_CLR => state.interrupt_raw &= !(value & Self::INTERRUPT_MASK),
            Self::CONF0 => state.conf0 = value,
            Self::TEST => {
                state.test_control = value & Self::TEST_CONTROL_MASK;
                let control = state.test_control;
                if control & Self::TEST_ENABLE != 0 && state.conf0 & Self::CONF0_USB_PAD_ENABLE != 0
                {
                    state.phy_events.push(EspUsbPhyEvent {
                        at,
                        output_enabled: control & Self::TEST_USB_OE != 0,
                        dp: control & Self::TEST_TX_DP != 0,
                        dm: control & Self::TEST_TX_DM != 0,
                    });
                }
            }
            _ => {
                state.registers.insert(offset, value);
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.lock().expect("USB Serial/JTAG lock poisoned");
        *state = EspUsbSerialJtagState {
            host_connected: true,
            interrupt_raw: Self::SERIAL_IN_EMPTY,
            conf0: Self::CONF0_RESET,
            ..EspUsbSerialJtagState::default()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_phy_registers_expose_documented_reset_and_test_lines() {
        let (mut device, handle) = EspUsbSerialJtag::new("usb");
        assert_eq!(
            device
                .read(EspUsbSerialJtag::CONF0, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            u64::from(EspUsbSerialJtag::CONF0_RESET)
        );
        assert_eq!(
            device
                .read(EspUsbSerialJtag::TEST, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            u64::from(EspUsbSerialJtag::TEST_RX_RCV | EspUsbSerialJtag::TEST_RX_DP)
        );

        device
            .write(
                EspUsbSerialJtag::TEST,
                AccessWidth::Word,
                u64::from(
                    EspUsbSerialJtag::TEST_ENABLE
                        | EspUsbSerialJtag::TEST_USB_OE
                        | EspUsbSerialJtag::TEST_TX_DM,
                ),
                SimTime::from_ticks(7),
            )
            .unwrap();
        assert_eq!(
            handle.phy_events(),
            vec![EspUsbPhyEvent {
                at: SimTime::from_ticks(7),
                output_enabled: true,
                dp: false,
                dm: true,
            }]
        );
        assert_eq!(
            device
                .read(EspUsbSerialJtag::TEST, AccessWidth::Word, SimTime::ZERO)
                .unwrap()
                & u64::from(EspUsbSerialJtag::TEST_RX_DP | EspUsbSerialJtag::TEST_RX_DM),
            u64::from(EspUsbSerialJtag::TEST_RX_DM)
        );

        handle.set_phy_input(true, false);
        assert_ne!(
            device
                .read(EspUsbSerialJtag::TEST, AccessWidth::Word, SimTime::ZERO)
                .unwrap()
                & u64::from(EspUsbSerialJtag::TEST_RX_DP),
            0
        );
    }

    #[test]
    fn raw_phy_oracle_decodes_stuffed_low_speed_data() {
        let payload = [0xff, 0xff, b'P'];
        let crc = usb_crc16(&payload);
        let mut bytes = vec![0xc3];
        bytes.extend_from_slice(&payload);
        bytes.extend_from_slice(&crc.to_le_bytes());
        let mut events = Vec::new();
        let mut at = 0_u64;
        for line in [
            (true, false),
            (false, true),
            (true, false),
            (false, true),
            (true, false),
            (false, true),
            (true, false),
            (true, false),
        ] {
            events.push(EspUsbPhyEvent {
                at: SimTime::from_ticks(at),
                output_enabled: true,
                dp: line.0,
                dm: line.1,
            });
            at += 104;
        }
        let mut line = (true, false);
        let mut ones = 0;
        for byte in bytes {
            for shift in 0..8 {
                let bit = byte >> shift & 1;
                if bit == 0 {
                    line = (line.1, line.0);
                    ones = 0;
                } else {
                    ones += 1;
                }
                events.push(EspUsbPhyEvent {
                    at: SimTime::from_ticks(at),
                    output_enabled: true,
                    dp: line.0,
                    dm: line.1,
                });
                at += 104;
                if ones == 6 {
                    line = (line.1, line.0);
                    events.push(EspUsbPhyEvent {
                        at: SimTime::from_ticks(at),
                        output_enabled: true,
                        dp: line.0,
                        dm: line.1,
                    });
                    at += 104;
                    ones = 0;
                }
            }
        }
        for line in [(false, false), (false, false), (false, true)] {
            events.push(EspUsbPhyEvent {
                at: SimTime::from_ticks(at),
                output_enabled: true,
                dp: line.0,
                dm: line.1,
            });
            at += 104;
        }
        assert_eq!(decode_low_speed_packets(&events), payload);
    }
}
