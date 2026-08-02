//! Functional SAM D21 USB device control and endpoint-register model.

use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use std::sync::{Arc, Mutex};

const ENDPOINT_COUNT: usize = 8;
const ENDPOINT_BASE: u64 = 0x100;
const ENDPOINT_STRIDE: u64 = 0x20;
const ENDPOINT_MASK: u8 = 0x77;
const DEVICE_FLAGS_MASK: u16 = 0x03ff;

/// Named SAM D21 USB device common-register offsets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum Samd21UsbRegister {
    /// Enable, operating mode, standby, and software reset.
    ControlA = 0x00,
    /// Synchronization status for control-A operations.
    SyncBusy = 0x02,
    /// Quality-of-service control.
    QosControl = 0x03,
    /// Device control, detach, and speed configuration.
    ControlB = 0x08,
    /// Device address and address-enable bit.
    DeviceAddress = 0x0a,
    /// Read-only line-state and speed status.
    Status = 0x0c,
    /// Device frame number.
    FrameNumber = 0x10,
    /// Device interrupt enable clear.
    InterruptEnableClear = 0x14,
    /// Device interrupt enable set.
    InterruptEnableSet = 0x18,
    /// Device interrupt flags, cleared by writing one.
    InterruptFlags = 0x1c,
    /// Endpoint interrupt summary.
    EndpointInterruptSummary = 0x20,
    /// Descriptor base address in system RAM.
    DescriptorAddress = 0x24,
    /// USB pad calibration values.
    PadCalibration = 0x28,
}

impl Samd21UsbRegister {
    fn from_offset(offset: u64) -> Option<Self> {
        match offset {
            0x00 => Some(Self::ControlA),
            0x02 => Some(Self::SyncBusy),
            0x03 => Some(Self::QosControl),
            0x08 => Some(Self::ControlB),
            0x0a => Some(Self::DeviceAddress),
            0x0c => Some(Self::Status),
            0x10 => Some(Self::FrameNumber),
            0x14 => Some(Self::InterruptEnableClear),
            0x18 => Some(Self::InterruptEnableSet),
            0x1c => Some(Self::InterruptFlags),
            0x20 => Some(Self::EndpointInterruptSummary),
            0x24 => Some(Self::DescriptorAddress),
            0x28 => Some(Self::PadCalibration),
            _ => None,
        }
    }
}

/// Named byte offsets within one SAM D21 USB endpoint register window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum Samd21UsbEndpointRegister {
    /// Endpoint direction/type configuration.
    Configuration = 0x00,
    /// Write-one-to-clear endpoint status aliases.
    StatusClear = 0x04,
    /// Write-one-to-set endpoint status aliases.
    StatusSet = 0x05,
    /// Endpoint status.
    Status = 0x06,
    /// Endpoint interrupt flags, cleared by writing one.
    InterruptFlags = 0x07,
    /// Endpoint interrupt enable clear.
    InterruptEnableClear = 0x08,
    /// Endpoint interrupt enable set.
    InterruptEnableSet = 0x09,
}

