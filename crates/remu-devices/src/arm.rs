use super::*;

/// RP2350 boot RAM and its single-owner boot-lock registers.
///
/// The interpreter currently runs one core at a time, so every boot-lock read can acquire the
/// requested lock immediately. Zero writes release it. The remaining window behaves as ordinary
/// little-endian storage.
pub struct Rp2350BootRam {
    name: String,
    bytes: Vec<u8>,
}

/// Functional RP2350 XIP cache-maintenance window.
///
/// Stores to the maintenance alias perform cache operations rather than modifying external flash.
/// The functional emulator has no timing cache, so those operations are acknowledged as ordering
/// points. Reads return zero because no cache tag or data state is exposed by this model.
pub struct Rp2350XipMaintenance {
    name: String,
}

impl Rp2350XipMaintenance {
    /// Creates a cache-maintenance facade.
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl Device for Rp2350XipMaintenance {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(
        &mut self,
        _offset: u64,
        _width: AccessWidth,
        _at: SimTime,
    ) -> Result<u64, DeviceError> {
        Ok(0)
    }

    fn write(
        &mut self,
        _offset: u64,
        _width: AccessWidth,
        _value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        Ok(())
    }
}

impl Rp2350BootRam {
    /// Creates the reset-state 4 KiB boot-RAM aperture.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            bytes: vec![0; 0x1000],
        }
    }

    fn boot_lock(offset: usize, width: AccessWidth) -> bool {
        width == AccessWidth::Word && (0x80c..=0x828).contains(&offset) && offset & 3 == 0
    }
}

impl Device for Rp2350BootRam {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if !width.is_aligned(offset) {
            return Err(DeviceError::new(
                "RP2350 boot RAM access is not naturally aligned",
            ));
        }
        let offset =
            usize::try_from(offset).map_err(|_| DeviceError::new("boot RAM offset overflow"))?;
        if Self::boot_lock(offset, width) {
            return Ok(1);
        }
        let length = usize::from(width.bytes());
        let bytes = self
            .bytes
            .get(offset..offset.saturating_add(length))
            .ok_or_else(|| DeviceError::new("RP2350 boot RAM read is outside its aperture"))?;
        Ok(bytes
            .iter()
            .enumerate()
            .fold(0_u64, |value, (index, byte)| {
                value | (u64::from(*byte) << (index * 8))
            }))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if !width.is_aligned(offset) {
            return Err(DeviceError::new(
                "RP2350 boot RAM access is not naturally aligned",
            ));
        }
        let offset =
            usize::try_from(offset).map_err(|_| DeviceError::new("boot RAM offset overflow"))?;
        if Self::boot_lock(offset, width) {
            return Ok(());
        }
        let length = usize::from(width.bytes());
        let destination = self
            .bytes
            .get_mut(offset..offset.saturating_add(length))
            .ok_or_else(|| DeviceError::new("RP2350 boot RAM write is outside its aperture"))?;
        destination.copy_from_slice(&value.to_le_bytes()[..length]);
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.bytes.fill(0);
    }
}

/// Shared view of an Arm M-profile private peripheral block.
#[derive(Clone)]
pub struct ArmPpbHandle {
    state: Arc<Mutex<ArmPpbState>>,
}

impl ArmPpbHandle {
    /// Returns the current vector-table base programmed through SCB VTOR.
    pub fn vector_base(&self) -> u32 {
        self.state
            .lock()
            .expect("Arm PPB lock poisoned")
            .vector_base
    }

    /// Returns whether firmware enabled an external interrupt line in the NVIC.
    pub fn interrupt_enabled(&self, line: u16) -> bool {
        let bank = usize::from(line / 32);
        let bit = line % 32;
        bank < 8
            && self
                .state
                .lock()
                .expect("Arm PPB lock poisoned")
                .interrupt_enable[bank]
                & (1_u32 << bit)
                != 0
    }

    /// Advances `SysTick` and consumes a pending architectural exception pulse.
    pub fn take_systick_pending(&self, now: SimTime) -> bool {
        let mut state = self.state.lock().expect("Arm PPB lock poisoned");
        ArmPrivatePeripheralBus::advance_systick(&mut state, now);
        std::mem::take(&mut state.systick_pending)
    }

