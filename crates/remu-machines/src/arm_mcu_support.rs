use super::*;

pub(super) enum VendorUart {
    Samd21(Samd21UsartHandle),
    Stm32(Vec<(Stm32UsartHandle, u16)>),
    Generic(Vec<(UartHandle, u16)>),
    Ra4m1(RaSciHandle),
}

impl VendorUart {
    pub(super) fn bytes(&self) -> Vec<u8> {
        match self {
            Self::Samd21(handle) => handle.bytes(),
            Self::Stm32(handles) => handles
                .iter()
                .flat_map(|(handle, _)| handle.bytes())
                .collect(),
            Self::Generic(handles) => handles
                .iter()
                .flat_map(|(handle, _)| handle.bytes())
                .collect(),
            Self::Ra4m1(handle) => handle.bytes(),
        }
    }

    pub(super) fn interrupt_pending(&self) -> bool {
        match self {
            Self::Samd21(handle) => handle.interrupt_pending(),
            Self::Stm32(handles) => handles.iter().any(|(handle, _)| handle.interrupt_pending()),
            Self::Generic(_) => false,
            Self::Ra4m1(handle) => handle.txi_pending(),
        }
    }
}

pub(super) enum VendorTimer {
    Samd21(Samd21TcHandle),
    Stm32(Stm32TimerHandle),
    Nrf52840(Nrf52840TimerHandle),
    Ra4m1(RaGptHandle),
}

impl VendorTimer {
    pub(super) fn poll(&self, now: SimTime) -> (Option<u16>, bool) {
        match self {
            Self::Samd21(handle) => (Some(18), handle.poll(now)),
            Self::Stm32(handle) => (Some(28), handle.poll(now)),
            Self::Nrf52840(handle) => (Some(8), handle.poll(now)),
            Self::Ra4m1(handle) => (None, handle.poll(now)),
        }
    }
}

pub(super) enum VendorWatchdog {
    Samd21(Samd21WdtHandle),
    Stm32(Stm32WatchdogHandle),
}

impl VendorWatchdog {
    pub(super) fn take_reset(&self, now: SimTime) -> bool {
        match self {
            Self::Samd21(handle) => handle.take_reset(now),
            Self::Stm32(handle) => handle.take_reset(now),
        }
    }
}
