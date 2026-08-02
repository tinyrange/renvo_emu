use super::*;
use remu_signals::{SignalId, SignalValue};

const CNTR: u64 = 0x40;
const ISTR: u64 = 0x44;
const FNR: u64 = 0x48;
const DADDR: u64 = 0x4c;
const BTABLE: u64 = 0x50;
const CORE_SIZE: u64 = 0x400;
const PMA_SIZE: usize = 0x400;

const CNTR_SOFM: u16 = 1 << 8;
const CNTR_ESOFM: u16 = 1 << 9;
const CNTR_RESETM: u16 = 1 << 10;
const CNTR_SUSPM: u16 = 1 << 11;
const CNTR_WKUPM: u16 = 1 << 12;
const CNTR_ERRM: u16 = 1 << 13;
const CNTR_CTRM: u16 = 1 << 15;
const CNTR_MASK: u16 =
    CNTR_SOFM | CNTR_ESOFM | CNTR_RESETM | CNTR_SUSPM | CNTR_WKUPM | CNTR_ERRM | CNTR_CTRM;
const ISTR_CTR: u16 = 1 << 15;
const ISTR_DIR: u16 = 1 << 4;
const ISTR_RESET: u16 = 1;

const EP_CTR_RX: u16 = 1 << 15;
const EP_DTOG_RX: u16 = 1 << 14;
const EP_STAT_RX: u16 = 3 << 12;
const EP_SETUP: u16 = 1 << 11;
const EP_TYPE: u16 = 3 << 9;
const EP_CTR_TX: u16 = 1 << 7;
const EP_DTOG_TX: u16 = 1 << 6;
const EP_STAT_TX: u16 = 3 << 4;
const EP_ADDRESS: u16 = 0x0f;

#[derive(Clone)]
struct UsbFsState {
    cntr: u16,
    istr: u16,
    fnr: u16,
    daddr: u16,
    btable: u16,
    endpoints: [u16; 8],
    pma: Vec<u8>,
}

impl Default for UsbFsState {
    fn default() -> Self {
        Self {
            cntr: 0,
            istr: 0,
            fnr: 0,
            daddr: 0,
            btable: 0,
            endpoints: [0; 8],
            pma: vec![0; PMA_SIZE],
        }
    }
}

/// Host-facing STM32 USB FS device-controller handle.
#[derive(Clone)]
pub struct Stm32UsbFsHandle {
    state: Arc<Mutex<UsbFsState>>,
    hub: SignalHub,
    interrupt_signal: SignalId,
    reset_signal: SignalId,
    out_signal: SignalId,
}

impl Stm32UsbFsHandle {
    /// Advances the functional USB controller and reports an enabled request.
    pub fn poll(&self, now: SimTime) -> bool {
        let (pending, reset, out) = {
            let state = self.state.lock().expect("STM32 USB FS lock poisoned");
            (
                Self::interrupt_pending(&state),
                state.istr & ISTR_RESET != 0,
                state.istr & ISTR_CTR != 0 && state.istr & ISTR_DIR != 0,
            )
        };
        self.publish(pending, reset, out, now);
        pending
    }

    /// Injects a USB bus reset and clears the device address and endpoints.
    pub fn bus_reset(&self, at: SimTime) {
        let mut state = self.state.lock().expect("STM32 USB FS lock poisoned");
        state.daddr = 0;
        state.endpoints = [0; 8];
        state.istr |= ISTR_RESET;
        drop(state);
        self.publish(true, true, false, at);
    }

    /// Injects an OUT packet into an endpoint's PMA receive buffer.
    pub fn inject_out(&self, endpoint: u8, data: &[u8], at: SimTime) -> Result<(), String> {
        let index = usize::from(endpoint);
        if index >= 8 || data.len() > 64 {
            return Err("USB endpoint or packet length is out of range".to_owned());
        }
        let mut state = self.state.lock().expect("STM32 USB FS lock poisoned");
        let address = Self::buffer_address(&state, index, false);
        let end = address.saturating_add(data.len());
        if end > state.pma.len() {
            return Err("USB PMA receive buffer is out of range".to_owned());
        }
        state.pma[address..end].copy_from_slice(data);
        Self::write_descriptor_count(&mut state, index, false, data.len());
        state.endpoints[index] |= EP_CTR_RX;
        if data.len() >= 8 && index == 0 {
            state.endpoints[index] |= EP_SETUP;
        }
        state.istr |= ISTR_CTR | ISTR_DIR;
        drop(state);
        self.publish(true, false, true, at);
        Ok(())
    }

