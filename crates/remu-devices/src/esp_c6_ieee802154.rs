
const IEEE802154_EVENT_TX_DONE: u32 = 1 << 0;
const IEEE802154_EVENT_RX_DONE: u32 = 1 << 1;
const IEEE802154_EVENT_RX_ABORT: u32 = 1 << 4;
const IEEE802154_EVENT_TX_ABORT: u32 = 1 << 5;
const IEEE802154_EVENT_ED_DONE: u32 = 1 << 6;
const IEEE802154_EVENT_TIMER0: u32 = 1 << 8;
const IEEE802154_EVENT_TIMER1: u32 = 1 << 9;
const IEEE802154_EVENT_MASK: u32 = 0x1fff;
/// Command written to the ESP32-C6 IEEE 802.15.4 command register.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum EspIeee802154Command {
    /// Begin DMA-backed transmission.
    TxStart = 0x41,
    /// Begin reception.
    RxStart = 0x42,
    /// Perform CCA and transmit if idle.
    CcaTxStart = 0x43,
    /// Begin energy detection.
    EnergyDetectStart = 0x44,
    /// Stop active TX/RX/ED work.
    Stop = 0x45,
    /// Begin continuous test transmission.
    TestTxStart = 0x46,
    /// Begin continuous test reception.
    TestRxStart = 0x47,
    /// Stop continuous test mode.
    TestStop = 0x48,
    /// Start MAC timer zero.
    Timer0Start = 0x4c,
    /// Stop MAC timer zero.
    Timer0Stop = 0x4d,
    /// Start MAC timer one.
    Timer1Start = 0x4e,
    /// Stop MAC timer one.
    Timer1Stop = 0x4f,
}

impl EspIeee802154Command {
    fn from_opcode(opcode: u8) -> Option<Self> {
        Some(match opcode {
            0x41 => Self::TxStart,
            0x42 => Self::RxStart,
            0x43 => Self::CcaTxStart,
            0x44 => Self::EnergyDetectStart,
            0x45 => Self::Stop,
            0x46 => Self::TestTxStart,
            0x47 => Self::TestRxStart,
            0x48 => Self::TestStop,
            0x4c => Self::Timer0Start,
            0x4d => Self::Timer0Stop,
            0x4e => Self::Timer1Start,
            0x4f => Self::Timer1Stop,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug)]
struct Ieee802154State {
    registers: [u32; 98],
    commands: VecDeque<EspIeee802154Command>,
    timer_started: [Option<SimTime>; 2],
    awaiting_ack_sequence: Option<u8>,
}

impl Ieee802154State {
    fn reset(&mut self) {
        self.registers = [0; 98];
        self.registers[0x184 / 4] = 0x22_06_22;
        self.commands.clear();
        self.timer_started = [None, None];
        self.awaiting_ack_sequence = None;
    }

    fn update_timers(&mut self, at: SimTime) {
        for timer in 0..2 {
            let Some(started) = self.timer_started[timer] else {
                continue;
            };
            let elapsed = at
                .checked_duration_since(started)
                .map_or(0, |time| time.ticks());
            let value_offset = if timer == 0 { 0xac } else { 0xb4 };
            let threshold_offset = if timer == 0 { 0xa8 } else { 0xb0 };
            self.registers[value_offset / 4] = elapsed as u32;
            if elapsed >= u64::from(self.registers[threshold_offset / 4]) {
                self.registers[0x64 / 4] |= if timer == 0 {
                    IEEE802154_EVENT_TIMER0
                } else {
                    IEEE802154_EVENT_TIMER1
                };
                self.timer_started[timer] = None;
            }
        }
    }
}

/// Host-side queue, completion, and interrupt API for the C6 802.15.4 MAC.
#[derive(Clone, Debug)]
pub struct EspIeee802154Handle {
    state: Arc<Mutex<Ieee802154State>>,
}

/// One firmware-programmed IEEE 802.15.4 PAN filter slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EspIeee802154Pan {
    /// PAN identifier.
    pub pan_id: u16,
    /// Sixteen-bit local address.
    pub short_address: u16,
    /// Eight-byte local address in MAC wire order.
    pub extended_address: [u8; 8],
}

/// Firmware-visible MAC policy needed by the packet engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EspIeee802154Configuration {
    /// Four optional PAN interfaces controlled by the multi-PAN mask.
    pub pans: [Option<EspIeee802154Pan>; 4],
    /// Accept frames without destination filtering.
    pub promiscuous: bool,
    /// Generate (transmit) an ACK for an accepted receive frame requesting one.
    pub automatic_ack_transmit: bool,
    /// Receive an ACK following transmission of a frame requesting one.
    pub automatic_ack_receive: bool,
    /// Frame-pending state inserted in generated ACKs.
    pub frame_pending: bool,
    /// CCA energy threshold as a signed dBm byte.
    pub cca_threshold_dbm: i8,
    /// Hardware CCA mode: carrier, ED, carrier-or-ED, or carrier-and-ED.
    pub cca_mode: u8,
    /// Programmed ED/CCA observation duration in IEEE 802.15.4 symbols.
    pub ed_duration_symbols: u32,
    /// Whether transmit AES-CCM* is enabled.
    pub transmit_security: bool,
    /// MAC payload byte offset measured from the first frame-control byte.
    pub security_offset: u8,
    /// Nonce source address in MAC wire order.
    pub security_address: [u8; 8],
    /// AES-128 key.
    pub security_key: [u8; 16],
}

