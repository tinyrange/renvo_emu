//! Reusable functional microcontroller peripheral models.

use renvo_bus::{Device, DeviceError};
use renvo_core::{AccessWidth, ResetKind, SimTime};
use renvo_signals::{
    DigitalNet, DriverId, Logic, SignalChange, SignalError, SignalId, SignalRegistry, SignalValue,
};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

/// Shared signal registry and append-only pending-change stream.
#[derive(Clone, Default)]
pub struct SignalHub {
    inner: Arc<Mutex<SignalHubState>>,
}

#[derive(Default)]
struct SignalHubState {
    registry: SignalRegistry,
    changes: Vec<SignalChange>,
}

impl SignalHub {
    /// Creates an empty signal hub.
    pub fn new() -> Self {
        Self::default()
    }

    /// Declares a signal.
    pub fn declare(
        &self,
        path: impl Into<String>,
        initial: SignalValue,
        description: Option<String>,
    ) -> Result<SignalId, SignalError> {
        self.inner
            .lock()
            .expect("signal hub lock poisoned")
            .registry
            .declare(path, initial, description)
    }

    /// Sets a value and queues a real transition.
    pub fn set(
        &self,
        signal: SignalId,
        value: SignalValue,
        at: SimTime,
    ) -> Result<(), SignalError> {
        let mut state = self.inner.lock().expect("signal hub lock poisoned");
        if let Some(change) = state.registry.set(signal, value, at)? {
            state.changes.push(change);
        }
        Ok(())
    }

    /// Runs a read-only operation against the registry.
    pub fn with_registry<T>(&self, operation: impl FnOnce(&SignalRegistry) -> T) -> T {
        let state = self.inner.lock().expect("signal hub lock poisoned");
        operation(&state.registry)
    }

    /// Removes all pending changes in chronological insertion order.
    pub fn drain_changes(&self) -> Vec<SignalChange> {
        let mut state = self.inner.lock().expect("signal hub lock poisoned");
        core::mem::take(&mut state.changes)
    }
}

/// Shared terminal UART output.
#[derive(Clone, Default)]
pub struct UartHandle {
    bytes: Arc<Mutex<Vec<u8>>>,
}

impl UartHandle {
    /// Returns all transmitted bytes.
    pub fn bytes(&self) -> Vec<u8> {
        self.bytes.lock().expect("UART lock poisoned").clone()
    }

    /// Returns lossy UTF-8 terminal output.
    pub fn text_lossy(&self) -> String {
        String::from_utf8_lossy(&self.bytes()).into_owned()
    }

    /// Clears captured output.
    pub fn clear(&self) {
        self.bytes.lock().expect("UART lock poisoned").clear();
    }

    /// Appends bytes transmitted by a functional ROM or peripheral service.
    pub fn transmit(&self, bytes: &[u8]) {
        self.bytes
            .lock()
            .expect("UART lock poisoned")
            .extend_from_slice(bytes);
    }
}

/// Configurable byte-oriented UART facade.
pub struct FunctionalUart {
    name: String,
    data_offset: u64,
    status_offset: u64,
    tx_ready_mask: u32,
    lenient_registers: bool,
    handle: UartHandle,
}

impl FunctionalUart {
    /// Creates a UART and a host handle.
    pub fn new(
        name: impl Into<String>,
        data_offset: u64,
        status_offset: u64,
        tx_ready_mask: u32,
    ) -> (Self, UartHandle) {
        let handle = UartHandle::default();
        (
            Self {
                name: name.into(),
                data_offset,
                status_offset,
                tx_ready_mask,
                lenient_registers: false,
                handle: handle.clone(),
            },
            handle,
        )
    }

    /// Creates a UART that stores bytes at `data_offset` and tolerates other
    /// control-register accesses. This is useful for bounded vendor facades.
    pub fn new_lenient(
        name: impl Into<String>,
        data_offset: u64,
        status_offset: u64,
        tx_ready_mask: u32,
    ) -> (Self, UartHandle) {
        let (mut device, handle) = Self::new(name, data_offset, status_offset, tx_ready_mask);
        device.lenient_registers = true;
        (device, handle)
    }
}

impl Device for FunctionalUart {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, _width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if offset == self.status_offset {
            Ok(u64::from(self.tx_ready_mask))
        } else if offset == self.data_offset || self.lenient_registers {
            Ok(0)
        } else {
            Err(DeviceError::new(format!(
                "unmodeled UART read at offset {offset:#x}"
            )))
        }
    }

    fn write(
        &mut self,
        offset: u64,
        _width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if offset != self.data_offset && !self.lenient_registers {
            return Err(DeviceError::new(format!(
                "unmodeled UART write at offset {offset:#x}"
            )));
        }
        if offset == self.data_offset {
            self.handle
                .bytes
                .lock()
                .expect("UART lock poisoned")
                .push(value.to_le_bytes()[0]);
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.handle.clear();
    }
}

/// Host-facing state for the ESP USB Serial/JTAG CDC-ACM data endpoint.
#[derive(Clone)]
pub struct EspUsbSerialJtagHandle {
    state: Arc<Mutex<EspUsbSerialJtagState>>,
}

#[derive(Default)]
struct EspUsbSerialJtagState {
    rx: VecDeque<u8>,
    tx_packet: Vec<u8>,
    output: Vec<u8>,
    input_queued: bool,
    raw_chunks_queued: usize,
    raw_chunks_completed: usize,
    interrupt_raw: u32,
    interrupt_enable: u32,
    registers: BTreeMap<u64, u32>,
}

impl EspUsbSerialJtagHandle {
    /// Queues bytes sent by the deterministic host to the CDC-ACM OUT endpoint.
    pub fn queue_input(&self, bytes: &[u8]) {
        let mut state = self.state.lock().expect("USB Serial/JTAG lock poisoned");
        state.rx.extend(bytes.iter().copied());
        if !bytes.is_empty() {
            state.input_queued = true;
            state.raw_chunks_queued = state
                .raw_chunks_queued
                .saturating_add(bytes.iter().filter(|byte| **byte == 0x04).count());
            state.interrupt_raw |= 1 << 2;
        }
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
            && state.raw_chunks_queued != 0
            && state.raw_chunks_completed >= state.raw_chunks_queued
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
    const SERIAL_OUT_RECV_PKT: u32 = 1 << 2;
    const SERIAL_IN_EMPTY: u32 = 1 << 3;
    const INTERRUPT_MASK: u32 = 0x7ffff;
    const ENDPOINT_SIZE: usize = 64;

    /// Creates the peripheral and its host-facing handle.
    pub fn new(name: impl Into<String>) -> (Self, EspUsbSerialJtagHandle) {
        let state = Arc::new(Mutex::new(EspUsbSerialJtagState {
            // Hardware reset state: an empty IN endpoint is writable and its
            // raw empty indication is asserted.
            interrupt_raw: Self::SERIAL_IN_EMPTY,
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
        state.raw_chunks_completed = state
            .output
            .windows(3)
            .filter(|window| *window == b"\x04\x04>")
            .count();
        // The functional host takes the packet immediately, making EP1
        // writable again and producing the hardware's IN-empty indication.
        state.interrupt_raw |= Self::SERIAL_IN_EMPTY;
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
        _at: SimTime,
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
            _ => {
                state.registers.insert(offset, value);
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.lock().expect("USB Serial/JTAG lock poisoned");
        *state = EspUsbSerialJtagState {
            interrupt_raw: Self::SERIAL_IN_EMPTY,
            ..EspUsbSerialJtagState::default()
        };
    }
}

struct GpioState {
    direction: u32,
    output: u32,
    nets: Vec<DigitalNet>,
}

/// Host-facing GPIO input and state control.
#[derive(Clone)]
pub struct GpioHandle {
    state: Arc<Mutex<GpioState>>,
    signals: Vec<SignalId>,
    hub: SignalHub,
}

impl GpioHandle {
    /// Number of pins exposed by this port.
    pub fn pin_count(&self) -> usize {
        self.signals.len()
    }

    /// Drives or releases one external pin source.
    pub fn set_input(&self, pin: u8, value: Logic, at: SimTime) -> Result<(), DeviceError> {
        let index = usize::from(pin);
        let mut state = self.state.lock().expect("GPIO lock poisoned");
        let net = state
            .nets
            .get_mut(index)
            .ok_or_else(|| DeviceError::new(format!("GPIO pin {pin} is out of range")))?;
        let update = net.drive(DriverId(1), value);
        drop(state);
        self.hub
            .set(
                self.signals[index],
                SignalValue::repeat(update.value, 1)
                    .expect("one-bit signal construction cannot fail"),
                at,
            )
            .map_err(|error| DeviceError::new(error.to_string()))
    }

    /// Current direction bit mask.
    pub fn direction(&self) -> u32 {
        self.state.lock().expect("GPIO lock poisoned").direction
    }

    /// Current output latch.
    pub fn output(&self) -> u32 {
        self.state.lock().expect("GPIO lock poisoned").output
    }
}

/// Simple GPIO register facade with direction, output, and input registers.
pub struct FunctionalGpio {
    name: String,
    pins: u8,
    direction_offset: u64,
    output_offset: u64,
    input_offset: u64,
    state: Arc<Mutex<GpioState>>,
    signals: Vec<SignalId>,
    hub: SignalHub,
}

impl FunctionalGpio {
    /// Creates a GPIO block and host input handle.
    pub fn new(
        name: impl Into<String>,
        pins: u8,
        path: &str,
        hub: SignalHub,
        direction_offset: u64,
        output_offset: u64,
        input_offset: u64,
    ) -> Result<(Self, GpioHandle), SignalError> {
        if pins == 0 || pins > 32 {
            return Err(SignalError::WidthMismatch {
                expected: 1,
                actual: u16::from(pins),
            });
        }
        let mut signals = Vec::with_capacity(usize::from(pins));
        for pin in 0..pins {
            signals.push(hub.declare(
                format!("{path}.pin{pin}"),
                SignalValue::repeat(Logic::Z, 1)?,
                Some(format!("GPIO pin {pin}")),
            )?);
        }
        let state = Arc::new(Mutex::new(GpioState {
            direction: 0,
            output: 0,
            nets: (0..pins).map(|_| DigitalNet::new()).collect(),
        }));
        let handle = GpioHandle {
            state: state.clone(),
            signals: signals.clone(),
            hub: hub.clone(),
        };
        Ok((
            Self {
                name: name.into(),
                pins,
                direction_offset,
                output_offset,
                input_offset,
                state,
                signals,
                hub,
            },
            handle,
        ))
    }

    fn mask(&self) -> u32 {
        if self.pins == 32 {
            u32::MAX
        } else {
            (1_u32 << self.pins) - 1
        }
    }

    fn refresh_outputs(&self, at: SimTime) -> Result<(), DeviceError> {
        refresh_gpio(&self.state, &self.signals, &self.hub, self.pins, at)
    }
}

fn refresh_gpio(
    shared: &Arc<Mutex<GpioState>>,
    signals: &[SignalId],
    hub: &SignalHub,
    pins: u8,
    at: SimTime,
) -> Result<(), DeviceError> {
    let mut state = shared.lock().expect("GPIO lock poisoned");
    for pin in 0..pins {
        let logic = if state.direction & (1_u32 << pin) == 0 {
            Logic::Z
        } else if state.output & (1_u32 << pin) == 0 {
            Logic::Zero
        } else {
            Logic::One
        };
        let update = state.nets[usize::from(pin)].drive(DriverId(0), logic);
        hub.set(
            signals[usize::from(pin)],
            SignalValue::repeat(update.value, 1).expect("one-bit signal construction cannot fail"),
            at,
        )
        .map_err(|error| DeviceError::new(error.to_string()))?;
    }
    Ok(())
}

type VendorGpioParts = (Arc<Mutex<GpioState>>, Vec<SignalId>, GpioHandle);

fn vendor_gpio(pins: u8, path: &str, hub: &SignalHub) -> Result<VendorGpioParts, SignalError> {
    if pins == 0 || pins > 32 {
        return Err(SignalError::WidthMismatch {
            expected: 32,
            actual: u16::from(pins),
        });
    }
    let mut signals = Vec::with_capacity(usize::from(pins));
    for pin in 0..pins {
        signals.push(hub.declare(
            format!("{path}.pin{pin}"),
            SignalValue::repeat(Logic::Z, 1)?,
            Some(format!("GPIO pin {pin}")),
        )?);
    }
    let state = Arc::new(Mutex::new(GpioState {
        direction: 0,
        output: 0,
        nets: (0..pins).map(|_| DigitalNet::new()).collect(),
    }));
    let handle = GpioHandle {
        state: state.clone(),
        signals: signals.clone(),
        hub: hub.clone(),
    };
    Ok((state, signals, handle))
}

impl Device for FunctionalGpio {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("GPIO facade requires word accesses"));
        }
        let state = self.state.lock().expect("GPIO lock poisoned");
        let value = if offset == self.direction_offset {
            state.direction
        } else if offset == self.output_offset {
            state.output
        } else if offset == self.input_offset {
            let mut resolved = 0_u32;
            for pin in 0..self.pins {
                if state.nets[usize::from(pin)].resolved() == Logic::One {
                    resolved |= 1_u32 << pin;
                }
            }
            resolved
        } else {
            return Err(DeviceError::new(format!(
                "unmodeled GPIO read at offset {offset:#x}"
            )));
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
            return Err(DeviceError::new("GPIO facade requires word accesses"));
        }
        let value = u32::try_from(value & u64::from(u32::MAX))
            .expect("masked value always fits in u32")
            & self.mask();
        {
            let mut state = self.state.lock().expect("GPIO lock poisoned");
            if offset == self.direction_offset {
                state.direction = value;
            } else if offset == self.output_offset {
                state.output = value;
            } else {
                return Err(DeviceError::new(format!(
                    "unmodeled GPIO write at offset {offset:#x}"
                )));
            }
        }
        self.refresh_outputs(at)
    }

    fn reset(&mut self, _kind: ResetKind) {
        {
            let mut state = self.state.lock().expect("GPIO lock poisoned");
            state.direction = 0;
            state.output = 0;
            for net in &mut state.nets {
                net.disconnect(DriverId(0));
            }
        }
        let _ = self.refresh_outputs(SimTime::ZERO);
    }
}

/// WCH `CH32V00x` GPIO register slice (`CFGLR/INDR/OUTDR/BSHR/BCR`).
pub struct WchGpio {
    name: String,
    pins: u8,
    config_low: u32,
    config_high: u32,
    state: Arc<Mutex<GpioState>>,
    signals: Vec<SignalId>,
    hub: SignalHub,
}

impl WchGpio {
    /// Creates one WCH GPIO port and an external-stimulus handle.
    pub fn new(
        name: impl Into<String>,
        pins: u8,
        path: &str,
        hub: SignalHub,
    ) -> Result<(Self, GpioHandle), SignalError> {
        let (state, signals, handle) = vendor_gpio(pins, path, &hub)?;
        Ok((
            Self {
                name: name.into(),
                pins,
                config_low: 0x4444_4444,
                config_high: 0x4444_4444,
                state,
                signals,
                hub,
            },
            handle,
        ))
    }

    fn update_direction(&mut self, at: SimTime) -> Result<(), DeviceError> {
        let mut direction = 0_u32;
        for pin in 0..self.pins {
            let config = if pin < 8 {
                self.config_low >> (u32::from(pin) * 4)
            } else {
                self.config_high >> (u32::from(pin - 8) * 4)
            };
            if config & 3 != 0 {
                direction |= 1_u32 << pin;
            }
        }
        self.state.lock().expect("GPIO lock poisoned").direction = direction;
        refresh_gpio(&self.state, &self.signals, &self.hub, self.pins, at)
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

impl Device for WchGpio {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("WCH GPIO requires word access"));
        }
        let state = self.state.lock().expect("GPIO lock poisoned");
        let value = match offset {
            0x00 => self.config_low,
            0x04 => self.config_high,
            0x08 => {
                drop(state);
                return Ok(u64::from(self.resolved_input()));
            }
            0x0c => state.output,
            0x10 | 0x14 | 0x18 => 0,
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled WCH GPIO read at offset {offset:#x}"
                )));
            }
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
            return Err(DeviceError::new("WCH GPIO requires word access"));
        }
        let value =
            u32::try_from(value & u64::from(u32::MAX)).expect("masked value always fits u32");
        let register_offset = offset & 0x0fff;
        match register_offset {
            0x00 => {
                self.config_low = value;
                return self.update_direction(at);
            }
            0x04 => {
                self.config_high = value;
                return self.update_direction(at);
            }
            0x0c => self.state.lock().expect("GPIO lock poisoned").output = value,
            0x10 => {
                let mut state = self.state.lock().expect("GPIO lock poisoned");
                state.output |= value & 0xffff;
                state.output &= !(value >> 16);
            }
            0x14 => self.state.lock().expect("GPIO lock poisoned").output &= !value,
            0x18 => {}
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled WCH GPIO write at offset {offset:#x}"
                )));
            }
        }
        refresh_gpio(&self.state, &self.signals, &self.hub, self.pins, at)
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.config_low = 0x4444_4444;
        self.config_high = 0x4444_4444;
        let mut state = self.state.lock().expect("GPIO lock poisoned");
        state.direction = 0;
        state.output = 0;
    }
}