    /// Returns an IN packet from an endpoint's PMA transmit buffer.
    pub fn take_in(&self, endpoint: u8, at: SimTime) -> Result<Vec<u8>, String> {
        let index = usize::from(endpoint);
        if index >= 8 {
            return Err("USB endpoint is out of range".to_owned());
        }
        let mut state = self.state.lock().expect("STM32 USB FS lock poisoned");
        let address = Self::buffer_address(&state, index, true);
        let length = Self::descriptor_count(&state, index, true).min(64);
        let end = address.saturating_add(length);
        if end > state.pma.len() {
            return Err("USB PMA transmit buffer is out of range".to_owned());
        }
        let packet = state.pma[address..end].to_vec();
        state.endpoints[index] &= !EP_CTR_TX;
        if state
            .endpoints
            .iter()
            .all(|value| value & (EP_CTR_RX | EP_CTR_TX) == 0)
        {
            state.istr &= !ISTR_CTR;
        }
        drop(state);
        self.poll(at);
        Ok(packet)
    }

    /// Returns the current USB device address, excluding the enable bit.
    pub fn device_address(&self) -> u8 {
        let state = self.state.lock().expect("STM32 USB FS lock poisoned");
        (state.daddr & 0x7f) as u8
    }

    fn descriptor_base(state: &UsbFsState, endpoint: usize) -> usize {
        usize::from(state.btable).saturating_add(endpoint.saturating_mul(8))
    }

    fn descriptor_count(state: &UsbFsState, endpoint: usize, tx: bool) -> usize {
        let base = Self::descriptor_base(state, endpoint);
        let offset = base.saturating_add(if tx { 2 } else { 6 });
        if offset + 2 > state.pma.len() {
            return 0;
        }
        usize::from(u16::from_le_bytes([
            state.pma[offset],
            state.pma[offset + 1],
        ]))
    }

    fn buffer_address(state: &UsbFsState, endpoint: usize, tx: bool) -> usize {
        let base = Self::descriptor_base(state, endpoint);
        if base + 2 <= state.pma.len() {
            let offset = base + if tx { 0 } else { 4 };
            if offset + 2 <= state.pma.len() {
                let address = usize::from(u16::from_le_bytes([
                    state.pma[offset],
                    state.pma[offset + 1],
                ]));
                if address != 0 {
                    return address;
                }
            }
        }
        (if tx { 0x20_usize } else { 0x120_usize }).saturating_add(endpoint.saturating_mul(64))
    }

    fn write_descriptor_count(state: &mut UsbFsState, endpoint: usize, tx: bool, length: usize) {
        let base = Self::descriptor_base(state, endpoint);
        let offset = base.saturating_add(if tx { 2 } else { 6 });
        if offset + 2 <= state.pma.len() {
            let value = u16::try_from(length).unwrap_or(u16::MAX).to_le_bytes();
            state.pma[offset..offset + 2].copy_from_slice(&value);
        }
    }

    fn interrupt_pending(state: &UsbFsState) -> bool {
        (state.istr & ISTR_CTR != 0 && state.cntr & CNTR_CTRM != 0)
            || (state.istr & ISTR_RESET != 0 && state.cntr & CNTR_RESETM != 0)
    }

    fn publish(&self, pending: bool, reset: bool, out: bool, at: SimTime) {
        for (signal, value, description) in [
            (
                self.interrupt_signal,
                pending,
                "USB FS interrupt signal width is valid",
            ),
            (
                self.reset_signal,
                reset,
                "USB FS reset signal width is valid",
            ),
            (self.out_signal, out, "USB FS OUT signal width is valid"),
        ] {
            self.hub
                .set(
                    signal,
                    SignalValue::from_u64(u64::from(value), 1).expect(description),
                    at,
                )
                .expect("USB FS signal is declared");
        }
    }
}

