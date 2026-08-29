use super::*;

pub(super) const fn cpu_profile(target: TargetId) -> Option<(ArmProfile, u32)> {
    match target {
        TargetId::Atsamd21e18 => Some((ArmProfile::CortexM0Plus, 0x410c_c200)),
        TargetId::Atsamd51j19a
        | TargetId::Stm32l432kc
        | TargetId::Stm32f411re
        | TargetId::Nrf52840
        | TargetId::R7fa4m1ab3cfm => Some((ArmProfile::CortexM4F, 0x410f_c241)),
        TargetId::Stm32f103c8 => Some((ArmProfile::CortexM3, 0x411f_c231)),
        _ => None,
    }
}

pub(super) const fn signal_paths(
    target: TargetId,
) -> Option<(&'static str, &'static str, &'static str)> {
    match target {
        TargetId::Atsamd21e18 => Some((
            "board.atsamd21e18.tc3.irq",
            "board.atsamd21e18.sercom0",
            "board.atsamd21e18.interrupt.request",
        )),
        TargetId::Atsamd51j19a => Some((
            "board.atsamd51j19a.tc0.irq",
            "board.atsamd51j19a.sercom0",
            "board.atsamd51j19a.interrupt.request",
        )),
        TargetId::Stm32l432kc => Some((
            "board.stm32l432kc.tim2.irq",
            "board.stm32l432kc.usart2",
            "board.stm32l432kc.interrupt.request",
        )),
        TargetId::Stm32f103c8 => Some((
            "board.stm32f103c8.tim2.irq",
            "board.stm32f103c8.usart1",
            "board.stm32f103c8.interrupt.request",
        )),
        TargetId::Stm32f411re => Some((
            "board.stm32f411re.tim2.irq",
            "board.stm32f411re.usart2",
            "board.stm32f411re.interrupt.request",
        )),
        TargetId::Nrf52840 => Some((
            "board.nrf52840.timer0.irq",
            "board.nrf52840.uart0",
            "board.nrf52840.interrupt.request",
        )),
        TargetId::R7fa4m1ab3cfm => Some((
            "board.r7fa4m1ab3cfm.gpt0.irq",
            "board.r7fa4m1ab3cfm.sci9",
            "board.r7fa4m1ab3cfm.icu.request",
        )),
        _ => None,
    }
}

pub(super) enum VendorUart {
    Samd21(Samd21UsartHandle),
    Samd51(Samd21UsartHandle),
    Stm32F1(Stm32F1UsartHandle),
    Stm32(Vec<(Stm32UsartHandle, u16)>),
    Generic(Vec<(UartHandle, u16)>),
    Ra4m1(RaSciHandle),
}

impl VendorUart {
    pub(super) fn bytes(&self) -> Vec<u8> {
        match self {
            Self::Samd21(handle) => handle.bytes(),
            Self::Samd51(handle) => handle.bytes(),
            Self::Stm32F1(handle) => handle.bytes(),
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
            Self::Samd51(handle) => handle.interrupt_pending(),
            Self::Stm32F1(handle) => handle.interrupt_pending(),
            Self::Stm32(handles) => handles.iter().any(|(handle, _)| handle.interrupt_pending()),
            Self::Generic(_) => false,
            Self::Ra4m1(handle) => handle.txi_pending(),
        }
    }
}

pub(super) enum VendorTimer {
    Samd21(Samd21TcHandle),
    Samd51(Samd51TcHandle),
    Stm32(Stm32TimerHandle),
    Nrf52840(Nrf52840TimerHandle),
    Ra4m1(RaGptHandle),
}

impl VendorTimer {
    pub(super) fn poll(&self, now: SimTime) -> (Option<u16>, bool) {
        match self {
            Self::Samd21(handle) => (Some(18), handle.poll(now)),
            Self::Samd51(handle) => (Some(107), handle.poll(now)),
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
