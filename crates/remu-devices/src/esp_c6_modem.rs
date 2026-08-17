/// ESP32-C6 modem clock/reset register bank.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EspC6ModemBank {
    /// High-performance modem system control at `0x600a9800`.
    Syscon,
    /// Low-power modem control at `0x600af000`.
    Lpcon,
}
#[derive(Clone, Debug)]
struct ModemState {
    syscon: [u32; 512],
    lpcon: [u32; 512],
    wifi_reset_generation: u64,
    ble_reset_generation: u64,
    ieee802154_reset_generation: u64,
    coexist_reset_generation: u64,
}

impl ModemState {
    fn reset(&mut self) {
        self.syscon = [0; 512];
        self.syscon[1] = 1 << 21;
        self.syscon[8] = 4 << 3;
        self.syscon[9] = 35_676_928;
        self.lpcon = [0; 512];
        self.lpcon[10] = (1 << 0) | (1 << 2) | (1 << 4) | (4 << 15);
        self.lpcon[11] = 35_676_736;
        self.wifi_reset_generation = self.wifi_reset_generation.wrapping_add(1);
        self.ble_reset_generation = self.ble_reset_generation.wrapping_add(1);
        self.ieee802154_reset_generation = self.ieee802154_reset_generation.wrapping_add(1);
        self.coexist_reset_generation = self.coexist_reset_generation.wrapping_add(1);
    }
}

/// Host-side view of C6 radio clock, reset, and power eligibility.
#[derive(Clone, Debug)]
pub struct EspC6ModemHandle {
    state: Arc<Mutex<ModemState>>,
}

impl EspC6ModemHandle {
    /// Returns whether the Wi-Fi MAC/APB clock is enabled or forced on and not reset.
    pub fn wifi_ready(&self) -> bool {
        let state = self.state.lock().expect("C6 modem state lock poisoned");
        let clocks = state.syscon[5] | state.syscon[6];
        clocks & ((1 << 9) | (1 << 10)) == ((1 << 9) | (1 << 10))
            && state.syscon[4] & ((1 << 8) | (1 << 10)) == 0
    }

    /// Returns whether the Bluetooth MAC/baseband clocks are enabled or forced on and not reset.
    pub fn ble_ready(&self) -> bool {
        let state = self.state.lock().expect("C6 modem state lock poisoned");
        let clocks = state.syscon[5] | state.syscon[6];
        clocks & ((1 << 17) | (1 << 18)) == ((1 << 17) | (1 << 18))
            && state.syscon[4] & ((1 << 15) | (1 << 16) | (1 << 17) | (1 << 18)) == 0
    }

    /// Returns whether the IEEE 802.15.4 APB/MAC clocks are enabled and not reset.
    pub fn ieee802154_ready(&self) -> bool {
        let state = self.state.lock().expect("C6 modem state lock poisoned");
        let clocks = state.syscon[1] | state.syscon[2];
        clocks & ((1 << 23) | (1 << 24)) == ((1 << 23) | (1 << 24))
            && state.syscon[4] & (1 << 24) == 0
    }

    /// Returns whether the low-power coexistence clock is enabled and not reset.
    pub fn coexistence_ready(&self) -> bool {
        let state = self.state.lock().expect("C6 modem state lock poisoned");
        let clocks = state.lpcon[6] | state.lpcon[7];
        clocks & (1 << 1) != 0
    }

    /// Reset generations for Wi-Fi, BLE, 802.15.4, and coexistence respectively.
    pub fn reset_generations(&self) -> [u64; 4] {
        let state = self.state.lock().expect("C6 modem state lock poisoned");
        [
            state.wifi_reset_generation,
            state.ble_reset_generation,
            state.ieee802154_reset_generation,
            state.coexist_reset_generation,
        ]
    }
}

/// One mapped half of the shared ESP32-C6 modem control block.
pub struct EspC6ModemControl {
    name: String,
    bank: EspC6ModemBank,
    state: Arc<Mutex<ModemState>>,
}