/// STM32 USB FS device-core registers and endpoint status.
pub struct Stm32UsbFs {
    name: String,
    state: Arc<Mutex<UsbFsState>>,
    handle: Stm32UsbFsHandle,
}

/// STM32 USB packet-memory-area device.
pub struct Stm32UsbPma {
    name: String,
    state: Arc<Mutex<UsbFsState>>,
}

impl Stm32UsbFs {
    /// Creates the USB FS core, PMA window, and host-facing handle.
    pub fn new(
        name: impl Into<String>,
        hub: SignalHub,
    ) -> Result<(Self, Stm32UsbPma, Stm32UsbFsHandle), remu_signals::SignalError> {
        let name = name.into();
        let interrupt_signal = hub.declare(
            format!("{name}.irq"),
            SignalValue::from_u64(0, 1)?,
            Some("USB FS interrupt request".to_owned()),
        )?;
        let reset_signal = hub.declare(
            format!("{name}.reset"),
            SignalValue::from_u64(0, 1)?,
            Some("USB FS bus reset".to_owned()),
        )?;
        let out_signal = hub.declare(
            format!("{name}.out"),
            SignalValue::from_u64(0, 1)?,
            Some("USB FS OUT packet activity".to_owned()),
        )?;
        let state = Arc::new(Mutex::new(UsbFsState::default()));
        let handle = Stm32UsbFsHandle {
            state: state.clone(),
            hub,
            interrupt_signal,
            reset_signal,
            out_signal,
        };
        Ok((
            Self {
                name: name.clone(),
                state: state.clone(),
                handle: handle.clone(),
            },
            Stm32UsbPma {
                name: format!("{name}.pma"),
                state,
            },
            handle,
        ))
    }

    /// Returns the host-facing USB handle.
    pub fn handle(&self) -> Stm32UsbFsHandle {
        self.handle.clone()
    }

    fn read_register(&self, offset: u64) -> u16 {
        let state = self.state.lock().expect("STM32 USB FS lock poisoned");
        if offset < 0x20 && offset % 4 == 0 {
            return state.endpoints[usize::try_from(offset / 4).expect("endpoint index fits")];
        }
        match offset {
            CNTR => state.cntr,
            ISTR => state.istr,
            FNR => state.fnr,
            DADDR => state.daddr,
            BTABLE => state.btable,
            _ => 0,
        }
    }

    fn write_register(&mut self, offset: u64, value: u16, at: SimTime) {
        let (pending, reset, out) = {
            let mut state = self.state.lock().expect("STM32 USB FS lock poisoned");
            if offset < 0x20 && offset % 4 == 0 {
                state.endpoints[usize::try_from(offset / 4).expect("endpoint index fits")] = value
                    & (EP_CTR_RX
                        | EP_DTOG_RX
                        | EP_STAT_RX
                        | EP_SETUP
                        | EP_TYPE
                        | EP_CTR_TX
                        | EP_DTOG_TX
                        | EP_STAT_TX
                        | EP_ADDRESS);
            } else {
                match offset {
                    CNTR => state.cntr = value & CNTR_MASK,
                    // USB ISTR flags clear when firmware writes zero to them.
                    ISTR => state.istr &= value,
                    FNR => state.fnr = value,
                    DADDR => state.daddr = value & 0x80ff,
                    BTABLE => state.btable = value & 0x03ff,
                    _ => {}
                }
            }
            (
                Stm32UsbFsHandle::interrupt_pending(&state),
                state.istr & ISTR_RESET != 0,
                state.istr & ISTR_CTR != 0 && state.istr & ISTR_DIR != 0,
            )
        };
        self.handle.publish(pending, reset, out, at);
    }
}

