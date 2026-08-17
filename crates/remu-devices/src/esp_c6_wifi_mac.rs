const C6_WIFI_MAC_RESET_CONTROL: u64 = 0xddc;
const C6_WIFI_MAC_RESET_START: u32 = 1 << 1;
const C6_WIFI_MAC_RESET_READY: u32 = 1 << 0;
const C6_WIFI_MAC_INTERRUPT_MASK: u64 = 0xc40;
const C6_WIFI_MAC_INTERRUPT_EVENT: u64 = 0xc48;
const C6_WIFI_MAC_INTERRUPT_CLEAR: u64 = 0xc4c;
const C6_WIFI_MAC_EVENT_TX_DONE: u32 = 1 << 7;
const C6_WIFI_MAC_EVENT_RX_DONE: u32 = 1 << 14;
const C6_WIFI_MAC_RX_CONTROL: u64 = 0x080;
const C6_WIFI_MAC_RX_DESCRIPTOR_RELOAD: u32 = 1 << 0;
const C6_WIFI_MAC_RX_BASE: u64 = 0x084;
const C6_WIFI_MAC_RX_NEXT: u64 = 0x088;
const C6_WIFI_MAC_RX_LAST: u64 = 0x08c;
const C6_WIFI_MAC_RX_ADDRESS_HIGH: u64 = 0xc70;
const C6_WIFI_MAC_INTERFACE_ADDRESS_LOW: u64 = 0x05c;
const C6_WIFI_MAC_INTERFACE_ADDRESS_HIGH: u64 = 0x060;
const C6_WIFI_MAC_INTERFACE_ADDRESS_STRIDE: u64 = 8;
const C6_WIFI_MAC_INTERFACE_ADDRESS_COUNT: usize = 4;
const C6_WIFI_MAC_INTERFACE_ADDRESS_VALID: u32 = 1 << 16;
const C6_WIFI_MAC_TX_QUEUE_STATE_CLEAR: u64 = 0xcb4;
const C6_WIFI_MAC_TX_QUEUE_STATE: u64 = 0xcb8;
const C6_WIFI_MAC_TX_QUEUE_CONTROL_HIGH: u64 = 0xd6c;
const C6_WIFI_MAC_TX_QUEUE_CONTROL_LOW: u64 = 0xd1c;
const C6_WIFI_MAC_TX_QUEUE_ENABLE: u32 = 3 << 30;
const C6_WIFI_MAC_TX_QUEUE_ENABLED: u32 = 1 << 31;
const C6_WIFI_MAC_TX_QUEUE_TIMEOUT_HIGH: u64 = 0xd68;
const C6_WIFI_MAC_TX_QUEUE_TIMEOUT_STRIDE: u64 = 0x10;
const C6_WIFI_MAC_TX_QUEUE_PROTECTION_HIGH: u64 = 0xd60;
const C6_WIFI_MAC_TX_QUEUE_RTS_ENABLED: u32 = 1 << 31;
const C6_WIFI_MAC_TX_QUEUE_COMPLETION_HIGH: u64 = 0x14ec;
const C6_WIFI_MAC_TX_QUEUE_COMPLETION_COUNT_HIGH: u64 = 0x14e8;
const C6_WIFI_MAC_TX_QUEUE_COMPLETION_STRIDE: u64 = 0x74;
const C6_WIFI_MAC_TX_QUEUE_COMPLETION_STATUS: u32 = 0xf << 12;
const C6_WIFI_MAC_TX_QUEUE_COMPLETION_COUNT: u32 = 0xff << 16;
const C6_WIFI_MAC_TX_QUEUE_BA_STATUS_HIGH: u64 = 0x14dc;
const C6_WIFI_MAC_TX_QUEUE_BA_BITMAP_LOW_HIGH: u64 = 0x14d8;
const C6_WIFI_MAC_TX_QUEUE_BA_BITMAP_HIGH_HIGH: u64 = 0x14d4;
const C6_WIFI_MAC_RX_BA_CONTROL_HIGH: u64 = 0x290;
const C6_WIFI_MAC_RX_BA_MAC_HIGH_HIGH: u64 = 0x294;
const C6_WIFI_MAC_RX_BA_MAC_LOW_HIGH: u64 = 0x298;
const C6_WIFI_MAC_RX_BA_SEQUENCE_HIGH: u64 = 0x2a0;
const C6_WIFI_MAC_RX_BA_BITMAP_LOW_HIGH: u64 = 0x2a8;
const C6_WIFI_MAC_RX_BA_BITMAP_HIGH_HIGH: u64 = 0x2b0;
const C6_WIFI_MAC_RX_BA_STRIDE: u64 = 0x28;
const C6_WIFI_MAC_RX_BA_COUNT: usize = 8;
const C6_WIFI_MAC_RX_BA_VALID: u32 = 1 << 31;
const C6_WIFI_MAC_RX_BA_ACTIVE: u32 = 3 << 30;
const C6_WIFI_MAC_RX_BA_MODE: u32 = 5;
const C6_WIFI_MAC_CRYPTO_VALID: u64 = 0x814;
const C6_WIFI_MAC_CRYPTO_TABLE: u64 = 0x1800;
const C6_WIFI_MAC_CRYPTO_ENTRY_STRIDE: u64 = 0x28;
const C6_WIFI_MAC_CRYPTO_ENTRY_WORDS: usize = 10;
const C6_WIFI_MAC_CRYPTO_ENTRY_COUNT: usize = 32;

