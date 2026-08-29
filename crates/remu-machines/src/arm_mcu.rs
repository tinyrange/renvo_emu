use crate::{
    ArmMachineError, MemoryKind, PinStimulus, RunResult, SignalEdge, SignalStop, TEST_EXIT,
    TEST_GPIO, TEST_TIMER, TEST_UART, TargetId, matching_signal_stop, resolve_signal_stop,
    target_manifest,
};
use remu_bus::{
    AddressSpace, BusAccessRecord, Endianness, Permissions, SharedBusAccessObserver, SharedMemory,
};
use remu_core::{
    AccessKind, AccessWidth, Bus, Cpu, ResetKind, RunLimits, RunStats, SimTime, StepReason,
    StopReason,
};
use remu_cpu_arm::{ArmCpu, ArmProfile};
use remu_devices::{
    ArmPpbHandle, ArmPrivatePeripheralBus, ExitDevice, ExitHandle, FunctionalGpio, FunctionalTimer,
    FunctionalUart, GpioHandle, RA4M1_EVENT_GPT0_OVERFLOW, RA4M1_EVENT_SCI9_TXI, RaGpt,
    RaGptHandle, RaIcu, RaIcuHandle, RaIoPort, RaPfs, RaSci, RaSciHandle, RegisterBank,
    STM32_FLASH_SIZE, Samd21Ac, Samd21AcHandle, Samd21Adc, Samd21AdcHandle, Samd21Dac,
    Samd21DacHandle, Samd21Dmac, Samd21DmacHandle, Samd21Eic, Samd21EicHandle, Samd21Evsys,
    Samd21I2s, Samd21I2sHandle, Samd21Port, Samd21RegisterBlock, Samd21Rtc, Samd21RtcHandle,
    Samd21Tc, Samd21TcHandle, Samd21Tcc, Samd21TccHandle, Samd21Usart, Samd21UsartHandle,
    Samd21UsbDevice, Samd21Wdt, Samd21WdtHandle, SignalHub, Stm32Adc, Stm32AdcHandle,
    Stm32AdvancedTimer, Stm32AdvancedTimerHandle, Stm32BasicTimer, Stm32BasicTimerHandle, Stm32Can,
    Stm32ComparatorHandle, Stm32Comparators, Stm32Crc, Stm32CrcHandle, Stm32Dac, Stm32Dma,
    Stm32DmaHandle, Stm32Exti, Stm32ExtiHandle, Stm32FlashController, Stm32FlashMemory, Stm32Gpio,
    Stm32I2c, Stm32I2cHandle, Stm32Lptim1, Stm32Lptim1Handle, Stm32Lptim2, Stm32Lptim2Handle,
    Stm32Opamp, Stm32OpampHandle, Stm32QuadSpi, Stm32QuadSpiHandle, Stm32Rng, Stm32RngHandle,
    Stm32Rtc, Stm32RtcHandle, Stm32Sai1, Stm32Sai1Handle, Stm32Spi, Stm32SpiHandle, Stm32Swpmi,
    Stm32SwpmiHandle, Stm32Tim7, Stm32Tim7Handle, Stm32Tim15, Stm32Tim15Handle, Stm32Tim16,
    Stm32Tim16Handle, Stm32Timer, Stm32TimerHandle, Stm32Tsc, Stm32TscHandle, Stm32Usart,
    Stm32UsartHandle, Stm32UsbFs, Stm32UsbFsHandle, Stm32UsbPma, Stm32Watchdog,
    Stm32WatchdogHandle, Stm32Wwdg, Stm32WwdgHandle, TimerHandle, UartHandle,
};
use remu_image::{FirmwareArchitecture, FirmwareImage};
use remu_signals::{Logic, SignalId, SignalValue};
use remu_trace::{TraceDigest, TraceSink};
use std::collections::BTreeSet;

const TEST_DEVICE_SIZE: usize = 0x100;
const TEST_EXIT_SIZE: usize = 4;

enum VendorUart {
    Samd21(Samd21UsartHandle),
    Stm32(Vec<(Stm32UsartHandle, u16)>),
    Ra4m1(RaSciHandle),
}

impl VendorUart {
    fn bytes(&self) -> Vec<u8> {
        match self {
            Self::Samd21(handle) => handle.bytes(),
            Self::Stm32(handles) => handles
                .iter()
                .flat_map(|(handle, _)| handle.bytes())
                .collect(),
            Self::Ra4m1(handle) => handle.bytes(),
        }
    }

    fn interrupt_pending(&self) -> bool {
        match self {
            Self::Samd21(handle) => handle.interrupt_pending(),
            Self::Stm32(handles) => handles.iter().any(|(handle, _)| handle.interrupt_pending()),
            Self::Ra4m1(handle) => handle.txi_pending(),
        }
    }
}

enum VendorTimer {
    Samd21(Samd21TcHandle),
    Stm32(Stm32TimerHandle),
    Ra4m1(RaGptHandle),
}

impl VendorTimer {
    fn poll(&self, now: SimTime) -> (Option<u16>, bool) {
        match self {
            Self::Samd21(handle) => (Some(18), handle.poll(now)),
            Self::Stm32(handle) => (Some(28), handle.poll(now)),
            Self::Ra4m1(handle) => (None, handle.poll(now)),
        }
    }
}

enum VendorWatchdog {
    Samd21(Samd21WdtHandle),
    Stm32(Stm32WatchdogHandle),
}

impl VendorWatchdog {
    fn take_reset(&self, now: SimTime) -> bool {
        match self {
            Self::Samd21(handle) => handle.take_reset(now),
            Self::Stm32(handle) => handle.take_reset(now),
        }
    }
}