/// Functional RP2040 reset controller, including the peripheral atomic-register aliases.
pub struct Rp2040Resets {
    name: String,
    reset: u32,
    watchdog_select: u32,
}

impl Rp2040Resets {
    const VALID_MASK: u32 = 0x01ff_ffff;

    /// Creates a reset controller in the boot-ROM handoff state.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            reset: 0,
            watchdog_select: 0,
        }
    }

    fn register_value(&self, register: u64) -> Result<u32, DeviceError> {
        match register {
            0x00 => Ok(self.reset),
            0x04 => Ok(self.watchdog_select),
            0x08 => Ok(!self.reset & Self::VALID_MASK),
            _ => Err(DeviceError::new(format!(
                "unmodeled RP2040 RESETS read at offset {register:#x}"
            ))),
        }
    }

    fn update(register: &mut u32, alias: u64, value: u32) -> Result<(), DeviceError> {
        match alias {
            0 => *register = value,
            1 => *register ^= value,
            2 => *register |= value,
            3 => *register &= !value,
            _ => return Err(DeviceError::new("invalid RP2040 atomic alias")),
        }
        Ok(())
    }
}

impl Device for Rp2040Resets {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("RP2040 RESETS requires word access"));
        }
        Ok(u64::from(self.register_value(offset & 0x0fff)?))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("RP2040 RESETS requires word access"));
        }
        let alias = (offset >> 12) & 3;
        let register = offset & 0x0fff;
        let value =
            u32::try_from(value & u64::from(u32::MAX)).expect("masked RP2040 register value fits");
        match register {
            0x00 => {
                Self::update(&mut self.reset, alias, value)?;
                self.reset &= Self::VALID_MASK;
            }
            0x04 => {
                Self::update(&mut self.watchdog_select, alias, value)?;
                self.watchdog_select &= Self::VALID_MASK;
            }
            0x08 => {
                return Err(DeviceError::new(
                    "RP2040 RESET_DONE is a read-only register",
                ));
            }
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled RP2040 RESETS write at offset {register:#x}"
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.reset = 0;
        self.watchdog_select = 0;
    }
}

/// Functional RP2040 clock controller with immediate source selection.
pub struct Rp2040Clocks {
    name: String,
    registers: [u32; 50],
}

impl Rp2040Clocks {
    /// Creates the reset-state clock register bank.
    pub fn new(name: impl Into<String>) -> Self {
        let mut registers = [0; 50];
        for offset in [0x04_usize, 0x10, 0x1c, 0x28, 0x34, 0x40, 0x58, 0x64, 0x70] {
            registers[offset / 4] = 0x100;
        }
        for offset in [
            0x08_usize, 0x14, 0x20, 0x2c, 0x38, 0x44, 0x50, 0x5c, 0x68, 0x74,
        ] {
            registers[offset / 4] = 1;
        }
        Self {
            name: name.into(),
            registers,
        }
    }

    fn update(register: &mut u32, alias: u64, value: u32) -> Result<(), DeviceError> {
        match alias {
            0 => *register = value,
            1 => *register ^= value,
            2 => *register |= value,
            3 => *register &= !value,
            _ => return Err(DeviceError::new("invalid RP2040 CLOCKS atomic alias")),
        }
        Ok(())
    }
}

impl Device for Rp2040Clocks {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "RP2040 CLOCKS requires aligned word access",
            ));
        }
        let register_offset = offset & 0x0fff;
        // Source switching is functional and instantaneous. The selected registers remain
        // one-hot because the SDK also uses exact equality while moving through the glitchless
        // reference and system-clock muxes.
        if register_offset == 0x38 {
            return Ok(u64::from(1_u32 << (self.registers[0x30 / 4] & 3)));
        }
        if register_offset == 0x44 {
            return Ok(u64::from(1_u32 << (self.registers[0x3c / 4] & 1)));
        }
        if matches!(
            register_offset,
            0x08 | 0x14 | 0x20 | 0x2c | 0x50 | 0x5c | 0x68 | 0x74
        ) {
            return Ok(1);
        }
        if register_offset == 0xb0 || register_offset == 0xb4 {
            return Ok(u64::from(u32::MAX));
        }
        let index = usize::try_from(register_offset / 4).expect("small clock offset fits");
        self.registers
            .get(index)
            .copied()
            .map(u64::from)
            .ok_or_else(|| {
                DeviceError::new(format!(
                    "unmodeled RP2040 CLOCKS read at offset {register_offset:#x}"
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
                "RP2040 CLOCKS requires aligned word access",
            ));
        }
        let alias = (offset >> 12) & 3;
        let register_offset = offset & 0x0fff;
        let index = usize::try_from(register_offset / 4).expect("small clock offset fits");
        let register = self.registers.get_mut(index).ok_or_else(|| {
            DeviceError::new(format!(
                "unmodeled RP2040 CLOCKS write at offset {register_offset:#x}"
            ))
        })?;
        let value =
            u32::try_from(value & u64::from(u32::MAX)).expect("masked clock register value fits");
        Self::update(register, alias, value)
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self = Self::new(self.name.clone());
    }
}

/// Functional RP2040 crystal oscillator with immediate stabilization.
pub struct Rp2040Xosc {
    name: String,
    control: u32,
    startup: u32,
    count: u32,
}

impl Rp2040Xosc {
    /// Creates a reset-state crystal oscillator.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            control: 0,
            startup: 0,
            count: 0,
        }
    }
}

impl Device for Rp2040Xosc {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("RP2040 XOSC requires word access"));
        }
        let value = match offset & 0x0fff {
            0x00 => self.control,
            0x04 => 0x8000_1000 | (self.control & 3),
            0x08 => 0,
            0x0c => self.startup,
            0x1c => self.count,
            register => {
                return Err(DeviceError::new(format!(
                    "unmodeled RP2040 XOSC read at offset {register:#x}"
                )));
            }
        };
        Ok(u64::from(value))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("RP2040 XOSC requires word access"));
        }
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("masked XOSC value fits");
        match offset & 0x0fff {
            0x00 => self.control = value & 0x00ff_ffff,
            0x04 => {}
            0x08 => {}
            0x0c => self.startup = value & 0x0010_3fff,
            0x1c => self.count = value & 0xff,
            register => {
                return Err(DeviceError::new(format!(
                    "unmodeled RP2040 XOSC write at offset {register:#x}"
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.control = 0;
        self.startup = 0;
        self.count = 0;
    }
}

/// Functional RP2040 PLL with immediate lock acquisition.
pub struct Rp2040Pll {
    name: String,
    control_status: u32,
    power: u32,
    feedback_divider: u32,
    primitive: u32,
}

impl Rp2040Pll {
    /// Creates a reset-state PLL.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            control_status: 1,
            power: 0x2d,
            feedback_divider: 0,
            primitive: 0x0007_7000,
        }
    }
}

impl Device for Rp2040Pll {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("RP2040 PLL requires word access"));
        }
        let value = match offset & 0x0fff {
            0x00 => self.control_status | 0x8000_0000,
            0x04 => self.power,
            0x08 => self.feedback_divider,
            0x0c => self.primitive,
            register => {
                return Err(DeviceError::new(format!(
                    "unmodeled RP2040 PLL read at offset {register:#x}"
                )));
            }
        };
        Ok(u64::from(value))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("RP2040 PLL requires word access"));
        }
        let alias = (offset >> 12) & 3;
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("masked PLL value fits");
        let register = match offset & 0x0fff {
            0x00 => &mut self.control_status,
            0x04 => &mut self.power,
            0x08 => &mut self.feedback_divider,
            0x0c => &mut self.primitive,
            register => {
                return Err(DeviceError::new(format!(
                    "unmodeled RP2040 PLL write at offset {register:#x}"
                )));
            }
        };
        Rp2040Clocks::update(register, alias, value)
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self = Self::new(self.name.clone());
    }
}

/// Functional RP2040 watchdog and microsecond-tick divider.
pub struct Rp2040Watchdog {
    name: String,
    registers: [u32; 12],
}

impl Rp2040Watchdog {
    /// Creates the watchdog reset state.
    pub fn new(name: impl Into<String>) -> Self {
        let mut registers = [0; 12];
        registers[0] = 0x0700_0000;
        registers[0x2c / 4] = 0x200;
        Self {
            name: name.into(),
            registers,
        }
    }
}

impl Device for Rp2040Watchdog {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "RP2040 WATCHDOG requires aligned word access",
            ));
        }
        let register_offset = offset & 0x0fff;
        let index = usize::try_from(register_offset / 4).expect("small watchdog offset fits");
        let mut value = *self.registers.get(index).ok_or_else(|| {
            DeviceError::new(format!(
                "unmodeled RP2040 WATCHDOG read at offset {register_offset:#x}"
            ))
        })?;
        if register_offset == 0x2c && value & 0x200 != 0 {
            value |= 0x400;
        }
        Ok(u64::from(value))
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
                "RP2040 WATCHDOG requires aligned word access",
            ));
        }
        let alias = (offset >> 12) & 3;
        let register_offset = offset & 0x0fff;
        if register_offset == 0x08 {
            return Err(DeviceError::new("RP2040 WATCHDOG REASON is read-only"));
        }
        let index = usize::try_from(register_offset / 4).expect("small watchdog offset fits");
        let register = self.registers.get_mut(index).ok_or_else(|| {
            DeviceError::new(format!(
                "unmodeled RP2040 WATCHDOG write at offset {register_offset:#x}"
            ))
        })?;
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("masked watchdog value fits");
        Rp2040Clocks::update(register, alias, value)
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self = Self::new(self.name.clone());
    }
}