struct EspC6WifiMacState {
    registers: Vec<u32>,
    pending_tx: VecDeque<EspC6WifiTxDescriptor>,
    active_tx: u32,
    rx_descriptor: Option<u32>,
    reset_generation: u64,
}

impl EspC6WifiMacState {
    fn reset(&mut self) {
        self.registers.fill(0);
        self.pending_tx.clear();
        self.active_tx = 0;
        self.rx_descriptor = None;
        self.reset_generation = self.reset_generation.wrapping_add(1);
    }
}

/// One native ESP32-C6 Wi-Fi transmit descriptor submitted by guest firmware.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EspC6WifiTxDescriptor {
    /// Native MAC queue index.
    pub queue: u8,
    /// Reconstructed DRAM address of the first DMA descriptor.
    pub address: u32,
}

/// One native ESP32-C6 Wi-Fi receive descriptor owned by the MAC DMA engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EspC6WifiRxDescriptor {
    /// Full DRAM address programmed through the native receive-base register.
    pub address: u32,
}

/// Scheduler-facing view of ESP32-C6 Wi-Fi MAC interrupt state.
#[derive(Clone)]
pub struct EspC6WifiMacHandle {
    state: Arc<Mutex<EspC6WifiMacState>>,
}

impl EspC6WifiMacHandle {
    /// Returns the generation incremented by each native MAC reset command.
    pub fn reset_generation(&self) -> u64 {
        self.state
            .lock()
            .expect("ESP32-C6 Wi-Fi MAC lock poisoned")
            .reset_generation
    }

    fn crypto_key_entry_from_state(
        state: &EspC6WifiMacState,
        slot: usize,
    ) -> Option<crate::EspWifiCryptoKeyEntry> {
        if slot >= C6_WIFI_MAC_CRYPTO_ENTRY_COUNT
            || state.registers[C6_WIFI_MAC_CRYPTO_VALID as usize / 4] & (1 << slot) == 0
        {
            return None;
        }
        let base =
            (C6_WIFI_MAC_CRYPTO_TABLE + slot as u64 * C6_WIFI_MAC_CRYPTO_ENTRY_STRIDE) as usize / 4;
        let words = state
            .registers
            .get(base..base + C6_WIFI_MAC_CRYPTO_ENTRY_WORDS)?;
        let mut key = [0_u8; 32];
        for (destination, word) in key.chunks_exact_mut(4).zip(&words[2..]) {
            destination.copy_from_slice(&word.to_le_bytes());
        }
        Some(crate::EspWifiCryptoKeyEntry {
            slot: slot as u8,
            match_low: words[0],
            control: words[1],
            key,
        })
    }