    /// Consumes software-pended, enabled NVIC external interrupts.
    pub fn take_pending_interrupts(&self) -> Vec<u16> {
        let mut state = self.state.lock().expect("Arm PPB lock poisoned");
        let mut pending = Vec::new();
        for bank in 0..8 {
            let ready = state.interrupt_pending[bank] & state.interrupt_enable[bank];
            state.interrupt_pending[bank] &= !ready;
            for bit in 0..32 {
                if ready & (1_u32 << bit) != 0 {
                    pending.push(u16::try_from(bank * 32 + bit).expect("NVIC line fits u16"));
                }
            }
        }
        pending
    }
}

struct ArmPpbState {
    bytes: Vec<u8>,
    vector_base: u32,
    interrupt_enable: [u32; 8],
    interrupt_pending: [u32; 8],
    systick_control: u32,
    systick_reload: u32,
    systick_current: u32,
    systick_countflag: bool,
    systick_pending: bool,
    systick_last_tick: u64,
}

/// Functional Cortex-M `SysTick`, NVIC, and SCB register window.
pub struct ArmPrivatePeripheralBus {
    name: String,
    state: Arc<Mutex<ArmPpbState>>,
    cpuid: u32,
}

impl ArmPrivatePeripheralBus {
    /// Creates a PPB register window with the selected architectural CPUID value.
    pub fn new(name: impl Into<String>, cpuid: u32) -> (Self, ArmPpbHandle) {
        let state = Arc::new(Mutex::new(ArmPpbState {
            bytes: vec![0; 0x1000],
            vector_base: 0,
            interrupt_enable: [0; 8],
            interrupt_pending: [0; 8],
            systick_control: 0,
            systick_reload: 0,
            systick_current: 0,
            systick_countflag: false,
            systick_pending: false,
            systick_last_tick: 0,
        }));
        let handle = ArmPpbHandle {
            state: state.clone(),
        };
        (
            Self {
                name: name.into(),
                state,
                cpuid,
            },
            handle,
        )
    }

