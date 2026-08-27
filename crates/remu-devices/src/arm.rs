use super::*;

#[path = "rp_usb.rs"]
mod rp_usb;
pub use rp_usb::*;

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
///
/// The values are offsets within the USB controller's 4 KiB register window. The controller
/// also accepts the RP2040 atomic aliases at `offset + 0x1000`, `+0x2000`, and `+0x3000`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum Rp2040UsbRegister {
    /// Main controller mode and enable.
    MainCtrl = 0x40,
    /// Host SOF frame number write register.
    SofWr = 0x44,
    /// Last observed SOF frame number.
    SofRd = 0x48,
    /// Serial interface engine control.
    SieCtrl = 0x4c,
    /// Serial interface engine status and write-clear events.
    SieStatus = 0x50,
    /// Host interrupt endpoint control.
    IntEpCtrl = 0x54,
    /// Endpoint buffer completion status and write-clear events.
    BuffStatus = 0x58,
    /// Double-buffer CPU ownership indication.
    BuffCpuShouldHandle = 0x5c,
    /// Endpoint abort control.
    EpAbort = 0x60,
    /// Endpoint abort completion status.
    EpAbortDone = 0x64,
    /// Endpoint zero stall arm control.
    EpStallArm = 0x68,
    /// Host NAK polling intervals.
    NakPoll = 0x6c,
    /// Endpoint stall/NAK status.
    EpStatusStallNak = 0x70,
    /// USB controller-to-PHY muxing.
    UsbMuxing = 0x74,
    /// VBUS and over-current power controls.
    UsbPwr = 0x78,
    /// Direct USB PHY controls.
    UsbPhyDirect = 0x7c,
    /// Direct USB PHY override enables.
    UsbPhyDirectOverride = 0x80,
    /// USB PHY trim values.
    UsbPhyTrim = 0x84,
    /// Raw interrupt status.
    Intr = 0x8c,
    /// Interrupt enable.
    Inte = 0x90,
    /// Interrupt force.
    Intf = 0x94,
    /// Masked and forced interrupt status.
    Ints = 0x98,
}

impl Rp2040UsbRegister {
    /// Converts a controller-relative register offset into a typed register ID.
    pub const fn from_offset(offset: u64) -> Option<Self> {
        Some(match offset {
            0x40 => Self::MainCtrl,
            0x44 => Self::SofWr,
            0x48 => Self::SofRd,
            0x4c => Self::SieCtrl,
            0x50 => Self::SieStatus,
            0x54 => Self::IntEpCtrl,
            0x58 => Self::BuffStatus,
            0x5c => Self::BuffCpuShouldHandle,
            0x60 => Self::EpAbort,
            0x64 => Self::EpAbortDone,
            0x68 => Self::EpStallArm,
            0x6c => Self::NakPoll,
            0x70 => Self::EpStatusStallNak,
            0x74 => Self::UsbMuxing,
            0x78 => Self::UsbPwr,
            0x7c => Self::UsbPhyDirect,
            0x80 => Self::UsbPhyDirectOverride,
            0x84 => Self::UsbPhyTrim,
            0x8c => Self::Intr,
            0x90 => Self::Inte,
            0x94 => Self::Intf,
            0x98 => Self::Ints,
            _ => return None,
        })
    }

    /// Returns the controller-relative byte offset for this register.
    pub const fn offset(self) -> u64 {
        self as u64
    }

    const fn index(self) -> usize {
        self.offset() as usize / 4
    }
}

/// Functional RP2040 USB controller register slice.
pub struct Rp2040UsbController {
    name: String,
    state: Arc<Mutex<Rp2040UsbState>>,
}

struct Rp2040UsbState {
    registers: [u32; 64],
    link: RpUsbLinkState,
}

/// Host-facing control of the RP2040 USB device controller.
#[derive(Clone)]
pub struct Rp2040UsbHandle {
    state: Arc<Mutex<Rp2040UsbState>>,
}