impl Device for Stm32UsbFs {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if !matches!(width, AccessWidth::HalfWord | AccessWidth::Word)
            || !width.is_aligned(offset)
            || offset >= CORE_SIZE
        {
            return Err(DeviceError::new(format!(
                "STM32 USB FS access at {offset:#x}"
            )));
        }
        let _ = at;
        Ok(u64::from(self.read_register(offset)))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        if !matches!(width, AccessWidth::HalfWord | AccessWidth::Word)
            || !width.is_aligned(offset)
            || offset >= CORE_SIZE
        {
            return Err(DeviceError::new(format!(
                "STM32 USB FS access at {offset:#x}"
            )));
        }
        self.write_register(offset, value as u16, at);
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("STM32 USB FS lock poisoned") = UsbFsState::default();
        self.handle.publish(false, false, false, SimTime::ZERO);
    }
}

impl Device for Stm32UsbPma {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let length = usize::from(width.bytes());
        let start = usize::try_from(offset).map_err(|_| DeviceError::new("USB PMA offset"))?;
        let end = start.saturating_add(length);
        if end > PMA_SIZE || !width.is_aligned(offset) {
            return Err(DeviceError::new(format!(
                "STM32 USB PMA access at {offset:#x}"
            )));
        }
        let state = self.state.lock().expect("STM32 USB PMA lock poisoned");
        let mut bytes = [0; 8];
        bytes[..length].copy_from_slice(&state.pma[start..end]);
        Ok(u64::from_le_bytes(bytes))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        let length = usize::from(width.bytes());
        let start = usize::try_from(offset).map_err(|_| DeviceError::new("USB PMA offset"))?;
        let end = start.saturating_add(length);
        if end > PMA_SIZE || !width.is_aligned(offset) {
            return Err(DeviceError::new(format!(
                "STM32 USB PMA access at {offset:#x}"
            )));
        }
        let mut state = self.state.lock().expect("STM32 USB PMA lock poisoned");
        state.pma[start..end].copy_from_slice(&value.to_le_bytes()[..length]);
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state
            .lock()
            .expect("STM32 USB PMA lock poisoned")
            .pma
            .fill(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usb_fs_reset_out_packet_and_pma_are_observable() {
        let hub = SignalHub::new();
        let (mut usb, mut pma, handle) = Stm32UsbFs::new("usb", hub).unwrap();
        usb.write(
            CNTR,
            AccessWidth::HalfWord,
            u64::from(CNTR_RESETM | CNTR_CTRM),
            SimTime::ZERO,
        )
        .unwrap();
        handle.bus_reset(SimTime::from_ticks(1));
        assert!(handle.poll(SimTime::from_ticks(1)));
        assert_eq!(usb.read(ISTR, AccessWidth::HalfWord, SimTime::ZERO), Ok(1));
        usb.write(ISTR, AccessWidth::HalfWord, 0, SimTime::from_ticks(2))
            .unwrap();
        handle
            .inject_out(0, b"hello", SimTime::from_ticks(3))
            .unwrap();
        assert_eq!(
            pma.read(0x120, AccessWidth::Byte, SimTime::ZERO),
            Ok(u64::from(b'h'))
        );
        assert!(handle.poll(SimTime::from_ticks(3)));
        assert_eq!(
            usb.read(0, AccessWidth::HalfWord, SimTime::ZERO).unwrap() & u64::from(EP_CTR_RX),
            u64::from(EP_CTR_RX)
        );
    }

    #[test]
    fn usb_fs_host_reads_firmware_pma_in_packet() {
        let hub = SignalHub::new();
        let (mut usb, mut pma, handle) = Stm32UsbFs::new("usb", hub).unwrap();
        pma.write(0x20, AccessWidth::Byte, u64::from(b'O'), SimTime::ZERO)
            .unwrap();
        pma.write(0x21, AccessWidth::Byte, u64::from(b'K'), SimTime::ZERO)
            .unwrap();
        pma.write(0x02, AccessWidth::HalfWord, 2, SimTime::ZERO)
            .unwrap();
        usb.write(
            0,
            AccessWidth::HalfWord,
            u64::from(EP_CTR_TX),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(handle.take_in(0, SimTime::from_ticks(1)).unwrap(), b"OK");
    }
}
