use crate::HOST_SCRIPT_COMPLETE_MARKER;
use crate::riscv::{TEST_DEVICE_SIZE, TEST_EXIT_SIZE};
use crate::{
    MemoryKind, PinStimulus, RunResult, SignalEdge, SignalStop, TEST_EXIT, TEST_GPIO, TEST_TIMER,
    TEST_UART, TargetId, matching_signal_stop, resolve_signal_stop, target_manifest,
};
use md5::{Digest, Md5};
use remu_bus::{
    AddressSpace, Endianness, MapError, Permissions, SharedBusAccessObserver, SharedMemory,
};
use remu_core::{
    AccessKind, AccessWidth, Bus, Cpu, CpuFault, RunLimits, RunStats, SimTime, StepReason,
    StopReason,
};
use remu_cpu_xtensa::{XtensaCpu, XtensaRegister};
use remu_devices::{
    DeterministicRng, Esp32S3Aes, Esp32S3AesHandle, Esp32S3Extmem, Esp32S3ExtmemHandle,
    Esp32S3IoMux, Esp32S3IoMuxHandle, Esp32S3LcdCam, Esp32S3LcdCamHandle, Esp32S3Ledc,
    Esp32S3LedcHandle, Esp32S3Mcpwm, Esp32S3McpwmHandle, Esp32S3Pcnt, Esp32S3PcntHandle,
    Esp32S3Pms, Esp32S3PmsHandle, Esp32S3SarAdc, Esp32S3SarAdcHandle, Esp32S3Sdmmc,
    Esp32S3SdmmcHandle, Esp32S3Sha, Esp32S3ShaHandle, Esp32S3Syscon, Esp32S3SysconHandle,
    Esp32S3Tsens, Esp32S3TsensHandle, Esp32S3Uhci, Esp32S3UhciHandle, Esp32S3UsbWrap,
    Esp32S3UsbWrapHandle, Esp32S3WorldController, Esp32S3WorldControllerHandle, Esp32S3XtsAes,
    Esp32s3I2c, Esp32s3I2s, Esp32s3Rmt, Esp32s3Spi, EspDigitalSignature, EspEfuse, EspGdma,
    EspGdmaHandle, EspGpio, EspHmac, EspInterruptMatrix, EspInterruptMatrixHandle, EspMmuTable,
    EspMmuTableHandle, EspRsa, EspRtcControl, EspRtcControlHandle, EspSpiMem, EspSystem,
    EspSystemHandle, EspSystimer, EspSystimerHandle, EspTimerGroup, EspTimerGroupHandle,
    EspTimerGroupKind, EspTwai, EspTwaiHandle, EspUsbOtg, EspUsbOtgHandle, EspUsbSerialJtag,
    EspUsbSerialJtagHandle, ExitDevice, ExitHandle, FunctionalGpio, FunctionalTimer,
    FunctionalUart, GpioHandle, SignalHub, TimerHandle, UartHandle,
};
use remu_image::{EspFlashImage, FirmwareArchitecture, FirmwareImage};
use remu_signals::{Logic, SignalError};
use remu_trace::{TraceDigest, TraceError, TraceSink};
use sha2::{Sha224, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use thiserror::Error;
mod functional_rom;
mod interrupts;
mod peripheral_dma;
mod peripheral_handles;
mod pms;
/// ESP32-S3 machine construction or execution failure.
#[derive(Debug, Error)]
pub enum XtensaMachineError {
    /// Only ESP32-S3 uses this initial LX7 machine.
    #[error("target {0} does not have the runnable Xtensa LX7 profile")]
    UnsupportedTarget(TargetId),
    /// Address map construction failed.
    #[error(transparent)]
    Map(#[from] MapError),
    /// CPU operation failed.
    #[error(transparent)]
    Cpu(#[from] CpuFault),
    /// Signal construction failed.
    #[error(transparent)]
    Signal(#[from] SignalError),
    /// Host peripheral operation failed.
    #[error(transparent)]
    Device(#[from] remu_bus::DeviceError),
    /// Trace output failed.
    #[error(transparent)]
    Trace(#[from] TraceError),
    /// Firmware has the wrong architecture.
    #[error("firmware architecture {0:?} does not match ESP32-S3 Xtensa")]
    Architecture(FirmwareArchitecture),
    /// Entry exceeds 32-bit address space.
    #[error("firmware entry {0:#x} exceeds the Xtensa address space")]
    EntryRange(u64),
    /// Segment is outside the direct-load map.
    #[error("cannot load firmware segment at {address:#x}: {message}")]
    Load {
        /// Segment start.
        address: u64,
        /// Bus diagnostic.
        message: String,
    },
    /// Runs must be bounded.
    #[error("at least one run limit is required")]
    MissingRunLimit,
    /// Virtual time overflowed.
    #[error("simulation time overflow")]
    TimeOverflow,
}
mod usb_host;
use usb_host::{EspDwc2Host, FunctionalSha256, appcpu_systimer_level};
/// Runnable direct-ELF ESP32-S3 CPU0/unicore slice.
pub struct XtensaMachine {
    cpu: XtensaCpu,
    cpu1: XtensaCpu,
    bus: AddressSpace,
    signals: SignalHub,
    gpio: GpioHandle,
    chip_gpio: GpioHandle,
    pub(crate) uart: UartHandle,
    pub(crate) chip_uart: UartHandle,
    auxiliary_uarts: Vec<UartHandle>,
    timer: TimerHandle,
    exit: ExitHandle,
    usb_serial_jtag: EspUsbSerialJtagHandle,
    usb_otg: EspUsbOtgHandle,
    usb_wrap: Esp32S3UsbWrapHandle,
    pms: Esp32S3PmsHandle,
    world_controller: Esp32S3WorldControllerHandle,
    extmem: Esp32S3ExtmemHandle,
    usb_host: EspDwc2Host,
    saradc: Esp32S3SarAdcHandle,
    tsens: Esp32S3TsensHandle,
    lcd_cam: Esp32S3LcdCamHandle,
    sdmmc: Esp32S3SdmmcHandle,
    sha: Esp32S3ShaHandle,
    aes: Esp32S3AesHandle,
    xts_aes: remu_devices::Esp32S3XtsAesHandle,
    system: EspSystemHandle,
    systimer: EspSystimerHandle,
    timer_groups: Vec<EspTimerGroupHandle>,
    ledc: Esp32S3LedcHandle,
    pcnt: Esp32S3PcntHandle,
    mcpwm: Vec<Esp32S3McpwmHandle>,
    twai: EspTwaiHandle,
    gdma: EspGdmaHandle,
    uhci: Esp32S3UhciHandle,
    uhci1: Esp32S3UhciHandle,
    peri_backup: remu_devices::Esp32S3PeriBackupHandle,
    syscon: Esp32S3SysconHandle,
    rtc_control: EspRtcControlHandle,
    rtc_i2c: remu_devices::Esp32S3RtcI2cHandle,
    rtc_io: remu_devices::Esp32S3RtcIoHandle,
    sdm: remu_devices::Esp32S3SdmHandle,
    mmu_table: EspMmuTableHandle,
    now: SimTime,
    stack: u32,
    instruction_cache_configured: bool,
    windowed_handoff_pending: bool,
    appcpu_boot_address: Option<u32>,
    interrupt_matrix: EspInterruptMatrixHandle,
    io_mux: Esp32S3IoMuxHandle,
    md5_contexts: BTreeMap<u32, Vec<u8>>,
    sha256_contexts: BTreeMap<u32, FunctionalSha256>,
    setjmp_contexts: BTreeMap<u32, XtensaCpu>,
    flash: Vec<u8>,
    stop_on_usb_input_complete: bool,
    breakpoints: BTreeSet<u64>,
    signal_stops: Vec<SignalStop>,
}
impl XtensaMachine {
    /// Creates the ESP32-S3 direct-mode map.
    pub fn new(target: TargetId) -> Result<Self, XtensaMachineError> {
        if target != TargetId::Esp32s3 {
            return Err(XtensaMachineError::UnsupportedTarget(target));
        }
        let manifest = target_manifest(target);
        let mut bus = AddressSpace::new(Endianness::Little);
        let signals = SignalHub::new();
        let mut stack = None;
        for region in manifest.memory {
            match region.kind {
                MemoryKind::Ram => {
                    let storage = SharedMemory::from_bytes(vec![0xa5; region.size]);
                    bus.map_shared(
                        region.name,
                        region.start,
                        region.size,
                        if region.executable {
                            Permissions::RWX
                        } else {
                            Permissions::RW
                        },
                        storage,
                        0,
                    )?;
                    if region.name == "dram" {
                        stack = Some(
                            u32::try_from(
                                region.start
                                    + u64::try_from(region.size).expect("memory size fits u64"),
                            )
                            .expect("ESP32-S3 DRAM end fits 32 bits"),
                        );
                    }
                }
                MemoryKind::Flash | MemoryKind::Rom => {
                    bus.map_shared(
                        region.name,
                        region.start,
                        region.size,
                        Permissions::RX,
                        SharedMemory::zeroed(region.size),
                        0,
                    )?;
                }
            }
        }
        bus.map_shared(
            "drom",
            0x3c00_0000,
            16 * 1024 * 1024,
            Permissions::RX,
            SharedMemory::from_bytes(vec![0xff; 16 * 1024 * 1024]),
            0,
        )?;
        let mut rom_layout_page = vec![0_u8; 0x1000];
        // ESP32-S3 ROM layout table. IDF uses the second pointer to exclude
        // the shared ROM/RTOS stack window from the heap.
        rom_layout_page[4..8].copy_from_slice(&0x3fce_9710_u32.to_le_bytes());
        rom_layout_page[0xffc..0x1000].copy_from_slice(&0x3ff1_f000_u32.to_le_bytes());
        bus.map_shared(
            "esp32s3.rom-layout",
            0x3ff1_f000,
            0x1000,
            Permissions::RO,
            SharedMemory::from_bytes(rom_layout_page),
            0,
        )?;
        let mut rom_service_data = vec![0_u8; 0x1000];
        let rom_service_signature = b"remu-coex-rom-v0.0\0";
        rom_service_data[..rom_service_signature.len()].copy_from_slice(rom_service_signature);
        bus.map_shared(
            "esp32s3.rom-service-data",
            0x3ff1_e000,
            0x1000,
            Permissions::RO,
            SharedMemory::from_bytes(rom_service_data),
            0,
        )?;
        bus.map_ram("rtc-fast-memory", 0x600f_e000, 0x2000, true)?;
        // ESP32-S3 TRM table 4.3-3 exposes the same 8 KiB RTC slow memory at
        // the ULP data address and the CPU peripheral-bus address.
        let rtc_slow_memory = bus.map_ram("rtc-slow-memory", 0x5000_0000, 0x2000, true)?;
        bus.map_shared(
            "esp32s3.rtc-slow-memory-alias",
            0x6002_1000,
            0x2000,
            Permissions::RWX,
            rtc_slow_memory,
            0,
        )?;
        let (world_controller_device, world_controller) =
            Esp32S3WorldController::new("esp32s3.world-controller");
        bus.map_device(
            "esp32s3.world-controller",
            0x600d_0000,
            0x1000,
            Box::new(world_controller_device),
        )?;
        let (pms_device, pms) = Esp32S3Pms::new("esp32s3.sensitive");
        bus.map_device(
            "esp32s3.sensitive",
            0x600c_1000,
            0x1000,
            Box::new(pms_device),
        )?;
        let (syscon_device, syscon) = Esp32S3Syscon::new("esp32s3.syscon");
        bus.map_device(
            "esp32s3.syscon",
            0x6002_6000,
            0x1000,
            Box::new(syscon_device),
        )?;
        let (usb_wrap_device, usb_wrap) = Esp32S3UsbWrap::new("esp32s3.usb-wrap");
        bus.map_device(
            "esp32s3.usb-wrap",
            0x6003_9000,
            0x1000,
            Box::new(usb_wrap_device),
        )?;
        let (io_mux_device, io_mux) = Esp32S3IoMux::new("esp32s3.io-mux");
        bus.map_device(
            "esp32s3.io-mux",
            0x6000_9000,
            0x1000,
            Box::new(io_mux_device),
        )?;
        for (name, base) in [("i2c0", 0x6001_3000), ("i2c1", 0x6002_7000)] {
            let device = Esp32s3I2c::new(format!("esp32s3.{name}"), signals.clone())?;
            bus.map_device(format!("esp32s3.{name}"), base, 0x1000, Box::new(device))?;
        }
        let rmt_device = Esp32s3Rmt::new("esp32s3.rmt", signals.clone())?;
        bus.map_device("esp32s3.rmt", 0x6001_6000, 0x1000, Box::new(rmt_device))?;
        let (ledc_device, ledc) =
            Esp32S3Ledc::new("esp32s3.ledc", "board.esp32s3.ledc", signals.clone())?;
        bus.map_device("esp32s3.ledc", 0x6001_9000, 0x1000, Box::new(ledc_device))?;
        let (pcnt_device, pcnt) =
            Esp32S3Pcnt::new("esp32s3.pcnt", "board.esp32s3.pcnt", signals.clone())?;
        bus.map_device("esp32s3.pcnt", 0x6001_7000, 0x1000, Box::new(pcnt_device))?;
        let mut mcpwm = Vec::new();
        for (instance, base) in [(0, 0x6001_e000), (1, 0x6002_c000)] {
            let (device, handle) = Esp32S3Mcpwm::new(
                format!("esp32s3.mcpwm{instance}"),
                &format!("board.esp32s3.mcpwm{instance}"),
                signals.clone(),
            )?;
            bus.map_device(
                format!("esp32s3.mcpwm{instance}"),
                base,
                0x1000,
                Box::new(device),
            )?;
            mcpwm.push(handle);
        }
        let (twai_device, twai) =
            EspTwai::new("esp32s3.twai", "board.esp32s3.twai", signals.clone())?;
        bus.map_device("esp32s3.twai", 0x6002_b000, 0x1000, Box::new(twai_device))?;
        let (peri_backup_device, peri_backup) =
            remu_devices::Esp32S3PeriBackup::new("esp32s3.peri-backup");
        bus.map_device(
            "esp32s3.peri-backup",
            0x6002_a000,
            0x1000,
            Box::new(peri_backup_device),
        )?;
        let (gdma_device, gdma) =
            EspGdma::new("esp32s3.gdma", "board.esp32s3.gdma", signals.clone())?;
        bus.map_device("esp32s3.gdma", 0x6003_f000, 0x1000, Box::new(gdma_device))?;
        let (saradc_device, saradc) = Esp32S3SarAdc::new("esp32s3.saradc", signals.clone())?;
        bus.map_device(
            "esp32s3.saradc",
            0x6004_0000,
            0x1000,
            Box::new(saradc_device),
        )?;
        let (rtc_i2c_device, rtc_i2c) = remu_devices::Esp32S3RtcI2c::new("esp32s3.rtc-i2c");
        bus.map_device(
            "esp32s3.rtc-i2c",
            0x6000_8c00,
            0x400,
            Box::new(rtc_i2c_device),
        )?;
        let (tsens_device, tsens) = Esp32S3Tsens::new_with_rtc_i2c(
            "esp32s3.tsens",
            signals.clone(),
            Some(rtc_i2c.clone()),
        )?;
        bus.map_device("esp32s3.tsens", 0x6000_8800, 0x200, Box::new(tsens_device))?;
        let (lcd_cam_device, lcd_cam) = Esp32S3LcdCam::new("esp32s3.lcd-cam", signals.clone())?;
        bus.map_device(
            "esp32s3.lcd-cam",
            0x6004_1000,
            0x1000,
            Box::new(lcd_cam_device),
        )?;
        let (sdmmc_device, sdmmc) = Esp32S3Sdmmc::new("esp32s3.sdmmc", signals.clone())?;
        bus.map_device("esp32s3.sdmmc", 0x6002_8000, 0x1000, Box::new(sdmmc_device))?;
        let (sha_device, sha) = Esp32S3Sha::new("esp32s3.sha", signals.clone())?;
        bus.map_device("esp32s3.sha", 0x6003_b000, 0x1000, Box::new(sha_device))?;
        let (aes_device, aes) = Esp32S3Aes::new("esp32s3.aes", signals.clone())?;
        bus.map_device("esp32s3.aes", 0x6003_a000, 0x1000, Box::new(aes_device))?;
        bus.map_device(
            "esp32s3.efuse",
            0x6000_7000,
            0x1000,
            Box::new(EspEfuse::new("esp32s3.efuse")),
        )?;
        bus.map_device(
            "esp32s3.hmac",
            0x6003_e000,
            0x1000,
            Box::new(EspHmac::new("esp32s3.hmac")),
        )?;
        bus.map_device(
            "esp32s3.rsa",
            0x6003_c000,
            0x1000,
            Box::new(EspRsa::new("esp32s3.rsa")),
        )?;
        bus.map_device(
            "esp32s3.digital-signature",
            0x6003_d000,
            0x1000,
            Box::new(EspDigitalSignature::new("esp32s3.digital-signature")),
        )?;
        bus.map_device(
            "esp32s3.rng",
            0x6003_5000,
            0x1000,
            Box::new(DeterministicRng::new("esp32s3.rng", 0x7c, 0x32f3_0001)),
        )?;
        bus.map_device(
            "esp32s3.spi1",
            0x6000_2000,
            0x1000,
            Box::new(EspSpiMem::new("esp32s3.spi1")),
        )?;
        bus.map_device(
            "esp32s3.spi0",
            0x6000_3000,
            0x1000,
            Box::new(EspSpiMem::new("esp32s3.spi0")),
        )?;
        for (name, base) in [("spi2", 0x6002_4000), ("spi3", 0x6002_5000)] {
            bus.map_device(
                format!("esp32s3.{name}"),
                base,
                0x1000,
                Box::new(Esp32s3Spi::new(format!("esp32s3.{name}"), signals.clone())?),
            )?;
        }
        for (name, base) in [("i2s0", 0x6000_f000), ("i2s1", 0x6002_d000)] {
            bus.map_device(
                format!("esp32s3.{name}"),
                base,
                0x1000,
                Box::new(Esp32s3I2s::new(format!("esp32s3.{name}"), signals.clone())?),
            )?;
        }
        let (rtc_control_device, rtc_control) =
            EspRtcControl::new_with_signals("esp32s3.rtc-control", signals.clone())?;
        bus.map_device(
            "esp32s3.rtc-control",
            0x6000_8000,
            0x400,
            Box::new(rtc_control_device),
        )?;
        let (interrupt_matrix_device, interrupt_matrix) =
            EspInterruptMatrix::new("esp32s3.interrupt-matrix");
        bus.map_device(
            "esp32s3.interrupt-matrix",
            0x600c_2000,
            0x1000,
            Box::new(interrupt_matrix_device),
        )?;
        let mut timer_groups = Vec::new();
        for (name, base) in [
            ("esp32s3.timer-group0", 0x6001_f000),
            ("esp32s3.timer-group1", 0x6002_0000),
        ] {
            let (device, handle) = EspTimerGroup::new(name, EspTimerGroupKind::Esp32S3);
            bus.map_device(name, base, 0x1000, Box::new(device))?;
            timer_groups.push(handle);
        }
        let (systimer_device, systimer) = EspSystimer::new("esp32s3.systimer");
        bus.map_device(
            "esp32s3.systimer",
            0x6002_3000,
            0x1000,
            Box::new(systimer_device),
        )?;
        let (system_device, system) = EspSystem::new("esp32s3.system");
        bus.map_device(
            "esp32s3.system",
            0x600c_0000,
            0x1000,
            Box::new(system_device),
        )?;
        let (xts_aes_device, xts_aes) = Esp32S3XtsAes::new("esp32s3.xts-aes", system.clone());
        bus.map_device(
            "esp32s3.xts-aes",
            0x600c_c000,
            0x1000,
            Box::new(xts_aes_device),
        )?;
        let (mmu_table_device, mmu_table) = EspMmuTable::new("esp32s3.mmu-table");
        bus.map_device(
            "esp32s3.mmu-table",
            0x600c_5000,
            0x1000,
            Box::new(mmu_table_device),
        )?;
        let (extmem_device, extmem) = Esp32S3Extmem::new("esp32s3.extmem");
        bus.map_device(
            "esp32s3.extmem",
            0x600c_4000,
            0x1000,
            Box::new(extmem_device),
        )?;
        let (gpio_device, gpio) = FunctionalGpio::new(
            "esp32s3.compiler-gpio",
            32,
            "board.esp32s3.gpio",
            signals.clone(),
            0,
            4,
            8,
        )?;
        let (uart_device, uart) = FunctionalUart::new("esp32s3.compiler-uart", 0, 4, 1);
        let (timer_device, timer) = FunctionalTimer::new("esp32s3.compiler-timer");
        let (exit_device, exit) = ExitDevice::new("esp32s3.compiler-exit");
        bus.map_device(
            "remu.test.gpio",
            TEST_GPIO,
            TEST_DEVICE_SIZE,
            Box::new(gpio_device),
        )?;
        bus.map_device(
            "remu.test.uart",
            TEST_UART,
            TEST_DEVICE_SIZE,
            Box::new(uart_device),
        )?;
        bus.map_device(
            "remu.test.timer",
            TEST_TIMER,
            TEST_DEVICE_SIZE,
            Box::new(timer_device),
        )?;
        bus.map_device(
            "remu.test.exit",
            TEST_EXIT,
            TEST_EXIT_SIZE,
            Box::new(exit_device),
        )?;
        let (chip_gpio_device, chip_gpio) = EspGpio::new(
            "esp32s3.gpio",
            49,
            "board.esp32s3.chip_gpio",
            signals.clone(),
        )?;
        bus.map_device(
            "esp32s3.gpio",
            0x6000_4000,
            0x0f00,
            Box::new(chip_gpio_device),
        )?;
        let (rtc_io_device, rtc_io) = remu_devices::Esp32S3RtcIo::new(
            "esp32s3.rtc-io",
            chip_gpio.clone(),
            rtc_control.clone(),
        );
        bus.map_device(
            "esp32s3.rtc-io",
            0x6000_8400,
            0x400,
            Box::new(rtc_io_device),
        )?;
        let (sdm_device, sdm) =
            remu_devices::Esp32S3Sdm::new("esp32s3.sdm", "board.esp32s3.sdm", signals.clone())?;
        bus.map_device("esp32s3.sdm", 0x6000_4f00, 0x100, Box::new(sdm_device))?;
        let (chip_uart_device, chip_uart) =
            FunctionalUart::new_lenient("esp32s3.uart0", 0, 0x1c, 0);
        bus.map_device(
            "esp32s3.uart0",
            0x6000_0000,
            0x1000,
            Box::new(chip_uart_device),
        )?;
        let mut auxiliary_uarts = Vec::new();
        for (name, base) in [("uart1", 0x6001_0000), ("uart2", 0x6002_e000)] {
            let (device, handle) =
                FunctionalUart::new_lenient(format!("esp32s3.{name}"), 0, 0x1c, 0);
            bus.map_device(format!("esp32s3.{name}"), base, 0x1000, Box::new(device))?;
            auxiliary_uarts.push(handle);
        }
        let (uhci_device, uhci) = Esp32S3Uhci::new(
            "esp32s3.uhci0",
            [
                chip_uart.clone(),
                auxiliary_uarts[0].clone(),
                auxiliary_uarts[1].clone(),
            ],
        );
        bus.map_device("esp32s3.uhci0", 0x6001_4000, 0x1000, Box::new(uhci_device))?;
        let (uhci1_device, uhci1) = Esp32S3Uhci::new(
            "esp32s3.uhci1",
            [
                chip_uart.clone(),
                auxiliary_uarts[0].clone(),
                auxiliary_uarts[1].clone(),
            ],
        );
        bus.map_device("esp32s3.uhci1", 0x6000_c000, 0x1000, Box::new(uhci1_device))?;
        let (usb_serial_jtag_device, usb_serial_jtag) =
            EspUsbSerialJtag::new("esp32s3.usb-serial-jtag");
        bus.map_device(
            "esp32s3.usb-serial-jtag",
            0x6003_8000,
            0x1000,
            Box::new(usb_serial_jtag_device),
        )?;
        let (usb_otg_device, usb_otg) = EspUsbOtg::new("esp32s3.usb-otg");
        bus.map_device(
            "esp32s3.usb-otg",
            0x6008_0000,
            0x1_0000,
            Box::new(usb_otg_device),
        )?;
        // The mask ROM normally initializes this pointer before handing off
        // to the second-stage bootloader. Direct verified-image handoff keeps
        // a zeroed functional legacy-flash data table in unused DRAM.
        bus.write(0x3fce_ffe4, AccessWidth::Word, 0x3fce_0000, SimTime::ZERO)
            .map_err(|error| XtensaMachineError::Load {
                address: 0x3fce_ffe4,
                message: error.to_string(),
            })?;
        // esp_rom_spiflash_legacy_data points at the mask-ROM flash
        // descriptor normally populated by the second-stage bootloader.
        for (offset, value) in [
            (0_u64, 0x0016_40c8_u64),
            (4, 16 * 1024 * 1024),
            (8, 64 * 1024),
            (12, 4 * 1024),
            (16, 256),
            (20, 0x0000_ffff),
        ] {
            bus.write(
                0x3fce_0000 + offset,
                AccessWidth::Word,
                value,
                SimTime::ZERO,
            )
            .map_err(|error| XtensaMachineError::Load {
                address: 0x3fce_0000 + offset,
                message: error.to_string(),
            })?;
        }
        let cpu = XtensaCpu::new();
        let mut cpu1 = XtensaCpu::new();
        cpu1.share_task_contexts_from(&cpu);

        Ok(Self {
            cpu,
            cpu1,
            bus,
            signals,
            gpio,
            chip_gpio,
            uart,
            chip_uart,
            auxiliary_uarts,
            timer,
            exit,
            usb_serial_jtag,
            usb_otg,
            usb_wrap,
            pms,
            world_controller,
            extmem,
            usb_host: EspDwc2Host::new(),
            saradc,
            tsens,
            lcd_cam,
            sdmmc,
            sha,
            aes,
            xts_aes,
            system,
            systimer,
            timer_groups,
            ledc,
            pcnt,
            mcpwm,
            twai,
            gdma,
            uhci,
            uhci1,
            peri_backup,
            syscon,
            rtc_control,
            rtc_i2c,
            rtc_io,
            sdm,
            mmu_table,
            now: SimTime::ZERO,
            stack: stack.expect("ESP32-S3 manifest includes DRAM"),
            instruction_cache_configured: false,
            windowed_handoff_pending: false,
            appcpu_boot_address: None,
            interrupt_matrix,
            io_mux,
            md5_contexts: BTreeMap::new(),
            sha256_contexts: BTreeMap::new(),
            setjmp_contexts: BTreeMap::new(),
            flash: Vec::new(),
            stop_on_usb_input_complete: false,
            breakpoints: BTreeSet::new(),
            signal_stops: Vec::new(),
        })
    }
    /// Loads an Xtensa ELF and establishes CPU0 direct state.
    pub fn load_firmware(&mut self, image: &FirmwareImage) -> Result<(), XtensaMachineError> {
        if image.architecture != FirmwareArchitecture::Xtensa {
            return Err(XtensaMachineError::Architecture(image.architecture));
        }
        // Direct ELF execution is intentionally the weaker debugging mode:
        // it permits XIP reads without reproducing the bootloader handoff.
        self.instruction_cache_configured = true;
        self.extmem.configure_boot_caches();
        self.windowed_handoff_pending = false;
        for segment in &image.segments {
            let initialized = segment
                .data
                .get(..segment.initialized_size)
                .ok_or_else(|| XtensaMachineError::Load {
                    address: segment.address,
                    message: format!(
                        "initialized ELF bytes ({}) exceed segment data ({})",
                        segment.initialized_size,
                        segment.data.len()
                    ),
                })?;
            self.bus
                .load(segment.address, initialized)
                .map_err(|error| XtensaMachineError::Load {
                    address: segment.address,
                    message: error.to_string(),
                })?;
        }
        let entry =
            u32::try_from(image.entry).map_err(|_| XtensaMachineError::EntryRange(image.entry))?;
        self.cpu.set_direct_state(self.stack, entry);
        Ok(())
    }
    /// Performs the documented ESP ROM verified-image handoff to an ESP32-S3
    /// application using the official merged flash image.
    pub fn load_esp_application(
        &mut self,
        image: &EspFlashImage,
    ) -> Result<(), XtensaMachineError> {
        const PAGE_SIZE: u32 = 64 * 1024;
        self.instruction_cache_configured = false;
        self.windowed_handoff_pending = true;
        for segment in &image.application.segments {
            let segment_end = usize::try_from(segment.flash_offset)
                .ok()
                .and_then(|start| start.checked_add(segment.data.len()))
                .ok_or_else(|| XtensaMachineError::Load {
                    address: u64::from(segment.address),
                    message: "ESP application segment flash range overflows the host address space"
                        .to_owned(),
                })?;
            if segment_end > self.flash.len() {
                return Err(XtensaMachineError::Load {
                    address: u64::from(segment.address),
                    message: format!(
                        "ESP application segment flash range {:#x}..{:#x} exceeds simulated flash size {:#x}",
                        segment.flash_offset,
                        segment_end,
                        self.flash.len()
                    ),
                });
            }
            self.bus
                .load(u64::from(segment.address), &segment.data)
                .map_err(|error| XtensaMachineError::Load {
                    address: u64::from(segment.address),
                    message: error.to_string(),
                })?;

            // The S3 has one unified cache-MMU table shared by its DROM and
            // IROM aliases. Reconstruct the second-stage bootloader's entries
            // for flash-mapped executable-image segments so IDF cache2phys
            // queries observe the same physical addresses as real hardware.
            let virtual_page = match segment.address {
                0x3c00_0000..=0x3dff_ffff => Some(segment.address - 0x3c00_0000),
                0x4200_0000..=0x43ff_ffff => Some(segment.address - 0x4200_0000),
                _ => None,
            };
            if let Some(virtual_page) = virtual_page {
                let address_offset = segment.address % PAGE_SIZE;
                let flash_offset = segment.flash_offset.checked_sub(address_offset).ok_or(
                    XtensaMachineError::Load {
                        address: u64::from(segment.address),
                        message: "ESP mapped segment flash/virtual offsets disagree".to_owned(),
                    },
                )?;
                if flash_offset % PAGE_SIZE != 0 {
                    return Err(XtensaMachineError::Load {
                        address: u64::from(segment.address),
                        message: "ESP mapped segment is not cache-page congruent".to_owned(),
                    });
                }
                let first_index = usize::try_from(virtual_page / PAGE_SIZE)
                    .expect("S3 cache-MMU index fits usize");
                let first_entry = flash_offset / PAGE_SIZE;
                let span = address_offset
                    .checked_add(u32::try_from(segment.data.len()).map_err(|_| {
                        XtensaMachineError::Load {
                            address: u64::from(segment.address),
                            message: "ESP mapped segment length exceeds u32".to_owned(),
                        }
                    })?)
                    .ok_or(XtensaMachineError::Load {
                        address: u64::from(segment.address),
                        message: "ESP mapped segment span overflow".to_owned(),
                    })?;
                let pages = usize::try_from(span.div_ceil(PAGE_SIZE))
                    .expect("S3 cache-MMU page count fits usize");
                for page in 0..pages {
                    self.mmu_table
                        .set_mapping(
                            first_index + page,
                            first_entry + u32::try_from(page).expect("MMU page fits u32"),
                        )
                        .map_err(|error| XtensaMachineError::Load {
                            address: u64::from(segment.address),
                            message: error.to_string(),
                        })?;
                }
            }
        }
        // The second-stage bootloader maps the 24-byte image header and the
        // first eight-byte segment header immediately before the first DROM
        // payload. IDF deliberately rereads this virtual preamble during
        // cpu_start, so preserve that part of the verified boot contract even
        // in direct application handoff mode.
        if let Some(first) = image.application.segments.first() {
            let mut preamble = [0_u8; 32];
            let header = &image.application.header;
            preamble[0] = 0xe9;
            preamble[1] = header.segment_count;
            preamble[2] = header.flash_mode;
            preamble[3] = header.flash_size_frequency;
            preamble[4..8].copy_from_slice(&header.entry.to_le_bytes());
            preamble[8] = header.write_protect_pin;
            preamble[9..12].copy_from_slice(&header.drive_settings);
            preamble[12..14].copy_from_slice(&header.chip_id.to_le_bytes());
            preamble[14] = header.minimum_revision_legacy;
            preamble[15..17].copy_from_slice(&header.minimum_revision.to_le_bytes());
            preamble[17..19].copy_from_slice(&header.maximum_revision.to_le_bytes());
            preamble[23] = u8::from(header.hash_appended);
            preamble[24..28].copy_from_slice(&first.address.to_le_bytes());
            preamble[28..32].copy_from_slice(&(first.data.len() as u32).to_le_bytes());
            let address = first.address.checked_sub(preamble.len() as u32).ok_or(
                XtensaMachineError::Load {
                    address: u64::from(first.address),
                    message: "first ESP segment has no room for its virtual image preamble"
                        .to_owned(),
                },
            )?;
            self.bus
                .load(u64::from(address), &preamble)
                .map_err(|error| XtensaMachineError::Load {
                    address: u64::from(address),
                    message: error.to_string(),
                })?;
        }
        self.cpu
            .set_windowed_entry_state(self.stack, image.application.header.entry);
        Ok(())
    }

    /// Installs the complete merged SPI-flash image used by runtime MMU maps.
    pub fn set_esp_flash_image(&mut self, bytes: &[u8]) {
        self.flash.clear();
        self.flash.extend_from_slice(bytes);
        self.flash.resize(16 * 1024 * 1024, 0xff);
    }

    /// Returns the complete mutable SPI-flash state for persistence.
    pub fn esp_flash_image(&self) -> &[u8] {
        &self.flash
    }

    fn apply_pending_mmu_mappings(&mut self) -> Result<(), XtensaMachineError> {
        const PAGE_SIZE: usize = 64 * 1024;
        for (index, entry) in self.mmu_table.drain_mappings() {
            // Bit 14 is the S3 invalid-entry marker. The remaining low bits
            // identify the 64-KiB physical SPI-flash page.
            if entry & 0x4000 != 0 || index >= 256 {
                continue;
            }
            let source = (entry as usize & 0x3fff) * PAGE_SIZE;
            let Some(end) = source
                .checked_add(PAGE_SIZE)
                .filter(|end| *end <= self.flash.len())
            else {
                continue;
            };
            // One table entry backs both cache aliases.
            for base in [0x3c00_0000_u64, 0x4200_0000_u64] {
                let destination = base + (index * PAGE_SIZE) as u64;
                self.bus
                    .load(destination, &self.flash[source..end])
                    .map_err(|error| XtensaMachineError::Load {
                        address: destination,
                        message: error.to_string(),
                    })?;
            }
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

    /// Returns completed bus operations when recording is enabled.
    pub fn access_log(&self) -> &[remu_bus::BusAccessRecord] {
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

    /// Returns the current CPU0 snapshot for debugger adapters.
    pub fn debug_snapshot(&self) -> remu_core::CpuSnapshot {
        self.cpu.snapshot()
    }

    /// Reads guest-visible bytes for a debugger.
    pub fn debug_read_memory(&mut self, address: u64, length: usize) -> Result<Vec<u8>, String> {
        (0..length)
            .map(|offset| {
                self.bus
                    .read(
                        address.saturating_add(offset as u64),
                        AccessWidth::Byte,
                        AccessKind::Read,
                        self.now,
                    )
                    .map(|value| value as u8)
                    .map_err(|error| error.to_string())
            })
            .collect()
    }

    /// Writes guest-visible bytes for a debugger.
    pub fn debug_write_memory(&mut self, address: u64, bytes: &[u8]) -> Result<(), String> {
        for (offset, byte) in bytes.iter().enumerate() {
            self.bus
                .write(
                    address.saturating_add(offset as u64),
                    AccessWidth::Byte,
                    u64::from(*byte),
                    self.now,
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    /// Stops after a completed CPU data access overlaps `address`.
    pub fn add_watchpoint(&mut self, address: u64) {
        self.bus.add_watchpoint(address);
    }

    /// Stops when the named signal satisfies `edge`.
    pub fn add_signal_stop(
        &mut self,
        path: &str,
        edge: SignalEdge,
    ) -> Result<(), XtensaMachineError> {
        self.signal_stops
            .push(resolve_signal_stop(&self.signals, path, edge)?);
        Ok(())
    }

    /// Removes configured user breakpoints and data watchpoints.
    pub fn clear_debug_stops(&mut self) {
        self.breakpoints.clear();
        self.bus.clear_watchpoints();
        self.signal_stops.clear();
    }

    /// Queues deterministic CDC-ACM input for the native USB console.
    pub fn queue_usb_input(&mut self, bytes: &[u8]) {
        self.usb_serial_jtag.queue_input(bytes);
        self.usb_host.queue_input(bytes);
    }

    /// Selects whether the ESP USB Serial/JTAG host is attached.
    ///
    /// The host is connected by default. When connected, the peripheral
    /// asserts its SOF raw interrupt every fixed abstract USB frame period;
    /// disconnected mode is useful for testing non-blocking console paths.
    pub fn set_usb_host_connected(&mut self, connected: bool) {
        self.usb_serial_jtag.set_host_connected(connected, self.now);
    }

    /// Stops a bounded run once all queued USB input returns to the raw-REPL prompt.
    pub fn stop_on_usb_input_complete(&mut self, enabled: bool) {
        self.stop_on_usb_input_complete = enabled;
    }

    /// Drives or releases one GPIO pin.
    pub fn set_pin(&self, pin: u8, value: Logic) -> Result<(), XtensaMachineError> {
        if usize::from(pin) < self.gpio.pin_count() {
            self.gpio.set_input(pin, value, self.now)?;
        }
        self.chip_gpio.set_input(pin, value, self.now)?;
        Ok(())
    }

    /// Applies a host-supplied edge to an ESP32-S3 PCNT unit.
    pub fn pulse_pcnt(
        &self,
        unit: usize,
        edge: remu_devices::EspPcntEdge,
    ) -> Result<bool, XtensaMachineError> {
        Ok(self.pcnt.pulse(unit, edge, self.now)?)
    }

    /// Runs until a terminal condition.
    pub fn run(
        &mut self,
        limits: RunLimits,
        trace: Option<&mut dyn TraceSink>,
    ) -> Result<RunResult, XtensaMachineError> {
        self.run_with_stimuli(limits, &[], trace)
    }

    /// Runs with timestamped external GPIO stimulus.
    pub fn run_with_stimuli(
        &mut self,
        limits: RunLimits,
        stimuli: &[PinStimulus],
        mut trace: Option<&mut dyn TraceSink>,
    ) -> Result<RunResult, XtensaMachineError> {
        if limits.instructions.is_none() && limits.deadline.is_none() {
            return Err(XtensaMachineError::MissingRunLimit);
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
        let mut timer_was_pending = false;
        let mut usb_was_pending = false;
        let mut usb_serial_was_pending = false;
        let mut uhci_was_pending = false;
        let mut peri_backup_was_pending = false;
        let mut syscon_was_pending = false;
        let mut extmem_was_pending = false;
        let mut pms_was_pending = false;
        let mut rtc_was_pending = false;
        let mut crosscore_was_pending = [false; 2];
        let mut systimer_was_pending = [false; 3];
        let mut timer_group_was_pending = [[false; 2]; 2];
        let mut next_core = 0_u8;
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
            if self.usb_serial_jtag.poll(self.now) {
                stats.events = stats.events.saturating_add(1);
            }
            if self.uhci.poll_gdma(&self.gdma) != 0 {
                stats.events = stats.events.saturating_add(1);
            }
            if self.uhci1.poll_gdma(&self.gdma) != 0 || self.service_peri_backup()? {
                stats.events = stats.events.saturating_add(1);
            }
            let uhci_pending = self.update_uhci_interrupt_lines()?;
            if uhci_pending && !uhci_was_pending {
                stats.events = stats.events.saturating_add(1);
            }
            uhci_was_pending = uhci_pending;
            let peri_backup_pending = self.update_peri_backup_interrupt_lines()?;
            if peri_backup_pending && !peri_backup_was_pending {
                stats.events = stats.events.saturating_add(1);
            }
            peri_backup_was_pending = peri_backup_pending;
            let syscon_pending = self.update_syscon_interrupt_lines()?;
            if syscon_pending && !syscon_was_pending {
                stats.events = stats.events.saturating_add(1);
            }
            syscon_was_pending = syscon_pending;
            let extmem_pending = self.update_extmem_interrupt_lines()?;
            if extmem_pending && !extmem_was_pending {
                stats.events = stats.events.saturating_add(1);
            }
            extmem_was_pending = extmem_pending;
            let pms_pending = self.update_pms_interrupt_lines()?;
            if pms_pending && !pms_was_pending {
                stats.events = stats.events.saturating_add(1);
            }
            pms_was_pending = pms_pending;
            let timer_pending = self.timer.poll(self.now);
            if timer_pending && !timer_was_pending {
                stats.events = stats.events.saturating_add(1);
            }
            timer_was_pending = timer_pending;
            self.cpu.set_interrupt(0, timer_pending)?;
            // Once CDC traffic starts, advance the external host only while
            // the application core is parked in WAITI. This models a
            // deterministic, low-speed host and prevents an endpoint
            // completion from preempting an arbitrary logical register
            // window in the functional CPU model.
            if !self.usb_host.input_started()
                || self.usb_host.can_poll()
                || self.cpu1.waiting_for_interrupt()
            {
                stats.events = stats.events.saturating_add(self.usb_host.poll(
                    self.now,
                    &self.usb_otg,
                    &self.usb_wrap,
                ));
            }
            if self.stop_on_usb_input_complete && self.usb_host.input_complete() {
                break StopReason::HostInputComplete;
            }
            let usb_pending = self.usb_otg.interrupt_pending();
            if usb_pending && !usb_was_pending {
                stats.events = stats.events.saturating_add(1);
            }
            usb_was_pending = usb_pending;
            if usb_pending
                && std::env::var_os("REMU_DEBUG_USB").is_some()
                && self.now.ticks().is_multiple_of(100_000)
            {
                let (ps0, pending0, enable0) = self.cpu.interrupt_state();
                let (ps1, pending1, enable1) = self.cpu1.interrupt_state();
                let table1 = self
                    .bus
                    .read(
                        0x3fc9_f448 + 2 * 8,
                        AccessWidth::Word,
                        AccessKind::Read,
                        self.now,
                    )
                    .unwrap_or(0);
                let table13 = self
                    .bus
                    .read(
                        0x3fc9_f448 + 13 * 2 * 8,
                        AccessWidth::Word,
                        AccessKind::Read,
                        self.now,
                    )
                    .unwrap_or(0);
                let table_usb = self
                    .bus
                    .read(
                        0x3fc9_f448 + (4 * 2 + 1) * 8,
                        AccessWidth::Word,
                        AccessKind::Read,
                        self.now,
                    )
                    .unwrap_or(0);
                eprintln!(
                    "dwc2 cpu pending at={} cpu0={pending0:#x}/{enable0:#x} ps={ps0:#x} cpu1={pending1:#x}/{enable1:#x} ps={ps1:#x} table1={table1:#x} table13={table13:#x} usb={table_usb:#x}",
                    self.now.ticks()
                );
            }
            for core in 0..2_u32 {
                // Native ESP32-S3 treats the internal CPU interrupt
                // destinations (including the reset value 16) as disabled
                // for peripheral sources; the matrix handle normalizes them
                // to its disabled sentinel.
                self.interrupt_matrix
                    .set_source_pending(core as usize, 38, usb_pending);
                let interrupt = self.interrupt_matrix.route(core as usize, 38);
                if interrupt != u8::MAX && interrupt != 6 {
                    if core == 0 {
                        self.cpu.set_interrupt(u16::from(interrupt), usb_pending)?;
                    } else if self.appcpu_boot_address.is_some() {
                        self.cpu1.set_interrupt(u16::from(interrupt), usb_pending)?;
                    }
                }
            }
            let usb_serial_pending = self.usb_serial_jtag.interrupt_pending();
            if usb_serial_pending && !usb_serial_was_pending {
                stats.events = stats.events.saturating_add(1);
            }
            usb_serial_was_pending = usb_serial_pending;
            for core in 0..2_u32 {
                // ESP-IDF uses interrupt-matrix source 48 for the USB
                // Serial/JTAG controller on ESP32-S3.
                let interrupt = self.interrupt_matrix.route(core as usize, 48);
                if interrupt == u8::MAX || interrupt == 6 {
                    continue;
                }
                if core == 0 {
                    self.cpu
                        .set_interrupt(u16::from(interrupt), usb_serial_pending)?;
                } else if self.appcpu_boot_address.is_some() {
                    self.cpu1
                        .set_interrupt(u16::from(interrupt), usb_serial_pending)?;
                }
            }
            let rtc_pending = self.update_rtc_interrupt_lines()?;
            if rtc_pending && !rtc_was_pending {
                stats.events = stats.events.saturating_add(1);
            }
            rtc_was_pending = rtc_pending;
            for core in 0..2_u32 {
                let crosscore_pending = self.system.from_cpu_pending(core as usize);
                let newly_pending = crosscore_pending && !crosscore_was_pending[core as usize];
                if newly_pending {
                    stats.events = stats.events.saturating_add(1);
                }
                crosscore_was_pending[core as usize] = crosscore_pending;
                let source = 79 + core;
                self.interrupt_matrix.set_source_pending(
                    core as usize,
                    source as usize,
                    crosscore_pending,
                );
                let interrupt = self.interrupt_matrix.route(core as usize, source as usize);
                if interrupt != u8::MAX {
                    if newly_pending && std::env::var_os("REMU_DEBUG_INTERRUPTS").is_some() {
                        let (ps, pending_bits, enable_bits) = if core == 0 {
                            self.cpu.interrupt_state()
                        } else {
                            self.cpu1.interrupt_state()
                        };
                        eprintln!(
                            "crosscore core={core} source={source} line={interrupt} at={} ps={ps:#x} pending={pending_bits:#x} enable={enable_bits:#x}",
                            self.now.ticks(),
                        );
                    }
                    if core == 0 {
                        self.cpu
                            .set_interrupt(u16::from(interrupt), crosscore_pending)?;
                    } else if self.appcpu_boot_address.is_some() {
                        self.cpu1
                            .set_interrupt(u16::from(interrupt), crosscore_pending)?;
                    }
                }
            }
            for (target, pending) in self.systimer.pending(self.now).into_iter().enumerate() {
                let newly_pending = pending && !systimer_was_pending[target];
                if newly_pending {
                    stats.events = stats.events.saturating_add(1);
                }
                systimer_was_pending[target] = pending;
                let source = 57 + u32::try_from(target).expect("three timer targets fit u32");
                let core = u32::try_from(target).expect("three timer targets fit u32");
                self.interrupt_matrix
                    .set_source_pending(core as usize, source as usize, pending);
                let interrupt = self.interrupt_matrix.route(core as usize, source as usize);
                if interrupt != u8::MAX {
                    if pending
                        && newly_pending
                        && std::env::var_os("REMU_DEBUG_INTERRUPTS").is_some()
                    {
                        let (ps, pending_bits, enable_bits) = if core == 0 {
                            self.cpu.interrupt_state()
                        } else {
                            self.cpu1.interrupt_state()
                        };
                        eprintln!(
                            "systimer target={target} source={source} line={interrupt} at={} ps={ps:#x} pending={pending_bits:#x} enable={enable_bits:#x}",
                            self.now.ticks(),
                        );
                    }
                    self.set_systimer_interrupt(core, u32::from(interrupt), pending)?;
                } else if pending
                    && newly_pending
                    && std::env::var_os("REMU_DEBUG_INTERRUPTS").is_some()
                {
                    eprintln!(
                        "systimer target={target} source={source} has no route at={}",
                        self.now.ticks()
                    );
                }
            }
            for (group, handle) in self.timer_groups.iter().enumerate() {
                for (timer, pending) in handle.pending(self.now).into_iter().enumerate() {
                    let source = match (group, timer) {
                        (0, 0) => 50,
                        (0, 1) => 51,
                        (1, 0) => 53,
                        (1, 1) => 54,
                        _ => unreachable!("two ESP32-S3 groups with two timers"),
                    };
                    if pending && !timer_group_was_pending[group][timer] {
                        stats.events = stats.events.saturating_add(1);
                    }
                    timer_group_was_pending[group][timer] = pending;
                    for core in 0..2_u32 {
                        self.interrupt_matrix.set_source_pending(
                            core as usize,
                            source as usize,
                            pending,
                        );
                        let interrupt = self.interrupt_matrix.route(core as usize, source as usize);
                        if interrupt == u8::MAX || interrupt == 6 {
                            continue;
                        }
                        if core == 0 {
                            self.cpu.set_interrupt(u16::from(interrupt), pending)?;
                        } else if self.appcpu_boot_address.is_some() {
                            self.cpu1.set_interrupt(u16::from(interrupt), pending)?;
                        }
                    }
                }
            }
            let running_cpu1 = next_core == 1 && self.appcpu_boot_address.is_some();
            let next_pc = if running_cpu1 {
                self.cpu1.snapshot().pc
            } else {
                self.cpu.snapshot().pc
            };
            if self.breakpoints.contains(&next_pc) {
                break StopReason::Breakpoint;
            }
            if running_cpu1 {
                std::mem::swap(&mut self.cpu, &mut self.cpu1);
            }
            self.bus.clear_watchpoint_hit();
            match self.service_functional_rom() {
                Ok(true) => {
                    if running_cpu1 {
                        std::mem::swap(&mut self.cpu, &mut self.cpu1);
                    }
                    stats.instructions = stats.instructions.saturating_add(1);
                    self.now = self
                        .now
                        .checked_add(remu_core::SimDuration::TICK)
                        .map_err(|_| XtensaMachineError::TimeOverflow)?;
                    stats.time = self.now;
                    self.ledc.poll(self.now)?;
                    for handle in &self.mcpwm {
                        handle.poll(self.now)?;
                    }
                    if let Some(hit) = self.bus.take_watchpoint_hit() {
                        break StopReason::Watchpoint {
                            address: hit.address,
                            access: hit.kind,
                        };
                    }
                    next_core = if self.appcpu_boot_address.is_some() {
                        next_core ^ 1
                    } else {
                        0
                    };
                    continue;
                }
                Ok(false) => {}
                Err(message) => {
                    if running_cpu1 {
                        std::mem::swap(&mut self.cpu, &mut self.cpu1);
                    }
                    break StopReason::Fault(message);
                }
            }
            if (0x4200_0000..0x4400_0000).contains(&self.cpu.pc())
                && !self.instruction_cache_configured
            {
                let pc = self.cpu.pc();
                if running_cpu1 {
                    std::mem::swap(&mut self.cpu, &mut self.cpu1);
                }
                break StopReason::Fault(format!(
                    "ESP32-S3 IROM fetch at {pc:#010x} before instruction-cache configuration"
                ));
            }
            if self.windowed_handoff_pending {
                let pc = self.cpu.pc();
                let instruction = match (
                    self.bus.read(
                        u64::from(pc),
                        AccessWidth::HalfWord,
                        AccessKind::Execute,
                        self.now,
                    ),
                    self.bus.read(
                        u64::from(pc.wrapping_add(2)),
                        AccessWidth::Byte,
                        AccessKind::Execute,
                        self.now,
                    ),
                ) {
                    (Ok(low), Ok(high)) => (low as u32) | ((high as u32) << 16),
                    (Err(error), _) | (_, Err(error)) => {
                        if running_cpu1 {
                            std::mem::swap(&mut self.cpu, &mut self.cpu1);
                        }
                        break StopReason::Fault(format!(
                            "ESP32-S3 verified handoff fetch at {pc:#010x} failed: {error}"
                        ));
                    }
                };
                if instruction & 0x0000_0fff != 0x0136 {
                    let (ps, depth) = self.cpu.window_state();
                    if running_cpu1 {
                        std::mem::swap(&mut self.cpu, &mut self.cpu1);
                    }
                    break StopReason::Fault(format!(
                        "ESP32-S3 verified handoff at {pc:#010x} requires ENTRY for CALLX8 window setup (PS={ps:#010x}, window_depth={depth})"
                    ));
                }
                self.windowed_handoff_pending = false;
            }
            let mut pms_bus = pms::Esp32S3PmsBus::new(
                &mut self.bus,
                &self.pms,
                &self.world_controller,
                &self.extmem,
                u8::from(running_cpu1),
            );
            let outcome = match self.cpu.step(&mut pms_bus, self.now) {
                Ok(outcome) => outcome,
                Err(error) => {
                    if running_cpu1 {
                        std::mem::swap(&mut self.cpu, &mut self.cpu1);
                    }
                    break StopReason::Fault(format!("CPU{}: {error}", u8::from(running_cpu1)));
                }
            };
            if running_cpu1 {
                std::mem::swap(&mut self.cpu, &mut self.cpu1);
            }
            self.apply_pending_mmu_mappings()?;
            stats.instructions = stats.instructions.saturating_add(1);
            self.now = self
                .now
                .checked_add(outcome.elapsed)
                .map_err(|_| XtensaMachineError::TimeOverflow)?;
            stats.time = self.now;
            self.ledc.poll(self.now)?;
            self.sdm.poll(self.now)?;
            for handle in &self.mcpwm {
                handle.poll(self.now)?;
            }
            next_core = if self.appcpu_boot_address.is_some() {
                next_core ^ 1
            } else {
                0
            };
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
        let mut uart = self.uart.bytes();
        uart.extend(self.chip_uart.bytes());
        for auxiliary_uart in &self.auxiliary_uarts {
            uart.extend(auxiliary_uart.bytes());
        }
        let mut usb = self.usb_host.output();
        usb.extend(self.usb_serial_jtag.output());
        Ok(RunResult {
            target: TargetId::Esp32s3,
            reason,
            stats,
            cpu: self.cpu.snapshot(),
            secondary_cpu: self.appcpu_boot_address.map(|_| self.cpu1.snapshot()),
            exit_code: self.exit.code(),
            uart,
            usb,
            trace_digest: digest.finish(),
        })
    }
}

#[cfg(test)]
mod aux_tests;
#[cfg(test)]
mod tests;