impl Rp2040UsbState {
    const VBUS_DETECTED: u32 = 1;
    const SIE_STATUS_DATA_SEQ_ERROR: u32 = 1 << 31;
    const SIE_STATUS_ACK_REC: u32 = 1 << 30;
    const SIE_STATUS_STALL_REC: u32 = 1 << 29;
    const SIE_STATUS_NAK_REC: u32 = 1 << 28;
    const SIE_STATUS_RX_TIMEOUT: u32 = 1 << 27;
    const SIE_STATUS_RX_OVERFLOW: u32 = 1 << 26;
    const SIE_STATUS_BIT_STUFF_ERROR: u32 = 1 << 25;
    const SIE_STATUS_CRC_ERROR: u32 = 1 << 24;
    const SIE_STATUS_BUS_RESET: u32 = 1 << 19;
    const SIE_STATUS_TRANS_COMPLETE: u32 = 1 << 18;
    const SIE_STATUS_SETUP_REC: u32 = 1 << 17;
    const SIE_STATUS_CONNECTED: u32 = 1 << 16;
    const SIE_STATUS_RESUME: u32 = 1 << 11;
    const SIE_STATUS_SPEED: u32 = 0b11 << 8;
    const SIE_STATUS_SUSPENDED: u32 = 1 << 4;
    const SIE_STATUS_LINE_STATE: u32 = 0b11 << 2;
    const SIE_STATUS_READ_ONLY: u32 = (1 << 10) | Self::SIE_STATUS_LINE_STATE | Self::VBUS_DETECTED;
    const SIE_STATUS_VALID: u32 = Self::SIE_STATUS_DATA_SEQ_ERROR
        | Self::SIE_STATUS_ACK_REC
        | Self::SIE_STATUS_STALL_REC
        | Self::SIE_STATUS_NAK_REC
        | Self::SIE_STATUS_RX_TIMEOUT
        | Self::SIE_STATUS_RX_OVERFLOW
        | Self::SIE_STATUS_BIT_STUFF_ERROR
        | Self::SIE_STATUS_CRC_ERROR
        | Self::SIE_STATUS_BUS_RESET
        | Self::SIE_STATUS_TRANS_COMPLETE
        | Self::SIE_STATUS_SETUP_REC
        | Self::SIE_STATUS_CONNECTED
        | Self::SIE_STATUS_RESUME
        | Self::SIE_STATUS_SPEED
        | Self::SIE_STATUS_SUSPENDED
        | Self::SIE_STATUS_READ_ONLY;

    const MAIN_CTRL_MASK: u32 = (1 << 31) | 0x3;
    const SOF_WR_MASK: u32 = 0x7ff;
    const SIE_CTRL_MASK: u32 = 0xff07_bf5f;
    const SIE_CTRL_SELF_CLEAR: u32 = (1 << 13) | (1 << 12) | (1 << 4) | 1;
    const INT_EP_CTRL_MASK: u32 = 0xfffe;
    const NAK_POLL_MASK: u32 = 0x03ff_03ff;
    const USB_MUXING_MASK: u32 = 0xf;
    const USB_PWR_MASK: u32 = 0x3f;
    const USB_PHY_DIRECT_MASK: u32 = 0xff77;
    const USB_PHY_DIRECT_READ_ONLY: u32 = 0x007f_0000;
    const USB_PHY_DIRECT_OVERRIDE_MASK: u32 = 0x9fff;
    const USB_PHY_TRIM_MASK: u32 = 0x1f1f;
    const INTERRUPT_MASK: u32 = 0x000f_ffff;

    fn reset_registers(registers: &mut [u32; 64]) {
        registers.fill(0);
        registers[Rp2040UsbRegister::SieStatus.index()] = Self::VBUS_DETECTED;
        registers[Rp2040UsbRegister::NakPoll.index()] = 0x0010_0010;
        registers[Rp2040UsbRegister::UsbPhyTrim.index()] = Self::USB_PHY_TRIM_MASK;
    }