    fn write_word(state: &mut ArmPpbState, offset: usize, value: u32) {
        state.bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn advance_systick(state: &mut ArmPpbState, now: SimTime) {
        let mut elapsed = now.ticks().saturating_sub(state.systick_last_tick);
        state.systick_last_tick = now.ticks();
        if state.systick_control & 1 == 0 || elapsed == 0 {
            return;
        }
        let reload = state.systick_reload & 0x00ff_ffff;
        while elapsed != 0 {
            if state.systick_current == 0 {
                state.systick_current = reload;
                elapsed -= 1;
                if reload == 0 {
                    state.systick_countflag = true;
                    if state.systick_control & 2 != 0 {
                        state.systick_pending = true;
                    }
                }
            } else if elapsed >= u64::from(state.systick_current) {
                elapsed -= u64::from(state.systick_current);
                state.systick_current = 0;
                state.systick_countflag = true;
                if state.systick_control & 2 != 0 {
                    state.systick_pending = true;
                }
            } else {
                state.systick_current -= u32::try_from(elapsed).expect("elapsed fits current");
                elapsed = 0;
            }
        }
    }
}

impl Device for ArmPrivatePeripheralBus {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if !width.is_aligned(offset) {
            return Err(DeviceError::new("Arm PPB access is not naturally aligned"));
        }
        let offset =
            usize::try_from(offset).map_err(|_| DeviceError::new("PPB offset overflow"))?;
        let length = usize::from(width.bytes());
        if offset.checked_add(length).is_none_or(|end| end > 0x1000) {
            return Err(DeviceError::new(
                "Arm PPB read is outside the modeled window",
            ));
        }
        let mut state = self.state.lock().expect("Arm PPB lock poisoned");
        Self::advance_systick(&mut state, at);
        match offset {
            0x010 => {
                let value =
                    state.systick_control | if state.systick_countflag { 1 << 16 } else { 0 };
                state.systick_countflag = false;
                Self::write_word(&mut state, 0x010, value);
            }
            0x014 => {
                let value = state.systick_reload;
                Self::write_word(&mut state, 0x014, value);
            }
            0x018 => {
                let value = state.systick_current;
                Self::write_word(&mut state, 0x018, value);
            }
            0x01c => Self::write_word(&mut state, 0x01c, 0),
            0x100..=0x11c if offset.is_multiple_of(4) => {
                let bank = (offset - 0x100) / 4;
                let value = state.interrupt_enable[bank];
                Self::write_word(&mut state, offset, value);
            }
            0x180..=0x19c if offset.is_multiple_of(4) => {
                let bank = (offset - 0x180) / 4;
                let value = state.interrupt_enable[bank];
                Self::write_word(&mut state, offset, value);
            }
            0x200..=0x21c if offset.is_multiple_of(4) => {
                let bank = (offset - 0x200) / 4;
                let value = state.interrupt_pending[bank];
                Self::write_word(&mut state, offset, value);
            }
            0x280..=0x29c if offset.is_multiple_of(4) => {
                let bank = (offset - 0x280) / 4;
                let value = state.interrupt_pending[bank];
                Self::write_word(&mut state, offset, value);
            }
            0xd00 => Self::write_word(&mut state, 0xd00, self.cpuid),
            0xd08 => {
                let value = state.vector_base;
                Self::write_word(&mut state, 0xd08, value);
            }
            _ => {}
        }
        let value = state.bytes[offset..offset + length]
            .iter()
            .enumerate()
            .fold(0_u64, |value, (index, byte)| {
                value | (u64::from(*byte) << (index * 8))
            });
        Ok(value)
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        if !width.is_aligned(offset) {
            return Err(DeviceError::new("Arm PPB access is not naturally aligned"));
        }
        let offset =
            usize::try_from(offset).map_err(|_| DeviceError::new("PPB offset overflow"))?;
        let length = usize::from(width.bytes());
        if offset.checked_add(length).is_none_or(|end| end > 0x1000) {
            return Err(DeviceError::new(
                "Arm PPB write is outside the modeled window",
            ));
        }
        let mut state = self.state.lock().expect("Arm PPB lock poisoned");
        Self::advance_systick(&mut state, at);
        let word = u32::try_from(value & u64::from(u32::MAX)).expect("masked PPB value fits");
        match (offset, width) {
            (0x010, AccessWidth::Word) => state.systick_control = word & 7,
            (0x014, AccessWidth::Word) => state.systick_reload = word & 0x00ff_ffff,
            (0x018, AccessWidth::Word) => {
                state.systick_current = 0;
                state.systick_countflag = false;
            }
            (0x100..=0x11c, AccessWidth::Word) if offset.is_multiple_of(4) => {
                state.interrupt_enable[(offset - 0x100) / 4] |= word;
            }
            (0x180..=0x19c, AccessWidth::Word) if offset.is_multiple_of(4) => {
                state.interrupt_enable[(offset - 0x180) / 4] &= !word;
            }
            (0x200..=0x21c, AccessWidth::Word) if offset.is_multiple_of(4) => {
                state.interrupt_pending[(offset - 0x200) / 4] |= word;
            }
            (0x280..=0x29c, AccessWidth::Word) if offset.is_multiple_of(4) => {
                state.interrupt_pending[(offset - 0x280) / 4] &= !word;
            }
            (0xf00, AccessWidth::Word) if word < 240 => {
                let line = usize::try_from(word).expect("NVIC line fits usize");
                state.interrupt_pending[line / 32] |= 1_u32 << (line % 32);
            }
            (0xd08, AccessWidth::Word) => {
                state.vector_base = word & !0x7f;
                let value = state.vector_base;
                Self::write_word(&mut state, offset, value);
            }
            _ => {
                let bytes = value.to_le_bytes();
                state.bytes[offset..offset + length].copy_from_slice(&bytes[..length]);
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.lock().expect("Arm PPB lock poisoned");
        state.bytes.fill(0);
        state.vector_base = 0;
        state.interrupt_enable = [0; 8];
        state.interrupt_pending = [0; 8];
        state.systick_control = 0;
        state.systick_reload = 0;
        state.systick_current = 0;
        state.systick_countflag = false;
        state.systick_pending = false;
        state.systick_last_tick = 0;
    }
}

/// RP2040 USB controller register slice used during TinyUSB device initialization.
///
/// Endpoint-buffer ownership and host transactions can be layered on this state; this initial
/// model keeps register writes deterministic and presents an attached VBUS source.
pub struct Rp2040UsbController {
    name: String,
    state: Arc<Mutex<Rp2040UsbState>>,
}

struct Rp2040UsbState {
    registers: [u32; 64],
}

/// Host-facing control of the RP2040 USB device controller.
#[derive(Clone)]
pub struct Rp2040UsbHandle {
    state: Arc<Mutex<Rp2040UsbState>>,
}

impl Rp2040UsbState {
    const SIE_STATUS_READ_ONLY: u32 = (1 << 10) | (0b11 << 2) | 1;