    fn rx_block_ack_slot(state: &EspC6WifiMacState, peer: &[u8; 6], tid: u8) -> Option<usize> {
        (0..C6_WIFI_MAC_RX_BA_COUNT).find(|slot| {
            let distance = *slot as u64 * C6_WIFI_MAC_RX_BA_STRIDE;
            let control = state.registers[(C6_WIFI_MAC_RX_BA_CONTROL_HIGH - distance) as usize / 4];
            let mac_low = state.registers[(C6_WIFI_MAC_RX_BA_MAC_LOW_HIGH - distance) as usize / 4];
            let mac_high =
                state.registers[(C6_WIFI_MAC_RX_BA_MAC_HIGH_HIGH - distance) as usize / 4];
            control & C6_WIFI_MAC_RX_BA_ACTIVE == C6_WIFI_MAC_RX_BA_ACTIVE
                && control & 0x0fff == C6_WIFI_MAC_RX_BA_MODE
                && (control >> 12) as u8 & 0xf == tid & 0xf
                && mac_low == u32::from_le_bytes(peer[..4].try_into().unwrap())
                && mac_high as u16 == u16::from_le_bytes(peer[4..].try_into().unwrap())
        })
    }

    /// Rejects active RX BA encodings that the pinned firmware never creates.
    pub fn validate_block_ack_sessions(&self) -> Result<(), String> {
        let state = self.state.lock().expect("ESP32-C6 Wi-Fi MAC lock poisoned");
        for slot in 0..C6_WIFI_MAC_RX_BA_COUNT {
            let distance = slot as u64 * C6_WIFI_MAC_RX_BA_STRIDE;
            let control = state.registers[(C6_WIFI_MAC_RX_BA_CONTROL_HIGH - distance) as usize / 4];
            if control & C6_WIFI_MAC_RX_BA_VALID != 0
                && (control & C6_WIFI_MAC_RX_BA_ACTIVE != C6_WIFI_MAC_RX_BA_ACTIVE
                    || control & 0x0fff != C6_WIFI_MAC_RX_BA_MODE)
            {
                return Err(format!(
                    "RX block-ACK slot {slot} has impossible active control {control:#010x}"
                ));
            }
        }
        Ok(())
    }

    /// Returns one valid firmware-programmed native crypto-table entry.
    pub fn crypto_key_entry(&self, slot: u8) -> Option<crate::EspWifiCryptoKeyEntry> {
        let state = self.state.lock().expect("ESP32-C6 Wi-Fi MAC lock poisoned");
        Self::crypto_key_entry_from_state(&state, usize::from(slot))
    }

    /// Selects the native CCMP key for a firmware-preformatted protected transmit.
    pub fn select_ccmp_tx_key(&self, frame: &[u8]) -> Result<[u8; 16], String> {
        let selector = crate::esp_wifi_common::EspWifiCcmpTxSelector::parse(frame)?;
        let state = self.state.lock().expect("ESP32-C6 Wi-Fi MAC lock poisoned");
        let interface = (0..C6_WIFI_MAC_INTERFACE_ADDRESS_COUNT)
            .find(|interface| {
                let offset = *interface as u64 * C6_WIFI_MAC_INTERFACE_ADDRESS_STRIDE;
                let low =
                    state.registers[(C6_WIFI_MAC_INTERFACE_ADDRESS_LOW + offset) as usize / 4];
                let high =
                    state.registers[(C6_WIFI_MAC_INTERFACE_ADDRESS_HIGH + offset) as usize / 4];
                high & C6_WIFI_MAC_INTERFACE_ADDRESS_VALID != 0
                    && selector.transmitter
                        == [
                            low as u8,
                            (low >> 8) as u8,
                            (low >> 16) as u8,
                            (low >> 24) as u8,
                            high as u8,
                            (high >> 8) as u8,
                        ]
            })
            .ok_or_else(|| {
                format!(
                    "hardware-protected TX transmitter {:02x?} does not match a valid interface",
                    selector.transmitter
                )
            })? as u8;
        crate::esp_wifi_common::select_esp_wifi_ccmp_tx_key(
            (0..C6_WIFI_MAC_CRYPTO_ENTRY_COUNT)
                .filter_map(|slot| Self::crypto_key_entry_from_state(&state, slot)),
            &selector,
            interface,
        )
    }