/// Shared RP2040 timer interrupt view.
#[derive(Clone)]
pub struct Rp2040TimerHandle {
    state: Arc<Mutex<Rp2040TimerState>>,
}

impl Rp2040TimerHandle {
    /// Returns the four masked alarm interrupt bits at `now`.
    pub fn pending(&self, now: SimTime) -> u8 {
        let mut state = self.state.lock().expect("RP2040 timer lock poisoned");
        let previous = state.raw_interrupt;
        state.update(now);
        let pending = (state.raw_interrupt | state.force_interrupt) & state.interrupt_enable;
        if state.raw_interrupt != previous && std::env::var_os("RENVO_DEBUG_TIMERS").is_some() {
            eprintln!(
                "RP timer alarm at={} raw={:#x} enabled={:#x} pending={pending:#x}",
                now.ticks(),
                state.raw_interrupt,
                state.interrupt_enable,
            );
        }
        pending
    }
}

struct Rp2040TimerState {
    alarms: [u32; 4],
    armed: u8,
    raw_interrupt: u8,
    interrupt_enable: u8,
    force_interrupt: u8,
    debug_pause: u32,
    paused: bool,
}

impl Rp2040TimerState {
    fn update(&mut self, now: SimTime) {
        if self.paused {
            return;
        }
        let current = now.ticks() as u32;
        for alarm in 0..4 {
            let mask = 1_u8 << alarm;
            if self.armed & mask != 0 && current.wrapping_sub(self.alarms[alarm]) < 0x8000_0000 {
                self.armed &= !mask;
                self.raw_interrupt |= mask;
            }
        }
    }
}

/// Functional RP2040 64-bit microsecond timer and four alarms.
pub struct Rp2040Timer {
    name: String,
    layout: RpTimerLayout,
    state: Arc<Mutex<Rp2040TimerState>>,
}

/// Register layout implemented by the Raspberry Pi timer block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RpTimerLayout {
    /// RP2040 timer layout with interrupt registers beginning at offset `0x34`.
    Rp2040,
    /// RP2350 timer layout with LOCKED/SOURCE and interrupts beginning at `0x3c`.
    Rp2350,
}

impl Rp2040Timer {
    /// Creates the free-running timer and a scheduler-facing handle.
    pub fn new(name: impl Into<String>, layout: RpTimerLayout) -> (Self, Rp2040TimerHandle) {
        let state = Arc::new(Mutex::new(Rp2040TimerState {
            alarms: [0; 4],
            armed: 0,
            raw_interrupt: 0,
            interrupt_enable: 0,
            force_interrupt: 0,
            debug_pause: 7,
            paused: false,
        }));
        let handle = Rp2040TimerHandle {
            state: state.clone(),
        };
        (
            Self {
                name: name.into(),
                layout,
                state,
            },
            handle,
        )
    }
}

impl Device for Rp2040Timer {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("RP2040 TIMER requires word access"));
        }
        let mut state = self.state.lock().expect("RP2040 timer lock poisoned");
        state.update(at);
        let ticks = at.ticks();
        let value = match offset & 0x0fff {
            0x00 | 0x08 | 0x24 => (ticks >> 32) as u32,
            0x04 | 0x0c | 0x28 => ticks as u32,
            0x10 | 0x14 | 0x18 | 0x1c => {
                state.alarms
                    [usize::try_from(((offset & 0x0fff) - 0x10) / 4).expect("alarm index fits")]
            }
            0x20 => u32::from(state.armed),
            0x2c => state.debug_pause,
            0x30 => u32::from(state.paused),
            0x34 if self.layout == RpTimerLayout::Rp2040 => u32::from(state.raw_interrupt),
            0x38 if self.layout == RpTimerLayout::Rp2040 => u32::from(state.interrupt_enable),
            0x3c if self.layout == RpTimerLayout::Rp2040 => u32::from(state.force_interrupt),
            0x40 if self.layout == RpTimerLayout::Rp2040 => {
                u32::from((state.raw_interrupt | state.force_interrupt) & state.interrupt_enable)
            }
            // RP2350 inserts read-only LOCKED and SOURCE registers before INTR.
            0x34 | 0x38 if self.layout == RpTimerLayout::Rp2350 => 0,
            0x3c if self.layout == RpTimerLayout::Rp2350 => u32::from(state.raw_interrupt),
            0x40 if self.layout == RpTimerLayout::Rp2350 => u32::from(state.interrupt_enable),
            0x44 if self.layout == RpTimerLayout::Rp2350 => u32::from(state.force_interrupt),
            0x48 if self.layout == RpTimerLayout::Rp2350 => {
                u32::from((state.raw_interrupt | state.force_interrupt) & state.interrupt_enable)
            }
            register => {
                return Err(DeviceError::new(format!(
                    "unmodeled RP2040 TIMER read at offset {register:#x}"
                )));
            }
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
            return Err(DeviceError::new("RP2040 TIMER requires word access"));
        }
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("masked timer value fits");
        let mut state = self.state.lock().expect("RP2040 timer lock poisoned");
        state.update(at);
        if std::env::var_os("RENVO_DEBUG_TIMERS").is_some() {
            eprintln!(
                "RP timer write at={} offset={offset:#06x} value={value:#010x}",
                at.ticks()
            );
        }
        let register = offset & 0x0fff;
        let alias = (offset >> 12) & 3;
        let update_alias = |current: u8, value: u8| match alias {
            0 => value,
            1 => current ^ value,
            2 => current | value,
            3 => current & !value,
            _ => unreachable!("two-bit alias"),
        };
        match register {
            register @ (0x10 | 0x14 | 0x18 | 0x1c) => {
                let alarm = usize::try_from((register - 0x10) / 4).expect("alarm index fits");
                state.alarms[alarm] = value;
                state.armed |= 1 << alarm;
                state.raw_interrupt &= !(1 << alarm);
                if std::env::var_os("RENVO_DEBUG_TIMERS").is_some() {
                    eprintln!(
                        "RP timer arm alarm={alarm} at={} compare={value:#010x}",
                        at.ticks()
                    );
                }
            }
            0x20 => state.armed &= !(value as u8),
            0x2c => state.debug_pause = value & 6,
            0x30 => state.paused = value & 1 != 0,
            0x34 if self.layout == RpTimerLayout::Rp2040 => {
                state.raw_interrupt &= !(value as u8);
            }
            0x38 if self.layout == RpTimerLayout::Rp2040 => {
                state.interrupt_enable = update_alias(state.interrupt_enable, value as u8 & 0xf);
                if std::env::var_os("RENVO_DEBUG_TIMERS").is_some() {
                    eprintln!(
                        "RP timer interrupt enable at={} mask={:#x}",
                        at.ticks(),
                        state.interrupt_enable
                    );
                }
            }
            0x3c if self.layout == RpTimerLayout::Rp2040 => {
                state.force_interrupt = update_alias(state.force_interrupt, value as u8 & 0xf);
            }
            // LOCKED is read-only. SOURCE selection is not timing-visible in
            // the functional model because both supported sources advance on
            // the same deterministic simulation timeline.
            0x34 | 0x38 if self.layout == RpTimerLayout::Rp2350 => {}
            0x3c if self.layout == RpTimerLayout::Rp2350 => {
                state.raw_interrupt &= !(value as u8);
            }
            0x40 if self.layout == RpTimerLayout::Rp2350 => {
                state.interrupt_enable = update_alias(state.interrupt_enable, value as u8 & 0xf);
                if std::env::var_os("RENVO_DEBUG_TIMERS").is_some() {
                    eprintln!(
                        "RP2350 timer interrupt enable at={} mask={:#x}",
                        at.ticks(),
                        state.interrupt_enable
                    );
                }
            }
            0x44 if self.layout == RpTimerLayout::Rp2350 => {
                state.force_interrupt = update_alias(state.force_interrupt, value as u8 & 0xf);
            }
            register => {
                return Err(DeviceError::new(format!(
                    "unmodeled RP2040 TIMER write at offset {register:#x}"
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.lock().expect("RP2040 timer lock poisoned");
        state.alarms = [0; 4];
        state.armed = 0;
        state.raw_interrupt = 0;
        state.interrupt_enable = 0;
        state.force_interrupt = 0;
        state.debug_pause = 7;
        state.paused = false;
    }
}

/// Storage-backed RP2040 APB register slice with atomic XOR, SET, and CLEAR aliases.
///
/// This is used for configuration-only blocks whose values affect observability but do not yet
/// schedule independent events, such as pad and GPIO-function selection registers.
pub struct Rp2040RegisterBank {
    name: String,
    reset_values: Vec<u32>,
    registers: Vec<u32>,
}

impl Rp2040RegisterBank {
    /// Creates a word-addressed register slice initialized from `reset_values`.
    pub fn new(name: impl Into<String>, reset_values: Vec<u32>) -> Self {
        Self {
            name: name.into(),
            registers: reset_values.clone(),
            reset_values,
        }
    }
}

impl Device for Rp2040RegisterBank {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "RP2040 register bank requires aligned word access",
            ));
        }
        let register_offset = offset & 0x0fff;
        let index = usize::try_from(register_offset / 4).expect("small register offset fits");
        self.registers
            .get(index)
            .copied()
            .map(u64::from)
            .ok_or_else(|| {
                DeviceError::new(format!(
                    "{} read outside modeled registers at offset {register_offset:#x}",
                    self.name
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
                "RP2040 register bank requires aligned word access",
            ));
        }
        let alias = (offset >> 12) & 3;
        let register_offset = offset & 0x0fff;
        let index = usize::try_from(register_offset / 4).expect("small register offset fits");
        let register = self.registers.get_mut(index).ok_or_else(|| {
            DeviceError::new(format!(
                "{} write outside modeled registers at offset {register_offset:#x}",
                self.name
            ))
        })?;
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("masked register value fits");
        Rp2040Clocks::update(register, alias, value)
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.registers.clone_from(&self.reset_values);
    }
}

/// Deterministic word-oriented hardware random-number register block.
///
/// Functional MCU models use this at the vendor RNG data-register address.
/// It deliberately provides reproducible pseudo-random words: firmware sees
/// changing entropy-like input while repeat traces remain byte-for-byte
/// stable.
pub struct DeterministicRng {
    name: String,
    data_offset: u64,
    seed: u32,
    state: u32,
}

impl DeterministicRng {
    /// Creates a deterministic RNG block with one readable data register.
    pub fn new(name: impl Into<String>, data_offset: u64, seed: u32) -> Self {
        let seed = if seed == 0 { 0x6d2b_79f5 } else { seed };
        Self {
            name: name.into(),
            data_offset,
            seed,
            state: seed,
        }
    }

    fn next(&mut self) -> u32 {
        let mut value = self.state;
        value ^= value << 13;
        value ^= value >> 17;
        value ^= value << 5;
        self.state = value;
        value
    }
}

impl Device for DeterministicRng {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "deterministic RNG requires aligned word access",
            ));
        }
        Ok(if offset == self.data_offset {
            u64::from(self.next())
        } else {
            0
        })
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        _value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "deterministic RNG requires aligned word access",
            ));
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state = self.seed;
    }
}

/// ESP timer-group register layout.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EspTimerGroupKind {
    /// ESP32-C6 timer group with one general-purpose timer.
    Esp32C6,
    /// ESP32-S3 timer group with two general-purpose timers.
    Esp32S3,
}

impl EspTimerGroupKind {
    const fn timer_count(self) -> usize {
        match self {
            Self::Esp32C6 => 1,
            Self::Esp32S3 => 2,
        }
    }
}

#[derive(Clone, Copy)]
struct EspTimerCounter {
    base_value: u64,
    base_time: SimTime,
    latched_value: u64,
}

struct EspTimerGroupState {
    registers: Vec<u32>,
    counters: [EspTimerCounter; 2],
    kind: EspTimerGroupKind,
}

impl EspTimerGroupState {
    const TIMER_STRIDE: usize = 0x24;
    const CONFIG: usize = 0x00;
    const COUNTER_LOW: usize = 0x04;
    const COUNTER_HIGH: usize = 0x08;
    const UPDATE: usize = 0x0c;
    const ALARM_LOW: usize = 0x10;
    const ALARM_HIGH: usize = 0x14;
    const LOAD_LOW: usize = 0x18;
    const LOAD_HIGH: usize = 0x1c;
    const LOAD: usize = 0x20;
    const INTERRUPT_ENABLE: usize = 0x70;
    const INTERRUPT_RAW: usize = 0x74;
    const INTERRUPT_STATUS: usize = 0x78;
    const INTERRUPT_CLEAR: usize = 0x7c;
    const COUNTER_MASK: u64 = (1_u64 << 54) - 1;

    fn new(kind: EspTimerGroupKind) -> Self {
        let counter = EspTimerCounter {
            base_value: 0,
            base_time: SimTime::ZERO,
            latched_value: 0,
        };
        let mut state = Self {
            registers: vec![0; 0x1000 / 4],
            counters: [counter; 2],
            kind,
        };
        state.reset();
        state
    }

    fn reset(&mut self) {
        self.registers.fill(0);
        self.counters.fill(EspTimerCounter {
            base_value: 0,
            base_time: SimTime::ZERO,
            latched_value: 0,
        });
        // The RTC calibration block completes synchronously in the functional
        // timing model. This represents a nominal 136 kHz slow clock measured
        // against a 40 MHz crystal.
        self.registers[0x68 / 4] = (1 << 12) | (1 << 15);
        self.registers[0x6c / 4] = (301_176 << 7) | 1;
        self.registers[0x80 / 4] = (3 << 3) | (0x01ff_ffff << 7);
        self.registers[0xf8 / 4] = 35_676_274;
    }