    fn raw_interrupts(&self) -> u32 {
        let sie_status = self.registers[0x50 / 4];
        let mut interrupts = 0;
        if self.registers[0x58 / 4] != 0 {
            interrupts |= 1 << 4;
        }
        if sie_status & (1 << 31) != 0 {
            interrupts |= 1 << 5;
        }
        if sie_status & (1 << 27) != 0 {
            interrupts |= 1 << 6;
        }
        if sie_status & (1 << 26) != 0 {
            interrupts |= 1 << 7;
        }
        if sie_status & (1 << 25) != 0 {
            interrupts |= 1 << 8;
        }
        if sie_status & (1 << 24) != 0 {
            interrupts |= 1 << 9;
        }
        if sie_status & (1 << 29) != 0 {
            interrupts |= 1 << 10;
        }
        if sie_status & (1 << 19) != 0 {
            interrupts |= 1 << 12;
        }
        if sie_status & (1 << 0) != 0 {
            interrupts |= 1 << 11;
        }
        if sie_status & (1 << 16) != 0 {
            interrupts |= 1 << 13;
        }
        if sie_status & (1 << 4) != 0 {
            interrupts |= 1 << 14;
        }
        if sie_status & (1 << 11) != 0 {
            interrupts |= 1 << 15;
        }
        if sie_status & (1 << 17) != 0 {
            interrupts |= 1 << 16;
        }
        if sie_status & (1 << 18) != 0 {
            interrupts |= 1 << 3;
        }
        interrupts
    }

    fn masked_interrupts(&self) -> u32 {
        (self.raw_interrupts() & self.registers[0x90 / 4]) | self.registers[0x94 / 4]
    }
}

impl Rp2040UsbHandle {
    /// Returns true after firmware enables the controller and device pull-up.
    pub fn device_connected(&self) -> bool {
        let state = self.state.lock().expect("RP2040 USB lock poisoned");
        state.registers[0x40 / 4] & 1 != 0
            && state.registers[0x4c / 4] & (1 << 16) != 0
            && state.registers[0x74 / 4] & 1 != 0
    }

    /// Reports a host bus reset to device firmware.
    pub fn inject_bus_reset(&self) {
        let mut state = self.state.lock().expect("RP2040 USB lock poisoned");
        state.registers[0x50 / 4] |= (1 << 19) | (1 << 16) | 1;
    }

    /// Reports a SETUP packet already placed in USB DPRAM.
    pub fn inject_setup(&self) {
        self.state
            .lock()
            .expect("RP2040 USB lock poisoned")
            .registers[0x50 / 4] |= 1 << 17;
    }

    /// Reports completion of one endpoint buffer.
    pub fn complete_buffer(&self, endpoint: u8, input: bool) {
        let bit = u32::from(endpoint) * 2 + u32::from(!input);
        self.state
            .lock()
            .expect("RP2040 USB lock poisoned")
            .registers[0x58 / 4] |= 1 << bit;
    }

    /// Returns whether the controller currently asserts its interrupt output.
    pub fn interrupt_pending(&self) -> bool {
        self.state
            .lock()
            .expect("RP2040 USB lock poisoned")
            .masked_interrupts()
            != 0
    }
}

/// Functional RP2040 XIP SSI register window.
///
/// The flash ROM helpers use the Synopsys SSI block for command-mode transfers. Every transmitted
/// byte clocks one received byte; the model returns deterministic zero data until a flash command
/// decoder is attached, while accurately maintaining the FIFO-ready status needed by those
/// helpers.
pub struct Rp2040Ssi {
    name: String,
    registers: [u32; 64],
    receive_fifo: VecDeque<u8>,
}

impl Rp2040Ssi {
    /// Creates an idle SSI controller.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            registers: [0; 64],
            receive_fifo: VecDeque::new(),
        }
    }
}

/// Functional RP2040 real-time clock register window.
pub struct Rp2040Rtc {
    name: String,
    registers: [u32; 16],
}

impl Rp2040Rtc {
    /// Creates a stopped RTC.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            registers: [0; 16],
        }
    }
}