    /// Rejects valid crypto slots with a control class the pinned HAL cannot emit.
    pub fn validate_crypto_key_table(&self) -> Result<(), String> {
        let state = self.state.lock().expect("ESP32-C6 Wi-Fi MAC lock poisoned");
        let valid = state.registers[C6_WIFI_MAC_CRYPTO_VALID as usize / 4];
        for slot in 0..C6_WIFI_MAC_CRYPTO_ENTRY_COUNT {
            if valid & (1 << slot) == 0 {
                continue;
            }
            let entry = Self::crypto_key_entry_from_state(&state, slot)
                .expect("C6 crypto table fits its native register window");
            let control_class = (entry.control >> 21) & 7;
            if !matches!(control_class, 3 | 6 | 7) {
                return Err(format!(
                    "crypto key slot {slot} has impossible control class {control_class} in {:#010x}",
                    entry.control
                ));
            }
        }
        Ok(())
    }

    /// Whether an enabled native MAC event asserts interrupt source zero.
    pub fn interrupt_pending(&self) -> bool {
        let state = self.state.lock().expect("ESP32-C6 Wi-Fi MAC lock poisoned");
        let mask = state.registers[C6_WIFI_MAC_INTERRUPT_MASK as usize / 4];
        let events = state.registers[C6_WIFI_MAC_INTERRUPT_EVENT as usize / 4];
        mask & events != 0
    }

    /// Removes the oldest native DMA transmit submitted by firmware.
    pub fn take_tx_descriptor(&self) -> Option<EspC6WifiTxDescriptor> {
        self.state
            .lock()
            .expect("ESP32-C6 Wi-Fi MAC lock poisoned")
            .pending_tx
            .pop_front()
    }

    /// Returns the firmware-programmed twelve-bit ACK timeout for a queue.
    pub fn tx_ack_timeout(&self, queue: u8) -> u16 {
        let state = self.state.lock().expect("ESP32-C6 Wi-Fi MAC lock poisoned");
        let offset = C6_WIFI_MAC_TX_QUEUE_TIMEOUT_HIGH
            .saturating_sub(u64::from(queue) * C6_WIFI_MAC_TX_QUEUE_TIMEOUT_STRIDE);
        state
            .registers
            .get(offset as usize / 4)
            .copied()
            .unwrap_or_default() as u16
            & 0x0fff
    }

    /// Whether the pinned HAL requested hardware-generated RTS protection.
    ///
    /// `mac_tx_set_plcp0` passes software-descriptor bit eight to
    /// `hal_he_set_tx_protection`, which stores it in bit 31 of the descending
    /// per-queue register at offset `0xd60`.
    pub fn tx_rts_enabled(&self, queue: u8) -> bool {
        let state = self.state.lock().expect("ESP32-C6 Wi-Fi MAC lock poisoned");
        let offset = C6_WIFI_MAC_TX_QUEUE_PROTECTION_HIGH
            .saturating_sub(u64::from(queue) * C6_WIFI_MAC_TX_QUEUE_TIMEOUT_STRIDE);
        state
            .registers
            .get(offset as usize / 4)
            .is_some_and(|value| value & C6_WIFI_MAC_TX_QUEUE_RTS_ENABLED != 0)
    }