    fn raw_interrupts(&self) -> u32 {
        let sie_status = self.registers[Rp2040UsbRegister::SieStatus.index()];
        let mut interrupts = 0;
        if self.registers[Rp2040UsbRegister::BuffStatus.index()] != 0 {
            interrupts |= 1 << 4;
        }
        if sie_status & Self::SIE_STATUS_DATA_SEQ_ERROR != 0 {
            interrupts |= 1 << 5;
        }
        if sie_status & Self::SIE_STATUS_RX_TIMEOUT != 0 {
            interrupts |= 1 << 6;
        }
        if sie_status & Self::SIE_STATUS_RX_OVERFLOW != 0 {
            interrupts |= 1 << 7;
        }
        if sie_status & Self::SIE_STATUS_BIT_STUFF_ERROR != 0 {
            interrupts |= 1 << 8;
        }
        if sie_status & Self::SIE_STATUS_CRC_ERROR != 0 {
            interrupts |= 1 << 9;
        }
        if sie_status & Self::SIE_STATUS_STALL_REC != 0 {
            interrupts |= 1 << 10;
        }
        if sie_status & Self::SIE_STATUS_BUS_RESET != 0 {
            interrupts |= 1 << 12;
        }
        if sie_status & Self::VBUS_DETECTED != 0 {
            interrupts |= 1 << 11;
        }
        if sie_status & Self::SIE_STATUS_CONNECTED != 0 {
            interrupts |= 1 << 13;
        }
        if sie_status & Self::SIE_STATUS_SUSPENDED != 0 {
            interrupts |= 1 << 14;
        }
        if sie_status & Self::SIE_STATUS_RESUME != 0 {
            interrupts |= 1 << 15;
        }
        if sie_status & Self::SIE_STATUS_SETUP_REC != 0 {
            interrupts |= 1 << 16;
        }
        if sie_status & Self::SIE_STATUS_TRANS_COMPLETE != 0 {
            interrupts |= 1 << 3;
        }
        if sie_status & Self::SIE_STATUS_SPEED != 0 {
            interrupts |= 1;
        }
        interrupts
    }

    fn masked_interrupts(&self) -> u32 {
        (self.raw_interrupts() & self.registers[Rp2040UsbRegister::Inte.index()])
            | self.registers[Rp2040UsbRegister::Intf.index()]
    }
}

impl Rp2040UsbHandle {
    /// Returns true after firmware enables the controller and device pull-up.
    pub fn device_connected(&self) -> bool {
        let state = self.state.lock().expect("RP2040 USB lock poisoned");
        state.registers[Rp2040UsbRegister::MainCtrl.index()] & 1 != 0
            && state.registers[Rp2040UsbRegister::SieCtrl.index()] & (1 << 16) != 0
            && state.registers[Rp2040UsbRegister::UsbMuxing.index()] & 1 != 0
    }

    /// Reports a host bus reset to device firmware.
    pub fn inject_bus_reset(&self) {
        let mut state = self.state.lock().expect("RP2040 USB lock poisoned");
        state.link.bus_reset();
        state.registers[Rp2040UsbRegister::SieStatus.index()] |=
            Rp2040UsbState::SIE_STATUS_BUS_RESET
                | Rp2040UsbState::SIE_STATUS_CONNECTED
                | Rp2040UsbState::VBUS_DETECTED;
    }

    /// Reports a SETUP packet already placed in USB DPRAM.
    pub fn inject_setup(&self) {
        self.state
            .lock()
            .expect("RP2040 USB lock poisoned")
            .registers[Rp2040UsbRegister::SieStatus.index()] |= 1 << 17;
    }