/// Direct-ELF Arm machine for vendor microcontrollers outside the Raspberry Pi family.
pub struct ArmMcuMachine {
    target: TargetId,
    cpu: ArmCpu,
    bus: AddressSpace,
    signals: SignalHub,
    gpio: GpioHandle,
    compiler_gpio: GpioHandle,
    uart: VendorUart,
    compiler_uart: UartHandle,
    timer: VendorTimer,
    samd_sercom_irqs: Vec<(u16, Samd21UsartHandle)>,
    samd_tc_irqs: Vec<(u16, Samd21TcHandle)>,
    samd_tcc_irqs: Vec<(u16, Samd21TccHandle)>,
    samd_rtc: Option<Samd21RtcHandle>,
    stm32_spi: Vec<(u16, Stm32SpiHandle)>,
    eic: Option<Samd21EicHandle>,
    dmac: Option<Samd21DmacHandle>,
    i2s: Option<Samd21I2sHandle>,
    adc: Option<Samd21AdcHandle>,
    ac: Option<Samd21AcHandle>,
    dac: Option<Samd21DacHandle>,
    ra_icu: Option<RaIcuHandle>,
    watchdog: Option<VendorWatchdog>,
    stm32_i2c: Vec<(u16, Stm32I2cHandle)>,
    stm32_adc: Option<Stm32AdcHandle>,
    stm32_crc: Option<Stm32CrcHandle>,
    stm32_rtc: Option<Stm32RtcHandle>,
    stm32_rng: Option<Stm32RngHandle>,
    stm32_tim1: Option<Stm32AdvancedTimerHandle>,
    stm32_exti: Option<Stm32ExtiHandle>,
    stm32_wwdg: Option<Stm32WwdgHandle>,
    stm32_tim6: Option<Stm32BasicTimerHandle>,
    stm32_tim7: Option<Stm32Tim7Handle>,
    stm32_tim15: Option<Stm32Tim15Handle>,
    stm32_tim16: Option<Stm32Tim16Handle>,
    stm32_lptim1: Option<Stm32Lptim1Handle>,
    stm32_lptim2: Option<Stm32Lptim2Handle>,
    usb_fs: Option<Stm32UsbFsHandle>,
    sai1: Option<Stm32Sai1Handle>,
    qspi: Option<Stm32QuadSpiHandle>,
    swpmi: Option<Stm32SwpmiHandle>,
    dma1: Option<Stm32DmaHandle>,
    dma2: Option<Stm32DmaHandle>,
    tsc: Option<Stm32TscHandle>,
    comparators: Option<Stm32ComparatorHandle>,
    opamp: Option<Stm32OpampHandle>,
    compiler_timer: TimerHandle,
    exit: ExitHandle,
    ppb: ArmPpbHandle,
    timer_irq_signal: SignalId,
    uart_byte_signal: SignalId,
    uart_strobe_signal: SignalId,
    interrupt_signal: SignalId,
    traced_uart_len: usize,
    uart_strobe: bool,
    now: SimTime,
    default_stack: u32,
    breakpoints: BTreeSet<u64>,
    signal_stops: Vec<SignalStop>,
}