impl EspIeee802154Handle {
    /// Advances MAC timers to a machine service timestamp.
    pub fn poll(&self, at: SimTime) {
        self.state
            .lock()
            .expect("802.15.4 state lock poisoned")
            .update_timers(at);
    }

    /// Removes the oldest command submitted by firmware.
    pub fn take_command(&self) -> Option<EspIeee802154Command> {
        self.state
            .lock()
            .expect("802.15.4 state lock poisoned")
            .commands
            .pop_front()
    }

    /// Current channel number as programmed by firmware.
    pub fn channel(&self) -> u8 {
        let frequency_code = self
            .state
            .lock()
            .expect("802.15.4 state lock poisoned")
            .registers[0x48 / 4] as u8
            & 0x7f;
        // The native register stores the PHY frequency code, not the IEEE
        // channel number: channel 11 is code 3 and each subsequent 5 MHz
        // channel advances the code by five.
        frequency_code
            .checked_sub(3)
            .filter(|offset| offset.is_multiple_of(5))
            .map_or(0, |offset| 11 + offset / 5)
    }

    /// Current encoded transmit-power setting.
    pub fn tx_power(&self) -> u8 {
        self.state
            .lock()
            .expect("802.15.4 state lock poisoned")
            .registers[0x4c / 4] as u8
            & 0x1f
    }

    /// Coexistence priority programmed for normal 802.15.4 traffic.
    pub fn coexistence_priority(&self) -> u8 {
        self.state
            .lock()
            .expect("802.15.4 state lock poisoned")
            .registers[0x70 / 4] as u8
            & 0x0f
    }

    /// Firmware-programmed TX and RX DMA addresses.
    pub fn dma_addresses(&self) -> (u32, u32) {
        let state = self.state.lock().expect("802.15.4 state lock poisoned");
        (state.registers[0xd0 / 4], state.registers[0xe0 / 4])
    }