    fn timer_register(&self, offset: usize) -> Option<(usize, usize)> {
        let timer = offset / Self::TIMER_STRIDE;
        let register = offset % Self::TIMER_STRIDE;
        (timer < self.kind.timer_count()).then_some((timer, register))
    }

    fn register(&self, timer: usize, register: usize) -> u32 {
        self.registers[(timer * Self::TIMER_STRIDE + register) / 4]
    }

    fn set_register(&mut self, timer: usize, register: usize, value: u32) {
        self.registers[(timer * Self::TIMER_STRIDE + register) / 4] = value;
    }

    fn divider(config: u32) -> u64 {
        match (config >> 13) & 0xffff {
            0 => 65_536,
            1 | 2 => 2,
            divider => u64::from(divider),
        }
    }

    fn counter_value(&self, timer: usize, now: SimTime) -> u64 {
        let counter = self.counters[timer];
        let config = self.register(timer, Self::CONFIG);
        if config & (1 << 31) == 0 {
            return counter.base_value;
        }
        let elapsed = now.ticks().saturating_sub(counter.base_time.ticks());
        // The functional timeline uses eight abstract source counts per
        // instruction. This preserves the divider relationship while avoiding
        // a claim of wall-clock or cycle accuracy.
        let increment = elapsed
            .saturating_mul(8)
            .checked_div(Self::divider(config))
            .unwrap_or(0);
        if config & (1 << 30) != 0 {
            counter.base_value.wrapping_add(increment) & Self::COUNTER_MASK
        } else {
            counter.base_value.wrapping_sub(increment) & Self::COUNTER_MASK
        }
    }

    fn materialize(&mut self, timer: usize, now: SimTime) {
        let value = self.counter_value(timer, now);
        self.counters[timer].base_value = value;
        self.counters[timer].base_time = now;
    }

    fn load_value(&self, timer: usize) -> u64 {
        (u64::from(self.register(timer, Self::LOAD_HIGH) & 0x003f_ffff) << 32)
            | u64::from(self.register(timer, Self::LOAD_LOW))
    }

    fn alarm_value(&self, timer: usize) -> u64 {
        (u64::from(self.register(timer, Self::ALARM_HIGH) & 0x003f_ffff) << 32)
            | u64::from(self.register(timer, Self::ALARM_LOW))
    }

    fn advance(&mut self, now: SimTime) {
        for timer in 0..self.kind.timer_count() {
            let config = self.register(timer, Self::CONFIG);
            let mask = 1_u32 << timer;
            if config & ((1 << 31) | (1 << 10)) != ((1 << 31) | (1 << 10))
                || self.registers[Self::INTERRUPT_RAW / 4] & mask != 0
            {
                continue;
            }
            let counter = self.counter_value(timer, now);
            let alarm = self.alarm_value(timer);
            let reached = if config & (1 << 30) != 0 {
                counter >= alarm
            } else {
                counter <= alarm
            };
            if !reached {
                continue;
            }

            self.registers[Self::INTERRUPT_RAW / 4] |= mask;
            self.set_register(timer, Self::CONFIG, config & !(1 << 10));
            if config & (1 << 29) != 0 {
                self.counters[timer].base_value = self.load_value(timer);
            } else {
                self.counters[timer].base_value = counter;
            }
            self.counters[timer].base_time = now;
        }
        self.registers[Self::INTERRUPT_STATUS / 4] =
            self.registers[Self::INTERRUPT_RAW / 4] & self.registers[Self::INTERRUPT_ENABLE / 4];
    }
}

/// Interrupt view of one ESP timer group.
#[derive(Clone)]
pub struct EspTimerGroupHandle {
    state: Arc<Mutex<EspTimerGroupState>>,
}

impl EspTimerGroupHandle {
    /// Advances the timers and returns masked timer interrupt levels.
    pub fn pending(&self, now: SimTime) -> [bool; 2] {
        let mut state = self
            .state
            .lock()
            .expect("ESP timer-group state lock poisoned");
        state.advance(now);
        let status = state.registers[EspTimerGroupState::INTERRUPT_STATUS / 4];
        [status & 1 != 0, status & 2 != 0]
    }
}

/// Functional ESP32-C6/S3 general-purpose timer group and RTC calibration block.
pub struct EspTimerGroup {
    name: String,
    state: Arc<Mutex<EspTimerGroupState>>,
}

impl EspTimerGroup {
    /// Creates a reset timer group and scheduler-facing interrupt handle.
    pub fn new(name: impl Into<String>, kind: EspTimerGroupKind) -> (Self, EspTimerGroupHandle) {
        let state = Arc::new(Mutex::new(EspTimerGroupState::new(kind)));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            EspTimerGroupHandle { state },
        )
    }
}

impl Device for EspTimerGroup {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP timer group requires aligned word access",
            ));
        }
        let index = usize::try_from(offset / 4).expect("timer-group offset fits");
        let mut state = self
            .state
            .lock()
            .expect("ESP timer-group state lock poisoned");
        state.advance(at);
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
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP timer group requires aligned word access",
            ));
        }
        let offset = usize::try_from(offset).expect("timer-group offset fits");
        let index = offset / 4;
        let value = value as u32;
        let mut state = self
            .state
            .lock()
            .expect("ESP timer-group state lock poisoned");
        if index >= state.registers.len() {
            return Err(DeviceError::new(format!(
                "{} write at {offset:#x}",
                self.name
            )));
        }
        state.advance(at);
        if std::env::var_os("RENVO_DEBUG_TIMERS").is_some() && offset <= 0x80 {
            eprintln!(
                "{} write at={} offset={offset:#04x} value={value:#010x}",
                self.name,
                at.ticks(),
            );
        }

        if let Some((timer, register)) = state.timer_register(offset) {
            match register {
                EspTimerGroupState::CONFIG => {
                    state.materialize(timer, at);
                    state.set_register(timer, register, value);
                }
                EspTimerGroupState::UPDATE => {
                    state.counters[timer].latched_value = state.counter_value(timer, at);
                    let latched = state.counters[timer].latched_value;
                    state.set_register(timer, EspTimerGroupState::COUNTER_LOW, latched as u32);
                    state.set_register(
                        timer,
                        EspTimerGroupState::COUNTER_HIGH,
                        u32::try_from(latched >> 32).expect("54-bit timer high word fits"),
                    );
                }
                EspTimerGroupState::LOAD => {
                    let load = state.load_value(timer);
                    state.counters[timer].base_value = load;
                    state.counters[timer].base_time = at;
                    state.counters[timer].latched_value = load;
                    state.set_register(timer, register, 0);
                }
                _ => state.set_register(timer, register, value),
            }
        } else {
            match offset {
                EspTimerGroupState::INTERRUPT_ENABLE => {
                    state.registers[index] = value & 3;
                }
                EspTimerGroupState::INTERRUPT_RAW | EspTimerGroupState::INTERRUPT_STATUS => {}
                EspTimerGroupState::INTERRUPT_CLEAR => {
                    state.registers[EspTimerGroupState::INTERRUPT_RAW / 4] &= !(value & 3);
                    state.registers[index] = 0;
                }
                _ => state.registers[index] = value,
            }
        }

        if offset == 0x68 && value & (1 << 31) != 0 {
            let calibration_cycles = ((value >> 16) & 0x7fff).max(1);
            let measured_xtal_cycles = (40_000_000_u64 * u64::from(calibration_cycles)) / 136_000;
            state.registers[0x68 / 4] |= 1 << 15;
            state.registers[0x6c / 4] =
                (u32::try_from(measured_xtal_cycles).unwrap_or(u32::MAX) & 0x01ff_ffff) << 7;
            state.registers[0x80 / 4] &= !1;
        }
        state.advance(at);
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state
            .lock()
            .expect("ESP timer-group state lock poisoned")
            .reset();
    }
}

/// Functional ESP32-S3 RTC control block with a latched 48-bit time counter.
pub struct EspRtcControl {
    name: String,
    registers: Vec<u32>,
}

impl EspRtcControl {
    /// Creates the RTC control page in its power-on state.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            registers: vec![0; 0x1000 / 4],
        }
    }
}

impl Device for EspRtcControl {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP RTC control requires aligned word access",
            ));
        }
        self.registers
            .get(usize::try_from(offset / 4).expect("RTC offset fits"))
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
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP RTC control requires aligned word access",
            ));
        }
        let index = usize::try_from(offset / 4).expect("RTC offset fits");
        let register = self
            .registers
            .get_mut(index)
            .ok_or_else(|| DeviceError::new(format!("{} write at {offset:#x}", self.name)))?;
        *register = value as u32;
        if offset == 0x0c && value & (1 << 31) != 0 {
            let counter = at.ticks();
            self.registers[0x10 / 4] = counter as u32;
            self.registers[0x14 / 4] = (counter >> 32) as u32;
        }
        // SENS_SAR_MEAS1_CTRL2 shares the RTC peripheral page at 0x800.
        // A software-triggered functional conversion completes immediately.
        // Keep the selected pad/control fields, clear START, assert DONE, and
        // return a deterministic zero sample in the low 16 bits.
        if matches!(offset, 0x80c | 0x830) && value & (1 << 17) != 0 {
            self.registers[index] = (value as u32 & !((1 << 17) | 0xffff)) | (1 << 16);
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.registers.fill(0);
    }
}

#[derive(Default)]
struct EspSystemState {
    registers: Vec<u32>,
    from_cpu_pending: [bool; 4],
}

/// Observation handle for the ESP32-S3 system block's cross-core interrupts.
#[derive(Clone)]
pub struct EspSystemHandle {
    state: Arc<Mutex<EspSystemState>>,
}

impl EspSystemHandle {
    /// Reports whether one FROM_CPU interrupt source is asserted.
    pub fn from_cpu_pending(&self, source: usize) -> bool {
        self.state
            .lock()
            .expect("ESP system state lock poisoned")
            .from_cpu_pending
            .get(source)
            .copied()
            .unwrap_or(false)
    }
}

/// Functional ESP32-S3 system register page.
///
/// Most registers retain software-written configuration. FROM_CPU trigger
/// registers additionally expose their level to the machine interrupt router.
pub struct EspSystem {
    name: String,
    state: Arc<Mutex<EspSystemState>>,
}

impl EspSystem {
    /// Creates the system register page and its interrupt observation handle.
    pub fn new(name: impl Into<String>) -> (Self, EspSystemHandle) {
        let state = Arc::new(Mutex::new(EspSystemState {
            registers: vec![0; 0x1000 / 4],
            from_cpu_pending: [false; 4],
        }));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            EspSystemHandle { state },
        )
    }
}

impl Device for EspSystem {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP system block requires aligned word access",
            ));
        }
        self.state
            .lock()
            .expect("ESP system state lock poisoned")
            .registers
            .get(usize::try_from(offset / 4).expect("system offset fits"))
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
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP system block requires aligned word access",
            ));
        }
        let mut state = self.state.lock().expect("ESP system state lock poisoned");
        let index = usize::try_from(offset / 4).expect("system offset fits");
        let register = state
            .registers
            .get_mut(index)
            .ok_or_else(|| DeviceError::new(format!("{} write at {offset:#x}", self.name)))?;
        *register = value as u32;
        match offset {
            0x30 => state.from_cpu_pending[0] = value & 1 != 0,
            0x34 => state.from_cpu_pending[1] = value & 1 != 0,
            0x38 => state.from_cpu_pending[2] = value & 1 != 0,
            0x3c => state.from_cpu_pending[3] = value & 1 != 0,
            _ => {}
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.lock().expect("ESP system state lock poisoned");
        state.registers.fill(0);
        state.from_cpu_pending = [false; 4];
    }
}

#[derive(Default)]
struct EspMmuTableState {
    registers: Vec<u32>,
    pending: Vec<(usize, u32)>,
}

/// Observation handle for ESP32-S3 cache-MMU entry updates.
#[derive(Clone)]
pub struct EspMmuTableHandle {
    state: Arc<Mutex<EspMmuTableState>>,
}

impl EspMmuTableHandle {
    /// Drains page-table writes in architectural order.
    pub fn drain_mappings(&self) -> Vec<(usize, u32)> {
        let mut state = self.state.lock().expect("ESP MMU state lock poisoned");
        std::mem::take(&mut state.pending)
    }

    /// Establishes one boot-time MMU entry and queues its backing-store map.
    pub fn set_mapping(&self, index: usize, entry: u32) -> Result<(), DeviceError> {
        let mut state = self.state.lock().expect("ESP MMU state lock poisoned");
        let register = state
            .registers
            .get_mut(index)
            .ok_or_else(|| DeviceError::new(format!("ESP MMU entry {index} is out of range")))?;
        *register = entry;
        state.pending.push((index, entry));
        Ok(())
    }
}

/// Functional ESP32-S3 cache-MMU table.
pub struct EspMmuTable {
    name: String,
    state: Arc<Mutex<EspMmuTableState>>,
}

impl EspMmuTable {
    /// Creates the MMU table and its mapping observation handle.
    pub fn new(name: impl Into<String>) -> (Self, EspMmuTableHandle) {
        let state = Arc::new(Mutex::new(EspMmuTableState {
            // ESP32-S3 uses bit 14, rather than the older ESP32 bit-8
            // convention, to mark a cache-MMU entry invalid.
            registers: vec![0x4000; 0x1000 / 4],
            pending: Vec::new(),
        }));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            EspMmuTableHandle { state },
        )
    }
}