impl ArmMcuMachine {
    /// Creates an evidence-backed vendor Arm machine.
    pub fn new(target: TargetId) -> Result<Self, ArmMachineError> {
        let (profile, cpuid) = match target {
            TargetId::Atsamd21e18 => (ArmProfile::CortexM0Plus, 0x410c_c200),
            TargetId::Stm32l432kc => (ArmProfile::CortexM4F, 0x410f_c241),
            TargetId::R7fa4m1ab3cfm => (ArmProfile::CortexM4F, 0x410f_c241),
            _ => return Err(ArmMachineError::UnsupportedTarget(target)),
        };
        let manifest = target_manifest(target);
        let mut bus = AddressSpace::new(Endianness::Little);
        let mut default_stack = None;
        let mut stm32_flash_controller = None;
        for region in manifest.memory {
            match region.kind {
                MemoryKind::Ram => {
                    bus.map_ram(region.name, region.start, region.size, region.executable)?;
                    let end =
                        region.start + u64::try_from(region.size).expect("memory size fits u64");
                    default_stack = Some(u32::try_from(end).expect("Arm memory fits u32"));
                }
                MemoryKind::Flash | MemoryKind::Rom => {
                    if target == TargetId::Stm32l432kc
                        && region.kind == MemoryKind::Flash
                        && region.start == u64::from(remu_devices::STM32_FLASH_BASE)
                    {
                        let (flash, controller) =
                            Stm32FlashMemory::new(region.name, STM32_FLASH_SIZE);
                        let alias = flash.alias("stm32l432kc.flash-alias");
                        bus.map_device_with_permissions(
                            region.name,
                            region.start,
                            region.size,
                            Permissions::RWX,
                            Box::new(flash),
                        )?;
                        bus.map_device_with_permissions(
                            "stm32l432kc.flash-alias",
                            0,
                            region.size,
                            Permissions::RX,
                            Box::new(alias),
                        )?;
                        stm32_flash_controller = Some(controller);
                        continue;
                    }
                    let storage = if region.kind == MemoryKind::Flash {
                        SharedMemory::from_bytes(vec![0xff; region.size])
                    } else {
                        SharedMemory::zeroed(region.size)
                    };
                    bus.map_shared(
                        region.name,
                        region.start,
                        region.size,
                        Permissions::RX,
                        storage.clone(),
                        0,
                    )?;
                    if target == TargetId::Stm32l432kc && region.start == 0x0800_0000 {
                        bus.map_shared(
                            "stm32l432kc.flash-alias",
                            0,
                            region.size,
                            Permissions::RX,
                            storage.clone(),
                            0,
                        )?;
                    }
                }
            }
        }

        let signals = SignalHub::new();
        let (timer_path, uart_path, interrupt_path) = match target {
            TargetId::Atsamd21e18 => (
                "board.atsamd21e18.tc3.irq",
                "board.atsamd21e18.sercom0",
                "board.atsamd21e18.interrupt.request",
            ),
            TargetId::Stm32l432kc => (
                "board.stm32l432kc.tim2.irq",
                "board.stm32l432kc.usart2",
                "board.stm32l432kc.interrupt.request",
            ),
            TargetId::R7fa4m1ab3cfm => (
                "board.r7fa4m1ab3cfm.gpt0.irq",
                "board.r7fa4m1ab3cfm.sci9",
                "board.r7fa4m1ab3cfm.icu.request",
            ),
            _ => unreachable!(),
        };
        let timer_irq_signal = signals.declare(
            timer_path,
            SignalValue::from_u64(0, 1)?,
            Some("selected timer interrupt request".to_owned()),
        )?;
        let uart_byte_signal = signals.declare(
            format!("{uart_path}.tx_byte"),
            SignalValue::from_u64(0, 8)?,
            Some("selected UART transmitted byte".to_owned()),
        )?;
        let uart_strobe_signal = signals.declare(
            format!("{uart_path}.tx_strobe"),
            SignalValue::from_u64(0, 1)?,
            Some("toggles for each selected UART byte".to_owned()),
        )?;
        let interrupt_signal = signals.declare(
            interrupt_path,
            SignalValue::from_u64(0, 1)?,
            Some("selected routed interrupt request".to_owned()),
        )?;
        let (compiler_gpio_device, compiler_gpio) = FunctionalGpio::new(
            format!("{target}.compiler-gpio"),
            manifest.gpio_count.min(32),
            &format!("board.{target}.compiler-gpio"),
            signals.clone(),
            0,
            4,
            8,
        )?;
        let (compiler_uart_device, compiler_uart) =
            FunctionalUart::new(format!("{target}.compiler-uart"), 0, 4, 1);
        let (compiler_timer_device, compiler_timer) =
            FunctionalTimer::new(format!("{target}.compiler-timer"));
        let (exit_device, exit) = ExitDevice::new(format!("{target}.compiler-exit"));
        bus.map_device(
            "remu.test.gpio",
            TEST_GPIO,
            TEST_DEVICE_SIZE,
            Box::new(compiler_gpio_device),
        )?;
        bus.map_device(
            "remu.test.uart",
            TEST_UART,
            TEST_DEVICE_SIZE,
            Box::new(compiler_uart_device),
        )?;
        bus.map_device(
            "remu.test.timer",
            TEST_TIMER,
            TEST_DEVICE_SIZE,
            Box::new(compiler_timer_device),
        )?;
        bus.map_device(
            "remu.test.exit",
            TEST_EXIT,
            TEST_EXIT_SIZE,
            Box::new(exit_device),
        )?;

        let (ppb_device, ppb) = ArmPrivatePeripheralBus::new(format!("{target}.ppb"), cpuid);
        bus.map_device(
            format!("{target}.ppb"),
            0xe000_e000,
            0x1000,
            Box::new(ppb_device),
        )?;

        let mut stm32_spi = Vec::new();
        let mut stm32_adc = None;
        let mut stm32_crc = None;
        let mut stm32_rtc = None;
        let mut stm32_rng = None;
        let mut stm32_tim1 = None;
        let mut stm32_exti = None;
        let mut stm32_wwdg = None;
        let mut stm32_tim6 = None;
        let mut stm32_tim7 = None;
        let mut stm32_tim15 = None;
        let mut stm32_tim16 = None;
        let mut stm32_lptim1 = None;
        let mut stm32_lptim2 = None;
        let mut usb_fs = None;
        let mut sai1 = None;
        let mut qspi = None;
        let mut swpmi = None;
        let mut dma1 = None;
        let mut dma2 = None;
        let mut tsc = None;
        let mut comparators = None;
        let mut opamp = None;
        let (
            gpio,
            uart,
            timer,
            samd_sercom_irqs,
            samd_tc_irqs,
            samd_tcc_irqs,
            samd_rtc,
            eic,
            dmac,
            i2s,
            adc,
            ac,
            dac,
            ra_icu,
            watchdog,
            stm32_i2c,
        ) = match target {
            TargetId::Atsamd21e18 => {
                let (port_device, gpio) = Samd21Port::new(
                    "atsamd21e18.porta",
                    26,
                    "board.atsamd21e18.porta",
                    signals.clone(),
                )?;
                let (tc3_device, timer) = Samd21Tc::new("atsamd21e18.tc3");
                let (tc4_device, tc4) = Samd21Tc::new("atsamd21e18.tc4");
                let (tc5_device, tc5) = Samd21Tc::new("atsamd21e18.tc5");
                let (tcc0_device, tcc0) = Samd21Tcc::new_with_signals(
                    "atsamd21e18.tcc0",
                    4,
                    signals.clone(),
                    "board.atsamd21e18.tcc0",
                )?;
                let (tcc1_device, tcc1) = Samd21Tcc::new_with_signals(
                    "atsamd21e18.tcc1",
                    2,
                    signals.clone(),
                    "board.atsamd21e18.tcc1",
                )?;
                let (tcc2_device, tcc2) = Samd21Tcc::new_with_signals(
                    "atsamd21e18.tcc2",
                    2,
                    signals.clone(),
                    "board.atsamd21e18.tcc2",
                )?;
                let (rtc_device, rtc) = Samd21Rtc::new("atsamd21e18.rtc");
                let (eic_device, eic) = Samd21Eic::new("atsamd21e18.eic");
                let (adc_device, adc) = Samd21Adc::new("atsamd21e18.adc");
                let (ac_device, ac) =
                    Samd21Ac::new("atsamd21e18.ac", "board.atsamd21e18.ac", signals.clone())?;
                let (watchdog_device, watchdog) = Samd21Wdt::new("atsamd21e18.wdt");
                let (sercom0_device, uart) = Samd21Usart::new("atsamd21e18.sercom0");
                let (sercom1_device, sercom1) = Samd21Usart::new("atsamd21e18.sercom1");
                let (sercom2_device, sercom2) = Samd21Usart::new("atsamd21e18.sercom2");
                let (sercom3_device, sercom3) = Samd21Usart::new("atsamd21e18.sercom3");
                let (evsys_device, _evsys) = Samd21Evsys::new("atsamd21e18.evsys");
                let (usb_device, _usb) = Samd21UsbDevice::new("atsamd21e18.usb");
                let (dmac_device, dmac) = Samd21Dmac::new("atsamd21e18.dmac");
                let (i2s_device, i2s) = Samd21I2s::new("atsamd21e18.i2s");
                let (dac_device, dac) = Samd21Dac::new_with_signals(
                    "atsamd21e18.dac",
                    signals.clone(),
                    "board.atsamd21e18.dac.output_code",
                )?;
                Self::map_samd21(
                    &mut bus,
                    port_device,
                    eic_device,
                    watchdog_device,
                    [tc3_device, tc4_device, tc5_device],
                    [tcc0_device, tcc1_device, tcc2_device],
                    rtc_device,
                    [
                        sercom0_device,
                        sercom1_device,
                        sercom2_device,
                        sercom3_device,
                    ],
                    evsys_device,
                    usb_device,
                    dmac_device,
                    i2s_device,
                    adc_device,
                    ac_device,
                    dac_device,
                )?;
                (
                    gpio,
                    VendorUart::Samd21(uart),
                    VendorTimer::Samd21(timer),
                    vec![(10, sercom1), (11, sercom2), (12, sercom3)],
                    vec![(19, tc4), (20, tc5)],
                    vec![(15, tcc0), (16, tcc1), (17, tcc2)],
                    Some(rtc),
                    Some(eic),
                    Some(dmac),
                    Some(i2s),
                    Some(adc),
                    Some(ac),
                    Some(dac),
                    None,
                    Some(VendorWatchdog::Samd21(watchdog)),
                    Vec::new(),
                )
            }
            TargetId::Stm32l432kc => {
                let (gpioa_device, gpio) = Stm32Gpio::new(
                    "stm32l432kc.gpioa",
                    "board.stm32l432kc.gpioa",
                    signals.clone(),
                )?;
                let (gpiob_device, _) = Stm32Gpio::new(
                    "stm32l432kc.gpiob",
                    "board.stm32l432kc.gpiob",
                    signals.clone(),
                )?;
                let (gpioc_device, _) = Stm32Gpio::new(
                    "stm32l432kc.gpioc",
                    "board.stm32l432kc.gpioc",
                    signals.clone(),
                )?;
                let (gpioh_device, _) = Stm32Gpio::new(
                    "stm32l432kc.gpioh",
                    "board.stm32l432kc.gpioh",
                    signals.clone(),
                )?;
                let (tim2_device, timer) = Stm32Timer::new("stm32l432kc.tim2");
                let (tim1_device, tim1) =
                    Stm32AdvancedTimer::new("board.stm32l432kc.tim1", signals.clone())?;
                stm32_tim1 = Some(tim1);
                let (usart1_device, usart1) = Stm32Usart::new("stm32l432kc.usart1");
                let (usart2_device, usart2) = Stm32Usart::new("stm32l432kc.usart2");
                let (lpuart1_device, lpuart1) = Stm32Usart::new("stm32l432kc.lpuart1");
                let (spi1_device, spi1) = Stm32Spi::new("stm32l432kc.spi1");
                let (spi3_device, spi3) = Stm32Spi::new("stm32l432kc.spi3");
                stm32_spi.extend([(35, spi1), (51, spi3)]);
                let (i2c1_device, i2c1) = Stm32I2c::new("stm32l432kc.i2c1");
                let (i2c3_device, i2c3) = Stm32I2c::new("stm32l432kc.i2c3");
                let (watchdog_device, watchdog) = Stm32Watchdog::new("stm32l432kc.iwdg");
                let (adc_device, adc) = Stm32Adc::new("stm32l432kc.adc1");
                stm32_adc = Some(adc);
                let (crc_device, crc) = Stm32Crc::new("stm32l432kc.crc");
                stm32_crc = Some(crc);
                let (rtc_device, rtc) = Stm32Rtc::new("stm32l432kc.rtc");
                stm32_rtc = Some(rtc);
                let (rng_device, rng) = Stm32Rng::new("stm32l432kc.rng");
                stm32_rng = Some(rng);
                let dac1_device = Stm32Dac::new("board.stm32l432kc.dac1", signals.clone())?;
                let (exti_device, exti) =
                    Stm32Exti::new("board.stm32l432kc.exti", signals.clone())?;
                stm32_exti = Some(exti);
                let (wwdg_device, wwdg) = Stm32Wwdg::new("stm32l432kc.wwdg", signals.clone())?;
                stm32_wwdg = Some(wwdg);
                let (tim6_device, tim6) =
                    Stm32BasicTimer::new("stm32l432kc.tim6", signals.clone())?;
                stm32_tim6 = Some(tim6);
                let (tim7_device, tim7) = Stm32Tim7::new("stm32l432kc.tim7", signals.clone())?;
                stm32_tim7 = Some(tim7);
                let (tim15_device, tim15) = Stm32Tim15::new("stm32l432kc.tim15", signals.clone())?;
                stm32_tim15 = Some(tim15);
                let (tim16_device, tim16) = Stm32Tim16::new("stm32l432kc.tim16", signals.clone())?;
                stm32_tim16 = Some(tim16);
                let (lptim1_device, lptim1) =
                    Stm32Lptim1::new("stm32l432kc.lptim1", signals.clone())?;
                stm32_lptim1 = Some(lptim1);
                let (lptim2_device, lptim2) =
                    Stm32Lptim2::new("stm32l432kc.lptim2", signals.clone())?;
                stm32_lptim2 = Some(lptim2);
                let (usb_device, usb_pma, usb_handle) =
                    Stm32UsbFs::new("stm32l432kc.usb", signals.clone())?;
                usb_fs = Some(usb_handle);
                let (sai1_device, sai1_handle) =
                    Stm32Sai1::new("stm32l432kc.sai1", signals.clone())?;
                sai1 = Some(sai1_handle);
                let (qspi_device, qspi_handle, qspi_flash) =
                    Stm32QuadSpi::new("stm32l432kc.quadspi", signals.clone())?;
                qspi = Some(qspi_handle);
                let (swpmi_device, swpmi_handle) =
                    Stm32Swpmi::new("stm32l432kc.swpmi", signals.clone())?;
                swpmi = Some(swpmi_handle);
                let (dma1_device, dma1_handle) = Stm32Dma::new("stm32l432kc.dma1");
                dma1 = Some(dma1_handle);
                let (dma2_device, dma2_handle) = Stm32Dma::new("stm32l432kc.dma2");
                dma2 = Some(dma2_handle);
                let (tsc_device, tsc_handle) = Stm32Tsc::new("stm32l432kc.tsc");
                tsc = Some(tsc_handle);
                let (comparators_device, comparator_handle) =
                    Stm32Comparators::new("stm32l432kc.comp");
                comparators = Some(comparator_handle);
                let (opamp_device, opamp_handle) = Stm32Opamp::new("stm32l432kc.opamp");
                opamp = Some(opamp_handle);
                Self::map_stm32l432(
                    &mut bus,
                    [gpioa_device, gpiob_device, gpioc_device, gpioh_device],
                    tim1_device,
                    tim2_device,
                    usart1_device,
                    usart2_device,
                    lpuart1_device,
                    spi1_device,
                    spi3_device,
                    i2c1_device,
                    i2c3_device,
                    watchdog_device,
                    adc_device,
                    crc_device,
                    rtc_device,
                    rng_device,
                    dac1_device,
                    exti_device,
                    wwdg_device,
                    tim6_device,
                    tim7_device,
                    tim15_device,
                    tim16_device,
                    lptim1_device,
                    lptim2_device,
                    usb_device,
                    usb_pma,
                    sai1_device,
                    qspi_device,
                    qspi_flash,
                    swpmi_device,
                    dma1_device,
                    dma2_device,
                    tsc_device,
                    comparators_device,
                    opamp_device,
                    stm32_flash_controller.expect("STM32 flash controller was mapped"),
                )?;
                (
                    gpio,
                    VendorUart::Stm32(vec![(usart1, 37), (usart2, 38), (lpuart1, 70)]),
                    VendorTimer::Stm32(timer),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(VendorWatchdog::Stm32(watchdog)),
                    vec![(31, i2c1), (72, i2c3)],
                )
            }
            TargetId::R7fa4m1ab3cfm => {
                let mut ports = Vec::new();
                let mut handles = Vec::new();
                for port in 0..15 {
                    let (device, handle) = RaIoPort::new(
                        format!("r7fa4m1ab3cfm.port{port}"),
                        &format!("board.r7fa4m1ab3cfm.port{port}"),
                        signals.clone(),
                    )?;
                    ports.push(device);
                    handles.push(handle);
                }
                let pfs = RaPfs::new("r7fa4m1ab3cfm.pfs", &ports);
                let (gpt0_device, timer) = RaGpt::new("r7fa4m1ab3cfm.gpt0");
                let (sci9_device, uart) = RaSci::new("r7fa4m1ab3cfm.sci9");
                let (icu_device, icu) = RaIcu::new("r7fa4m1ab3cfm.icu");
                Self::map_ra4m1(&mut bus, ports, pfs, icu_device, gpt0_device, sci9_device)?;
                (
                    handles.remove(1),
                    VendorUart::Ra4m1(uart),
                    VendorTimer::Ra4m1(timer),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(icu),
                    None,
                    Vec::new(),
                )
            }
            _ => unreachable!(),
        };

        Ok(Self {
            target,
            cpu: ArmCpu::new(profile),
            bus,
            signals,
            gpio,
            compiler_gpio,
            uart,
            compiler_uart,
            timer,
            samd_sercom_irqs,
            samd_tc_irqs,
            samd_tcc_irqs,
            samd_rtc,
            stm32_spi,
            eic,
            dmac,
            i2s,
            adc,
            ac,
            dac,
            ra_icu,
            watchdog,
            stm32_i2c,
            stm32_adc,
            stm32_crc,
            stm32_rtc,
            stm32_rng,
            stm32_tim1,
            stm32_exti,
            stm32_wwdg,
            stm32_tim6,
            stm32_tim7,
            stm32_tim15,
            stm32_tim16,
            stm32_lptim1,
            stm32_lptim2,
            usb_fs,
            sai1,
            qspi,
            swpmi,
            dma1,
            dma2,
            tsc,
            comparators,
            opamp,
            compiler_timer,
            exit,
            ppb,
            timer_irq_signal,
            uart_byte_signal,
            uart_strobe_signal,
            interrupt_signal,
            traced_uart_len: 0,
            uart_strobe: false,
            now: SimTime::ZERO,
            default_stack: default_stack.expect("Arm target manifest has RAM"),
            breakpoints: BTreeSet::new(),
            signal_stops: Vec::new(),
        })
    }