    /// Reports completion of one endpoint buffer.
    pub fn complete_buffer(&self, endpoint: u8, input: bool) {
        let bit = u32::from(endpoint) * 2 + u32::from(!input);
        self.state
            .lock()
            .expect("RP2040 USB lock poisoned")
            .registers[Rp2040UsbRegister::BuffStatus.index()] |= 1 << bit;
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

#[path = "rp2040_rtc.rs"]
mod rp2040_rtc;
pub use rp2040_rtc::*;

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
        Rp2040UsbState::reset_registers(&mut registers);
        let state = Arc::new(Mutex::new(Rp2040UsbState {
            registers,
            link: RpUsbLinkState::reset(),
        }));
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

    fn replicated_write(width: AccessWidth, value: u64) -> Result<u32, DeviceError> {
        match width {
            AccessWidth::Byte => {
                let byte = u32::try_from(value & 0xff).expect("byte write fits");
                Ok(byte.wrapping_mul(0x0101_0101))
            }
            AccessWidth::HalfWord => {
                let half = u32::try_from(value & 0xffff).expect("halfword write fits");
                Ok(half | (half << 16))
            }
            AccessWidth::Word => u32::try_from(value & u64::from(u32::MAX)).map_err(|_| {
                DeviceError::new("RP2040 USB word write does not fit in the register bus")
            }),
            AccessWidth::DoubleWord => Err(DeviceError::new(
                "RP2040 USB controller does not support doubleword access",
            )),
        }
    }

    fn narrow_read(value: u32, offset: u64, width: AccessWidth) -> u64 {
        let shift = (offset & 3) * 8;
        (u64::from(value) >> shift) & width.value_mask()
    }

    fn update_masked(
        register: &mut u32,
        alias: u64,
        value: u32,
        mask: u32,
        read_only: u32,
        self_clear: u32,
    ) -> Result<(), DeviceError> {
        let preserved = *register & read_only;
        Self::update(register, alias, value)?;
        *register = (*register & mask & !read_only) | preserved;
        *register &= !self_clear;
        Ok(())
    }

    fn update_write_clear(
        register: &mut u32,
        alias: u64,
        value: u32,
        valid: u32,
        read_only: u32,
    ) -> Result<(), DeviceError> {
        let preserved = *register & read_only;
        let value = value & valid & !read_only;
        if alias == 0 || alias == 3 {
            *register &= !value;
        } else {
            Self::update(register, alias, value)?;
        }
        *register = (*register & valid & !read_only) | preserved;
        Ok(())
    }
}

impl Device for Rp2040UsbController {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width == AccessWidth::DoubleWord {
            return Err(DeviceError::new(
                "RP2040 USB controller does not support doubleword access",
            ));
        }
        let register_offset = offset & 0x0fff;
        let register_offset = register_offset & !3;
        let register = Rp2040UsbRegister::from_offset(register_offset).ok_or_else(|| {
            DeviceError::new(format!(
                "unmodeled RP2040 USB read at offset {register_offset:#x}"
            ))
        })?;
        let state = self.state.lock().expect("RP2040 USB lock poisoned");
        let value = match register {
            Rp2040UsbRegister::Intr => state.raw_interrupts(),
            Rp2040UsbRegister::Ints => state.masked_interrupts(),
            Rp2040UsbRegister::SofRd => u32::from(state.link.frame),
            Rp2040UsbRegister::SieStatus => {
                let status =
                    state.registers[register.index()] & !Rp2040UsbState::SIE_STATUS_LINE_STATE;
                status | (state.link.line.status_bits() << 2)
            }
            Rp2040UsbRegister::UsbPhyDirect => {
                let (dp, dm) = state.link.line.pins();
                state.registers[register.index()]
                    | (u32::from(dp) << 17)
                    | (u32::from(dm) << 18)
                    | (u32::from(dp != dm) << 16)
            }
            _ => state.registers[register.index()],
        };
        Ok(Self::narrow_read(value, offset, width))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width == AccessWidth::DoubleWord {
            return Err(DeviceError::new(
                "RP2040 USB controller does not support doubleword access",
            ));
        }
        let alias = (offset >> 12) & 3;
        let register_offset = offset & 0x0fff;
        let register_offset = register_offset & !3;
        let register = Rp2040UsbRegister::from_offset(register_offset).ok_or_else(|| {
            DeviceError::new(format!(
                "unmodeled RP2040 USB write at offset {register_offset:#x}"
            ))
        })?;
        let mut state = self.state.lock().expect("RP2040 USB lock poisoned");
        let value = Self::replicated_write(width, value)?;
        match register {
            // SIE_STATUS and BUFF_STATUS are write-clear event registers in the
            // RP2040 USB block. The atomic clear alias is equivalent to a base
            // write for these registers; status bits are never assigned raw.
            Rp2040UsbRegister::SieStatus => Self::update_write_clear(
                &mut state.registers[register.index()],
                alias,
                value,
                Rp2040UsbState::SIE_STATUS_VALID,
                Rp2040UsbState::SIE_STATUS_READ_ONLY,
            )?,
            Rp2040UsbRegister::BuffStatus
            | Rp2040UsbRegister::EpAbortDone
            | Rp2040UsbRegister::EpStatusStallNak => Self::update_write_clear(
                &mut state.registers[register.index()],
                alias,
                value,
                u32::MAX,
                0,
            )?,
            Rp2040UsbRegister::MainCtrl => Self::update_masked(
                &mut state.registers[register.index()],
                alias,
                value,
                Rp2040UsbState::MAIN_CTRL_MASK,
                0,
                0,
            )?,
            Rp2040UsbRegister::SofWr => {
                Self::update_masked(
                    &mut state.registers[register.index()],
                    alias,
                    value,
                    Rp2040UsbState::SOF_WR_MASK,
                    0,
                    0,
                )?;
                state.link.frame = state.registers[register.index()] as u16;
            }
            Rp2040UsbRegister::SofRd
            | Rp2040UsbRegister::BuffCpuShouldHandle
            | Rp2040UsbRegister::Intr
            | Rp2040UsbRegister::Ints => {}
            Rp2040UsbRegister::SieCtrl => Self::update_masked(
                &mut state.registers[register.index()],
                alias,
                value,
                Rp2040UsbState::SIE_CTRL_MASK,
                0,
                Rp2040UsbState::SIE_CTRL_SELF_CLEAR,
            )?,
            Rp2040UsbRegister::IntEpCtrl => Self::update_masked(
                &mut state.registers[register.index()],
                alias,
                value,
                Rp2040UsbState::INT_EP_CTRL_MASK,
                0,
                0,
            )?,
            Rp2040UsbRegister::EpAbort => {
                Self::update_masked(
                    &mut state.registers[register.index()],
                    alias,
                    value,
                    u32::MAX,
                    0,
                    0,
                )?;
                let aborted = state.registers[register.index()];
                state.registers[Rp2040UsbRegister::EpAbortDone.index()] |= aborted;
                state.registers[register.index()] = 0;
            }
            Rp2040UsbRegister::EpStallArm => Self::update_masked(
                &mut state.registers[register.index()],
                alias,
                value,
                0x3,
                0,
                0,
            )?,
            Rp2040UsbRegister::NakPoll => Self::update_masked(
                &mut state.registers[register.index()],
                alias,
                value,
                Rp2040UsbState::NAK_POLL_MASK,
                0,
                0,
            )?,
            Rp2040UsbRegister::UsbMuxing => Self::update_masked(
                &mut state.registers[register.index()],
                alias,
                value,
                Rp2040UsbState::USB_MUXING_MASK,
                0,
                0,
            )?,
            Rp2040UsbRegister::UsbPwr => Self::update_masked(
                &mut state.registers[register.index()],
                alias,
                value,
                Rp2040UsbState::USB_PWR_MASK,
                0,
                0,
            )?,
            Rp2040UsbRegister::UsbPhyDirect => Self::update_masked(
                &mut state.registers[register.index()],
                alias,
                value,
                Rp2040UsbState::USB_PHY_DIRECT_MASK,
                Rp2040UsbState::USB_PHY_DIRECT_READ_ONLY,
                0,
            )?,
            Rp2040UsbRegister::UsbPhyDirectOverride => Self::update_masked(
                &mut state.registers[register.index()],
                alias,
                value,
                Rp2040UsbState::USB_PHY_DIRECT_OVERRIDE_MASK,
                0,
                0,
            )?,
            Rp2040UsbRegister::UsbPhyTrim => Self::update_masked(
                &mut state.registers[register.index()],
                alias,
                value,
                Rp2040UsbState::USB_PHY_TRIM_MASK,
                0,
                0,
            )?,
            Rp2040UsbRegister::Inte | Rp2040UsbRegister::Intf => Self::update_masked(
                &mut state.registers[register.index()],
                alias,
                value,
                Rp2040UsbState::INTERRUPT_MASK,
                0,
                0,
            )?,
        }
        // VBUS_DETECTED is driven by the functional host and remains asserted.
        state.registers[Rp2040UsbRegister::SieStatus.index()] |= Rp2040UsbState::VBUS_DETECTED;
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.lock().expect("RP2040 USB lock poisoned");
        Rp2040UsbState::reset_registers(&mut state.registers);
        state.link = RpUsbLinkState::reset();
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
        let (state, signals, handle) = match layout {
            RpSioLayout::Rp2040 => vendor_gpio(pins, path, &hub)?,
            RpSioLayout::Rp2350 => vendor_gpio_wide(pins, path, &hub)?,
        };
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