impl Device for EspMmuTable {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP MMU table requires aligned word access",
            ));
        }
        self.state
            .lock()
            .expect("ESP MMU state lock poisoned")
            .registers
            .get(usize::try_from(offset / 4).expect("MMU offset fits"))
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
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP MMU table requires aligned word access",
            ));
        }
        let mut state = self.state.lock().expect("ESP MMU state lock poisoned");
        let index = usize::try_from(offset / 4).expect("MMU offset fits");
        let value = value as u32;
        let register = state
            .registers
            .get_mut(index)
            .ok_or_else(|| DeviceError::new(format!("{} write at {offset:#x}", self.name)))?;
        *register = value;
        state.pending.push((index, value));
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.lock().expect("ESP MMU state lock poisoned");
        state.registers.fill(0x4000);
        state.pending.clear();
    }
}

#[derive(Default)]
struct EspSystimerState {
    registers: Vec<u32>,
    latched: [u64; 2],
}

/// Observation and interrupt handle for the ESP32-S3 system timer.
#[derive(Clone)]
pub struct EspSystimerHandle {
    state: Arc<Mutex<EspSystimerState>>,
}

impl EspSystimerHandle {
    /// Advances comparator state and returns enabled target interrupts.
    pub fn pending(&self, now: SimTime) -> [bool; 3] {
        const COUNTER_MASK: u64 = (1_u64 << 52) - 1;
        let mut state = self
            .state
            .lock()
            .expect("ESP system timer state lock poisoned");
        let current = now.ticks() & COUNTER_MASK;
        let config = state.registers[0];
        for target in 0..3 {
            let work_enable = 1_u32 << (24 - target);
            if config & work_enable == 0 {
                continue;
            }
            let high = u64::from(state.registers[(0x1c + target * 8) / 4] & 0x000f_ffff);
            let low = u64::from(state.registers[(0x20 + target * 8) / 4]);
            let compare = ((high << 32) | low) & COUNTER_MASK;
            if current >= compare {
                state.registers[0x68 / 4] |= 1 << target;
                let target_config = state.registers[(0x34 + target * 4) / 4];
                if target_config & (1 << 30) != 0 {
                    let period = u64::from(target_config & 0x03ff_ffff).max(1);
                    let elapsed_periods = current.saturating_sub(compare) / period + 1;
                    let next =
                        compare.wrapping_add(elapsed_periods.saturating_mul(period)) & COUNTER_MASK;
                    state.registers[(0x1c + target * 8) / 4] =
                        u32::try_from(next >> 32).expect("52-bit high word fits");
                    state.registers[(0x20 + target * 8) / 4] = next as u32;
                } else {
                    state.registers[0] &= !work_enable;
                }
            }
        }
        let asserted = state.registers[0x68 / 4] & state.registers[0x64 / 4];
        [asserted & 1 != 0, asserted & 2 != 0, asserted & 4 != 0]
    }
}

/// Functional ESP32-S3 system timer with synchronous counter latching.
pub struct EspSystimer {
    name: String,
    state: Arc<Mutex<EspSystimerState>>,
}

impl EspSystimer {
    /// Creates a continuously running 52-bit system timer.
    pub fn new(name: impl Into<String>) -> (Self, EspSystimerHandle) {
        let mut registers = vec![0; 0x1000 / 4];
        registers[0] = 1 << 30;
        let state = Arc::new(Mutex::new(EspSystimerState {
            registers,
            latched: [0; 2],
        }));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            EspSystimerHandle { state },
        )
    }
}

impl Device for EspSystimer {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP system timer requires aligned word access",
            ));
        }
        let state = self
            .state
            .lock()
            .expect("ESP system timer state lock poisoned");
        let value = match offset {
            0x04 | 0x08 => 1 << 29,
            0x40 => (state.latched[0] >> 32) as u32,
            0x44 => state.latched[0] as u32,
            0x48 => (state.latched[1] >> 32) as u32,
            0x4c => state.latched[1] as u32,
            _ => state
                .registers
                .get(usize::try_from(offset / 4).expect("systimer offset fits"))
                .copied()
                .ok_or_else(|| DeviceError::new(format!("{} read at {offset:#x}", self.name)))?,
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
                "ESP system timer requires aligned word access",
            ));
        }
        let mut state = self
            .state
            .lock()
            .expect("ESP system timer state lock poisoned");
        if matches!(offset, 0x04 | 0x08) {
            let unit = usize::from(offset == 0x08);
            state.latched[unit] = at.ticks() & ((1_u64 << 52) - 1);
            return Ok(());
        }
        if offset == 0x6c {
            state.registers[0x68 / 4] &= !(value as u32 & 0x7);
            return Ok(());
        }
        if matches!(offset, 0x50 | 0x54 | 0x58) && value & 1 != 0 {
            let target = usize::try_from((offset - 0x50) / 4).expect("three targets fit usize");
            let target_config = state.registers[(0x34 + target * 4) / 4];
            let period = u64::from(target_config & 0x03ff_ffff).max(1);
            let compare = at.ticks().wrapping_add(period) & ((1_u64 << 52) - 1);
            state.registers[(0x1c + target * 8) / 4] =
                u32::try_from(compare >> 32).expect("52-bit high word fits");
            state.registers[(0x20 + target * 8) / 4] = compare as u32;
        }
        let register = state
            .registers
            .get_mut(usize::try_from(offset / 4).expect("systimer offset fits"))
            .ok_or_else(|| DeviceError::new(format!("{} write at {offset:#x}", self.name)))?;
        *register = value as u32;
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self
            .state
            .lock()
            .expect("ESP system timer state lock poisoned");
        state.registers.fill(0);
        state.registers[0] = 1 << 30;
        state.latched = [0; 2];
    }
}

/// Functional register slice of the ESP32-S3 Synopsys DWC2 USB OTG core.
///
/// The core reset handshake is synchronous in Renvo's abstract-time model.
/// Endpoint and host-enumeration behavior is layered onto this register file
/// by the machine as qualification reaches the TinyUSB device path.
pub struct EspUsbOtg {
    name: String,
    state: Arc<Mutex<EspUsbOtgState>>,
}

struct EspUsbOtgState {
    registers: Vec<u32>,
    rx_status: VecDeque<u32>,
    rx_fifo: VecDeque<u32>,
    tx_fifo: Vec<Vec<u8>>,
    in_transfer_size: [usize; 16],
    reset_injected: bool,
}

/// Host-side control surface for an ESP32-S3 DWC2 device controller.
#[derive(Clone)]
pub struct EspUsbOtgHandle {
    state: Arc<Mutex<EspUsbOtgState>>,
}

impl EspUsbOtg {
    /// Creates a reset device-mode DWC2 core.
    pub fn new(name: impl Into<String>) -> (Self, EspUsbOtgHandle) {
        let state = Arc::new(Mutex::new(EspUsbOtgState::reset()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            EspUsbOtgHandle { state },
        )
    }
}

impl EspUsbOtgState {
    fn reset() -> Self {
        let mut registers = vec![0; 0x1_0000 / 4];
        // GRSTCTL.AHBIDL and Espressif's fixed DWC2 release identifier.
        registers[0x10 / 4] = 1 << 31;
        registers[0x40 / 4] = 0x4f54_400a;
        // Slave-only full-speed device core, six non-control endpoints and
        // dynamic FIFO sizing. TinyUSB uses NUM_DEV_EP to bound DAINT scans.
        registers[0x48 / 4] = 4 | (1 << 8) | (6 << 10) | (1 << 19);
        // DSTS.ENUMSPD reports full speed on the S3's dedicated 48-MHz PHY.
        registers[0x808 / 4] = 3 << 1;
        // The functional FIFO drains synchronously into the host packet
        // queue, so each IN endpoint always reports the full 1-KiB shared
        // FIFO as available to TinyUSB.
        for endpoint in 0..16 {
            registers[(0x918 + endpoint * 0x20) / 4] = 256;
        }
        Self {
            registers,
            rx_status: VecDeque::new(),
            rx_fifo: VecDeque::new(),
            tx_fifo: vec![Vec::new(); 16],
            in_transfer_size: [0; 16],
            reset_injected: false,
        }
    }

    fn endpoint_interrupts(&self) -> u32 {
        let mut daint = 0_u32;
        let diepmsk = self.registers[0x810 / 4];
        let doepmsk = self.registers[0x814 / 4];
        let fifo_empty_mask = self.registers[0x834 / 4];
        for endpoint in 0..16 {
            let mut input = self.registers[(0x908 + endpoint * 0x20) / 4];
            if fifo_empty_mask & (1 << endpoint) != 0 {
                input |= 1 << 7;
            }
            // TXFE has its own DIEPEMPMSK hierarchy and does not pass
            // through the common DIEPMSK register.
            if input & diepmsk != 0 || fifo_empty_mask & (1 << endpoint) != 0 {
                daint |= 1 << endpoint;
            }
            let output = self.registers[(0xb08 + endpoint * 0x20) / 4];
            if output & doepmsk != 0 {
                daint |= 1 << (16 + endpoint);
            }
        }
        daint
    }

    fn interrupt_status(&self) -> u32 {
        let mut status = self.registers[0x14 / 4];
        if !self.rx_status.is_empty() {
            status |= 1 << 4;
        }
        let endpoints = self.endpoint_interrupts() & self.registers[0x81c / 4];
        if endpoints & 0x0000_ffff != 0 {
            status |= 1 << 18;
        }
        if endpoints & 0xffff_0000 != 0 {
            status |= 1 << 19;
        }
        status
    }

    fn pop_rx_status(&mut self) -> u32 {
        let status = self.rx_status.pop_front().unwrap_or(0);
        let endpoint = usize::try_from(status & 0xf).expect("endpoint number fits");
        match (status >> 17) & 0xf {
            // SETUP_DONE asserts DOEPINT.SETUP after its status entry is popped.
            4 => self.registers[(0xb08 + endpoint * 0x20) / 4] |= 1 << 3,
            // RX_COMPLETE asserts the transfer-complete endpoint interrupt.
            3 => {
                self.registers[(0xb00 + endpoint * 0x20) / 4] &= !(1 << 31);
                self.registers[(0xb08 + endpoint * 0x20) / 4] |= 1;
            }
            _ => {}
        }
        status
    }

    fn write_fifo(&mut self, endpoint: usize, value: u32) {
        let size_index = (0x910 + endpoint * 0x20) / 4;
        let remaining = usize::try_from(self.registers[size_index] & 0x7ffff)
            .expect("DWC2 transfer size fits usize");
        let count = remaining.min(4);
        self.tx_fifo[endpoint].extend_from_slice(&value.to_le_bytes()[..count]);
        self.registers[size_index] =
            (self.registers[size_index] & !0x7ffff) | (remaining - count) as u32;
    }
}

impl EspUsbOtgHandle {
    /// Returns whether TinyUSB has connected the device and enabled interrupts.
    pub fn device_connected(&self) -> bool {
        let state = self.state.lock().expect("ESP USB OTG state lock poisoned");
        state.registers[0x804 / 4] & (1 << 1) == 0 && state.registers[0x08 / 4] & 1 != 0
    }

    /// Injects full-speed bus reset and enumeration-complete conditions once.
    pub fn inject_bus_reset(&self) {
        let mut state = self.state.lock().expect("ESP USB OTG state lock poisoned");
        state.registers[0x14 / 4] |= (1 << 12) | (1 << 13);
        state.reset_injected = true;
    }

    /// Returns whether a globally enabled DWC2 interrupt is asserted.
    pub fn interrupt_pending(&self) -> bool {
        let state = self.state.lock().expect("ESP USB OTG state lock poisoned");
        state.registers[0x08 / 4] & 1 != 0
            && state.interrupt_status() & state.registers[0x18 / 4] != 0
    }

    /// Returns key interrupt registers for deterministic diagnostics.
    pub fn interrupt_diagnostic(&self) -> (u32, u32, u32, u32, u32) {
        let state = self.state.lock().expect("ESP USB OTG state lock poisoned");
        (
            state.registers[0x08 / 4],
            state.interrupt_status(),
            state.registers[0x18 / 4],
            state.endpoint_interrupts(),
            state.registers[0x81c / 4],
        )
    }

    /// Returns endpoint register state for deterministic diagnostics.
    pub fn endpoint_diagnostic(&self, endpoint: u8) -> (u32, u32, u32, u32, u32, u32, u32) {
        let state = self.state.lock().expect("ESP USB OTG state lock poisoned");
        let endpoint = usize::from(endpoint);
        (
            state.registers[(0x900 + endpoint * 0x20) / 4],
            state.registers[(0x908 + endpoint * 0x20) / 4],
            state.registers[(0x910 + endpoint * 0x20) / 4],
            state.registers[(0xb00 + endpoint * 0x20) / 4],
            state.registers[(0xb08 + endpoint * 0x20) / 4],
            state.registers[(0xb10 + endpoint * 0x20) / 4],
            state.registers[0x834 / 4],
        )
    }

    /// Places a host SETUP packet in receive FIFO zero.
    pub fn inject_setup(&self, setup: [u8; 8]) {
        let mut state = self.state.lock().expect("ESP USB OTG state lock poisoned");
        state.rx_fifo.push_back(u32::from_le_bytes(
            setup[0..4].try_into().expect("four bytes"),
        ));
        state.rx_fifo.push_back(u32::from_le_bytes(
            setup[4..8].try_into().expect("four bytes"),
        ));
        state.rx_status.push_back((8 << 4) | (6 << 17));
        state.rx_status.push_back(4 << 17);
    }