    /// Loads an Arm ELF and uses its vector reset when it contains a valid vector table.
    pub fn load_firmware(&mut self, image: &FirmwareImage) -> Result<(), ArmMachineError> {
        if image.architecture != FirmwareArchitecture::Arm {
            return Err(ArmMachineError::Architecture {
                target: self.target,
                actual: image.architecture,
            });
        }
        for segment in &image.segments {
            self.bus
                .load(segment.address, &segment.data)
                .map_err(|error| ArmMachineError::Load {
                    address: segment.address,
                    message: error.to_string(),
                })?;
        }
        let flash_base = target_manifest(self.target)
            .memory
            .iter()
            .find(|region| region.executable && region.kind != MemoryKind::Ram)
            .map_or(0, |region| region.start);
        let image_word = |address: u64| {
            image.segments.iter().find_map(|segment| {
                let offset = usize::try_from(address.checked_sub(segment.address)?).ok()?;
                let bytes: [u8; 4] = segment.data.get(offset..offset + 4)?.try_into().ok()?;
                Some(u32::from_le_bytes(bytes))
            })
        };
        let stack = image_word(flash_base).unwrap_or(u32::MAX);
        let reset = image_word(flash_base + 4).unwrap_or(u32::MAX);
        let ram_contains_stack = target_manifest(self.target).memory.iter().any(|region| {
            region.kind == MemoryKind::Ram
                && u64::from(stack) >= region.start
                && u64::from(stack)
                    <= region.start + u64::try_from(region.size).expect("memory size fits u64")
        });
        let executable_reset = target_manifest(self.target).memory.iter().any(|region| {
            let address = u64::from(reset & !1);
            region.executable
                && address >= region.start
                && address
                    < region.start + u64::try_from(region.size).expect("memory size fits u64")
        });
        if ram_contains_stack && reset & 1 != 0 && executable_reset {
            self.cpu.reset(ResetKind::PowerOn, &mut self.bus)?;
        } else {
            let entry =
                u32::try_from(image.entry).map_err(|_| ArmMachineError::EntryRange(image.entry))?;
            if let Some(segment) = image.segments.iter().find(|segment| segment.executable) {
                self.cpu.set_vector_base(
                    u32::try_from(segment.address)
                        .map_err(|_| ArmMachineError::EntryRange(segment.address))?,
                );
            }
            self.cpu.set_direct_state(self.default_stack, entry | 1)?;
        }
        Ok(())
    }