impl EspC6ModemControl {
    /// Creates the paired MODEM_SYSCON and MODEM_LPCON devices.
    pub fn new_pair(
        syscon_name: impl Into<String>,
        lpcon_name: impl Into<String>,
    ) -> (Self, Self, EspC6ModemHandle) {
        let state = Arc::new(Mutex::new(ModemState {
            syscon: [0; 512],
            lpcon: [0; 512],
            wifi_reset_generation: 0,
            ble_reset_generation: 0,
            ieee802154_reset_generation: 0,
            coexist_reset_generation: 0,
        }));
        state.lock().expect("C6 modem state lock poisoned").reset();
        (
            Self {
                name: syscon_name.into(),
                bank: EspC6ModemBank::Syscon,
                state: state.clone(),
            },
            Self {
                name: lpcon_name.into(),
                bank: EspC6ModemBank::Lpcon,
                state: state.clone(),
            },
            EspC6ModemHandle { state },
        )
    }

    fn registers<'a>(&self, state: &'a ModemState) -> &'a [u32] {
        match self.bank {
            EspC6ModemBank::Syscon => &state.syscon,
            EspC6ModemBank::Lpcon => &state.lpcon,
        }
    }

    fn write_mask(&self, index: usize) -> u32 {
        const SYSCON: [u32; 10] = [
            0x0000_0001,
            0xffe0_0000,
            0xffc0_0000,
            0xffff_ff00,
            0xefc7_c500,
            0x00ff_ffff,
            0x00ff_ffff,
            0xffff_ffff,
            0x0000_00ff,
            0x0fff_ffff,
        ];
        const LPCON: [u32; 12] = [
            0x0000_0003,
            0x0000_ffff,
            0x0000_ffff,
            0x0000_ffff,
            0x0000_0001,
            0x0000_0003,
            0x0000_000f,
            0x0000_03ff,
            0xffff_0000,
            0x0000_000f,
            0x000f_ffff,
            0x0fff_ffff,
        ];
        match self.bank {
            EspC6ModemBank::Syscon => SYSCON.get(index).copied().unwrap_or(u32::MAX),
            EspC6ModemBank::Lpcon => LPCON.get(index).copied().unwrap_or(u32::MAX),
        }
    }

    fn record_reset_edges(&self, state: &mut ModemState, old: u32, new: u32) {
        let rising = !old & new;
        match self.bank {
            EspC6ModemBank::Syscon => {
                if rising & ((1 << 8) | (1 << 10) | (1 << 14)) != 0 {
                    state.wifi_reset_generation = state.wifi_reset_generation.wrapping_add(1);
                }
                if rising & ((1 << 15) | (1 << 16) | (1 << 17) | (1 << 18)) != 0 {
                    state.ble_reset_generation = state.ble_reset_generation.wrapping_add(1);
                }
                if rising & (1 << 24) != 0 {
                    state.ieee802154_reset_generation =
                        state.ieee802154_reset_generation.wrapping_add(1);
                }
            }
            EspC6ModemBank::Lpcon => {
                if new & (1 << 1) != 0 {
                    state.coexist_reset_generation = state.coexist_reset_generation.wrapping_add(1);
                }
            }
        }
    }
}

impl Device for EspC6ModemControl {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let index = checked_word_index(&self.name, offset, width)?;
        let state = self.state.lock().expect("C6 modem state lock poisoned");
        self.registers(&state)
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
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        let index = checked_word_index(&self.name, offset, width)?;
        let mut state = self.state.lock().expect("C6 modem state lock poisoned");
        if index >= self.registers(&state).len() {
            return Err(DeviceError::new(format!(
                "{} write at {offset:#x}",
                self.name
            )));
        }
        let old = self.registers(&state)[index];
        let mask = self.write_mask(index);
        let new = (old & !mask) | ((value as u32) & mask);
        if self.bank == EspC6ModemBank::Lpcon && index == 9 {
            self.record_reset_edges(&mut state, old, value as u32);
            return Ok(());
        }
        match self.bank {
            EspC6ModemBank::Syscon => state.syscon[index] = new,
            EspC6ModemBank::Lpcon => state.lpcon[index] = new,
        }
        if self.bank == EspC6ModemBank::Syscon && index == 4 {
            self.record_reset_edges(&mut state, old, new);
        }
        Ok(())
    }

    fn trace_value(&self, offset: u64, width: AccessWidth, _at: SimTime) -> Option<u64> {
        let index = checked_word_index(&self.name, offset, width).ok()?;
        let state = self.state.lock().ok()?;
        self.registers(&state).get(index).copied().map(u64::from)
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state
            .lock()
            .expect("C6 modem state lock poisoned")
            .reset();
    }
}