impl Device for Rp2040Rtc {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || !width.is_aligned(offset) {
            return Err(DeviceError::new(
                "RP2040 RTC requires aligned word accesses",
            ));
        }
        let register_offset = offset & 0x0fff;
        self.registers
            .get(usize::try_from(register_offset / 4).expect("RTC offset fits usize"))
            .copied()
            .map(u64::from)
            .ok_or_else(|| DeviceError::new("RP2040 RTC read outside register window"))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || !width.is_aligned(offset) {
            return Err(DeviceError::new(
                "RP2040 RTC requires aligned word accesses",
            ));
        }
        let alias = (offset >> 12) & 3;
        let register_offset = offset & 0x0fff;
        let register = self
            .registers
            .get_mut(usize::try_from(register_offset / 4).expect("RTC offset fits usize"))
            .ok_or_else(|| DeviceError::new("RP2040 RTC write outside register window"))?;
        Rp2040Resets::update(register, alias, value as u32)?;
        if register_offset == 0x0c {
            // CTRL.ENABLE becomes CTRL.RTC_ACTIVE immediately in the functional time model.
            if *register & 1 != 0 {
                *register |= 2;
            } else {
                *register &= !2;
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.registers.fill(0);
    }
}

impl Device for Rp2040Ssi {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || !width.is_aligned(offset) {
            return Err(DeviceError::new(
                "RP2040 SSI requires aligned word accesses",
            ));
        }
        let register_offset = offset & 0x0fff;
        match register_offset {
            0x20 => Ok(0),
            0x24 => Ok(self.receive_fifo.len() as u64),
            // SR: transmit FIFO empty/not full, plus receive FIFO not empty when data awaits.
            0x28 => Ok(0x06 | u64::from(!self.receive_fifo.is_empty()) << 3),
            0x60 => Ok(u64::from(self.receive_fifo.pop_front().unwrap_or(0))),
            _ => self
                .registers
                .get(usize::try_from(register_offset / 4).expect("SSI offset fits usize"))
                .copied()
                .map(u64::from)
                .ok_or_else(|| DeviceError::new("RP2040 SSI read outside register window")),
        }
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || !width.is_aligned(offset) {
            return Err(DeviceError::new(
                "RP2040 SSI requires aligned word accesses",
            ));
        }
        let alias = (offset >> 12) & 3;
        let register_offset = offset & 0x0fff;
        let index = usize::try_from(register_offset / 4).expect("SSI offset fits usize");
        let register = self
            .registers
            .get_mut(index)
            .ok_or_else(|| DeviceError::new("RP2040 SSI write outside register window"))?;
        Rp2040Resets::update(register, alias, value as u32)?;
        if register_offset == 0x60 {
            self.receive_fifo.push_back(0);
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.registers.fill(0);
        self.receive_fifo.clear();
    }
}

impl Rp2040UsbController {
    /// Creates a USB device controller with VBUS present.
    pub fn new(name: impl Into<String>) -> Self {
        Self::new_with_handle(name).0
    }

    /// Creates a USB controller and its functional-host handle.
    pub fn new_with_handle(name: impl Into<String>) -> (Self, Rp2040UsbHandle) {
        let mut registers = [0; 64];
        registers[0x50 / 4] = 1;
        let state = Arc::new(Mutex::new(Rp2040UsbState { registers }));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Rp2040UsbHandle { state },
        )
    }

    fn update(register: &mut u32, alias: u64, value: u32) -> Result<(), DeviceError> {
        match alias {
            0 => *register = value,
            1 => *register ^= value,
            2 => *register |= value,
            3 => *register &= !value,
            _ => return Err(DeviceError::new("invalid RP2040 USB atomic alias")),
        }
        Ok(())
    }

    fn update_write_clear(
        register: &mut u32,
        alias: u64,
        value: u32,
        read_only: u32,
    ) -> Result<(), DeviceError> {
        let preserved = *register & read_only;
        if alias == 0 || alias == 3 {
            *register &= !value;
        } else {
            Self::update(register, alias, value)?;
        }
        *register = (*register & !read_only) | preserved;
        Ok(())
    }
}

impl Device for Rp2040UsbController {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "RP2040 USB controller requires aligned word access",
            ));
        }
        let register_offset = offset & 0x0fff;
        let index = usize::try_from(register_offset / 4).expect("small USB offset fits");
        let state = self.state.lock().expect("RP2040 USB lock poisoned");
        let value = match register_offset {
            0x8c => Some(state.raw_interrupts()),
            0x98 => Some(state.masked_interrupts()),
            _ => state.registers.get(index).copied(),
        };
        value.map(u64::from).ok_or_else(|| {
            DeviceError::new(format!(
                "unmodeled RP2040 USB read at offset {register_offset:#x}"
            ))
        })
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
                "RP2040 USB controller requires aligned word access",
            ));
        }
        let alias = (offset >> 12) & 3;
        let register_offset = offset & 0x0fff;
        let index = usize::try_from(register_offset / 4).expect("small USB offset fits");
        let mut state = self.state.lock().expect("RP2040 USB lock poisoned");
        let value =
            u32::try_from(value & u64::from(u32::MAX)).expect("masked USB register value fits");
        let register = state.registers.get_mut(index).ok_or_else(|| {
            DeviceError::new(format!(
                "unmodeled RP2040 USB write at offset {register_offset:#x}"
            ))
        })?;
        match register_offset {
            // SIE_STATUS and BUFF_STATUS are write-clear event registers in the
            // RP2040 USB block. The atomic clear alias is equivalent to a base
            // write for these registers; status bits are never assigned raw.
            0x50 => Self::update_write_clear(
                register,
                alias,
                value,
                Rp2040UsbState::SIE_STATUS_READ_ONLY,
            )?,
            0x58 => Self::update_write_clear(register, alias, value, 0)?,
            _ => Self::update(register, alias, value)?,
        }
        // VBUS_DETECTED is driven by the functional host and remains asserted.
        state.registers[0x50 / 4] |= 1;
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.lock().expect("RP2040 USB lock poisoned");
        state.registers.fill(0);
        state.registers[0x50 / 4] = 1;
    }
}