    /// Enables or disables completed bus-access recording.
    pub fn set_access_recording(&mut self, enabled: bool) {
        self.bus.set_access_recording(enabled);
    }

    /// Installs or removes a streaming completed-access observer.
    pub fn set_access_observer(&mut self, observer: Option<SharedBusAccessObserver>) {
        self.bus.set_access_observer(observer);
    }

    /// Returns completed bus accesses retained for diagnostics.
    pub fn access_log(&self) -> &[BusAccessRecord] {
        self.bus.access_log()
    }

    /// Stops before executing an instruction at `address`.
    pub fn add_breakpoint(&mut self, address: u64) {
        self.breakpoints.insert(address);
    }

    /// Removes one debugger execution breakpoint.
    pub fn remove_breakpoint(&mut self, address: u64) {
        self.breakpoints.remove(&address);
    }

    /// Returns the current architectural snapshot.
    pub fn debug_snapshot(&self) -> remu_core::CpuSnapshot {
        self.cpu.snapshot()
    }

    /// Stops after a completed CPU data access overlaps `address`.
    pub fn add_watchpoint(&mut self, address: u64) {
        self.bus.add_watchpoint(address);
    }

    /// Stops when a named signal satisfies an edge condition.
    pub fn add_signal_stop(&mut self, path: &str, edge: SignalEdge) -> Result<(), ArmMachineError> {
        self.signal_stops
            .push(resolve_signal_stop(&self.signals, path, edge)?);
        Ok(())
    }

    /// Drives or releases one package GPIO pin.
    pub fn set_pin(&self, pin: u8, value: Logic) -> Result<(), ArmMachineError> {
        self.gpio.set_input(pin, value, self.now)?;
        if usize::from(pin) < self.compiler_gpio.pin_count() {
            self.compiler_gpio.set_input(pin, value, self.now)?;
        }
        Ok(())
    }

    /// Supplies one deterministic host-side sample to the ATSAMD21 ADC.
    ///
    /// The ADC conversion still starts only when guest firmware writes
    /// `SWTRIG.START`; this method models the external analog source without
    /// introducing host-dependent voltages or timing.
    pub fn set_adc_sample(&self, channel: u8, value: u16) -> Result<(), ArmMachineError> {
        let Some(adc) = &self.adc else {
            return Err(ArmMachineError::UnsupportedTarget(self.target));
        };
        adc.inject_sample(channel, value)?;
        Ok(())
    }

    /// Supplies one deterministic host-side analog code to the ATSAMD21 AC.
    pub fn set_ac_input(&self, input: u8, value: u16) -> Result<(), ArmMachineError> {
        let Some(ac) = &self.ac else {
            return Err(ArmMachineError::UnsupportedTarget(self.target));
        };
        ac.inject_input(input, value)?;
        Ok(())
    }

    /// Current vendor GPIO output latch.
    pub fn gpio_output(&self) -> u32 {
        self.gpio.output()
    }

    /// Returns the host-facing STM32 ADC1 sample handle.
    pub fn adc(&self) -> Option<Stm32AdcHandle> {
        self.stm32_adc.clone()
    }

    /// Returns the host-facing STM32 CRC state.
    pub fn crc(&self) -> Option<Stm32CrcHandle> {
        self.stm32_crc.clone()
    }

    /// Returns the host-facing STM32 RTC state.
    pub fn rtc(&self) -> Option<Stm32RtcHandle> {
        self.stm32_rtc.clone()
    }

    /// Returns the host-facing STM32 RNG state.
    pub fn rng(&self) -> Option<Stm32RngHandle> {
        self.stm32_rng.clone()
    }

    /// Loads bytes into the STM32L432 external QUADSPI flash window.
    pub fn qspi_load_flash(&self, offset: usize, bytes: &[u8]) -> Result<(), ArmMachineError> {
        let Some(qspi) = &self.qspi else {
            return Err(ArmMachineError::UnsupportedTarget(self.target));
        };
        if qspi.load_flash(offset, bytes) {
            Ok(())
        } else {
            Err(remu_bus::DeviceError::new("QUADSPI flash range is out of bounds").into())
        }
    }

    /// Returns a copy of the STM32L432 external QUADSPI flash.
    pub fn qspi_flash(&self) -> Option<Vec<u8>> {
        self.qspi.as_ref().map(Stm32QuadSpiHandle::flash)
    }