    /// Returns the current filter, ACK, CCA, and security configuration.
    pub fn configuration(&self) -> EspIeee802154Configuration {
        let state = self.state.lock().expect("802.15.4 state lock poisoned");
        let conf = state.registers[0x04 / 4];
        let pans = std::array::from_fn(|index| {
            if conf & (1 << (28 + index)) == 0 {
                return None;
            }
            let base = 0x08 / 4 + index * 4;
            let mut extended_address = [0_u8; 8];
            extended_address[..4].copy_from_slice(&state.registers[base + 2].to_le_bytes());
            extended_address[4..].copy_from_slice(&state.registers[base + 3].to_le_bytes());
            Some(EspIeee802154Pan {
                short_address: state.registers[base] as u16,
                pan_id: state.registers[base + 1] as u16,
                extended_address,
            })
        });
        let security_control = state.registers[0x128 / 4];
        let mut security_address = [0_u8; 8];
        security_address[..4].copy_from_slice(&state.registers[0x12c / 4].to_le_bytes());
        security_address[4..].copy_from_slice(&state.registers[0x130 / 4].to_le_bytes());
        let mut security_key = [0_u8; 16];
        for word in 0..4 {
            security_key[word * 4..word * 4 + 4]
                .copy_from_slice(&state.registers[0x134 / 4 + word].to_le_bytes());
        }
        EspIeee802154Configuration {
            pans,
            promiscuous: conf & (1 << 7) != 0,
            automatic_ack_transmit: conf & 1 != 0,
            automatic_ack_receive: conf & (1 << 3) != 0,
            frame_pending: state.registers[0x6c / 4] & 1 != 0,
            cca_threshold_dbm: state.registers[0x54 / 4] as u8 as i8,
            cca_mode: ((state.registers[0x54 / 4] >> 14) & 3) as u8,
            ed_duration_symbols: state.registers[0x50 / 4] & 0x00ff_ffff,
            transmit_security: security_control & 1 != 0,
            security_offset: ((security_control >> 8) & 0x7f) as u8,
            security_address,
            security_key,
        }
    }

    /// Returns whether firmware has armed the receiver.
    pub fn receiving(&self) -> bool {
        self.state
            .lock()
            .expect("802.15.4 state lock poisoned")
            .registers[0x88 / 4]
            & (1 << 9)
            != 0
    }

    /// Completes a transmit operation and raises TX-done state.
    pub fn complete_tx(&self) {
        let mut state = self.state.lock().expect("802.15.4 state lock poisoned");
        state.registers[0x84 / 4] = 0;
        state.registers[0x88 / 4] &= !((1 << 8) | 0xf);
        state.registers[0x64 / 4] |= IEEE802154_EVENT_TX_DONE;
    }

    /// Completes TX and enters the hardware-owned ACK receive phase.
    pub fn complete_tx_expect_ack(&self, sequence: u8) {
        let mut state = self.state.lock().expect("802.15.4 state lock poisoned");
        state.registers[0x84 / 4] = 0;
        state.registers[0x88 / 4] = (1 << 9) | 1;
        state.registers[0x80 / 4] = 1 << 16;
        state.registers[0x64 / 4] |= IEEE802154_EVENT_TX_DONE;
        state.awaiting_ack_sequence = Some(sequence);
    }

    /// Returns the sequence number required by the active ACK receive phase.
    pub fn awaiting_ack_sequence(&self) -> Option<u8> {
        self.state
            .lock()
            .expect("802.15.4 state lock poisoned")
            .awaiting_ack_sequence
    }

    /// Completes reception of a validated frame of `length` bytes.
    pub fn complete_rx(&self, length: u8) {
        let mut state = self.state.lock().expect("802.15.4 state lock poisoned");
        state.registers[0xa4 / 4] = u32::from(length.min(127));
        state.registers[0x80 / 4] = 0;
        state.registers[0x88 / 4] &= !((1 << 9) | 0xf);
        state.registers[0x64 / 4] |= IEEE802154_EVENT_RX_DONE;
    }

    /// Records completion of an automatically transmitted ACK.
    pub fn complete_ack_tx(&self) {
        let mut state = self.state.lock().expect("802.15.4 state lock poisoned");
        state.registers[0x64 / 4] |= 1 << 2;
    }

    /// Records completion of an expected ACK receive operation.
    pub fn complete_ack_rx(&self, length: u8) {
        let mut state = self.state.lock().expect("802.15.4 state lock poisoned");
        state.registers[0xa4 / 4] = u32::from(length.min(127));
        state.registers[0x80 / 4] = 0;
        state.registers[0x88 / 4] &= !((1 << 9) | 0xf);
        state.registers[0x64 / 4] |= 1 << 3;
        state.awaiting_ack_sequence = None;
    }

    /// Records a receive filter failure and increments its debug counter.
    pub fn record_filter_failure(&self) {
        let mut state = self.state.lock().expect("802.15.4 state lock poisoned");
        state.registers[0x80 / 4] = (5 << 4) | 1;
        state.registers[0x88 / 4] &= !((1 << 9) | 0xf);
        state.registers[0x154 / 4] = state.registers[0x154 / 4].wrapping_add(1);
        state.registers[0x64 / 4] |= IEEE802154_EVENT_RX_ABORT;
    }

