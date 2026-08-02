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
    FunctionalUart, GpioHandle, RA4M1_EVENT_GPT0_OVERFLOW, RA4M1_EVENT_GPT3_OVERFLOW,
    RA4M1_EVENT_SCI9_TXI, RaGpt, RaGptHandle, RaIcu, RaIcuHandle, RaIoPort, RaPfs, RaSci,
    RaSciHandle, RegisterBank, Samd21Eic, Samd21EicHandle, Samd21Port, Samd21RegisterBlock,
    Samd21Tc, Samd21TcHandle, Samd21Usart, Samd21UsartHandle, Samd21Wdt, Samd21WdtHandle,
    SignalHub, Stm32Gpio, Stm32Timer, Stm32TimerHandle, Stm32Usart, Stm32UsartHandle, TimerHandle,
    UartHandle,
};
use remu_image::{FirmwareArchitecture, FirmwareImage};
use remu_signals::{Logic, SignalId, SignalValue};
use remu_trace::{TraceDigest, TraceSink};
use std::collections::BTreeSet;

const TEST_DEVICE_SIZE: usize = 0x100;
const TEST_EXIT_SIZE: usize = 4;

enum VendorUart {
    Samd21(Samd21UsartHandle),
    Stm32(Stm32UsartHandle),
    Ra4m1(RaSciHandle),
}

impl VendorUart {
    fn bytes(&self) -> Vec<u8> {
        match self {
            Self::Samd21(handle) => handle.bytes(),
            Self::Stm32(handle) => handle.bytes(),
            Self::Ra4m1(handle) => handle.bytes(),
        }
    }