    /// Returns whether an IN endpoint has a complete packet ready for the host.
    pub fn input_ready(&self, endpoint: u8) -> bool {
        let state = self.state.lock().expect("ESP USB OTG state lock poisoned");
        let endpoint = usize::from(endpoint);
        let control = state.registers[(0x900 + endpoint * 0x20) / 4];
        let remaining = state.registers[(0x910 + endpoint * 0x20) / 4] & 0x7ffff;
        control & (1 << 31) != 0
            && remaining == 0
            && state.tx_fifo[endpoint].len() >= state.in_transfer_size[endpoint]
    }

    /// Consumes one device-to-host packet and asserts transfer completion.
    pub fn take_input(&self, endpoint: u8) -> Option<Vec<u8>> {
        let mut state = self.state.lock().expect("ESP USB OTG state lock poisoned");
        let endpoint = usize::from(endpoint);
        let control_index = (0x900 + endpoint * 0x20) / 4;
        let size_index = (0x910 + endpoint * 0x20) / 4;
        if state.registers[control_index] & (1 << 31) == 0
            || state.registers[size_index] & 0x7ffff != 0
            || state.tx_fifo[endpoint].len() < state.in_transfer_size[endpoint]
        {
            return None;
        }
        let length = state.in_transfer_size[endpoint];
        let packet = state.tx_fifo[endpoint].drain(..length).collect();
        state.registers[control_index] &= !(1 << 31);
        state.registers[(0x908 + endpoint * 0x20) / 4] |= 1;
        Some(packet)
    }

    /// Returns whether an OUT endpoint is armed to receive host data.
    pub fn output_ready(&self, endpoint: u8) -> bool {
        let state = self.state.lock().expect("ESP USB OTG state lock poisoned");
        state.registers[(0xb00 + usize::from(endpoint) * 0x20) / 4] & (1 << 31) != 0
    }

    /// Returns the number of bytes currently scheduled on an OUT endpoint.
    pub fn output_capacity(&self, endpoint: u8) -> usize {
        let state = self.state.lock().expect("ESP USB OTG state lock poisoned");
        usize::try_from(state.registers[(0xb10 + usize::from(endpoint) * 0x20) / 4] & 0x7ffff)
            .expect("DWC2 transfer size fits usize")
    }

    /// Delivers one host-to-device packet through the shared receive FIFO.
    pub fn inject_output(&self, endpoint: u8, bytes: &[u8]) {
        let mut state = self.state.lock().expect("ESP USB OTG state lock poisoned");
        let endpoint_index = usize::from(endpoint);
        for chunk in bytes.chunks(4) {
            let mut word = [0_u8; 4];
            word[..chunk.len()].copy_from_slice(chunk);
            state.rx_fifo.push_back(u32::from_le_bytes(word));
        }
        let size_index = (0xb10 + endpoint_index * 0x20) / 4;
        let remaining = state.registers[size_index] & 0x7ffff;
        state.registers[size_index] =
            (state.registers[size_index] & !0x7ffff) | remaining.saturating_sub(bytes.len() as u32);
        state
            .rx_status
            .push_back(u32::from(endpoint) | ((bytes.len() as u32) << 4) | (2 << 17));
        state.rx_status.push_back(u32::from(endpoint) | (3 << 17));
    }
}

impl Device for EspUsbOtg {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP USB OTG core requires aligned word access",
            ));
        }
        let mut state = self.state.lock().expect("ESP USB OTG state lock poisoned");
        let value = match offset {
            0x14 => state.interrupt_status(),
            0x1c => state.rx_status.front().copied().unwrap_or(0),
            0x20 => state.pop_rx_status(),
            0x818 => state.endpoint_interrupts(),
            offset if (0x1000..0x1_0000).contains(&offset) => {
                state.rx_fifo.pop_front().unwrap_or(0)
            }
            offset if (0x908..0xb00).contains(&offset) && (offset - 0x908) % 0x20 == 0 => {
                let endpoint =
                    usize::try_from((offset - 0x908) / 0x20).expect("endpoint number fits usize");
                let mut value = state.registers[offset as usize / 4];
                if state.registers[0x834 / 4] & (1 << endpoint) != 0 {
                    value |= 1 << 7;
                }
                value
            }
            _ => state
                .registers
                .get(usize::try_from(offset / 4).expect("USB OTG offset fits"))
                .copied()
                .ok_or_else(|| DeviceError::new(format!("{} read at {offset:#x}", self.name)))?,
        };
        if std::env::var_os("RENVO_DEBUG_USB").is_some()
            && (offset == 0x14
                || offset == 0x818
                || (0x908..0xd00).contains(&offset) && (offset - 0x908) % 0x20 == 0)
        {
            eprintln!("dwc2 reg read {offset:#x} -> {value:#x}");
        }
        Ok(u64::from(value))
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
                "ESP USB OTG core requires aligned word access",
            ));
        }
        let index = usize::try_from(offset / 4).expect("USB OTG offset fits");
        let mut state = self.state.lock().expect("ESP USB OTG state lock poisoned");
        if std::env::var_os("RENVO_DEBUG_USB").is_some()
            && (offset == 0x14
                || offset == 0x818
                || (0x908..0xd00).contains(&offset) && (offset - 0x908) % 0x20 == 0)
        {
            eprintln!("dwc2 reg write {offset:#x} <- {value:#x}");
        }
        if (0x1000..0x1_0000).contains(&offset) {
            let endpoint =
                usize::try_from((offset - 0x1000) / 0x1000).expect("endpoint number fits usize");
            state.write_fifo(endpoint, value as u32);
            return Ok(());
        }
        let register = state
            .registers
            .get_mut(index)
            .ok_or_else(|| DeviceError::new(format!("{} write at {offset:#x}", self.name)))?;
        if offset == 0x04 || offset == 0x14 {
            // GOTGINT and writable GINTSTS causes are write-one-to-clear.
            *register &= !(value as u32);
        } else if offset == 0x10 {
            // CSRST and the FIFO flush strobes self-clear once the functional
            // operation has completed. AHB remains idle for the next access.
            *register = value as u32 & !((1 << 0) | (1 << 4) | (1 << 5));
            *register |= 1 << 31;
            if value & (1 << 4) != 0 {
                state.rx_status.clear();
                state.rx_fifo.clear();
            }
            if value & (1 << 5) != 0 {
                for fifo in &mut state.tx_fifo {
                    fifo.clear();
                }
            }
        } else if offset == 0x804 {
            *register = value as u32;
            if value & (1 << 7) != 0 || value & (1 << 9) != 0 {
                // Global NAK effective is observable synchronously.
                state.registers[0x14 / 4] |= 1 << 7;
            }
            if value & (1 << 8) != 0 || value & (1 << 10) != 0 {
                state.registers[0x14 / 4] &= !(1 << 7);
            }
        } else if (0x908..0xb00).contains(&offset) && (offset - 0x908) % 0x20 == 0
            || (0xb08..0xd00).contains(&offset) && (offset - 0xb08) % 0x20 == 0
        {
            // Endpoint interrupt registers are write-one-to-clear.
            *register &= !(value as u32);
        } else if (0x900..0xb00).contains(&offset) && (offset - 0x900) % 0x20 == 0 {
            let endpoint =
                usize::try_from((offset - 0x900) / 0x20).expect("endpoint number fits usize");
            *register = value as u32;
            if value & (1 << 30) != 0 {
                *register &= !(1 << 31);
                state.registers[(0x908 + endpoint * 0x20) / 4] |= 1 << 1;
            }
            if value & (1 << 31) != 0 {
                let size =
                    usize::try_from(state.registers[(0x910 + endpoint * 0x20) / 4] & 0x7ffff)
                        .expect("DWC2 transfer size fits usize");
                state.in_transfer_size[endpoint] = size;
                state.tx_fifo[endpoint].clear();
            }
        } else if (0xb00..0xd00).contains(&offset) && (offset - 0xb00) % 0x20 == 0 {
            let endpoint =
                usize::try_from((offset - 0xb00) / 0x20).expect("endpoint number fits usize");
            *register = value as u32;
            if value & (1 << 30) != 0 {
                *register &= !(1 << 31);
                state.registers[(0xb08 + endpoint * 0x20) / 4] |= 1 << 1;
            }
        } else {
            *register = value as u32;
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("ESP USB OTG state lock poisoned") = EspUsbOtgState::reset();
    }
}

/// ESP32-C6 analog-register I²C master and its internal byte registers.
///
/// ESP-IDF accesses calibration and regulator state by writing packed
/// slave/address/data commands to the two master control words. Commands
/// complete synchronously in the functional model.
pub struct EspAnalogI2c {
    name: String,
    registers: Vec<u32>,
    analog: BTreeMap<(u8, u8), u8>,
}

impl EspAnalogI2c {
    /// Creates a reset analog I²C master.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            registers: vec![0; 0x1000 / 4],
            analog: BTreeMap::new(),
        }
    }
}

impl Device for EspAnalogI2c {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP analog I2C requires aligned word access",
            ));
        }
        let index = usize::try_from(offset / 4).expect("analog-I2C offset fits");
        self.registers
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
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP analog I2C requires aligned word access",
            ));
        }
        let index = usize::try_from(offset / 4).expect("analog-I2C offset fits");
        let command = value as u32;
        if index >= self.registers.len() {
            return Err(DeviceError::new(format!(
                "{} write at {offset:#x}",
                self.name
            )));
        }
        self.registers[index] = command;

        if matches!(offset, 0x804 | 0x808) {
            let slave = command as u8;
            let address = (command >> 8) as u8;
            if command & (1 << 24) != 0 {
                let data = (command >> 16) as u8;
                self.analog.insert((slave, address), data);
                // A completed BBPLL configuration makes the hardware
                // calibration-done status visible in I2C_MST_ANA_CONF0.
                // Functional time completes the calibration synchronously.
                if slave == 0x66 {
                    self.registers[0x818 / 4] |= 1 << 24;
                }
                // Releasing the ULP analog reset completes the deterministic
                // O-code and band-gap calibration.
                if slave == 0x61 && address == 0 && data & 1 != 0 {
                    self.analog
                        .entry((0x61, 3))
                        .and_modify(|value| *value |= 0x09)
                        .or_insert(0x09);
                }
            } else {
                let data = self.analog.get(&(slave, address)).copied().unwrap_or(0);
                self.registers[index] = (command & !(0xff << 16)) | (u32::from(data) << 16);
            }
            self.registers[index] &= !(1 << 25);
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.registers.fill(0);
        self.analog.clear();
    }
}

/// Functional ESP SPI-memory controller command window.
///
/// User commands complete synchronously. The facade currently exposes the
/// identification/status responses needed to discover a conventional 4 MiB
/// JEDEC flash; memory-mapped application bytes remain owned by the machine's
/// flash mapping.
pub struct EspSpiMem {
    name: String,
    registers: Vec<u32>,
    write_enabled: bool,
}

impl EspSpiMem {
    /// Creates a reset SPI-memory controller.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            registers: vec![0; 0x1000 / 4],
            write_enabled: false,
        }
    }

    fn execute_user_command(&mut self) {
        let command = self.registers[0x20 / 4] as u8;
        let response = match command {
            // RDID: GigaDevice GD25Q32-compatible 4 MiB part. ESP's ROM
            // helper consumes the bytes in this little-endian word order.
            0x9f => 0x0016_40c8,
            // RDSR / RDSR2. Flash is idle; preserve WEL while applicable.
            0x05 => u32::from(self.write_enabled) << 1,
            0x35 => 0,
            // RDSFDP returns an unavailable signature for now, causing IDF
            // to use its JEDEC-ID fallback table deterministically.
            0x5a => 0,
            0x06 => {
                self.write_enabled = true;
                0
            }
            0x04 => {
                self.write_enabled = false;
                0
            }
            _ => 0,
        };
        self.registers[0x58 / 4] = response;
        self.registers[0] &= !(1 << 18);
    }
}

impl Device for EspSpiMem {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width == AccessWidth::DoubleWord || !width.is_aligned(offset) {
            return Err(DeviceError::new(
                "ESP SPI memory controller requires naturally aligned access",
            ));
        }
        let index = usize::try_from(offset / 4).expect("SPI-memory offset fits");
        self.registers
            .get(index)
            .copied()
            .map(|value| {
                let shift = (offset & 3) * 8;
                let mask = match width {
                    AccessWidth::Byte => 0xff,
                    AccessWidth::HalfWord => 0xffff,
                    AccessWidth::Word => u64::from(u32::MAX),
                    AccessWidth::DoubleWord => unreachable!("double-word access rejected"),
                };
                (u64::from(value) >> shift) & mask
            })
            .ok_or_else(|| DeviceError::new(format!("{} read at {offset:#x}", self.name)))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width == AccessWidth::DoubleWord || !width.is_aligned(offset) {
            return Err(DeviceError::new(
                "ESP SPI memory controller requires naturally aligned access",
            ));
        }
        let index = usize::try_from(offset / 4).expect("SPI-memory offset fits");
        let register = self
            .registers
            .get_mut(index)
            .ok_or_else(|| DeviceError::new(format!("{} write at {offset:#x}", self.name)))?;
        let shift = ((offset & 3) * 8) as u32;
        let mask = match width {
            AccessWidth::Byte => 0xff_u32,
            AccessWidth::HalfWord => 0xffff,
            AccessWidth::Word => u32::MAX,
            AccessWidth::DoubleWord => unreachable!("double-word access rejected"),
        } << shift;
        *register = (*register & !mask) | (((value as u32) << shift) & mask);
        if offset & !3 == 0 {
            let command = *register;
            if command & (1 << 30) != 0 {
                self.write_enabled = true;
            }
            if command & (1 << 29) != 0 {
                self.write_enabled = false;
            }
            if command & (1 << 28) != 0 {
                self.registers[0x58 / 4] = 0x0016_40c8;
            }
            if command & (1 << 27) != 0 {
                self.registers[0x58 / 4] = u32::from(self.write_enabled) << 1;
            }
            if command & (1 << 18) != 0 {
                self.execute_user_command();
            }
            // Every operation trigger in CMD[31:17] is self-clearing after
            // the synchronous functional transaction completes.
            self.registers[0] &= 0x0001_ffff;
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.registers.fill(0);
        self.write_enabled = false;
    }
}

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
    pub fn interrupt_enabled(&self, line: u8) -> bool {
        line < 32
            && self
                .state
                .lock()
                .expect("Arm PPB lock poisoned")
                .interrupt_enable
                & (1_u32 << line)
                != 0
    }
}