/// RP2040/RP2350 SIO GPIO register slice.
pub struct RpSioGpio {
    name: String,
    pins: u8,
    layout: RpSioLayout,
    state: Arc<Mutex<GpioState>>,
    signals: Vec<SignalId>,
    hub: SignalHub,
    spinlocks: u32,
    dividend: u32,
    quotient: u32,
    remainder: u32,
    divider_dirty: bool,
    multicore: Rc<RefCell<RpSioMulticoreState>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RpSioLayout {
    Rp2040,
    Rp2350,
}

/// Architectural state supplied by the RP boot ROM after the six-word core-1
/// launch handshake.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RpCoreLaunch {
    /// Core-1 vector-table base (Arm VTOR or the RISC-V trap-vector value).
    pub vector_table: u32,
    /// Initial core-1 stack pointer.
    pub stack_pointer: u32,
    /// Initial core-1 entry point.
    pub entry: u32,
}

struct RpSioMulticoreState {
    selected_core: u8,
    inbound: [VecDeque<u32>; 2],
    fifo_error: [u32; 2],
    launch_sequence: Vec<u32>,
    pending_launch: Option<RpCoreLaunch>,
    core1_launched: bool,
}

impl Default for RpSioMulticoreState {
    fn default() -> Self {
        let mut inbound = [VecDeque::new(), VecDeque::new()];
        // When core 1 leaves reset, the RP boot ROM drains its FIFO and sends
        // this ready word to core 0. The functional model starts with core 1
        // held in that ROM protocol, so the first reset/launch sequence sees
        // the same deterministic acknowledgement.
        inbound[0].push_back(0);
        Self {
            selected_core: 0,
            inbound,
            fifo_error: [0; 2],
            launch_sequence: Vec::new(),
            pending_launch: None,
            core1_launched: false,
        }
    }
}

/// Machine-facing handle for selecting an RP core and observing a completed
/// SIO boot-ROM launch protocol.
#[derive(Clone)]
pub struct RpSioHandle {
    state: Rc<RefCell<RpSioMulticoreState>>,
}

impl RpSioHandle {
    /// Selects which processor owns subsequent accesses to the shared SIO
    /// device. Machines call this immediately before stepping that processor.
    pub fn select_core(&self, core: u8) {
        self.state.borrow_mut().selected_core = core.min(1);
    }

    /// Takes a core-1 launch that completed the documented six-word ROM
    /// handshake.
    pub fn take_core1_launch(&self) -> Option<RpCoreLaunch> {
        self.state.borrow_mut().pending_launch.take()
    }

    /// Returns whether core 1 has left its boot-ROM launch protocol.
    pub fn core1_launched(&self) -> bool {
        self.state.borrow().core1_launched
    }
}

impl RpSioGpio {
    /// Creates SIO GPIO state and an external-stimulus handle.
    pub fn new(
        name: impl Into<String>,
        pins: u8,
        path: &str,
        hub: SignalHub,
    ) -> Result<(Self, GpioHandle), SignalError> {
        let (device, gpio, _) = Self::new_with_layout(name, pins, path, hub, RpSioLayout::Rp2040)?;
        Ok((device, gpio))
    }

    /// Creates RP2040 SIO state with both GPIO and machine-facing multicore
    /// handles.
    pub fn new_with_multicore(
        name: impl Into<String>,
        pins: u8,
        path: &str,
        hub: SignalHub,
    ) -> Result<(Self, GpioHandle, RpSioHandle), SignalError> {
        Self::new_with_layout(name, pins, path, hub, RpSioLayout::Rp2040)
    }