    fn interrupt_pending(&self) -> bool {
        match self {
            Self::Samd21(handle) => handle.interrupt_pending(),
            Self::Stm32(handle) => handle.interrupt_pending(),
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
    ra_gpt3: Option<RaGptHandle>,
    ra_gpt3_irq_signal: Option<SignalId>,
    eic: Option<Samd21EicHandle>,
    ra_icu: Option<RaIcuHandle>,
    watchdog: Option<Samd21WdtHandle>,
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
        for region in manifest.memory {
            match region.kind {
                MemoryKind::Ram => {
                    bus.map_ram(region.name, region.start, region.size, region.executable)?;
                    let end =
                        region.start + u64::try_from(region.size).expect("memory size fits u64");
                    default_stack = Some(u32::try_from(end).expect("Arm memory fits u32"));
                }
                MemoryKind::Flash | MemoryKind::Rom => {
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
        let ra_gpt3_irq_signal = (target == TargetId::R7fa4m1ab3cfm)
            .then(|| {
                signals.declare(
                    "board.r7fa4m1ab3cfm.gpt3.irq",
                    SignalValue::from_u64(0, 1).expect("one-bit signal is valid"),
                    Some("functional GPT3 overflow request".to_owned()),
                )
            })
            .transpose()?;
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

        let (gpio, uart, timer, ra_gpt3, eic, ra_icu, watchdog) = match target {
            TargetId::Atsamd21e18 => {
                let (port_device, gpio) = Samd21Port::new(
                    "atsamd21e18.porta",
                    26,
                    "board.atsamd21e18.porta",
                    signals.clone(),
                )?;
                let (tc3_device, timer) = Samd21Tc::new("atsamd21e18.tc3");
                let (eic_device, eic) = Samd21Eic::new("atsamd21e18.eic");
                let (watchdog_device, watchdog) = Samd21Wdt::new("atsamd21e18.wdt");
                let (sercom0_device, uart) = Samd21Usart::new("atsamd21e18.sercom0");
                Self::map_samd21(
                    &mut bus,
                    port_device,
                    eic_device,
                    watchdog_device,
                    tc3_device,
                    sercom0_device,
                )?;
                (
                    gpio,
                    VendorUart::Samd21(uart),
                    VendorTimer::Samd21(timer),
                    None,
                    Some(eic),
                    None,
                    Some(watchdog),
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
                let (usart2_device, uart) = Stm32Usart::new("stm32l432kc.usart2");
                Self::map_stm32l432(
                    &mut bus,
                    [gpioa_device, gpiob_device, gpioc_device, gpioh_device],
                    tim2_device,
                    usart2_device,
                )?;
                (
                    gpio,
                    VendorUart::Stm32(uart),
                    VendorTimer::Stm32(timer),
                    None,
                    None,
                    None,
                    None,
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
                let (gpt3_device, gpt3) = RaGpt::new("r7fa4m1ab3cfm.gpt3");
                let (sci9_device, uart) = RaSci::new("r7fa4m1ab3cfm.sci9");
                let (icu_device, icu) = RaIcu::new("r7fa4m1ab3cfm.icu");
                Self::map_ra4m1(
                    &mut bus,
                    ports,
                    pfs,
                    icu_device,
                    gpt0_device,
                    gpt3_device,
                    sci9_device,
                )?;
                (
                    handles.remove(1),
                    VendorUart::Ra4m1(uart),
                    VendorTimer::Ra4m1(timer),
                    Some(gpt3),
                    None,
                    Some(icu),
                    None,
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
            ra_gpt3,
            ra_gpt3_irq_signal,
            eic,
            ra_icu,
            watchdog,
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

    fn map_samd21(
        bus: &mut AddressSpace,
        port: Samd21Port,
        eic: Samd21Eic,
        watchdog: Samd21Wdt,
        tc3: Samd21Tc,
        sercom0: Samd21Usart,
    ) -> Result<(), remu_bus::MapError> {
        bus.map_device(
            "atsamd21e18.pm",
            0x4000_0400,
            0x100,
            Box::new(Samd21RegisterBlock::new("atsamd21e18.pm", 0x100, [])),
        )?;
        // PCLKSR.OSC8MRDY and DFLLRDY are asserted in the functional clock model.
        bus.map_device(
            "atsamd21e18.sysctrl",
            0x4000_0800,
            0x100,
            Box::new(Samd21RegisterBlock::new(
                "atsamd21e18.sysctrl",
                0x100,
                [(0x0c, 0x18)],
            )),
        )?;
        bus.map_device(
            "atsamd21e18.gclk",
            0x4000_0c00,
            0x100,
            Box::new(Samd21RegisterBlock::new("atsamd21e18.gclk", 0x100, [])),
        )?;
        bus.map_device("atsamd21e18.wdt", 0x4000_1000, 0x100, Box::new(watchdog))?;
        bus.map_device("atsamd21e18.eic", 0x4000_1800, 0x100, Box::new(eic))?;
        bus.map_device("atsamd21e18.sercom0", 0x4200_0800, 0x40, Box::new(sercom0))?;
        bus.map_device("atsamd21e18.tc3", 0x4200_2c00, 0x40, Box::new(tc3))?;
        // NVMCTRL.INTFLAG.READY is set after reset.
        bus.map_device(
            "atsamd21e18.nvmctrl",
            0x4100_4000,
            0x400,
            Box::new(Samd21RegisterBlock::new(
                "atsamd21e18.nvmctrl",
                0x400,
                [(0x14, 1)],
            )),
        )?;
        bus.map_device("atsamd21e18.port", 0x4100_4400, 0x100, Box::new(port))?;
        Ok(())
    }

    fn map_stm32l432(
        bus: &mut AddressSpace,
        gpio: [Stm32Gpio; 4],
        tim2: Stm32Timer,
        usart2: Stm32Usart,
    ) -> Result<(), remu_bus::MapError> {
        bus.map_device(
            "stm32l432kc.rcc",
            0x4002_1000,
            0x400,
            Box::new(RegisterBank::new(
                "stm32l432kc.rcc",
                [
                    (0x00, 0x0000_0063, u32::MAX),
                    (0x08, 0x0000_1000, u32::MAX),
                    (0x4c, 0, u32::MAX),
                    (0x58, 0, u32::MAX),
                    (0x5c, 0, u32::MAX),
                    (0x88, 0, u32::MAX),
                ],
            )),
        )?;
        bus.map_device(
            "stm32l432kc.pwr",
            0x4000_7000,
            0x400,
            Box::new(RegisterBank::new(
                "stm32l432kc.pwr",
                [(0x00, 0x0000_0200, u32::MAX), (0x14, 0, u32::MAX)],
            )),
        )?;
        bus.map_device(
            "stm32l432kc.flash-control",
            0x4002_2000,
            0x400,
            Box::new(RegisterBank::new(
                "stm32l432kc.flash-control",
                [(0x00, 0x0000_0600, u32::MAX), (0x08, 0, u32::MAX)],
            )),
        )?;
        bus.map_device(
            "stm32l432kc.syscfg",
            0x4001_0000,
            0x400,
            Box::new(RegisterBank::new(
                "stm32l432kc.syscfg",
                [
                    (0x00, 0, u32::MAX),
                    (0x08, 0, u32::MAX),
                    (0x0c, 0, u32::MAX),
                ],
            )),
        )?;
        bus.map_device(
            "stm32l432kc.exti",
            0x4001_0400,
            0x400,
            Box::new(RegisterBank::new(
                "stm32l432kc.exti",
                [
                    (0x00, 0, u32::MAX),
                    (0x08, 0, u32::MAX),
                    (0x0c, 0, u32::MAX),
                    (0x14, 0, u32::MAX),
                ],
            )),
        )?;
        bus.map_device("stm32l432kc.tim2", 0x4000_0000, 0x400, Box::new(tim2))?;
        bus.map_device("stm32l432kc.usart2", 0x4000_4400, 0x400, Box::new(usart2))?;
        let [gpioa, gpiob, gpioc, gpioh] = gpio;
        bus.map_device("stm32l432kc.gpioa", 0x4800_0000, 0x400, Box::new(gpioa))?;
        bus.map_device("stm32l432kc.gpiob", 0x4800_0400, 0x400, Box::new(gpiob))?;
        bus.map_device("stm32l432kc.gpioc", 0x4800_0800, 0x400, Box::new(gpioc))?;
        bus.map_device("stm32l432kc.gpioh", 0x4800_1c00, 0x400, Box::new(gpioh))?;
        Ok(())
    }

    fn map_ra4m1(
        bus: &mut AddressSpace,
        ports: Vec<RaIoPort>,
        pfs: RaPfs,
        icu: RaIcu,
        gpt0: RaGpt,
        gpt3: RaGpt,
        sci9: RaSci,
    ) -> Result<(), remu_bus::MapError> {
        // Functional clock/reset surface. OSCSF reports the reset-selected HOCO stable.
        bus.map_device(
            "r7fa4m1ab3cfm.system",
            0x4001_e000,
            0x1000,
            Box::new(Samd21RegisterBlock::new(
                "r7fa4m1ab3cfm.system",
                0x1000,
                [(0x3c, 1)],
            )),
        )?;
        bus.map_device(
            "r7fa4m1ab3cfm.mstp",
            0x4004_6ffc,
            0x20,
            Box::new(Samd21RegisterBlock::new("r7fa4m1ab3cfm.mstp", 0x20, [])),
        )?;
        bus.map_device("r7fa4m1ab3cfm.icu", 0x4000_6000, 0x480, Box::new(icu))?;
        bus.map_device("r7fa4m1ab3cfm.gpt0", 0x4007_8000, 0x100, Box::new(gpt0))?;
        bus.map_device("r7fa4m1ab3cfm.gpt3", 0x4007_8300, 0x100, Box::new(gpt3))?;
        bus.map_device("r7fa4m1ab3cfm.sci9", 0x4007_0120, 0x20, Box::new(sci9))?;
        bus.map_device("r7fa4m1ab3cfm.pfs", 0x4004_0800, 0x3c0, Box::new(pfs))?;
        bus.map_device(
            "r7fa4m1ab3cfm.pmisc",
            0x4004_0d00,
            0x100,
            Box::new(Samd21RegisterBlock::new(
                "r7fa4m1ab3cfm.pmisc",
                0x100,
                [(3, 0x80)],
            )),
        )?;
        for (port, device) in ports.into_iter().enumerate() {
            bus.map_device(
                format!("r7fa4m1ab3cfm.port{port}"),
                0x4004_0000 + u64::try_from(port).expect("port index fits u64") * 0x20,
                0x10,
                Box::new(device),
            )?;
        }
        // Functional WDT startup surface; refresh/timeout fidelity is deferred to the target report.
        bus.map_device(
            "r7fa4m1ab3cfm.wdt",
            0x4004_4200,
            0x10,
            Box::new(Samd21RegisterBlock::new("r7fa4m1ab3cfm.wdt", 0x10, [])),
        )?;
        Ok(())
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

    /// Current vendor GPIO output latch.
    pub fn gpio_output(&self) -> u32 {
        self.gpio.output()
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

            let (timer_line, timer_pending) = self.timer.poll(self.now);
            let compiler_pending = self.compiler_timer.poll(self.now);
            let mut interrupt_requested = timer_pending;
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
            if let (Some(gpt3), Some(icu), Some(signal)) =
                (&self.ra_gpt3, &self.ra_icu, self.ra_gpt3_irq_signal)
            {
                let gpt3_pending = gpt3.poll(self.now);
                interrupt_requested |= gpt3_pending;
                if gpt3_pending {
                    for line in icu.route_event(RA4M1_EVENT_GPT3_OVERFLOW) {
                        self.cpu
                            .set_interrupt(line, self.ppb.interrupt_enabled(line))?;
                    }
                }
                self.signals.set(
                    signal,
                    SignalValue::from_u64(u64::from(gpt3_pending), 1)?,
                    self.now,
                )?;
            }
            match self.target {
                TargetId::Atsamd21e18 | TargetId::Stm32l432kc => {
                    let uart_line = if self.target == TargetId::Atsamd21e18 {
                        9
                    } else {
                        38
                    };
                    let uart_pending = self.uart.interrupt_pending();
                    interrupt_requested |= uart_pending;
                    self.cpu.set_interrupt(
                        uart_line,
                        uart_pending && self.ppb.interrupt_enabled(uart_line),
                    )?;
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
            self.signals.set(
                self.timer_irq_signal,
                SignalValue::from_u64(u64::from(timer_pending), 1)?,
                self.now,
            )?;
            self.signals.set(
                self.interrupt_signal,
                SignalValue::from_u64(u64::from(interrupt_requested), 1)?,
                self.now,
            )?;
            self.cpu.set_interrupt(0, compiler_pending)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use remu_image::FirmwareSegment;

    #[test]
    fn samd21_firmware_drives_porta_and_produces_a_trace() {
        let mut code = vec![
            0x02, 0x48, 0x80, 0x21, 0x81, 0x60, 0x81, 0x61, 0x30, 0xbf, 0, 0,
        ];
        code.extend_from_slice(&0x4100_4400_u32.to_le_bytes());
        let image = FirmwareImage {
            architecture: FirmwareArchitecture::Arm,
            entry: 1,
            segments: vec![FirmwareSegment {
                address: 0,
                load_address: None,
                initialized_size: code.len(),
                data: code,
                executable: true,
                writable: false,
                alignment: 4,
            }],
            symbols: Vec::new(),
        };
        let mut machine = ArmMcuMachine::new(TargetId::Atsamd21e18).unwrap();
        machine.load_firmware(&image).unwrap();
        let result = machine
            .run_with_stimuli(
                RunLimits {
                    instructions: Some(4),
                    deadline: None,
                },
                &[],
                None,
            )
            .unwrap();
        assert_eq!(machine.gpio_output(), 1 << 7);
        assert_eq!(result.reason, StopReason::InstructionLimit);
        assert_ne!(result.trace_digest, "");
    }

    #[test]
    fn stm32l432_uses_the_distinct_m4f_profile_and_gpioa_bsrr() {
        let mut machine = ArmMcuMachine::new(TargetId::Stm32l432kc).unwrap();
        assert_eq!(machine.cpu.profile(), ArmProfile::CortexM4F);
        machine
            .bus
            .write(0x4800_0000, AccessWidth::Word, 1 << 10, SimTime::ZERO)
            .unwrap();
        machine
            .bus
            .write(0x4800_0018, AccessWidth::Word, 1 << 5, SimTime::ZERO)
            .unwrap();
        assert_eq!(machine.gpio_output(), 1 << 5);
    }

    #[test]
    fn ra4m1_uses_m4f_and_its_own_ioport_and_icu_map() {
        let mut machine = ArmMcuMachine::new(TargetId::R7fa4m1ab3cfm).unwrap();
        assert_eq!(machine.cpu.profile(), ArmProfile::CortexM4F);
        machine
            .bus
            .write(0x4004_0020, AccessWidth::Word, 1 << 11, SimTime::ZERO)
            .unwrap();
        machine
            .bus
            .write(0x4004_0028, AccessWidth::Word, 1 << 11, SimTime::ZERO)
            .unwrap();
        assert_eq!(machine.gpio_output(), 1 << 11);
        machine
            .bus
            .write(
                0x4000_6300,
                AccessWidth::Word,
                u64::from(RA4M1_EVENT_GPT0_OVERFLOW),
                SimTime::ZERO,
            )
            .unwrap();
        machine
            .bus
            .write(
                0x4000_6320,
                AccessWidth::Word,
                u64::from(RA4M1_EVENT_GPT3_OVERFLOW),
                SimTime::ZERO,
            )
            .unwrap();
        machine
            .bus
            .write(0x4007_8364, AccessWidth::Word, 3, SimTime::ZERO)
            .unwrap();
        machine
            .bus
            .write(0x4007_8338, AccessWidth::Word, 1 << 6, SimTime::ZERO)
            .unwrap();
        machine
            .bus
            .write(0x4007_832c, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            machine
                .bus
                .read(
                    0x4007_8364,
                    AccessWidth::Word,
                    AccessKind::Read,
                    SimTime::ZERO
                )
                .unwrap(),
            3
        );
    }
}