impl Samd21UsbEndpointRegister {
    fn from_offset(offset: u64) -> Option<Self> {
        match offset {
            0x00 => Some(Self::Configuration),
            0x04 => Some(Self::StatusClear),
            0x05 => Some(Self::StatusSet),
            0x06 => Some(Self::Status),
            0x07 => Some(Self::InterruptFlags),
            0x08 => Some(Self::InterruptEnableClear),
            0x09 => Some(Self::InterruptEnableSet),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct EndpointState {
    configuration: u8,
    status: u8,
    interrupt_flags: u8,
    interrupt_enable: u8,
}

struct UsbState {
    control_a: u8,
    sync_busy: u8,
    qos_control: u8,
    control_b: u16,
    device_address: u8,
    status: u8,
    frame_number: u16,
    interrupt_enable: u16,
    interrupt_flags: u16,
    descriptor_address: u32,
    pad_calibration: u16,
    endpoints: [EndpointState; ENDPOINT_COUNT],
}

impl Default for UsbState {
    fn default() -> Self {
        Self {
            control_a: 0,
            sync_busy: 0,
            qos_control: 0,
            control_b: 1,
            device_address: 0,
            // The datasheet reset value is line-state J and full-speed.
            status: 0x40,
            frame_number: 0,
            interrupt_enable: 0,
            interrupt_flags: 0,
            descriptor_address: 0,
            pad_calibration: 0,
            endpoints: [EndpointState::default(); ENDPOINT_COUNT],
        }
    }
}

impl UsbState {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn endpoint_summary(&self) -> u16 {
        self.endpoints
            .iter()
            .enumerate()
            .fold(0_u16, |summary, (endpoint, state)| {
                let pending = state.interrupt_flags & state.interrupt_enable != 0;
                summary | (u16::from(pending) << endpoint)
            })
    }

    fn interrupt_pending(&self) -> bool {
        (self.interrupt_flags & self.interrupt_enable) != 0 || self.endpoint_summary() != 0
    }

    fn clear_endpoint_after_disable(state: &mut EndpointState) {
        state.status &= !(0x30);
        state.interrupt_flags = 0;
        state.interrupt_enable = 0;
    }

    fn bus_reset(&mut self) {
        self.device_address = 0;
        self.frame_number = 0;
        for (endpoint, state) in self.endpoints.iter_mut().enumerate() {
            if endpoint != 0 {
                state.configuration = 0;
            }
            state.status = 0;
            state.interrupt_flags = 0;
            state.interrupt_enable = 0;
        }
        self.interrupt_flags |= 1 << 3;
    }

    fn start_of_frame(&mut self, frame: u16) {
        self.frame_number = (frame & 0x07ff) << 3;
        self.interrupt_flags |= 1 << 2;
    }

    fn receive_setup(&mut self, endpoint: u8) -> bool {
        let Some(state) = self.endpoints.get_mut(usize::from(endpoint)) else {
            return false;
        };
        if state.configuration & 0x7 == 0 {
            return false;
        }
        state.interrupt_flags |= 1 << 4;
        true
    }
}

/// Host-facing handle for deterministic USB bus stimuli and interrupt state.
#[derive(Clone)]
pub struct Samd21UsbDeviceHandle(Arc<Mutex<UsbState>>);

impl Samd21UsbDeviceHandle {
    /// Delivers a USB bus reset to the device and latches `INTFLAG.EORST`.
    pub fn bus_reset(&self) {
        self.0.lock().expect("USB lock poisoned").bus_reset();
    }

    /// Delivers a Start-of-Frame token with an 11-bit frame number.
    pub fn start_of_frame(&self, frame: u16) {
        self.0
            .lock()
            .expect("USB lock poisoned")
            .start_of_frame(frame);
    }

    /// Delivers a SETUP token to an enabled OUT/control endpoint.
    pub fn receive_setup(&self, endpoint: u8) -> bool {
        self.0
            .lock()
            .expect("USB lock poisoned")
            .receive_setup(endpoint)
    }

    /// Returns whether a common or endpoint interrupt is enabled and pending.
    pub fn interrupt_pending(&self) -> bool {
        self.0
            .lock()
            .expect("USB lock poisoned")
            .interrupt_pending()
    }

    /// Returns the current endpoint interrupt summary.
    pub fn endpoint_interrupt_summary(&self) -> u16 {
        self.0.lock().expect("USB lock poisoned").endpoint_summary()
    }
}

/// Functional SAM D21 USB device common and endpoint register block.
pub struct Samd21UsbDevice {
    name: String,
    state: Arc<Mutex<UsbState>>,
}

impl Samd21UsbDevice {
    /// Creates the USB device model and a host-stimulus handle.
    pub fn new(name: impl Into<String>) -> (Self, Samd21UsbDeviceHandle) {
        let state = Arc::new(Mutex::new(UsbState::default()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Samd21UsbDeviceHandle(state),
        )
    }

    fn require(width: AccessWidth, expected: AccessWidth, what: &str) -> Result<(), DeviceError> {
        if width != expected {
            return Err(DeviceError::new(format!(
                "SAM D21 USB {what} requires {expected:?} access"
            )));
        }
        Ok(())
    }

    fn endpoint_offset(offset: u64) -> Option<(usize, Samd21UsbEndpointRegister)> {
        if offset < ENDPOINT_BASE {
            return None;
        }
        let relative = offset - ENDPOINT_BASE;
        let endpoint = usize::try_from(relative / ENDPOINT_STRIDE).ok()?;
        if endpoint >= ENDPOINT_COUNT {
            return None;
        }
        let register = Samd21UsbEndpointRegister::from_offset(relative % ENDPOINT_STRIDE)?;
        Some((endpoint, register))
    }

    fn read_common(
        state: &UsbState,
        register: Samd21UsbRegister,
        width: AccessWidth,
    ) -> Result<u64, DeviceError> {
        let value = match register {
            Samd21UsbRegister::ControlA => {
                Self::require(width, AccessWidth::Byte, "CTRLA")?;
                u64::from(state.control_a)
            }
            Samd21UsbRegister::SyncBusy => {
                Self::require(width, AccessWidth::Byte, "SYNCBUSY")?;
                u64::from(state.sync_busy)
            }
            Samd21UsbRegister::QosControl => {
                Self::require(width, AccessWidth::Byte, "QOSCTRL")?;
                u64::from(state.qos_control)
            }
            Samd21UsbRegister::ControlB => {
                Self::require(width, AccessWidth::HalfWord, "CTRLB")?;
                u64::from(state.control_b)
            }
            Samd21UsbRegister::DeviceAddress => {
                Self::require(width, AccessWidth::Byte, "DADD")?;
                u64::from(state.device_address)
            }
            Samd21UsbRegister::Status => {
                Self::require(width, AccessWidth::Byte, "STATUS")?;
                u64::from(state.status)
            }
            Samd21UsbRegister::FrameNumber => {
                Self::require(width, AccessWidth::HalfWord, "FNUM")?;
                u64::from(state.frame_number)
            }
            Samd21UsbRegister::InterruptEnableClear | Samd21UsbRegister::InterruptEnableSet => {
                Self::require(width, AccessWidth::HalfWord, "interrupt enables")?;
                u64::from(state.interrupt_enable)
            }
            Samd21UsbRegister::InterruptFlags => {
                Self::require(width, AccessWidth::HalfWord, "INTFLAG")?;
                u64::from(state.interrupt_flags)
            }
            Samd21UsbRegister::EndpointInterruptSummary => {
                Self::require(width, AccessWidth::HalfWord, "EPINTSMRY")?;
                u64::from(state.endpoint_summary())
            }
            Samd21UsbRegister::DescriptorAddress => {
                Self::require(width, AccessWidth::Word, "DESCADD")?;
                u64::from(state.descriptor_address)
            }
            Samd21UsbRegister::PadCalibration => {
                Self::require(width, AccessWidth::HalfWord, "PADCAL")?;
                u64::from(state.pad_calibration)
            }
        };
        Ok(value)
    }

    fn write_common(
        state: &mut UsbState,
        register: Samd21UsbRegister,
        width: AccessWidth,
        value: u64,
    ) -> Result<(), DeviceError> {
        match register {
            Samd21UsbRegister::ControlA => {
                Self::require(width, AccessWidth::Byte, "CTRLA")?;
                let value = value as u8;
                if value & 1 != 0 {
                    state.reset();
                } else {
                    state.control_a = value & 0x86;
                }
            }
            Samd21UsbRegister::SyncBusy => {
                Self::require(width, AccessWidth::Byte, "SYNCBUSY")?;
            }
            Samd21UsbRegister::QosControl => {
                Self::require(width, AccessWidth::Byte, "QOSCTRL")?;
                state.qos_control = value as u8 & 0x0f;
            }
            Samd21UsbRegister::ControlB => {
                Self::require(width, AccessWidth::HalfWord, "CTRLB")?;
                state.control_b = value as u16 & 0x0f7f;
            }
            Samd21UsbRegister::DeviceAddress => {
                Self::require(width, AccessWidth::Byte, "DADD")?;
                state.device_address = value as u8;
            }
            Samd21UsbRegister::Status => {
                Self::require(width, AccessWidth::Byte, "STATUS")?;
            }
            Samd21UsbRegister::FrameNumber => {
                Self::require(width, AccessWidth::HalfWord, "FNUM")?;
            }
            Samd21UsbRegister::InterruptEnableClear => {
                Self::require(width, AccessWidth::HalfWord, "INTENCLR")?;
                state.interrupt_enable &= !(value as u16 & DEVICE_FLAGS_MASK);
            }
            Samd21UsbRegister::InterruptEnableSet => {
                Self::require(width, AccessWidth::HalfWord, "INTENSET")?;
                state.interrupt_enable |= value as u16 & DEVICE_FLAGS_MASK;
            }
            Samd21UsbRegister::InterruptFlags => {
                Self::require(width, AccessWidth::HalfWord, "INTFLAG")?;
                state.interrupt_flags &= !(value as u16 & DEVICE_FLAGS_MASK);
            }
            Samd21UsbRegister::EndpointInterruptSummary => {
                Self::require(width, AccessWidth::HalfWord, "EPINTSMRY")?;
            }
            Samd21UsbRegister::DescriptorAddress => {
                Self::require(width, AccessWidth::Word, "DESCADD")?;
                state.descriptor_address = value as u32 & !3;
            }
            Samd21UsbRegister::PadCalibration => {
                Self::require(width, AccessWidth::HalfWord, "PADCAL")?;
                state.pad_calibration = value as u16 & 0x5fff;
            }
        }
        Ok(())
    }

    fn read_endpoint(
        state: &UsbState,
        endpoint: usize,
        register: Samd21UsbEndpointRegister,
        width: AccessWidth,
    ) -> Result<u64, DeviceError> {
        Self::require(width, AccessWidth::Byte, "endpoint register")?;
        let state = state.endpoints[endpoint];
        Ok(u64::from(match register {
            Samd21UsbEndpointRegister::Configuration => state.configuration,
            Samd21UsbEndpointRegister::StatusClear
            | Samd21UsbEndpointRegister::StatusSet
            | Samd21UsbEndpointRegister::Status => state.status,
            Samd21UsbEndpointRegister::InterruptFlags => state.interrupt_flags,
            Samd21UsbEndpointRegister::InterruptEnableClear
            | Samd21UsbEndpointRegister::InterruptEnableSet => state.interrupt_enable,
        }))
    }

    fn write_endpoint(
        state: &mut UsbState,
        endpoint: usize,
        register: Samd21UsbEndpointRegister,
        width: AccessWidth,
        value: u64,
    ) -> Result<(), DeviceError> {
        Self::require(width, AccessWidth::Byte, "endpoint register")?;
        let value = value as u8;
        let endpoint_state = &mut state.endpoints[endpoint];
        match register {
            Samd21UsbEndpointRegister::Configuration => {
                endpoint_state.configuration = value & ENDPOINT_MASK;
                if endpoint_state.configuration & ENDPOINT_MASK == 0 {
                    UsbState::clear_endpoint_after_disable(endpoint_state);
                }
            }
            Samd21UsbEndpointRegister::StatusClear => endpoint_state.status &= !value,
            Samd21UsbEndpointRegister::StatusSet => endpoint_state.status |= value,
            Samd21UsbEndpointRegister::Status => {}
            Samd21UsbEndpointRegister::InterruptFlags => endpoint_state.interrupt_flags &= !value,
            Samd21UsbEndpointRegister::InterruptEnableClear => {
                endpoint_state.interrupt_enable &= !value
            }
            Samd21UsbEndpointRegister::InterruptEnableSet => {
                endpoint_state.interrupt_enable |= value
            }
        }
        Ok(())
    }
}

impl Device for Samd21UsbDevice {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let state = self.state.lock().expect("USB lock poisoned");
        if let Some((endpoint, register)) = Self::endpoint_offset(offset) {
            return Self::read_endpoint(&state, endpoint, register, width);
        }
        let register = Samd21UsbRegister::from_offset(offset).ok_or_else(|| {
            DeviceError::new(format!("unmodeled SAM D21 USB read at {offset:#x}"))
        })?;
        Self::read_common(&state, register, width)
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        let mut state = self.state.lock().expect("USB lock poisoned");
        if let Some((endpoint, register)) = Self::endpoint_offset(offset) {
            return Self::write_endpoint(&mut state, endpoint, register, width, value);
        }
        let register = Samd21UsbRegister::from_offset(offset).ok_or_else(|| {
            DeviceError::new(format!("unmodeled SAM D21 USB write at {offset:#x}"))
        })?;
        Self::write_common(&mut state, register, width, value)
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state.lock().expect("USB lock poisoned").reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_control_address_and_endpoint_registers_are_functional() {
        let (mut usb, handle) = Samd21UsbDevice::new("usb");
        usb.write(0x00, AccessWidth::Byte, 2, SimTime::ZERO)
            .unwrap();
        usb.write(0x08, AccessWidth::HalfWord, 0, SimTime::ZERO)
            .unwrap();
        usb.write(0x0a, AccessWidth::Byte, 0x85, SimTime::ZERO)
            .unwrap();
        usb.write(0x24, AccessWidth::Word, 0x2000_0103, SimTime::ZERO)
            .unwrap();
        usb.write(0x120, AccessWidth::Byte, 0x11, SimTime::ZERO)
            .unwrap();
        usb.write(0x125, AccessWidth::Byte, 1 << 6, SimTime::ZERO)
            .unwrap();
        usb.write(0x129, AccessWidth::Byte, 1 << 4, SimTime::ZERO)
            .unwrap();
        assert_eq!(usb.read(0x00, AccessWidth::Byte, SimTime::ZERO).unwrap(), 2);
        assert_eq!(
            usb.read(0x0a, AccessWidth::Byte, SimTime::ZERO).unwrap(),
            0x85
        );
        assert_eq!(
            usb.read(0x24, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0x2000_0100
        );
        assert_eq!(
            usb.read(0x126, AccessWidth::Byte, SimTime::ZERO).unwrap(),
            0x40
        );
        assert!(handle.receive_setup(1));
        assert_eq!(handle.endpoint_interrupt_summary(), 1 << 1);
        assert!(handle.interrupt_pending());
    }

    #[test]
    fn bus_reset_preserves_endpoint_zero_and_latches_end_of_reset() {
        let (mut usb, handle) = Samd21UsbDevice::new("usb");
        usb.write(0x100, AccessWidth::Byte, 0x11, SimTime::ZERO)
            .unwrap();
        usb.write(0x120, AccessWidth::Byte, 0x11, SimTime::ZERO)
            .unwrap();
        usb.write(0x0a, AccessWidth::Byte, 0x85, SimTime::ZERO)
            .unwrap();
        usb.write(0x18, AccessWidth::HalfWord, 1 << 3, SimTime::ZERO)
            .unwrap();
        handle.bus_reset();
        assert_eq!(usb.read(0x0a, AccessWidth::Byte, SimTime::ZERO).unwrap(), 0);
        assert_eq!(
            usb.read(0x100, AccessWidth::Byte, SimTime::ZERO).unwrap(),
            0x11
        );
        assert_eq!(
            usb.read(0x120, AccessWidth::Byte, SimTime::ZERO).unwrap(),
            0
        );
        assert_eq!(
            usb.read(0x1c, AccessWidth::HalfWord, SimTime::ZERO)
                .unwrap(),
            1 << 3
        );
        assert!(handle.interrupt_pending());
    }

    #[test]
    fn sof_updates_frame_and_w1c_clears_common_flag() {
        let (mut usb, handle) = Samd21UsbDevice::new("usb");
        usb.write(0x18, AccessWidth::HalfWord, 1 << 2, SimTime::ZERO)
            .unwrap();
        handle.start_of_frame(0x345);
        assert_eq!(
            usb.read(0x10, AccessWidth::HalfWord, SimTime::ZERO)
                .unwrap(),
            0x345 << 3
        );
        assert!(handle.interrupt_pending());
        usb.write(0x1c, AccessWidth::HalfWord, 1 << 2, SimTime::ZERO)
            .unwrap();
        assert!(!handle.interrupt_pending());
    }
}