    /// Records an AES-CCM* transmit security failure.
    pub fn record_security_failure(&self, reason: u8) {
        let mut state = self.state.lock().expect("802.15.4 state lock poisoned");
        state.registers[0x84 / 4] = (19 << 4) | (u32::from(reason & 0x0f) << 16);
        state.registers[0x88 / 4] = 0;
        state.registers[0x178 / 4] = state.registers[0x178 / 4].wrapping_add(1);
        state.registers[0x64 / 4] |= IEEE802154_EVENT_TX_ABORT;
    }

    /// Completes a CCA-gated transmit with the published busy abort reason.
    pub fn record_cca_busy(&self) {
        let mut state = self.state.lock().expect("802.15.4 state lock poisoned");
        state.registers[0x84 / 4] = 25 << 4;
        state.registers[0x88 / 4] = 0;
        state.registers[0x17c / 4] = state.registers[0x17c / 4].wrapping_add(1);
        state.registers[0x64 / 4] |= IEEE802154_EVENT_TX_ABORT;
    }

    /// Completes energy detection with an RSSI byte and CCA result.
    pub fn complete_energy_detect(&self, rss: i8, busy: bool) {
        let mut state = self.state.lock().expect("802.15.4 state lock poisoned");
        let configuration = state.registers[0x54 / 4] & !((0xff << 16) | (1 << 24));
        state.registers[0x54 / 4] =
            configuration | (u32::from(rss as u8) << 16) | (u32::from(busy) << 24);
        state.registers[0x88 / 4] &= !((1 << 10) | 0xf);
        state.registers[0x64 / 4] |= IEEE802154_EVENT_ED_DONE;
    }

    /// Aborts active TX or RX work using the published reason encoding.
    pub fn abort(&self, transmit: bool, reason: u8) {
        let mut state = self.state.lock().expect("802.15.4 state lock poisoned");
        if transmit {
            state.registers[0x84 / 4] = u32::from(reason & 0x1f) << 4;
            state.registers[0x64 / 4] |= IEEE802154_EVENT_TX_ABORT;
        } else {
            state.registers[0x80 / 4] = u32::from(reason & 0x1f) << 4;
            state.registers[0x64 / 4] |= IEEE802154_EVENT_RX_ABORT;
        }
        state.registers[0x88 / 4] = 0;
        state.awaiting_ack_sequence = None;
    }

    /// Whether an enabled event currently asserts the Zigbee-MAC interrupt.
    pub fn interrupt_pending(&self) -> bool {
        let state = self.state.lock().expect("802.15.4 state lock poisoned");
        state.registers[0x60 / 4] & state.registers[0x64 / 4] & IEEE802154_EVENT_MASK != 0
    }
}

/// ESP32-C6 IEEE 802.15.4 MAC register frontend.
pub struct EspIeee802154 {
    name: String,
    state: Arc<Mutex<Ieee802154State>>,
}