struct ArmPpbState {
    bytes: Vec<u8>,
    vector_base: u32,
    interrupt_enable: u32,
    interrupt_pending: u32,
}

/// Functional Cortex-M SysTick, NVIC, and SCB register window.
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
            interrupt_enable: 0,
            interrupt_pending: 0,
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

    fn read_word(state: &ArmPpbState, offset: usize) -> u32 {
        u32::from_le_bytes(
            state.bytes[offset..offset + 4]
                .try_into()
                .expect("PPB word range validated"),
        )
    }

    fn write_word(state: &mut ArmPpbState, offset: usize, value: u32) {
        state.bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
}

impl Device for ArmPrivatePeripheralBus {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
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
        match offset {
            0x100 => {
                let value = state.interrupt_enable;
                Self::write_word(&mut state, 0x100, value);
            }
            0x180 => {
                let value = state.interrupt_enable;
                Self::write_word(&mut state, 0x180, value);
            }
            0x200 => {
                let value = state.interrupt_pending;
                Self::write_word(&mut state, 0x200, value);
            }
            0x280 => {
                let value = state.interrupt_pending;
                Self::write_word(&mut state, 0x280, value);
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
        _at: SimTime,
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
        let word = u32::try_from(value & u64::from(u32::MAX)).expect("masked PPB value fits");
        match (offset, width) {
            (0x100, AccessWidth::Word) => state.interrupt_enable |= word,
            (0x180, AccessWidth::Word) => state.interrupt_enable &= !word,
            (0x200, AccessWidth::Word) => state.interrupt_pending |= word,
            (0x280, AccessWidth::Word) => state.interrupt_pending &= !word,
            (0xd08, AccessWidth::Word) => {
                state.vector_base = word & !0x7f;
                let value = state.vector_base;
                Self::write_word(&mut state, offset, value);
            }
            (0x18, AccessWidth::Word) => Self::write_word(&mut state, offset, 0),
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
        state.interrupt_enable = 0;
        state.interrupt_pending = 0;
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
    fn raw_interrupts(&self) -> u32 {
        let sie_status = self.registers[0x50 / 4];
        let mut interrupts = 0;
        if self.registers[0x58 / 4] != 0 {
            interrupts |= 1 << 4;
        }
        if sie_status & (1 << 19) != 0 {
            interrupts |= 1 << 12;
        }
        if sie_status & (1 << 0) != 0 {
            interrupts |= 1 << 11;
        }
        if sie_status & (1 << 17) != 0 {
            interrupts |= 1 << 16;
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
        let register = state.registers.get_mut(index).ok_or_else(|| {
            DeviceError::new(format!(
                "unmodeled RP2040 USB write at offset {register_offset:#x}"
            ))
        })?;
        let value =
            u32::try_from(value & u64::from(u32::MAX)).expect("masked USB register value fits");
        Self::update(register, alias, value)?;
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
    multicore: Arc<Mutex<RpSioMulticoreState>>,
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
    state: Arc<Mutex<RpSioMulticoreState>>,
}

impl RpSioHandle {
    /// Selects which processor owns subsequent accesses to the shared SIO
    /// device. Machines call this immediately before stepping that processor.
    pub fn select_core(&self, core: u8) {
        self.state
            .lock()
            .expect("RP SIO multicore lock poisoned")
            .selected_core = core.min(1);
    }

    /// Takes a core-1 launch that completed the documented six-word ROM
    /// handshake.
    pub fn take_core1_launch(&self) -> Option<RpCoreLaunch> {
        self.state
            .lock()
            .expect("RP SIO multicore lock poisoned")
            .pending_launch
            .take()
    }

    /// Returns whether core 1 has left its boot-ROM launch protocol.
    pub fn core1_launched(&self) -> bool {
        self.state
            .lock()
            .expect("RP SIO multicore lock poisoned")
            .core1_launched
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
        let multicore = Arc::new(Mutex::new(RpSioMulticoreState::default()));
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
            let mut state = self
                .multicore
                .lock()
                .expect("RP SIO multicore lock poisoned");
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
            let mut state = self
                .multicore
                .lock()
                .expect("RP SIO multicore lock poisoned");
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
        *self
            .multicore
            .lock()
            .expect("RP SIO multicore lock poisoned") = RpSioMulticoreState::default();
        self.spinlocks = u32::MAX;
        self.dividend = 0;
        self.quotient = 0;
        self.remainder = 0;
        self.divider_dirty = false;
    }
}

/// ESP32 GPIO matrix output/enable register slice for pins 0 through 31.
pub struct EspGpio {
    name: String,
    pins: u8,
    state: Arc<Mutex<GpioState>>,
    signals: Vec<SignalId>,
    hub: SignalHub,
}

impl EspGpio {
    /// Creates the low GPIO bank and an external-stimulus handle.
    pub fn new(
        name: impl Into<String>,
        pins: u8,
        path: &str,
        hub: SignalHub,
    ) -> Result<(Self, GpioHandle), SignalError> {
        let (state, signals, handle) = vendor_gpio(pins, path, &hub)?;
        Ok((
            Self {
                name: name.into(),
                pins,
                state,
                signals,
                hub,
            },
            handle,
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

impl Device for EspGpio {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("ESP GPIO requires word access"));
        }
        let state = self.state.lock().expect("GPIO lock poisoned");
        let value = match offset {
            0x04 | 0x08 | 0x0c => state.output,
            0x20 | 0x24 | 0x28 => state.direction,
            0x3c => {
                drop(state);
                return Ok(u64::from(self.resolved_input()));
            }
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
            return Err(DeviceError::new("ESP GPIO requires word access"));
        }
        let value =
            u32::try_from(value & u64::from(u32::MAX)).expect("masked value always fits u32");
        let mut state = self.state.lock().expect("GPIO lock poisoned");
        match offset {
            0x04 => state.output = value,
            0x08 => state.output |= value,
            0x0c => state.output &= !value,
            0x20 => state.direction = value,
            0x24 => state.direction |= value,
            0x28 => state.direction &= !value,
            _ => return Ok(()),
        }
        drop(state);
        refresh_gpio(&self.state, &self.signals, &self.hub, self.pins, at)
    }
}

struct TimerState {
    enabled: bool,
    periodic: bool,
    compare: u64,
    period: u64,
    pending: bool,
}

/// Host/machine-facing timer state.
#[derive(Clone)]
pub struct TimerHandle {
    state: Arc<Mutex<TimerState>>,
}

impl TimerHandle {
    /// Updates pending state at the current simulation time.
    pub fn poll(&self, now: SimTime) -> bool {
        let mut state = self.state.lock().expect("timer lock poisoned");
        if state.enabled && now.ticks() >= state.compare {
            state.pending = true;
            if state.periodic && state.period != 0 {
                while state.compare <= now.ticks() {
                    state.compare = state.compare.saturating_add(state.period);
                }
            } else {
                state.enabled = false;
            }
        }
        state.pending
    }

    /// Clears the interrupt pending latch.
    pub fn clear(&self) {
        self.state.lock().expect("timer lock poisoned").pending = false;
    }

    /// Current pending state.
    pub fn pending(&self) -> bool {
        self.state.lock().expect("timer lock poisoned").pending
    }
}

/// Functional timer with counter, compare, control, period, and status words.
pub struct FunctionalTimer {
    name: String,
    state: Arc<Mutex<TimerState>>,
}

impl FunctionalTimer {
    /// Counter offset.
    pub const COUNTER: u64 = 0x00;
    /// Compare offset.
    pub const COMPARE: u64 = 0x08;
    /// Control offset: bit 0 enable, bit 1 periodic.
    pub const CONTROL: u64 = 0x10;
    /// Period offset.
    pub const PERIOD: u64 = 0x18;
    /// Status offset: bit 0 pending; write bit 0 to clear.
    pub const STATUS: u64 = 0x20;

    /// Creates a stopped timer and machine handle.
    pub fn new(name: impl Into<String>) -> (Self, TimerHandle) {
        let state = Arc::new(Mutex::new(TimerState {
            enabled: false,
            periodic: false,
            compare: u64::MAX,
            period: 0,
            pending: false,
        }));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            TimerHandle { state },
        )
    }
}

impl Device for FunctionalTimer {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if !matches!(width, AccessWidth::Word | AccessWidth::DoubleWord) {
            return Err(DeviceError::new(
                "timer requires word or double-word access",
            ));
        }
        let state = self.state.lock().expect("timer lock poisoned");
        match offset {
            Self::COUNTER => Ok(at.ticks()),
            Self::COMPARE => Ok(state.compare),
            Self::CONTROL => Ok(u64::from(state.enabled) | (u64::from(state.periodic) << 1)),
            Self::PERIOD => Ok(state.period),
            Self::STATUS => Ok(u64::from(state.pending)),
            _ => Err(DeviceError::new(format!(
                "unmodeled timer read at offset {offset:#x}"
            ))),
        }
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if !matches!(width, AccessWidth::Word | AccessWidth::DoubleWord) {
            return Err(DeviceError::new(
                "timer requires word or double-word access",
            ));
        }
        let mut state = self.state.lock().expect("timer lock poisoned");
        match offset {
            Self::COMPARE => state.compare = value,
            Self::CONTROL => {
                state.enabled = value & 1 != 0;
                state.periodic = value & 2 != 0;
            }
            Self::PERIOD => state.period = value,
            Self::STATUS if value & 1 != 0 => state.pending = false,
            Self::STATUS => {}
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled timer write at offset {offset:#x}"
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.lock().expect("timer lock poisoned");
        state.enabled = false;
        state.periodic = false;
        state.compare = u64::MAX;
        state.period = 0;
        state.pending = false;
    }
}

/// Host handle for a deterministic exit convention.
#[derive(Clone, Default)]
pub struct ExitHandle {
    code: Arc<Mutex<Option<u32>>>,
}

impl ExitHandle {
    /// Returns a requested exit code.
    pub fn code(&self) -> Option<u32> {
        *self.code.lock().expect("exit device lock poisoned")
    }
}

/// Write-only MMIO exit device.
pub struct ExitDevice {
    name: String,
    handle: ExitHandle,
}

impl ExitDevice {
    /// Creates an exit device and observation handle.
    pub fn new(name: impl Into<String>) -> (Self, ExitHandle) {
        let handle = ExitHandle::default();
        (
            Self {
                name: name.into(),
                handle: handle.clone(),
            },
            handle,
        )
    }
}

impl Device for ExitDevice {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(
        &mut self,
        _offset: u64,
        _width: AccessWidth,
        _at: SimTime,
    ) -> Result<u64, DeviceError> {
        Ok(self.handle.code().map_or(0, u64::from))
    }

    fn write(
        &mut self,
        offset: u64,
        _width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if offset != 0 {
            return Err(DeviceError::new("exit device only implements offset zero"));
        }
        *self.handle.code.lock().expect("exit device lock poisoned") =
            Some(u32::try_from(value & u64::from(u32::MAX)).expect("masked value fits in u32"));
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.handle.code.lock().expect("exit device lock poisoned") = None;
    }
}

/// Sparse functional register bank for clock/reset facades and documented reset values.
pub struct RegisterBank {
    name: String,
    reset: BTreeMap<u64, u32>,
    values: BTreeMap<u64, u32>,
    writable_masks: BTreeMap<u64, u32>,
}

impl RegisterBank {
    /// Constructs a bank from `(offset, reset_value, writable_mask)` entries.
    pub fn new(
        name: impl Into<String>,
        registers: impl IntoIterator<Item = (u64, u32, u32)>,
    ) -> Self {
        let mut reset = BTreeMap::new();
        let mut writable_masks = BTreeMap::new();
        for (offset, value, mask) in registers {
            reset.insert(offset, value);
            writable_masks.insert(offset, mask);
        }
        Self {
            name: name.into(),
            values: reset.clone(),
            reset,
            writable_masks,
        }
    }
}

impl Device for RegisterBank {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("register bank requires word access"));
        }
        self.values
            .get(&offset)
            .copied()
            .map(u64::from)
            .ok_or_else(|| {
                DeviceError::new(format!("unmodeled register read at offset {offset:#x}"))
            })
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("register bank requires word access"));
        }
        let current = self.values.get_mut(&offset).ok_or_else(|| {
            DeviceError::new(format!("unmodeled register write at offset {offset:#x}"))
        })?;
        let mask = self.writable_masks[&offset];
        let value =
            u32::try_from(value & u64::from(u32::MAX)).expect("masked value always fits in u32");
        *current = (*current & !mask) | (value & mask);
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.values.clone_from(&self.reset);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpio_emits_resolved_changes_and_contention() {
        let hub = SignalHub::new();
        let (mut gpio, handle) =
            FunctionalGpio::new("gpio", 2, "board.gpio", hub.clone(), 0, 4, 8).unwrap();
        gpio.write(0, AccessWidth::Word, 1, SimTime::ZERO).unwrap();
        gpio.write(4, AccessWidth::Word, 1, SimTime::from_ticks(1))
            .unwrap();
        handle
            .set_input(0, Logic::Zero, SimTime::from_ticks(2))
            .unwrap();
        let changes = hub.drain_changes();
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[0].value.bit(0), Some(Logic::Zero));
        assert_eq!(changes[1].value.bit(0), Some(Logic::One));
        assert_eq!(changes[2].value.bit(0), Some(Logic::X));
    }

    #[test]
    fn timer_latches_and_clears_interrupt() {
        let (mut timer, handle) = FunctionalTimer::new("timer");
        timer
            .write(
                FunctionalTimer::COMPARE,
                AccessWidth::DoubleWord,
                10,
                SimTime::ZERO,
            )
            .unwrap();
        timer
            .write(
                FunctionalTimer::CONTROL,
                AccessWidth::Word,
                1,
                SimTime::ZERO,
            )
            .unwrap();
        assert!(!handle.poll(SimTime::from_ticks(9)));
        assert!(handle.poll(SimTime::from_ticks(10)));
        timer
            .write(
                FunctionalTimer::STATUS,
                AccessWidth::Word,
                1,
                SimTime::from_ticks(10),
            )
            .unwrap();
        assert!(!handle.pending());
    }

    #[test]
    fn rp_timer_interrupt_aliases_accumulate_and_clear_bits() {
        let (mut timer, handle) = Rp2040Timer::new("timer", RpTimerLayout::Rp2040);
        timer
            .write(0x2038, AccessWidth::Word, 0x8, SimTime::ZERO)
            .unwrap();
        timer
            .write(0x2038, AccessWidth::Word, 0x4, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            timer.read(0x38, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0xc
        );

        timer
            .write(0x203c, AccessWidth::Word, 0x8, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.pending(SimTime::ZERO), 0x8);
        timer
            .write(0x303c, AccessWidth::Word, 0x8, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.pending(SimTime::ZERO), 0);
    }

    #[test]
    fn rp2350_timer_uses_shifted_interrupt_registers() {
        let (mut timer, handle) = Rp2040Timer::new("timer", RpTimerLayout::Rp2350);
        timer
            .write(0x2040, AccessWidth::Word, 0x8, SimTime::ZERO)
            .unwrap();
        timer
            .write(0x2040, AccessWidth::Word, 0x4, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            timer.read(0x40, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0xc
        );

        timer
            .write(0x1c, AccessWidth::Word, 10, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.pending(SimTime::from_ticks(10)), 0x8);
        timer
            .write(0x3c, AccessWidth::Word, 0x8, SimTime::from_ticks(10))
            .unwrap();
        assert_eq!(handle.pending(SimTime::from_ticks(10)), 0);

        timer
            .write(0x2044, AccessWidth::Word, 0x8, SimTime::from_ticks(10))
            .unwrap();
        assert_eq!(handle.pending(SimTime::from_ticks(10)), 0x8);
        timer
            .write(0x3044, AccessWidth::Word, 0x8, SimTime::from_ticks(10))
            .unwrap();
        assert_eq!(handle.pending(SimTime::from_ticks(10)), 0);
    }

    #[test]
    fn esp_timer_group_schedules_and_clears_alarm_interrupts() {
        let (mut group, handle) = EspTimerGroup::new("timer-group", EspTimerGroupKind::Esp32C6);
        group
            .write(0x18, AccessWidth::Word, 0, SimTime::ZERO)
            .unwrap();
        group
            .write(0x1c, AccessWidth::Word, 0, SimTime::ZERO)
            .unwrap();
        group
            .write(0x20, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        group
            .write(0x10, AccessWidth::Word, 100, SimTime::ZERO)
            .unwrap();
        group
            .write(0x14, AccessWidth::Word, 0, SimTime::ZERO)
            .unwrap();
        group
            .write(0x70, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        let config = (1 << 31) | (1 << 30) | (1 << 29) | (1 << 10) | (8 << 13);
        group
            .write(0, AccessWidth::Word, config, SimTime::ZERO)
            .unwrap();

        assert_eq!(handle.pending(SimTime::from_ticks(99)), [false, false]);
        assert_eq!(handle.pending(SimTime::from_ticks(100)), [true, false]);
        group
            .write(0x7c, AccessWidth::Word, 1, SimTime::from_ticks(100))
            .unwrap();
        assert_eq!(handle.pending(SimTime::from_ticks(100)), [false, false]);

        group
            .write(0, AccessWidth::Word, config, SimTime::from_ticks(100))
            .unwrap();
        assert_eq!(handle.pending(SimTime::from_ticks(200)), [true, false]);
    }

    #[test]
    fn esp32s3_timer_group_exposes_second_timer_interrupt() {
        let (mut group, handle) = EspTimerGroup::new("timer-group", EspTimerGroupKind::Esp32S3);
        group
            .write(0x3c, AccessWidth::Word, 0, SimTime::ZERO)
            .unwrap();
        group
            .write(0x40, AccessWidth::Word, 0, SimTime::ZERO)
            .unwrap();
        group
            .write(0x44, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        group
            .write(0x34, AccessWidth::Word, 20, SimTime::ZERO)
            .unwrap();
        group
            .write(0x38, AccessWidth::Word, 0, SimTime::ZERO)
            .unwrap();
        group
            .write(0x70, AccessWidth::Word, 2, SimTime::ZERO)
            .unwrap();
        group
            .write(
                0x24,
                AccessWidth::Word,
                (1 << 31) | (1 << 30) | (1 << 10) | (8 << 13),
                SimTime::ZERO,
            )
            .unwrap();

        assert_eq!(handle.pending(SimTime::from_ticks(20)), [false, true]);
    }

    #[test]
    fn uart_captures_low_byte() {
        let (mut uart, handle) = FunctionalUart::new("uart", 0, 4, 1);
        uart.write(0, AccessWidth::Word, b'A'.into(), SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.text_lossy(), "A");
    }

    #[test]
    fn esp_usb_serial_jtag_moves_deterministic_host_packets() {
        let (mut usb, handle) = EspUsbSerialJtag::new("usb-serial-jtag");
        handle.queue_input(b"x\x04");
        usb.write(0x10, AccessWidth::Word, 1 << 2, SimTime::ZERO)
            .unwrap();
        assert!(handle.interrupt_pending());
        assert_eq!(
            usb.read(0x04, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0b110
        );
        assert_eq!(
            usb.read(0x00, AccessWidth::Word, SimTime::ZERO).unwrap(),
            u64::from(b'x')
        );
        assert_eq!(
            usb.read(0x00, AccessWidth::Word, SimTime::ZERO).unwrap(),
            u64::from(0x04_u8)
        );
        assert_eq!(
            usb.read(0x04, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0b010
        );

        for byte in b"hello" {
            usb.write(0, AccessWidth::Word, u64::from(*byte), SimTime::ZERO)
                .unwrap();
        }
        assert!(handle.output().is_empty());
        usb.write(4, AccessWidth::Word, 1, SimTime::ZERO).unwrap();
        assert_eq!(handle.output(), b"hello");
        assert!(!handle.input_complete());

        for byte in b"\x04\x04>" {
            usb.write(0, AccessWidth::Word, u64::from(*byte), SimTime::ZERO)
                .unwrap();
        }
        usb.write(4, AccessWidth::Word, 1, SimTime::ZERO).unwrap();
        assert!(handle.input_complete());
    }

    #[test]
    fn vendor_gpio_set_clear_registers_drive_signals() {
        let hub = SignalHub::new();
        let (mut sio, handle) = RpSioGpio::new("sio", 4, "board.rp.gpio", hub.clone()).unwrap();
        sio.write(0x024, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        sio.write(0x014, AccessWidth::Word, 1, SimTime::from_ticks(1))
            .unwrap();
        assert_eq!(handle.direction(), 1);
        assert_eq!(handle.output(), 1);
        let changes = hub.drain_changes();
        assert_eq!(changes.last().unwrap().value.bit(0), Some(Logic::One));
    }

    #[test]
    fn rp2350_sio_uses_interleaved_low_and_high_gpio_registers() {
        let hub = SignalHub::new();
        let (mut sio, handle) =
            RpSioGpio::new_rp2350("sio", 4, "board.rp2350.gpio", hub.clone()).unwrap();
        sio.write(0x038, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        sio.write(0x018, AccessWidth::Word, 1, SimTime::from_ticks(1))
            .unwrap();
        assert_eq!(handle.direction(), 1);
        assert_eq!(handle.output(), 1);
        assert_eq!(
            sio.read(0x030, AccessWidth::Word, SimTime::ZERO).unwrap(),
            1
        );
        assert_eq!(
            sio.read(0x010, AccessWidth::Word, SimTime::ZERO).unwrap(),
            1
        );
        sio.write(0x020, AccessWidth::Word, 1, SimTime::from_ticks(2))
            .unwrap();
        assert_eq!(handle.output(), 0);
        assert_eq!(
            hub.drain_changes().last().unwrap().value.bit(0),
            Some(Logic::Zero)
        );
    }

    #[test]
    fn rp_sio_echoes_bootrom_launch_and_routes_live_fifo_words() {
        let hub = SignalHub::new();
        let (mut sio, _, multicore) =
            RpSioGpio::new_with_multicore("sio", 4, "board.rp.gpio", hub).unwrap();

        // Initial core-1 ROM ready acknowledgement.
        assert_eq!(
            sio.read(0x050, AccessWidth::Word, SimTime::ZERO).unwrap() & 1,
            1
        );
        assert_eq!(
            sio.read(0x058, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0
        );

        let sequence: [u32; 6] = [0, 0, 1, 0x1000_0000, 0x2004_0000, 0x1000_0101];
        for word in sequence {
            sio.write(0x054, AccessWidth::Word, u64::from(word), SimTime::ZERO)
                .unwrap();
            assert_eq!(
                sio.read(0x058, AccessWidth::Word, SimTime::ZERO).unwrap(),
                u64::from(word)
            );
        }
        assert_eq!(
            multicore.take_core1_launch(),
            Some(RpCoreLaunch {
                vector_table: 0x1000_0000,
                stack_pointer: 0x2004_0000,
                entry: 0x1000_0101,
            })
        );

        multicore.select_core(1);
        assert_eq!(sio.read(0, AccessWidth::Word, SimTime::ZERO).unwrap(), 1);
        sio.write(0x054, AccessWidth::Word, 0xfeed_beef, SimTime::ZERO)
            .unwrap();
        multicore.select_core(0);
        assert_eq!(
            sio.read(0x058, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0xfeed_beef
        );
    }

    #[test]
    fn rp2040_resets_supports_set_and_clear_aliases() {
        let mut resets = Rp2040Resets::new("resets");
        resets
            .write(0x2000, AccessWidth::Word, 0x21, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            resets.read(0, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0x21
        );
        assert_eq!(
            resets.read(8, AccessWidth::Word, SimTime::ZERO).unwrap(),
            u64::from(Rp2040Resets::VALID_MASK & !0x21)
        );
        resets
            .write(0x3000, AccessWidth::Word, 0x20, SimTime::ZERO)
            .unwrap();
        assert_eq!(resets.read(0, AccessWidth::Word, SimTime::ZERO).unwrap(), 1);
    }

    #[test]
    fn deterministic_rng_changes_words_and_restarts_from_its_seed() {
        let mut rng = DeterministicRng::new("rng", 0x7c, 0x1234_5678);
        let first = rng.read(0x7c, AccessWidth::Word, SimTime::ZERO).unwrap();
        let second = rng.read(0x7c, AccessWidth::Word, SimTime::ZERO).unwrap();
        assert_ne!(first, second);
        rng.reset(ResetKind::PowerOn);
        assert_eq!(
            rng.read(0x7c, AccessWidth::Word, SimTime::ZERO).unwrap(),
            first
        );
    }

    #[test]
    fn rp2040_usb_exposes_vbus_and_atomic_aliases() {
        let mut usb = Rp2040UsbController::new("usb");
        assert_eq!(
            usb.read(0x50, AccessWidth::Word, SimTime::ZERO).unwrap() & 1,
            1
        );
        usb.write(0x204c, AccessWidth::Word, 0x10, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            usb.read(0x4c, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0x10
        );
    }

    #[test]
    fn rp_sio_spinlocks_claim_on_read_and_release_on_write() {
        let hub = SignalHub::new();
        let (mut sio, _) = RpSioGpio::new("sio", 4, "board.rp.gpio", hub).unwrap();
        assert_eq!(
            sio.read(0x12c, AccessWidth::Word, SimTime::ZERO).unwrap(),
            1 << 11
        );
        assert_eq!(
            sio.read(0x12c, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0
        );
        sio.write(0x12c, AccessWidth::Word, 0, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            sio.read(0x12c, AccessWidth::Word, SimTime::ZERO).unwrap(),
            1 << 11
        );
    }
}