    /// Whether a queue has been kicked but has not yet received a completion.
    pub fn tx_active(&self, queue: u8) -> bool {
        let state = self.state.lock().expect("ESP32-C6 Wi-Fi MAC lock poisoned");
        state.active_tx & (1 << queue) != 0
    }

    /// Publishes one native completion record and raises the TX interrupt.
    pub fn complete_tx(&self, queue: u8, outcome: crate::EspWifiTxOutcome) -> bool {
        let successful_mpdu_count = u8::from(outcome == crate::EspWifiTxOutcome::Success);
        self.complete_tx_record(queue, outcome, successful_mpdu_count, None)
    }

    /// Publishes the complete recovered TX/BA record and raises the TX interrupt.
    pub fn complete_tx_record(
        &self,
        queue: u8,
        outcome: crate::EspWifiTxOutcome,
        successful_mpdu_count: u8,
        block_ack: Option<crate::EspWifiTxBlockAck>,
    ) -> bool {
        let mut state = self.state.lock().expect("ESP32-C6 Wi-Fi MAC lock poisoned");
        let bit = 1_u32 << queue;
        if state.active_tx & bit == 0 {
            return false;
        }
        let offset = C6_WIFI_MAC_TX_QUEUE_COMPLETION_HIGH
            .saturating_sub(u64::from(queue) * C6_WIFI_MAC_TX_QUEUE_COMPLETION_STRIDE);
        let Some(completion) = state.registers.get_mut(offset as usize / 4) else {
            return false;
        };
        *completion =
            (*completion & !C6_WIFI_MAC_TX_QUEUE_COMPLETION_STATUS) | (outcome.status() << 12);
        let count_offset = C6_WIFI_MAC_TX_QUEUE_COMPLETION_COUNT_HIGH
            .saturating_sub(u64::from(queue) * C6_WIFI_MAC_TX_QUEUE_COMPLETION_STRIDE);
        let Some(count) = state.registers.get_mut(count_offset as usize / 4) else {
            return false;
        };
        // hal_mac_get_txq_complete extracts the number of successfully
        // completed MPDUs from bits 16..23 of the preceding completion word.
        *count = (*count & !C6_WIFI_MAC_TX_QUEUE_COMPLETION_COUNT)
            | (u32::from(successful_mpdu_count) << 16);
        let distance = u64::from(queue) * C6_WIFI_MAC_TX_QUEUE_COMPLETION_STRIDE;
        let ba_status_index = (C6_WIFI_MAC_TX_QUEUE_BA_STATUS_HIGH - distance) as usize / 4;
        let ba_low_index = (C6_WIFI_MAC_TX_QUEUE_BA_BITMAP_LOW_HIGH - distance) as usize / 4;
        let ba_high_index = (C6_WIFI_MAC_TX_QUEUE_BA_BITMAP_HIGH_HIGH - distance) as usize / 4;
        let block_ack = block_ack.unwrap_or(crate::EspWifiTxBlockAck {
            status: 0,
            starting_sequence: 0,
            bitmap: 0,
        });
        state.registers[ba_status_index] = (state.registers[ba_status_index] & !0x000f_0fff)
            | (u32::from(block_ack.status & 0x0f) << 16)
            | u32::from(block_ack.starting_sequence & 0x0fff);
        state.registers[ba_low_index] = block_ack.bitmap as u32;
        state.registers[ba_high_index] = (block_ack.bitmap >> 32) as u32;
        let control_offset = C6_WIFI_MAC_TX_QUEUE_CONTROL_HIGH
            .saturating_sub(u64::from(queue) * C6_WIFI_MAC_TX_QUEUE_TIMEOUT_STRIDE);
        let Some(control) = state.registers.get_mut(control_offset as usize / 4) else {
            return false;
        };
        // The native queue's enabled bit is hardware-owned after a kick.  It
        // drops at completion while the adjacent valid bit remains available
        // for the HAL to invalidate explicitly.  Keeping enabled asserted
        // makes hal_mac_deinit_twt_tx flush an already completed and recycled
        // MSDU when the station enters its TWT sleep interval.
        *control &= !C6_WIFI_MAC_TX_QUEUE_ENABLED;
        state.active_tx &= !bit;
        state.registers[C6_WIFI_MAC_TX_QUEUE_STATE as usize / 4] |= bit;
        state.registers[C6_WIFI_MAC_INTERRUPT_EVENT as usize / 4] |= C6_WIFI_MAC_EVENT_TX_DONE;
        true
    }