impl EspIeee802154 {
    /// Creates a reset MAC and its explicit host-side handle.
    pub fn new(name: impl Into<String>) -> (Self, EspIeee802154Handle) {
        let state = Arc::new(Mutex::new(Ieee802154State {
            registers: [0; 98],
            commands: VecDeque::new(),
            timer_started: [None, None],
            awaiting_ack_sequence: None,
        }));
        state.lock().expect("802.15.4 state lock poisoned").reset();
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            EspIeee802154Handle { state },
        )
    }

    fn execute_command(state: &mut Ieee802154State, command: EspIeee802154Command, at: SimTime) {
        state.commands.push_back(command);
        match command {
            EspIeee802154Command::TxStart
            | EspIeee802154Command::CcaTxStart
            | EspIeee802154Command::TestTxStart => {
                state.registers[0x88 / 4] = (1 << 8) | 1;
                state.registers[0x84 / 4] = 1;
            }
            EspIeee802154Command::RxStart | EspIeee802154Command::TestRxStart => {
                state.registers[0x88 / 4] = (1 << 9) | 1;
                state.registers[0x80 / 4] = 1 << 16;
            }
            EspIeee802154Command::EnergyDetectStart => {
                state.registers[0x88 / 4] = (1 << 10) | 1;
            }
            EspIeee802154Command::Stop | EspIeee802154Command::TestStop => {
                state.registers[0x88 / 4] = 0;
                state.awaiting_ack_sequence = None;
            }
            EspIeee802154Command::Timer0Start => state.timer_started[0] = Some(at),
            EspIeee802154Command::Timer0Stop => state.timer_started[0] = None,
            EspIeee802154Command::Timer1Start => state.timer_started[1] = Some(at),
            EspIeee802154Command::Timer1Stop => state.timer_started[1] = None,
        }
    }

    fn writable_mask(offset: usize) -> u32 {
        match offset {
            0x00 => 0xff,
            0x04 => 0xfbc0_58eb,
            0x08 | 0x0c | 0x18 | 0x1c | 0x28 | 0x2c | 0x38 | 0x3c => 0xffff,
            0x10 | 0x14 | 0x20 | 0x24 | 0x30 | 0x34 | 0x40 | 0x44 => u32::MAX,
            0x48 => 0x7f,
            0x4c => 0x1f,
            0x50 => 0x0f00_ffff,
            0x54 => 0x0000_ffff,
            0x58 => 0x03ff_00ff,
            0x5c => 0xffff,
            0x60 | 0x64 => IEEE802154_EVENT_MASK,
            0x68 | 0x78 => 0x7fff_ffff,
            0x6c => 0xffff_0001,
            0x70 => 0x1ff,
            0x7c => u32::MAX,
            0xa8 | 0xb0 | 0xb8 | 0xc4 | 0xc8 => u32::MAX,
            0xd0 | 0xe0 => u32::MAX,
            0xd4 => 0x7,
            0xe4 => 0x0300_0007,
            0xf0 | 0xf4 => u32::MAX,
            0x100..=0x120 => u32::MAX,
            0x128 => 0x7f01,
            0x12c..=0x140 => u32::MAX,
            0x180 => 0x7fff,
            0x184 => u32::MAX,
            _ => 0,
        }
    }
}

impl Device for EspIeee802154 {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        let index = checked_word_index(&self.name, offset, width)?;
        let mut state = self.state.lock().expect("802.15.4 state lock poisoned");
        state.update_timers(at);
        state
            .registers
            .get(index)
            .copied()
            .map(u64::from)
            .ok_or_else(|| DeviceError::new(format!("{} read at {offset:#x}", self.name)))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        let index = checked_word_index(&self.name, offset, width)?;
        let mut state = self.state.lock().expect("802.15.4 state lock poisoned");
        state.update_timers(at);
        if index >= state.registers.len() {
            return Err(DeviceError::new(format!(
                "{} write at {offset:#x}",
                self.name
            )));
        }
        let offset = index * 4;
        let value = value as u32;
        if offset == 0 {
            state.registers[0] = value & 0xff;
            if let Some(command) = EspIeee802154Command::from_opcode(value as u8) {
                Self::execute_command(&mut state, command, at);
            }
            return Ok(());
        }
        if offset == 0x64 {
            state.registers[index] &= !(value & IEEE802154_EVENT_MASK);
            return Ok(());
        }
        if offset == 0x180 {
            for (bit, counter_offset) in [
                (0, 0x168),
                (1, 0x17c),
                (2, 0x150),
                (3, 0x14c),
                (4, 0x178),
                (5, 0x174),
                (6, 0x164),
                (7, 0x170),
                (8, 0x160),
                (9, 0x16c),
                (10, 0x15c),
                (11, 0x158),
                (12, 0x154),
                (13, 0x148),
                (14, 0x144),
            ] {
                if value & (1 << bit) != 0 {
                    state.registers[counter_offset / 4] = 0;
                }
            }
            return Ok(());
        }
        let mask = Self::writable_mask(offset);
        state.registers[index] = (state.registers[index] & !mask) | (value & mask);
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state
            .lock()
            .expect("802.15.4 state lock poisoned")
            .reset();
    }
}