    /// Injects one STM32L432 SWPMI receive frame.
    pub fn inject_swpmi_rx(&self, word: u32, frame_bytes: u8) -> Result<(), ArmMachineError> {
        let Some(swpmi) = &self.swpmi else {
            return Err(ArmMachineError::UnsupportedTarget(self.target));
        };
        swpmi.inject_rx(word, frame_bytes, self.now);
        Ok(())
    }

    /// Takes words transmitted by the STM32L432 SWPMI endpoint.
    pub fn take_swpmi_tx(&self) -> Result<Vec<u32>, ArmMachineError> {
        let Some(swpmi) = &self.swpmi else {
            return Err(ArmMachineError::UnsupportedTarget(self.target));
        };
        Ok(swpmi.take_tx())
    }

    /// Supplies a deterministic touch-acquisition count to the STM32 TSC host.
    pub fn set_stm32_tsc_group_count(
        &self,
        group: usize,
        count: u32,
    ) -> Result<(), ArmMachineError> {
        let Some(tsc) = &self.tsc else {
            return Err(
                remu_bus::DeviceError::new("STM32 TSC is not available on this target").into(),
            );
        };
        if tsc.set_group_count(group, count) {
            Ok(())
        } else {
            Err(remu_bus::DeviceError::new("STM32 TSC group index is outside 0..7").into())
        }
    }

    /// Supplies host-side input levels to one STM32 comparator.
    pub fn set_stm32_comparator_inputs(
        &self,
        comparator: usize,
        plus: u16,
        minus: u16,
    ) -> Result<(), ArmMachineError> {
        let Some(comparators) = &self.comparators else {
            return Err(remu_bus::DeviceError::new(
                "STM32 comparators are not available on this target",
            )
            .into());
        };
        if comparators.set_inputs(comparator, plus, minus) {
            Ok(())
        } else {
            Err(remu_bus::DeviceError::new("STM32 comparator index is outside 0..2").into())
        }
    }

    /// Supplies host-side input levels to the STM32 OPAMP.
    pub fn set_stm32_opamp_inputs(&self, plus: u16, minus: u16) -> Result<(), ArmMachineError> {
        let Some(opamp) = &self.opamp else {
            return Err(
                remu_bus::DeviceError::new("STM32 OPAMP is not available on this target").into(),
            );
        };
        opamp.set_inputs(plus, minus);
        Ok(())
    }

    /// Reads guest-visible bytes for qualification and debugger adapters.
    pub fn debug_read_memory(&mut self, address: u64, length: usize) -> Result<Vec<u8>, String> {
        (0..length)
            .map(|offset| {
                self.bus
                    .read(
                        address + offset as u64,
                        AccessWidth::Byte,
                        AccessKind::Read,
                        self.now,
                    )
                    .map(|value| value as u8)
                    .map_err(|error| error.to_string())
            })
            .collect()
    }