    /// Creates an RP2350 SIO GPIO slice.
    ///
    /// RP2350 interleaves the high GPIO bank between the low-bank atomic
    /// registers, so its low-bank output and output-enable offsets differ from
    /// RP2040 despite the common SIO base address.
    pub fn new_rp2350(
        name: impl Into<String>,
        pins: u8,
        path: &str,
        hub: SignalHub,
    ) -> Result<(Self, GpioHandle), SignalError> {
        let (device, gpio, _) = Self::new_with_layout(name, pins, path, hub, RpSioLayout::Rp2350)?;
        Ok((device, gpio))
    }

    /// Creates RP2350 SIO state with both GPIO and machine-facing multicore
    /// handles.
    pub fn new_rp2350_with_multicore(
        name: impl Into<String>,
        pins: u8,
        path: &str,
        hub: SignalHub,
    ) -> Result<(Self, GpioHandle, RpSioHandle), SignalError> {
        Self::new_with_layout(name, pins, path, hub, RpSioLayout::Rp2350)
    }

    fn new_with_layout(
        name: impl Into<String>,
        pins: u8,
        path: &str,
        hub: SignalHub,
        layout: RpSioLayout,
    ) -> Result<(Self, GpioHandle, RpSioHandle), SignalError> {
        let (state, signals, handle) = vendor_gpio(pins, path, &hub)?;
        let multicore = Rc::new(RefCell::new(RpSioMulticoreState::default()));
        Ok((
            Self {
                name: name.into(),
                pins,
                layout,
                state,
                signals,
                hub,
                spinlocks: u32::MAX,
                dividend: 0,
                quotient: 0,
                remainder: 0,
                divider_dirty: false,
                multicore: multicore.clone(),
            },
            handle,
            RpSioHandle { state: multicore },
        ))
    }

    fn resolved_input(&self) -> u32 {
        let state = self.state.lock().expect("GPIO lock poisoned");
        (0..self.pins).fold(0_u32, |value, pin| {
            if state.nets[usize::from(pin)].resolved() == Logic::One {
                value | (1_u32 << pin)
            } else {
                value
            }
        })
    }
}