    /// Returns the current firmware-provided receive descriptor, if armed.
    pub fn rx_descriptor(&self) -> Option<EspC6WifiRxDescriptor> {
        self.state
            .lock()
            .expect("ESP32-C6 Wi-Fi MAC lock poisoned")
            .rx_descriptor
            .map(|address| EspC6WifiRxDescriptor { address })
    }

    /// Returns the native RX-interface match bitmap for an 802.11 receiver address.
    ///
    /// Vendor firmware programs one address-filter slot per virtual interface.
    /// Exact receiver addresses select their configured slot. Other group frames
    /// match every valid slot. A reset MAC has no valid slots, in which case
    /// freestanding firmware receives through the hardware-default slot zero.
    pub fn rx_match_mask(&self, receiver: &[u8]) -> u8 {
        let Some(receiver) = receiver.get(..6) else {
            return 0;
        };
        let state = self.state.lock().expect("ESP32-C6 Wi-Fi MAC lock poisoned");
        let mut configured = 0_u8;
        let mut matches = 0_u8;
        for interface in 0..C6_WIFI_MAC_INTERFACE_ADDRESS_COUNT {
            let offset = interface as u64 * C6_WIFI_MAC_INTERFACE_ADDRESS_STRIDE;
            let low = state.registers[(C6_WIFI_MAC_INTERFACE_ADDRESS_LOW + offset) as usize / 4];
            let high = state.registers[(C6_WIFI_MAC_INTERFACE_ADDRESS_HIGH + offset) as usize / 4];
            if high & C6_WIFI_MAC_INTERFACE_ADDRESS_VALID == 0 {
                continue;
            }
            let bit = 1_u8 << interface;
            configured |= bit;
            let address = [
                low as u8,
                (low >> 8) as u8,
                (low >> 16) as u8,
                (low >> 24) as u8,
                high as u8,
                (high >> 8) as u8,
            ];
            if receiver == address {
                matches |= bit;
            }
        }
        if configured == 0 {
            1
        } else if matches != 0 {
            matches
        } else if receiver[0] & 1 != 0 {
            configured
        } else {
            0
        }
    }

    /// Records a received QoS MPDU in the matching firmware-owned RX BA window.
    ///
    /// The register layout and descending slot stride are the native C6
    /// `hal_agreement_add_rx_ba` contract. Sequence arithmetic is modulo the
    /// twelve-bit 802.11 sequence space; frames older than the current window
    /// do not move it backwards.
    pub fn record_block_ack_mpdu(&self, peer: &[u8; 6], tid: u8, sequence: u16) -> bool {
        let mut state = self.state.lock().expect("ESP32-C6 Wi-Fi MAC lock poisoned");
        let Some(slot) = Self::rx_block_ack_slot(&state, peer, tid) else {
            return false;
        };
        let distance = slot as u64 * C6_WIFI_MAC_RX_BA_STRIDE;
        let sequence_index = (C6_WIFI_MAC_RX_BA_SEQUENCE_HIGH - distance) as usize / 4;
        let bitmap_low_index = (C6_WIFI_MAC_RX_BA_BITMAP_LOW_HIGH - distance) as usize / 4;
        let bitmap_high_index = (C6_WIFI_MAC_RX_BA_BITMAP_HIGH_HIGH - distance) as usize / 4;
        let mut origin = state.registers[sequence_index] as u16 & 0x0fff;
        let sequence = sequence & 0x0fff;
        let delta = sequence.wrapping_sub(origin) & 0x0fff;
        if delta >= 0x0800 {
            return true;
        }
        let mut bitmap = u64::from(state.registers[bitmap_low_index])
            | (u64::from(state.registers[bitmap_high_index]) << 32);
        if delta < 64 {
            bitmap |= 1_u64 << delta;
        } else {
            let shift = u32::from(delta - 63);
            bitmap = bitmap.checked_shr(shift).unwrap_or(0) | (1_u64 << 63);
            origin = origin.wrapping_add(shift as u16) & 0x0fff;
            state.registers[sequence_index] =
                (state.registers[sequence_index] & !0x0fff) | u32::from(origin);
        }
        state.registers[bitmap_low_index] = bitmap as u32;
        state.registers[bitmap_high_index] = (bitmap >> 32) as u32;
        true
    }