    /// Writes guest-visible bytes for debugger adapters.
    pub fn debug_write_memory(&mut self, address: u64, bytes: &[u8]) -> Result<(), String> {
        for (offset, byte) in bytes.iter().copied().enumerate() {
            self.bus
                .write(
                    address.saturating_add(offset as u64),
                    AccessWidth::Byte,
                    u64::from(byte),
                    self.now,
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    /// Services one deterministic transfer unit on each active STM32 DMA channel.
    ///
    /// Normal `run` calls invoke this automatically. The explicit helper is
    /// useful for host-driven peripheral tests that do not need to boot a
    /// firmware image merely to exercise a memory transfer.
    pub fn service_stm32_dma(&mut self) -> Result<usize, ArmMachineError> {
        let mut serviced: usize = 0;
        if let Some(dma) = &self.dma1 {
            serviced = serviced.saturating_add(dma.service(&mut self.bus, self.now)?);
        }
        if let Some(dma) = &self.dma2 {
            serviced = serviced.saturating_add(dma.service(&mut self.bus, self.now)?);
        }
        Ok(serviced)
    }

    /// Runs without externally scheduled stimuli.
    pub fn run(
        &mut self,
        limits: RunLimits,
        trace: Option<&mut dyn TraceSink>,
    ) -> Result<RunResult, ArmMachineError> {
        self.run_with_stimuli(limits, &[], trace)
    }

    /// Runs with timestamped external GPIO stimulus.
    pub fn run_with_stimuli(
        &mut self,
        limits: RunLimits,
        stimuli: &[PinStimulus],
        mut trace: Option<&mut dyn TraceSink>,
    ) -> Result<RunResult, ArmMachineError> {
        if limits.instructions.is_none() && limits.deadline.is_none() {
            return Err(ArmMachineError::MissingRunLimit);
        }
        let mut digest = TraceDigest::new();
        self.signals.with_registry(|registry| {
            digest.begin(registry);
            trace
                .as_deref_mut()
                .map_or(Ok(()), |sink| sink.begin(registry))
        })?;
        let mut stats = RunStats {
            instructions: 0,
            time: self.now,
            events: 0,
        };
        let mut stimuli = stimuli.to_vec();
        stimuli.sort_by_key(|stimulus| stimulus.at);
        let mut next_stimulus = 0;
        let reason = loop {
            while stimuli
                .get(next_stimulus)
                .is_some_and(|stimulus| stimulus.at <= self.now)
            {
                let stimulus = stimuli[next_stimulus];
                self.set_pin(stimulus.pin, stimulus.value)?;
                stats.events = stats.events.saturating_add(1);
                next_stimulus += 1;
            }
            if self.exit.code().is_some() {
                break StopReason::Halted;
            }
            if limits
                .instructions
                .is_some_and(|limit| stats.instructions >= limit)
            {
                break StopReason::InstructionLimit;
            }
            if limits.deadline.is_some_and(|deadline| self.now >= deadline) {
                break StopReason::TimeLimit;
            }
            if self.breakpoints.contains(&self.cpu.snapshot().pc) {
                break StopReason::Breakpoint;
            }

            if self
                .watchdog
                .as_ref()
                .is_some_and(|watchdog| watchdog.take_reset(self.now))
            {
                self.bus.reset_devices(ResetKind::Watchdog);
                if let Err(error) = self.cpu.reset(ResetKind::Watchdog, &mut self.bus) {
                    break StopReason::Fault(error.to_string());
                }
                stats.events = stats.events.saturating_add(1);
                continue;
            }

            let dma_events = self.service_stm32_dma()?;
            stats.events = stats
                .events
                .saturating_add(u64::try_from(dma_events).unwrap_or(u64::MAX));

            let wwdg_early = if let Some(wwdg) = &self.stm32_wwdg {
                let (early, reset) = wwdg.poll(self.now);
                if reset {
                    self.bus.reset_devices(ResetKind::Watchdog);
                    if let Err(error) = self.cpu.reset(ResetKind::Watchdog, &mut self.bus) {
                        break StopReason::Fault(error.to_string());
                    }
                    stats.events = stats.events.saturating_add(1);
                    continue;
                }
                early
            } else {
                false
            };
            let (timer_line, timer_pending) = self.timer.poll(self.now);
            let advanced_timer_pending = self
                .stm32_tim1
                .as_ref()
                .is_some_and(|timer| timer.poll(self.now));
            let tim6_pending = self
                .stm32_tim6
                .as_ref()
                .is_some_and(|timer| timer.poll(self.now));
            let tim7_pending = self
                .stm32_tim7
                .as_ref()
                .is_some_and(|timer| timer.poll(self.now));
            let tim15_pending = self
                .stm32_tim15
                .as_ref()
                .is_some_and(|timer| timer.poll(self.now));
            let tim16_pending = self
                .stm32_tim16
                .as_ref()
                .is_some_and(|timer| timer.poll(self.now));
            let lptim1_pending = self
                .stm32_lptim1
                .as_ref()
                .is_some_and(|timer| timer.poll(self.now));
            let lptim2_pending = self
                .stm32_lptim2
                .as_ref()
                .is_some_and(|timer| timer.poll(self.now));
            let usb_pending = self.usb_fs.as_ref().is_some_and(|usb| usb.poll(self.now));
            let sai_pending = self.sai1.as_ref().is_some_and(|sai| sai.poll(self.now));
            let qspi_pending = self
                .qspi
                .as_ref()
                .is_some_and(Stm32QuadSpiHandle::interrupt_pending);
            let swpmi_pending = self
                .swpmi
                .as_ref()
                .is_some_and(Stm32SwpmiHandle::interrupt_pending);
            let tsc_pending = self
                .tsc
                .as_ref()
                .is_some_and(Stm32TscHandle::interrupt_pending);
            let comparator_pending = self
                .comparators
                .as_ref()
                .is_some_and(Stm32ComparatorHandle::interrupt_pending);
            let compiler_pending = self.compiler_timer.poll(self.now);
            let mut interrupt_requested = timer_pending
                || advanced_timer_pending
                || tim6_pending
                || tim7_pending
                || tim15_pending
                || tim16_pending
                || lptim1_pending
                || lptim2_pending
                || usb_pending
                || sai_pending
                || qspi_pending
                || swpmi_pending
                || tsc_pending
                || comparator_pending
                || wwdg_early;
            let package_inputs = (0..self.gpio.pin_count().min(16)).fold(0_u32, |value, pin| {
                let pin = u8::try_from(pin).expect("pin index fits u8");
                value | (u32::from(self.gpio.resolved(pin) == Ok(Logic::One)) << pin)
            });
            if let Some(eic) = &self.eic {
                let eic_pending = eic.poll(package_inputs);
                interrupt_requested |= eic_pending;
                self.cpu
                    .set_interrupt(4, eic_pending && self.ppb.interrupt_enabled(4))?;
            }
            if let Some(exti) = &self.stm32_exti {
                let exti_pending = exti.poll(package_inputs, self.now);
                interrupt_requested |= exti_pending != 0;
                for line in 0..5_u32 {
                    let irq = 6 + line as u16;
                    let pending = exti_pending & (1 << line) != 0;
                    self.cpu
                        .set_interrupt(irq, pending && self.ppb.interrupt_enabled(irq))?;
                }
                self.cpu.set_interrupt(
                    23,
                    exti_pending & 0x03e0 != 0 && self.ppb.interrupt_enabled(23),
                )?;
                self.cpu.set_interrupt(
                    40,
                    exti_pending & 0xfc00 != 0 && self.ppb.interrupt_enabled(40),
                )?;
            }
            for (line, sercom) in &self.samd_sercom_irqs {
                let pending = sercom.interrupt_pending();
                interrupt_requested |= pending;
                self.cpu
                    .set_interrupt(*line, pending && self.ppb.interrupt_enabled(*line))?;
            }
            for (line, timer) in &self.samd_tc_irqs {
                let pending = timer.poll(self.now);
                interrupt_requested |= pending;
                self.cpu
                    .set_interrupt(*line, pending && self.ppb.interrupt_enabled(*line))?;
            }
            for (line, tcc) in &self.samd_tcc_irqs {
                let pending = tcc.poll(self.now)?;
                interrupt_requested |= pending;
                self.cpu
                    .set_interrupt(*line, pending && self.ppb.interrupt_enabled(*line))?;
            }
            if let Some(rtc) = &self.samd_rtc {
                let pending = rtc.poll(self.now);
                interrupt_requested |= pending;
                self.cpu
                    .set_interrupt(3, pending && self.ppb.interrupt_enabled(3))?;
            }
            if let Some(dmac) = &self.dmac {
                let dmac_pending = dmac.interrupt_pending();
                interrupt_requested |= dmac_pending;
                self.cpu
                    .set_interrupt(6, dmac_pending && self.ppb.interrupt_enabled(6))?;
            }
            if let Some(i2s) = &self.i2s {
                let i2s_pending = i2s.interrupt_pending();
                interrupt_requested |= i2s_pending;
                self.cpu
                    .set_interrupt(27, i2s_pending && self.ppb.interrupt_enabled(27))?;
            }
            if let Some(adc) = &self.adc {
                let adc_pending = adc.interrupt_pending();
                interrupt_requested |= adc_pending;
                self.cpu
                    .set_interrupt(23, adc_pending && self.ppb.interrupt_enabled(23))?;
            }
            if let Some(ac) = &self.ac {
                let ac_pending = ac.poll(self.now)?;
                interrupt_requested |= ac_pending;
                self.cpu
                    .set_interrupt(24, ac_pending && self.ppb.interrupt_enabled(24))?;
            }
            if let Some(dac) = &self.dac {
                let dac_pending = dac.interrupt_pending();
                interrupt_requested |= dac_pending;
                self.cpu
                    .set_interrupt(25, dac_pending && self.ppb.interrupt_enabled(25))?;
            }
            if let Some(timer_line) = timer_line {
                self.cpu.set_interrupt(
                    timer_line,
                    timer_pending && self.ppb.interrupt_enabled(timer_line),
                )?;
            } else if timer_pending {
                if let Some(icu) = &self.ra_icu {
                    for line in icu.route_event(RA4M1_EVENT_GPT0_OVERFLOW) {
                        self.cpu
                            .set_interrupt(line, self.ppb.interrupt_enabled(line))?;
                    }
                }
            }
            if self.target == TargetId::Stm32l432kc {
                self.cpu.set_interrupt(
                    25,
                    (advanced_timer_pending || tim16_pending) && self.ppb.interrupt_enabled(25),
                )?;
                self.cpu
                    .set_interrupt(54, tim6_pending && self.ppb.interrupt_enabled(54))?;
                self.cpu
                    .set_interrupt(55, tim7_pending && self.ppb.interrupt_enabled(55))?;
                self.cpu
                    .set_interrupt(24, tim15_pending && self.ppb.interrupt_enabled(24))?;
                self.cpu
                    .set_interrupt(65, lptim1_pending && self.ppb.interrupt_enabled(65))?;
                self.cpu
                    .set_interrupt(66, lptim2_pending && self.ppb.interrupt_enabled(66))?;
                self.cpu
                    .set_interrupt(67, usb_pending && self.ppb.interrupt_enabled(67))?;
                self.cpu
                    .set_interrupt(74, sai_pending && self.ppb.interrupt_enabled(74))?;
                self.cpu
                    .set_interrupt(71, qspi_pending && self.ppb.interrupt_enabled(71))?;
                self.cpu
                    .set_interrupt(76, swpmi_pending && self.ppb.interrupt_enabled(76))?;
                self.cpu
                    .set_interrupt(77, tsc_pending && self.ppb.interrupt_enabled(77))?;
                self.cpu
                    .set_interrupt(64, comparator_pending && self.ppb.interrupt_enabled(64))?;
            }
            let mut dma_pending = false;
            const DMA1_IRQS: [u16; 7] = [11, 12, 13, 14, 15, 16, 17];
            const DMA2_IRQS: [u16; 7] = [56, 57, 58, 59, 60, 68, 69];
            for (dma, lines) in [(&self.dma1, &DMA1_IRQS), (&self.dma2, &DMA2_IRQS)] {
                if let Some(dma) = dma {
                    for (index, line) in lines.iter().copied().enumerate() {
                        let pending = dma.channel_pending(index);
                        dma_pending |= pending;
                        self.cpu
                            .set_interrupt(line, pending && self.ppb.interrupt_enabled(line))?;
                    }
                }
            }
            interrupt_requested |= dma_pending;
            for (line, i2c) in &self.stm32_i2c {
                let pending = i2c.interrupt_pending();
                interrupt_requested |= pending;
                self.cpu
                    .set_interrupt(*line, pending && self.ppb.interrupt_enabled(*line))?;
            }
            match self.target {
                TargetId::Atsamd21e18 => {
                    let uart_line = 9;
                    let uart_pending = self.uart.interrupt_pending();
                    interrupt_requested |= uart_pending;
                    self.cpu.set_interrupt(
                        uart_line,
                        uart_pending && self.ppb.interrupt_enabled(uart_line),
                    )?;
                }
                TargetId::Stm32l432kc => {
                    let VendorUart::Stm32(handles) = &self.uart else {
                        unreachable!("STM32 target always has STM32 USART handles")
                    };
                    for (handle, uart_line) in handles {
                        let uart_pending = handle.interrupt_pending();
                        interrupt_requested |= uart_pending;
                        self.cpu.set_interrupt(
                            *uart_line,
                            uart_pending && self.ppb.interrupt_enabled(*uart_line),
                        )?;
                    }
                }
                TargetId::R7fa4m1ab3cfm if self.uart.interrupt_pending() => {
                    interrupt_requested = true;
                    if let Some(icu) = &self.ra_icu {
                        for line in icu.route_event(RA4M1_EVENT_SCI9_TXI) {
                            self.cpu
                                .set_interrupt(line, self.ppb.interrupt_enabled(line))?;
                        }
                    }
                }
                TargetId::R7fa4m1ab3cfm => {}
                _ => unreachable!(),
            }
            for (line, spi) in &self.stm32_spi {
                let pending = spi.interrupt_pending();
                interrupt_requested |= pending;
                self.cpu
                    .set_interrupt(*line, pending && self.ppb.interrupt_enabled(*line))?;
            }
            self.signals.set(
                self.timer_irq_signal,
                SignalValue::from_u64(
                    u64::from(
                        timer_pending
                            || advanced_timer_pending
                            || tim6_pending
                            || tim7_pending
                            || tim15_pending
                            || tim16_pending
                            || lptim1_pending
                            || lptim2_pending,
                    ),
                    1,
                )?,
                self.now,
            )?;
            self.signals.set(
                self.interrupt_signal,
                SignalValue::from_u64(u64::from(interrupt_requested), 1)?,
                self.now,
            )?;
            self.cpu.set_interrupt(
                0,
                compiler_pending || (wwdg_early && self.ppb.interrupt_enabled(0)),
            )?;
            if self.ppb.take_systick_pending(self.now) {
                self.cpu.set_systick_interrupt(true);
            }
            for line in self.ppb.take_pending_interrupts() {
                self.cpu.set_interrupt(line, true)?;
            }
            let vector_base = self.ppb.vector_base();
            if vector_base != 0 {
                self.cpu.set_vector_base(vector_base);
            }

            self.bus.clear_watchpoint_hit();
            let outcome = match self.cpu.step(&mut self.bus, self.now) {
                Ok(outcome) => outcome,
                Err(error) => break StopReason::Fault(error.to_string()),
            };
            stats.instructions = stats.instructions.saturating_add(1);
            self.now = self
                .now
                .checked_add(outcome.elapsed)
                .map_err(|_| ArmMachineError::TimeOverflow)?;
            stats.time = self.now;
            if let Some(dmac) = &self.dmac {
                if dmac.service(&mut self.bus, self.now) {
                    stats.events = stats.events.saturating_add(1);
                }
            }
            let uart = self.uart.bytes();
            for byte in uart.iter().skip(self.traced_uart_len) {
                self.uart_strobe = !self.uart_strobe;
                self.signals.set(
                    self.uart_byte_signal,
                    SignalValue::from_u64(u64::from(*byte), 8)?,
                    self.now,
                )?;
                self.signals.set(
                    self.uart_strobe_signal,
                    SignalValue::from_u64(u64::from(self.uart_strobe), 1)?,
                    self.now,
                )?;
            }
            self.traced_uart_len = uart.len();
            let mut signal_stop = None;
            for change in self.signals.drain_changes() {
                signal_stop =
                    signal_stop.or_else(|| matching_signal_stop(&change, &self.signal_stops));
                digest.change(&change);
                if let Some(sink) = trace.as_deref_mut() {
                    sink.change(&change)?;
                }
            }
            if let Some(path) = signal_stop {
                break StopReason::Signal(path);
            }
            if let Some(hit) = self.bus.take_watchpoint_hit() {
                break StopReason::Watchpoint {
                    address: hit.address,
                    access: hit.kind,
                };
            }
            match outcome.reason {
                StepReason::Advanced | StepReason::WaitForInterrupt => {}
                StepReason::Halted => break StopReason::Halted,
                StepReason::Breakpoint => break StopReason::Breakpoint,
            }
        };
        if let Some(sink) = trace {
            sink.finish()?;
        }
        let mut uart = self.compiler_uart.bytes();
        uart.extend(self.uart.bytes());
        Ok(RunResult {
            target: self.target,
            reason,
            stats,
            cpu: self.cpu.snapshot(),
            secondary_cpu: None,
            exit_code: self.exit.code(),
            uart,
            usb: Vec::new(),
            trace_digest: digest.finish(),
        })
    }
}

#[path = "arm_mcu_maps.rs"]
mod maps;

#[cfg(test)]
#[path = "arm_mcu_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "arm_mcu_samd_instance_tests.rs"]
mod samd_instance_tests;

#[cfg(test)]
#[path = "arm_mcu_stm32_extended_tests.rs"]
mod stm32_extended_tests;