impl Device for RpSioGpio {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if offset == 0 && matches!(width, AccessWidth::Byte | AccessWidth::HalfWord) {
            return Ok(0);
        }
        if width != AccessWidth::Word {
            return Err(DeviceError::new("RP SIO requires word access"));
        }
        if matches!(offset, 0x000 | 0x050 | 0x058) {
            let mut state = self.multicore.borrow_mut();
            let core = usize::from(state.selected_core);
            return match offset {
                0x000 => Ok(u64::from(state.selected_core)),
                0x050 => {
                    let valid = u32::from(!state.inbound[core].is_empty());
                    let other = core ^ 1;
                    let ready = u32::from(state.inbound[other].len() < 8) << 1;
                    Ok(u64::from(valid | ready | state.fifo_error[core]))
                }
                0x058 => {
                    let value = state.inbound[core].pop_front().unwrap_or_else(|| {
                        state.fifo_error[core] |= 1 << 3;
                        0
                    });
                    Ok(u64::from(value))
                }
                _ => unreachable!(),
            };
        }
        if (0x100..=0x17c).contains(&offset) && offset & 3 == 0 {
            let lock = u32::try_from((offset - 0x100) / 4).expect("SIO spinlock index fits");
            let mask = 1_u32 << lock;
            if self.spinlocks & mask != 0 {
                self.spinlocks &= !mask;
                return Ok(u64::from(mask));
            }
            return Ok(0);
        }
        if offset == 0x5c {
            return Ok(u64::from(self.spinlocks));
        }
        match offset {
            0x70 => return Ok(u64::from(self.quotient)),
            0x74 => return Ok(u64::from(self.remainder)),
            0x78 => return Ok(1 | (u64::from(self.divider_dirty) << 1)),
            _ => {}
        }
        let state = self.state.lock().expect("GPIO lock poisoned");
        let value = match (self.layout, offset) {
            (_, 0x004) => {
                drop(state);
                return Ok(u64::from(self.resolved_input()));
            }
            (RpSioLayout::Rp2040, 0x010..=0x01c)
            | (RpSioLayout::Rp2350, 0x010 | 0x018 | 0x020 | 0x028) => state.output,
            (RpSioLayout::Rp2040, 0x020..=0x02c)
            | (RpSioLayout::Rp2350, 0x030 | 0x038 | 0x040 | 0x048) => state.direction,
            _ => 0,
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
        if width != AccessWidth::Word {
            return Err(DeviceError::new("RP SIO requires word access"));
        }
        let value =
            u32::try_from(value & u64::from(u32::MAX)).expect("masked value always fits u32");
        if matches!(offset, 0x050 | 0x054) {
            let mut state = self.multicore.borrow_mut();
            let core = usize::from(state.selected_core);
            if offset == 0x050 {
                // FIFO_ST WOF/ROE are write-one-to-clear.
                state.fifo_error[core] &= !(value & ((1 << 2) | (1 << 3)));
                return Ok(());
            }
            let other = core ^ 1;
            if state.inbound[other].len() >= 8 {
                state.fifo_error[core] |= 1 << 2;
                return Ok(());
            }
            if core == 0 && !state.core1_launched {
                // Core 1's resident boot ROM echoes every launch word. This
                // preserves the SDK-visible FIFO protocol without pretending
                // that the ROM itself is application code.
                state.inbound[0].push_back(value);
                state.launch_sequence.push(value);
                if state.launch_sequence.len() > 6 {
                    state.launch_sequence.remove(0);
                }
                if state.launch_sequence.len() == 6 && state.launch_sequence[0..3] == [0, 0, 1] {
                    let launch = RpCoreLaunch {
                        vector_table: state.launch_sequence[3],
                        stack_pointer: state.launch_sequence[4],
                        entry: state.launch_sequence[5],
                    };
                    state.pending_launch = Some(launch);
                    state.core1_launched = true;
                    state.launch_sequence.clear();
                }
            } else {
                state.inbound[other].push_back(value);
            }
            return Ok(());
        }
        if (0x100..=0x17c).contains(&offset) && offset & 3 == 0 {
            let lock = u32::try_from((offset - 0x100) / 4).expect("SIO spinlock index fits");
            self.spinlocks |= 1_u32 << lock;
            return Ok(());
        }
        match offset {
            0x60 | 0x68 => {
                self.dividend = value;
                self.divider_dirty = true;
                return Ok(());
            }
            0x64 => {
                if value == 0 {
                    self.quotient = u32::MAX;
                    self.remainder = self.dividend;
                } else {
                    self.quotient = self.dividend / value;
                    self.remainder = self.dividend % value;
                }
                self.divider_dirty = true;
                return Ok(());
            }
            0x6c => {
                let dividend = self.dividend as i32;
                let divisor = value as i32;
                if divisor == 0 {
                    self.quotient = if dividend < 0 { 1 } else { u32::MAX };
                    self.remainder = self.dividend;
                } else {
                    self.quotient = dividend.wrapping_div(divisor) as u32;
                    self.remainder = dividend.wrapping_rem(divisor) as u32;
                }
                self.divider_dirty = true;
                return Ok(());
            }
            0x70 => {
                self.quotient = value;
                return Ok(());
            }
            0x74 => {
                self.remainder = value;
                return Ok(());
            }
            _ => {}
        }
        let mut state = self.state.lock().expect("GPIO lock poisoned");
        match (self.layout, offset) {
            (_, 0x010) => state.output = value,
            (RpSioLayout::Rp2040, 0x014) | (RpSioLayout::Rp2350, 0x018) => {
                state.output |= value;
            }
            (RpSioLayout::Rp2040, 0x018) | (RpSioLayout::Rp2350, 0x020) => {
                state.output &= !value;
            }
            (RpSioLayout::Rp2040, 0x01c) | (RpSioLayout::Rp2350, 0x028) => {
                state.output ^= value;
            }
            (RpSioLayout::Rp2040, 0x020) | (RpSioLayout::Rp2350, 0x030) => {
                state.direction = value;
            }
            (RpSioLayout::Rp2040, 0x024) | (RpSioLayout::Rp2350, 0x038) => {
                state.direction |= value;
            }
            (RpSioLayout::Rp2040, 0x028) | (RpSioLayout::Rp2350, 0x040) => {
                state.direction &= !value;
            }
            (RpSioLayout::Rp2040, 0x02c) | (RpSioLayout::Rp2350, 0x048) => {
                state.direction ^= value;
            }
            _ => return Ok(()),
        }
        drop(state);
        refresh_gpio(&self.state, &self.signals, &self.hub, self.pins, at)
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut gpio = self.state.lock().expect("GPIO lock poisoned");
        gpio.direction = 0;
        gpio.output = 0;
        drop(gpio);
        *self.multicore.borrow_mut() = RpSioMulticoreState::default();
        self.spinlocks = u32::MAX;
        self.dividend = 0;
        self.quotient = 0;
        self.remainder = 0;
        self.divider_dirty = false;
    }
}