    /// Returns the matching compressed block-ACK bitmap at a requested origin.
    pub fn block_ack_bitmap(&self, peer: &[u8; 6], tid: u8, starting_sequence: u16) -> Option<u64> {
        let state = self.state.lock().expect("ESP32-C6 Wi-Fi MAC lock poisoned");
        let slot = Self::rx_block_ack_slot(&state, peer, tid)?;
        let distance = slot as u64 * C6_WIFI_MAC_RX_BA_STRIDE;
        let origin = state.registers[(C6_WIFI_MAC_RX_BA_SEQUENCE_HIGH - distance) as usize / 4]
            as u16
            & 0x0fff;
        let bitmap =
            u64::from(state.registers[(C6_WIFI_MAC_RX_BA_BITMAP_LOW_HIGH - distance) as usize / 4])
                | (u64::from(
                    state.registers[(C6_WIFI_MAC_RX_BA_BITMAP_HIGH_HIGH - distance) as usize / 4],
                ) << 32);
        let requested = starting_sequence & 0x0fff;
        let forward = requested.wrapping_sub(origin) & 0x0fff;
        if forward < 0x0800 {
            Some(bitmap.checked_shr(u32::from(forward)).unwrap_or(0))
        } else {
            let backward = origin.wrapping_sub(requested) & 0x0fff;
            Some(bitmap.checked_shl(u32::from(backward)).unwrap_or(0))
        }
    }

    /// Advances the native receive ring and raises the hardware RX event.
    pub fn complete_rx_descriptor(&self, address: u32, next: u32) {
        let mut state = self.state.lock().expect("ESP32-C6 Wi-Fi MAC lock poisoned");
        state.registers[C6_WIFI_MAC_RX_NEXT as usize / 4] = next;
        state.registers[C6_WIFI_MAC_RX_LAST as usize / 4] = address & 0x000f_ffff;
        state.registers[C6_WIFI_MAC_RX_ADDRESS_HIGH as usize / 4] = address & 0xfff0_0000;
        state.rx_descriptor = (next != 0).then_some(next);
        state.registers[C6_WIFI_MAC_INTERRUPT_EVENT as usize / 4] |= C6_WIFI_MAC_EVENT_RX_DONE;
    }
}

/// ESP32-C6 Wi-Fi MAC register page.
///
/// The MAC reset command acknowledges synchronously in simulation while all
/// other words retain firmware-visible read/modify/write state.
pub struct EspC6WifiMacRegisters {
    name: String,
    state: Arc<Mutex<EspC6WifiMacState>>,
}