    fn resolved_input(&self, first_pin: u8, count: u8) -> u32 {
        let state = self.state.lock().expect("GPIO lock poisoned");
        let end = first_pin.saturating_add(count).min(self.pins);
        (first_pin..end).fold(0_u32, |value, pin| {
            if state.nets[usize::from(pin)].resolved() == Logic::One {
                value | (1_u32 << (pin - first_pin))
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
                return Ok(u64::from(self.resolved_input(0, 32)));
            }
            (RpSioLayout::Rp2350, 0x008) => {
                drop(state);
                return Ok(u64::from(self.resolved_input(32, 16)));
            }
            (RpSioLayout::Rp2040, 0x010..=0x01c)
            | (RpSioLayout::Rp2350, 0x010 | 0x018 | 0x020 | 0x028) => state.output,
            (RpSioLayout::Rp2040, 0x020..=0x02c)
            | (RpSioLayout::Rp2350, 0x030 | 0x038 | 0x040 | 0x048) => state.direction,
            (RpSioLayout::Rp2350, 0x014 | 0x01c | 0x024 | 0x02c) => state.output_high,
            (RpSioLayout::Rp2350, 0x034 | 0x03c | 0x044 | 0x04c) => state.direction_high,
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
            (RpSioLayout::Rp2350, 0x014) => state.output_high = value & 0xffff,
            (RpSioLayout::Rp2350, 0x01c) => state.output_high |= value & 0xffff,
            (RpSioLayout::Rp2350, 0x024) => state.output_high &= !(value & 0xffff),
            (RpSioLayout::Rp2350, 0x02c) => state.output_high ^= value & 0xffff,
            (RpSioLayout::Rp2350, 0x034) => state.direction_high = value & 0xffff,
            (RpSioLayout::Rp2350, 0x03c) => state.direction_high |= value & 0xffff,
            (RpSioLayout::Rp2350, 0x044) => state.direction_high &= !(value & 0xffff),
            (RpSioLayout::Rp2350, 0x04c) => state.direction_high ^= value & 0xffff,
            _ => return Ok(()),
        }
        drop(state);
        refresh_gpio(&self.state, &self.signals, &self.hub, self.pins, at)
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut gpio = self.state.lock().expect("GPIO lock poisoned");
        gpio.direction = 0;
        gpio.output = 0;
        gpio.direction_high = 0;
        gpio.output_high = 0;
        drop(gpio);
        *self.multicore.borrow_mut() = RpSioMulticoreState::default();
        self.spinlocks = u32::MAX;
        self.dividend = 0;
        self.quotient = 0;
        self.remainder = 0;
        self.divider_dirty = false;
    }
}