impl EspC6WifiMacRegisters {
    /// Creates a reset Wi-Fi MAC register page.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            state: Arc::new(Mutex::new(EspC6WifiMacState {
                registers: vec![0; 0x3000 / 4],
                pending_tx: VecDeque::new(),
                active_tx: 0,
                rx_descriptor: None,
                reset_generation: 0,
            })),
        }
    }

    /// Returns the interrupt handle coupled to this register frontend.
    pub fn handle(&self) -> EspC6WifiMacHandle {
        EspC6WifiMacHandle {
            state: self.state.clone(),
        }
    }

    fn tx_queue(offset: u64) -> Option<u8> {
        if !(C6_WIFI_MAC_TX_QUEUE_CONTROL_LOW..=C6_WIFI_MAC_TX_QUEUE_CONTROL_HIGH).contains(&offset)
        {
            return None;
        }
        let distance = C6_WIFI_MAC_TX_QUEUE_CONTROL_HIGH - offset;
        distance.is_multiple_of(16).then_some((distance / 16) as u8)
    }
}

impl Device for EspC6WifiMacRegisters {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let index = checked_word_index(&self.name, offset, width)?;
        let state = self.state.lock().expect("ESP32-C6 Wi-Fi MAC lock poisoned");
        state
            .registers
            .get(index)
            .copied()
            .map(u64::from)
            .ok_or_else(|| DeviceError::new(format!("{} read outside native page", self.name)))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        let index = checked_word_index(&self.name, offset, width)?;
        let mut state = self.state.lock().expect("ESP32-C6 Wi-Fi MAC lock poisoned");
        let value = u32::try_from(value)
            .map_err(|_| DeviceError::new("ESP32-C6 Wi-Fi MAC rejects wide writes"))?;
        if offset == C6_WIFI_MAC_INTERRUPT_CLEAR {
            state.registers[C6_WIFI_MAC_INTERRUPT_EVENT as usize / 4] &= !value;
            return Ok(());
        }
        if offset == C6_WIFI_MAC_TX_QUEUE_STATE_CLEAR {
            state.registers[C6_WIFI_MAC_TX_QUEUE_STATE as usize / 4] &= !value;
            state.registers[index] = value;
            return Ok(());
        }
        if offset == C6_WIFI_MAC_RESET_CONTROL && value & C6_WIFI_MAC_RESET_START != 0 {
            state.reset();
            state.registers[index] = value | C6_WIFI_MAC_RESET_READY;
            return Ok(());
        }
        *state.registers.get_mut(index).ok_or_else(|| {
            DeviceError::new(format!("{} write outside native page", self.name))
        })? = value;
        if offset == C6_WIFI_MAC_RX_CONTROL && value & C6_WIFI_MAC_RX_DESCRIPTOR_RELOAD != 0 {
            // hal_mac_rx_set_dscr_reload raises a hardware command bit and
            // hal_mac_rx_is_dscr_reload polls until the MAC consumes it.  The
            // command completes synchronously in this functional frontend;
            // retaining bit zero deadlocks genuine net80211 in the poll loop.
            state.registers[index] &= !C6_WIFI_MAC_RX_DESCRIPTOR_RELOAD;
        }
        if offset == C6_WIFI_MAC_RX_BASE {
            state.rx_descriptor = (value != 0).then_some(value);
            state.registers[C6_WIFI_MAC_RX_NEXT as usize / 4] = value;
        }
        if let Some(queue) = Self::tx_queue(offset)
            && value & C6_WIFI_MAC_TX_QUEUE_ENABLE == C6_WIFI_MAC_TX_QUEUE_ENABLE
        {
            let bit = 1_u32 << queue;
            if state.active_tx & bit != 0
                || state.registers[C6_WIFI_MAC_TX_QUEUE_STATE as usize / 4] & bit != 0
            {
                return Err(DeviceError::new(format!(
                    "ESP32-C6 Wi-Fi queue {queue} was kicked before its previous completion was cleared"
                )));
            }
            let descriptor = 0x4080_0000 | (value & 0x000f_ffff);
            if descriptor != 0x4080_0000 {
                state.active_tx |= bit;
                state.pending_tx.push_back(EspC6WifiTxDescriptor {
                    queue,
                    address: descriptor,
                });
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state
            .lock()
            .expect("ESP32-C6 Wi-Fi MAC lock poisoned")
            .reset();
    }
}
