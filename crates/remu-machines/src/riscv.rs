use crate::arm::Rp2040UsbHost;
use crate::{
    MemoryKind, PinStimulus, SignalEdge, SignalStop, TargetId, matching_signal_stop,
    resolve_signal_stop, target_manifest,
};
use md5::{Digest, Md5};
use remu_bus::{
    AddressSpace, Endianness, MapError, Permissions, SharedBusAccessObserver, SharedMemory,
};
use remu_core::{
    AccessKind, AccessWidth, Bus, Cpu, CpuFault, CpuSnapshot, ResetKind, RunLimits, RunStats,
    SimTime, StepReason, StopReason,
};
use remu_cpu_riscv::{RiscVCpu, RiscVProfile, RiscVRegister};
use remu_devices::{
    EspAnalogI2c, EspGpio, EspI2s, EspSpiMem, EspTimerGroup, EspTimerGroupHandle,
    EspTimerGroupKind, EspUsbSerialJtag, EspUsbSerialJtagHandle, ExitDevice, ExitHandle,
    FunctionalGpio, FunctionalTimer, FunctionalUart, GpioHandle, RegisterBank, Rp2040Clocks,
    Rp2040Pll, Rp2040RegisterBank, Rp2040Timer, Rp2040TimerHandle, Rp2040UsbController,
    Rp2040UsbHandle, Rp2040Xosc, Rp2350BootRam, Rp2350XipMaintenance, RpPio, RpPioHandle,
    RpSioGpio, RpSioHandle, RpTimerLayout, SignalHub, TimerHandle, UartHandle, WchGpio, WchPfic,
    WchPficHandle, WchTimer, WchTimerHandle, WchUsart,
};
use remu_image::{
    EspExecutableImage, EspFlashImage, FirmwareArchitecture, FirmwareImage, Uf2Error, Uf2Image,
};
use remu_signals::{Logic, SignalError};
use remu_trace::{TraceDigest, TraceError, TraceSink};
use serde::Serialize;
use sha2::{Sha224, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

mod bootrom_support;
mod esp_bootrom_primary;
mod esp_bootrom_secondary;
mod heap;
use heap::EspFunctionalHeap;
mod image;
mod rp_bootrom;

/// Synthetic, stable GPIO facade used by compiler cases.
pub const TEST_GPIO: u64 = 0xffff_0000;
/// Synthetic, stable UART facade used by compiler cases.
pub const TEST_UART: u64 = 0xffff_0100;
/// Synthetic, stable timer facade used by compiler cases.
pub const TEST_TIMER: u64 = 0xffff_0200;
/// Synthetic, stable exit word used by compiler cases.
pub const TEST_EXIT: u64 = 0xffff_fff0;
pub(crate) const TEST_DEVICE_SIZE: usize = 0x100;
pub(crate) const TEST_EXIT_SIZE: usize = 0x10;
const TIMER_INTERRUPT: u16 = 7;
const ESP_ROM_FLASH_START_STUB: u32 = 0x4004_fe00;
const ESP_ROM_FLASH_END_STUB: u32 = 0x4004_fe04;
const ESP_ROM_FLASH_CHIP_CHECK_STUB: u32 = 0x4004_fe08;
const ESP_ROM_FLASH_DETECT_SIZE_STUB: u32 = 0x4004_fe0c;
const ESP_ROM_FLASH_OK_STUB: u32 = 0x4004_fe10;
const ESP_ROM_COEX_VERSION: u32 = 0x4004_fdc0;
const ESP_ROM_DEFAULT_FLASH: u32 = 0x4087_fa00;
const ESP_ROM_FLASH_DRIVER: u32 = 0x4087_f900;
const ESP_ROM_FLASH_HOST: u32 = 0x4087_f700;
const ESP_FUNCTIONAL_MMAP_BASE: u32 = 0x4280_0000;
const ESP32C6_SYSTIMER_BASE: u64 = 0x6000_a000;
const ESP32C6_SYSTIMER_TARGET_VALUE: u64 = ESP32C6_SYSTIMER_BASE + 0x1c;
const ESP32C6_SYSTIMER_TARGET_CONF: u64 = ESP32C6_SYSTIMER_BASE + 0x34;
const ESP32C6_SYSTIMER_INT_ENA: u64 = ESP32C6_SYSTIMER_BASE + 0x64;

#[derive(Clone, Debug, Default)]
struct EspFunctionalSha256 {
    sha224: bool,
    input: Vec<u8>,
}

/// Failure while constructing, loading, or running a machine.
#[derive(Debug, Error)]
pub enum MachineError {
    /// Target does not expose the selected CPU family.
    #[error("target {0} does not currently have a runnable RISC-V profile")]
    UnsupportedTarget(TargetId),
    /// Address map construction failed.
    #[error(transparent)]
    Map(#[from] MapError),
    /// CPU construction or configuration failed.
    #[error(transparent)]
    Cpu(#[from] CpuFault),
    /// Signal model construction failed.
    #[error(transparent)]
    Signal(#[from] SignalError),
    /// Peripheral host operation failed.
    #[error("device operation failed: {0}")]
    Device(#[from] remu_bus::DeviceError),
    /// Memory or MMIO access failed while servicing the machine.
    #[error(transparent)]
    Bus(#[from] remu_core::BusFault),
    /// Trace output failed.
    #[error(transparent)]
    Trace(#[from] TraceError),
    /// Firmware architecture does not match this machine.
    #[error("firmware architecture {actual:?} does not match RISC-V target {target}")]
    Architecture {
        /// Selected target.
        target: TargetId,
        /// Architecture in the ELF header.
        actual: FirmwareArchitecture,
    },
    /// Firmware entry cannot be represented by an RV32 CPU.
    #[error("firmware entry {0:#x} exceeds the RV32 address space")]
    EntryRange(u64),
    /// Firmware segment could not be loaded.
    #[error("cannot load firmware segment at {address:#x}: {message}")]
    Load {
        /// Segment start.
        address: u64,
        /// Bus diagnostic.
        message: String,
    },
    /// Run limits are required to make a non-halting run bounded.
    #[error("at least one run limit is required")]
    MissingRunLimit,
    /// Virtual time overflowed.
    #[error("simulation time overflow")]
    TimeOverflow,
    /// UF2 parsing or flash reconstruction failed.
    #[error(transparent)]
    Uf2(#[from] Uf2Error),
    /// The UF2 target family does not select RP2350 RISC-V.
    #[error("UF2 family {actual:#010x} does not match RP2350 RISC-V; expected {expected:#010x}")]
    Uf2Family {
        /// Required RISC-V family identifier.
        expected: u32,
        /// Family identifier present in the artifact.
        actual: u32,
    },
    /// The RP2350 RISC-V image-definition entry or initial stack is invalid.
    #[error("invalid RP2350 RISC-V boot block: {0}")]
    BootBlock(String),
    /// An esptool application image cannot follow the ESP32-C6 boot handoff.
    #[error("ESP32-C6 application image is not boot-compatible: {0}")]
    Esp32c6BootLayout(String),
}

/// Stable machine-readable outcome of one invocation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RunResult {
    /// Target identity.
    pub target: TargetId,
    /// Terminal reason.
    pub reason: StopReason,
    /// Execution counters.
    pub stats: RunStats,
    /// Final architectural state.
    pub cpu: CpuSnapshot,
    /// Final state of a second processor, when the target is multicore.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary_cpu: Option<CpuSnapshot>,
    /// Optional code written to [`TEST_EXIT`].
    pub exit_code: Option<u32>,
    /// Bytes written to the compiler UART facade.
    pub uart: Vec<u8>,
    /// Bytes transmitted to the functional USB host.
    pub usb: Vec<u8>,
    /// Canonical digest over signal declarations and changes.
    pub trace_digest: String,
}

/// Runnable direct-ELF RISC-V vertical slice.
pub struct RiscVMachine {
    target: TargetId,
    cpu: RiscVCpu,
    cpu1: RiscVCpu,
    cpu1_active: bool,
    sio: Option<RpSioHandle>,
    bus: AddressSpace,
    signals: SignalHub,
    gpio: GpioHandle,
    chip_gpio: Vec<GpioHandle>,
    uart: UartHandle,
    chip_uarts: Vec<UartHandle>,
    timer: TimerHandle,
    exit: ExitHandle,
    now: SimTime,
    bootrom_services: BTreeMap<u32, u32>,
    esp_cpu_frequency_mhz: u32,
    esp_enabled_watchdogs: BTreeSet<u32>,
    esp_interrupt_routes: BTreeMap<u32, u32>,
    esp_enabled_interrupts: BTreeSet<u32>,
    esp_interrupt_priorities: BTreeMap<u32, u32>,
    esp_interrupt_threshold: u32,
    esp_md5_contexts: BTreeMap<u32, Vec<u8>>,
    esp_sha256_contexts: BTreeMap<u32, EspFunctionalSha256>,
    esp_heaps: BTreeMap<u32, EspFunctionalHeap>,
    esp_systimer_offset: u64,
    esp_systimer_alarms: [u64; 3],
    esp_systimer_periods: [u64; 3],
    esp_systimer_next: [u64; 3],
    esp_systimer_interrupt_enabled: [bool; 3],
    esp_systimer_raw: u8,
    esp_flash_guard: u32,
    esp_flash: Vec<u8>,
    esp_timer_groups: Vec<EspTimerGroupHandle>,
    flash_storage: Option<SharedMemory>,
    chip_timers: Vec<Rp2040TimerHandle>,
    pio: Vec<RpPioHandle>,
    wch_timer: Option<WchTimerHandle>,
    wch_pfic: Option<WchPficHandle>,
    usb: Option<Rp2040UsbHandle>,
    usb_dpram: Option<SharedMemory>,
    usb_host: Option<Rp2040UsbHost>,
    esp_usb_serial_jtag: Option<EspUsbSerialJtagHandle>,
    stop_on_usb_input_complete: bool,
    breakpoints: BTreeSet<u64>,
    signal_stops: Vec<SignalStop>,
}

impl RiscVMachine {
    /// Builds a RISC-V mode for WCH, ESP32-C6, or RP2350 Hazard3.
    pub fn new(target: TargetId) -> Result<Self, MachineError> {
        let profile = match target {
            TargetId::Ch32v003 => RiscVProfile::ch32v003(),
            TargetId::Ch32v006 => RiscVProfile::ch32v006(),
            TargetId::Esp32c6 => RiscVProfile::esp32c6(),
            TargetId::Rp2350 => RiscVProfile::rp2350_hazard3(),
            TargetId::Rp2040
            | TargetId::Esp32s3
            | TargetId::Atsamd21e18
            | TargetId::Stm32l432kc
            | TargetId::R7fa4m1ab3cfm
            | TargetId::Atmega328pb
            | TargetId::Msp430fr2433
            | TargetId::Pic16f15376
            | TargetId::Efm8bb52f32g => {
                return Err(MachineError::UnsupportedTarget(target));
            }
        };
        let manifest = target_manifest(target);
        let mut bus = AddressSpace::new(Endianness::Little);
        let mut chip_timers = Vec::new();
        let mut pio = Vec::new();
        let mut usb = None;
        let mut usb_dpram = None;
        let mut usb_host = None;
        let mut esp_usb_serial_jtag = None;
        let mut esp_timer_groups = Vec::new();
        let mut wch_timer = None;
        let mut wch_pfic = None;
        let mut sio = None;
        if target == TargetId::Rp2350 {
            let mut rom = vec![0; 32 * 1024];
            // Functional core-1 return point: the physical ROM parks a hart
            // after its entry function returns.
            rom[0x80..0x84].copy_from_slice(&0x1050_0073_u32.to_le_bytes());
            // RISC-V code obtains the function and data lookup entries through
            // the two well-known halfwords immediately below BOOTROM_ENTRY_OFFSET.
            // Both are serviced by the functional lookup gateway below.
            rom[0x7df8..0x7dfa].copy_from_slice(&0x0020_u16.to_le_bytes());
            rom[0x7dfa..0x7dfc].copy_from_slice(&0x0020_u16.to_le_bytes());
            bus.map_write_ignored_rom("rp2350.bootrom-functional", 0, rom)?;
        }
        let mut flash_storage = None;
        for region in manifest.memory {
            match region.kind {
                MemoryKind::Ram => {
                    if target == TargetId::Esp32c6 {
                        bus.map_shared(
                            region.name,
                            region.start,
                            region.size,
                            if region.executable {
                                Permissions::RWX
                            } else {
                                Permissions::RW
                            },
                            SharedMemory::from_bytes(vec![0xa5; region.size]),
                            0,
                        )?;
                    } else {
                        bus.map_ram(region.name, region.start, region.size, region.executable)?;
                    }
                }
                MemoryKind::Flash | MemoryKind::Rom => {
                    let storage = if region.kind == MemoryKind::Flash {
                        SharedMemory::from_bytes(vec![0xff; region.size])
                    } else {
                        SharedMemory::zeroed(region.size)
                    };
                    if region.kind == MemoryKind::Flash {
                        flash_storage = Some(storage.clone());
                    }
                    bus.map_shared(
                        region.name,
                        region.start,
                        region.size,
                        remu_bus::Permissions::RX,
                        storage,
                        0,
                    )?;
                }
            }
        }
        if target == TargetId::Rp2350 {
            let storage = flash_storage
                .as_ref()
                .expect("RP2350 manifest includes XIP flash");
            let size = storage.len();
            bus.map_shared(
                "rp2350.xip-nocache-noalloc",
                0x1400_0000,
                size,
                Permissions::RX,
                storage.clone(),
                0,
            )?;
            let mut psm_reset = vec![0; 4];
            psm_reset[3] = 0x0fff_ffff;
            bus.map_device(
                "rp2350.psm",
                0x4001_8000,
                0x4000,
                Box::new(Rp2040RegisterBank::new("rp2350.psm", psm_reset)),
            )?;
            bus.map_device(
                "rp2350.xip-maintenance",
                0x1800_0000,
                0x0400_0000,
                Box::new(Rp2350XipMaintenance::new("rp2350.xip-maintenance")),
            )?;
            bus.map_shared(
                "rp2350.xip-nocache-noalloc-notranslate",
                0x1c00_0000,
                size,
                Permissions::RX,
                storage.clone(),
                0,
            )?;
            bus.map_device(
                "rp2350.clocks",
                0x4001_0000,
                0x4000,
                Box::new(Rp2040Clocks::new("rp2350.clocks")),
            )?;
            bus.map_device(
                "rp2350.xosc",
                0x4004_8000,
                0x4000,
                Box::new(Rp2040Xosc::new("rp2350.xosc")),
            )?;
            bus.map_device(
                "rp2350.pll-sys",
                0x4005_0000,
                0x4000,
                Box::new(Rp2040Pll::new("rp2350.pll-sys")),
            )?;
            bus.map_device(
                "rp2350.pll-usb",
                0x4005_8000,
                0x4000,
                Box::new(Rp2040Pll::new("rp2350.pll-usb")),
            )?;
            let mut reset_values = vec![0; 0x1000 / 4];
            reset_values[0] = u32::MAX;
            reset_values[2] = u32::MAX;
            bus.map_device(
                "rp2350.resets",
                0x4002_0000,
                0x4000,
                Box::new(Rp2040RegisterBank::new("rp2350.resets", reset_values)),
            )?;
            bus.map_device(
                "rp2350.io-bank0",
                0x4002_8000,
                0x4000,
                Box::new(Rp2040RegisterBank::new(
                    "rp2350.io-bank0",
                    vec![0; 0x1000 / 4],
                )),
            )?;
            let mut io_qspi_reset = vec![0; 0x1000 / 4];
            for offset in (0..=0x28).step_by(8) {
                io_qspi_reset[offset / 4] = 1 << 9;
            }
            bus.map_device(
                "rp2350.io-qspi",
                0x4003_0000,
                0x4000,
                Box::new(Rp2040RegisterBank::new("rp2350.io-qspi", io_qspi_reset)),
            )?;
            let mut bank_pad_reset = vec![0x56; 0x1000 / 4];
            bank_pad_reset[0] = 0;
            bus.map_device(
                "rp2350.pads-bank0",
                0x4003_8000,
                0x4000,
                Box::new(Rp2040RegisterBank::new("rp2350.pads-bank0", bank_pad_reset)),
            )?;
            let mut qspi_pad_reset = vec![0x56; 0x1000 / 4];
            qspi_pad_reset[0] = 0;
            bus.map_device(
                "rp2350.pads-qspi",
                0x4004_0000,
                0x4000,
                Box::new(Rp2040RegisterBank::new("rp2350.pads-qspi", qspi_pad_reset)),
            )?;
            bus.map_device(
                "rp2350.xip-qmi",
                0x400d_0000,
                0x4000,
                Box::new(Rp2040RegisterBank::new(
                    "rp2350.xip-qmi",
                    vec![0; 0x1000 / 4],
                )),
            )?;
            bus.map_device(
                "rp2350.bootram",
                0x400e_0000,
                0x1000,
                Box::new(Rp2350BootRam::new("rp2350.bootram")),
            )?;
            bus.map_device(
                "rp2350.ticks",
                0x4010_8000,
                0x4000,
                Box::new(Rp2040RegisterBank::new("rp2350.ticks", vec![0; 0x1000 / 4])),
            )?;
            bus.map_device(
                "rp2350.powman",
                0x4010_0000,
                0x4000,
                Box::new(Rp2040RegisterBank::new(
                    "rp2350.powman",
                    vec![0; 0x1000 / 4],
                )),
            )?;
            for (name, base) in [
                ("rp2350.uart1", 0x4007_8000),
                ("rp2350.spi0", 0x4008_0000),
                ("rp2350.spi1", 0x4008_8000),
                ("rp2350.i2c0", 0x4009_0000),
                ("rp2350.i2c1", 0x4009_8000),
                ("rp2350.adc", 0x400a_0000),
                ("rp2350.pwm", 0x400a_8000),
                ("rp2350.dma", 0x5000_0000),
                ("rp2350.pio1", 0x5030_0000),
                ("rp2350.pio2", 0x5040_0000),
            ] {
                bus.map_device(
                    name,
                    base,
                    0x4000,
                    Box::new(Rp2040RegisterBank::new(name, vec![0; 0x1000 / 4])),
                )?;
            }
            for (name, base) in [
                ("rp2350.timer0", 0x400b_0000),
                ("rp2350.timer1", 0x400b_8000),
            ] {
                let (timer, timer_handle) = Rp2040Timer::new(name, RpTimerLayout::Rp2350);
                bus.map_device(name, base, 0x4000, Box::new(timer))?;
                chip_timers.push(timer_handle);
            }
            let dpram = SharedMemory::zeroed(0x1000);
            bus.map_shared(
                "rp2350.usb-dpram",
                0x5010_0000,
                0x1000,
                Permissions::RW,
                dpram.clone(),
                0,
            )?;
            usb_dpram = Some(dpram);
            let (usb_controller, usb_handle) =
                Rp2040UsbController::new_with_handle("rp2350.usbctrl");
            bus.map_device(
                "rp2350.usbctrl",
                0x5011_0000,
                0x4000,
                Box::new(usb_controller),
            )?;
            usb = Some(usb_handle);
            usb_host = Some(Rp2040UsbHost::new());
        }

        let signals = SignalHub::new();
        let facade_pins = manifest.gpio_count.min(32);
        let (gpio_device, gpio) = FunctionalGpio::new(
            format!("{target}.compiler-gpio"),
            facade_pins,
            &format!("board.{target}.gpio"),
            signals.clone(),
            0,
            4,
            8,
        )?;
        let (uart_device, uart) = FunctionalUart::new(format!("{target}.compiler-uart"), 0, 4, 1);
        let (timer_device, timer) = FunctionalTimer::new(format!("{target}.compiler-timer"));
        let (exit_device, exit) = ExitDevice::new(format!("{target}.compiler-exit"));
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

        let mut chip_gpio = Vec::new();
        let mut chip_uarts = Vec::new();
        match target {
            TargetId::Ch32v003 | TargetId::Ch32v006 => {
                for (port, base) in [
                    ("gpioa", 0x4001_0800),
                    ("gpioc", 0x4001_1000),
                    ("gpiod", 0x4001_1400),
                ] {
                    let (device, handle) = WchGpio::new(
                        format!("{target}.{port}"),
                        16,
                        &format!("board.{target}.{port}"),
                        signals.clone(),
                    )?;
                    bus.map_device(format!("{target}.{port}"), base, 0x400, Box::new(device))?;
                    chip_gpio.push(handle);
                }
                let rcc = RegisterBank::new(
                    format!("{target}.rcc"),
                    [
                        (0x00, 0x0000_0083, u32::MAX),
                        (0x04, 0, u32::MAX),
                        (0x08, 0, u32::MAX),
                        (0x0c, 0, u32::MAX),
                        (0x10, 0, u32::MAX),
                        (0x14, 0x0000_0014, u32::MAX),
                        (0x18, 0, u32::MAX),
                        (0x1c, 0, u32::MAX),
                        (0x20, 0, u32::MAX),
                        (0x24, 0x0c00_0000, u32::MAX),
                    ],
                );
                bus.map_device(format!("{target}.rcc"), 0x4002_1000, 0x400, Box::new(rcc))?;
                let (wch_uart, handle) = WchUsart::new(format!("{target}.usart1"));
                bus.map_device(
                    format!("{target}.usart1"),
                    0x4001_3800,
                    0x400,
                    Box::new(wch_uart),
                )?;
                chip_uarts.push(handle);
                let (tim2, handle) = WchTimer::new(format!("{target}.tim2"));
                bus.map_device(format!("{target}.tim2"), 0x4000_0000, 0x400, Box::new(tim2))?;
                wch_timer = Some(handle);
                let (pfic, handle) = WchPfic::new(format!("{target}.pfic"));
                bus.map_device(
                    format!("{target}.pfic"),
                    0xe000_e000,
                    0x1000,
                    Box::new(pfic),
                )?;
                wch_pfic = Some(handle);
            }
            TargetId::Esp32c6 => {
                bus.map_device(
                    "esp32c6.interrupt-controller",
                    0x2000_0000,
                    0x1000,
                    Box::new(Rp2040RegisterBank::new(
                        "esp32c6.interrupt-controller",
                        vec![0; 0x1000 / 4],
                    )),
                )?;
                bus.map_device(
                    "esp32c6.interrupt-controller-cpu0",
                    0x2000_1000,
                    0x1000,
                    Box::new(Rp2040RegisterBank::new(
                        "esp32c6.interrupt-controller-cpu0",
                        vec![0; 0x1000 / 4],
                    )),
                )?;
                bus.map_device(
                    "esp32c6.assist-debug",
                    0x600c_2000,
                    0x1000,
                    Box::new(Rp2040RegisterBank::new(
                        "esp32c6.assist-debug",
                        vec![0; 0x1000 / 4],
                    )),
                )?;
                bus.map_device(
                    "esp32c6.extmem",
                    0x600c_8000,
                    0x1000,
                    Box::new(Rp2040RegisterBank::new(
                        "esp32c6.extmem",
                        vec![0; 0x1000 / 4],
                    )),
                )?;
                bus.map_device(
                    "esp32c6.pcr",
                    0x6009_6000,
                    0x1000,
                    Box::new(Rp2040RegisterBank::new("esp32c6.pcr", vec![0; 0x1000 / 4])),
                )?;
                bus.map_device(
                    "esp32c6.lp-aon",
                    0x600b_1000,
                    0x1000,
                    Box::new(Rp2040RegisterBank::new(
                        "esp32c6.lp-aon",
                        vec![0; 0x1000 / 4],
                    )),
                )?;
                bus.map_device(
                    "esp32c6.modem-lpcon",
                    0x600a_f000,
                    0x1000,
                    Box::new(EspAnalogI2c::new("esp32c6.modem-lpcon")),
                )?;
                bus.map_device(
                    "esp32c6.spimem0",
                    0x6000_2000,
                    0x1000,
                    Box::new(EspSpiMem::new("esp32c6.spimem0")),
                )?;
                bus.map_device(
                    "esp32c6.spimem1",
                    0x6000_3000,
                    0x1000,
                    Box::new(EspSpiMem::new("esp32c6.spimem1")),
                )?;
                let (i2s, _) = EspI2s::new("esp32c6.i2s", "board.esp32c6.i2s", signals.clone())?;
                bus.map_device("esp32c6.i2s", 0x6000_c000, 0x1000, Box::new(i2s))?;
                for (name, base) in [
                    ("esp32c6.i2c0", 0x6000_4000),
                    ("esp32c6.uhci0", 0x6000_5000),
                    ("esp32c6.rmt", 0x6000_6000),
                    ("esp32c6.ledc", 0x6000_7000),
                    ("esp32c6.systimer", 0x6000_a000),
                    ("esp32c6.twai0", 0x6000_b000),
                    ("esp32c6.twai1", 0x6000_d000),
                    ("esp32c6.interrupt-matrix", 0x6001_0000),
                    ("esp32c6.atomic", 0x6001_1000),
                    ("esp32c6.pcnt", 0x6001_2000),
                    ("esp32c6.etm", 0x6001_3000),
                    ("esp32c6.mcpwm", 0x6001_4000),
                    ("esp32c6.parlio", 0x6001_5000),
                    ("esp32c6.hinf", 0x6001_6000),
                    ("esp32c6.slc", 0x6001_7000),
                    ("esp32c6.slchost", 0x6001_8000),
                    ("esp32c6.pvt-monitor", 0x6001_9000),
                    ("esp32c6.gdma", 0x6008_0000),
                    ("esp32c6.spi2", 0x6008_1000),
                    ("esp32c6.aes", 0x6008_8000),
                    ("esp32c6.sha", 0x6008_9000),
                    ("esp32c6.rsa", 0x6008_a000),
                    ("esp32c6.ecc", 0x6008_b000),
                    ("esp32c6.ds", 0x6008_c000),
                    ("esp32c6.hmac", 0x6008_d000),
                    ("esp32c6.io-mux", 0x6009_0000),
                    ("esp32c6.mem-monitor", 0x6009_2000),
                    ("esp32c6.pau", 0x6009_3000),
                    ("esp32c6.hp-system", 0x6009_5000),
                    ("esp32c6.tee", 0x6009_8000),
                    ("esp32c6.hp-apm", 0x6009_9000),
                    ("esp32c6.misc", 0x6009_f000),
                    ("esp32c6.power-detector", 0x600a_0000),
                    ("esp32c6.ieee802154", 0x600a_3000),
                    ("esp32c6.modem-syscon", 0x600a_9800),
                    ("esp32c6.pmu-efuse-lp-timer", 0x600b_0000),
                    ("esp32c6.lp-io-analog", 0x600b_2000),
                    ("esp32c6.lp-security-debug", 0x600b_3000),
                    ("esp32c6.trace", 0x600c_0000),
                    ("esp32c6.interrupt-priority", 0x600c_5000),
                ] {
                    bus.map_device(
                        name,
                        base,
                        0x1000,
                        Box::new(Rp2040RegisterBank::new(name, vec![0; 0x1000 / 4])),
                    )?;
                }
                let (usb_serial_jtag, handle) = EspUsbSerialJtag::new("esp32c6.usb-serial-jtag");
                bus.map_device(
                    "esp32c6.usb-serial-jtag",
                    0x6000_f000,
                    0x1000,
                    Box::new(usb_serial_jtag),
                )?;
                esp_usb_serial_jtag = Some(handle);
                let mut saradc_reset = vec![0; 0x1000 / 4];
                saradc_reset[0x2c / 4] = 2048;
                saradc_reset[0x44 / 4] = (1 << 31) | (1 << 30);
                bus.map_device(
                    "esp32c6.saradc",
                    0x6000_e000,
                    0x1000,
                    Box::new(Rp2040RegisterBank::new("esp32c6.saradc", saradc_reset)),
                )?;
                for (name, base) in [
                    ("esp32c6.timer-group0", 0x6000_8000),
                    ("esp32c6.timer-group1", 0x6000_9000),
                ] {
                    let (device, handle) = EspTimerGroup::new(name, EspTimerGroupKind::Esp32C6);
                    bus.map_device(name, base, 0x1000, Box::new(device))?;
                    esp_timer_groups.push(handle);
                }
                let (device, handle) = EspGpio::new(
                    "esp32c6.gpio",
                    31,
                    "board.esp32c6.chip_gpio",
                    signals.clone(),
                )?;
                bus.map_device("esp32c6.gpio", 0x6009_1000, 0x1000, Box::new(device))?;
                chip_gpio.push(handle);
                let (uart0, handle) = FunctionalUart::new_lenient("esp32c6.uart0", 0x00, 0x1c, 0);
                bus.map_device("esp32c6.uart0", 0x6000_0000, 0x1000, Box::new(uart0))?;
                chip_uarts.push(handle);
            }
            TargetId::Rp2350 => {
                let (device, handle, multicore) = RpSioGpio::new_rp2350_with_multicore(
                    "rp2350.sio",
                    32,
                    "board.rp2350.chip_gpio",
                    signals.clone(),
                )?;
                bus.map_device("rp2350.sio", 0xd000_0000, 0x200, Box::new(device))?;
                chip_gpio.push(handle);
                sio = Some(multicore);
                let (uart0, handle) =
                    FunctionalUart::new_lenient("rp2350.uart0", 0x00, 0x18, 0x0090);
                bus.map_device("rp2350.uart0", 0x4007_0000, 0x1000, Box::new(uart0))?;
                chip_uarts.push(handle);
                let (pio0, handle) = RpPio::new(
                    "rp2350.pio0",
                    u16::from(manifest.gpio_count.min(32)),
                    "board.rp2350.pio0.gpio",
                    signals.clone(),
                )?;
                bus.map_device("rp2350.pio0", 0x5020_0000, 0x4000, Box::new(pio0))?;
                pio.push(handle);
            }
            TargetId::Rp2040
            | TargetId::Esp32s3
            | TargetId::Atsamd21e18
            | TargetId::Stm32l432kc
            | TargetId::R7fa4m1ab3cfm
            | TargetId::Atmega328pb
            | TargetId::Msp430fr2433
            | TargetId::Pic16f15376
            | TargetId::Efm8bb52f32g => unreachable!(),
        }

        Ok(Self {
            target,
            cpu: RiscVCpu::new(profile.clone())?,
            cpu1: RiscVCpu::new(profile)?,
            cpu1_active: false,
            sio,
            bus,
            signals,
            gpio,
            chip_gpio,
            uart,
            chip_uarts,
            timer,
            exit,
            now: SimTime::ZERO,
            bootrom_services: BTreeMap::new(),
            esp_cpu_frequency_mhz: 40,
            esp_enabled_watchdogs: BTreeSet::new(),
            esp_interrupt_routes: BTreeMap::new(),
            esp_enabled_interrupts: BTreeSet::new(),
            esp_interrupt_priorities: BTreeMap::new(),
            esp_interrupt_threshold: 0,
            esp_md5_contexts: BTreeMap::new(),
            esp_sha256_contexts: BTreeMap::new(),
            esp_heaps: BTreeMap::new(),
            esp_systimer_offset: 0,
            esp_systimer_alarms: [u64::MAX; 3],
            esp_systimer_periods: [0; 3],
            esp_systimer_next: [u64::MAX; 3],
            esp_systimer_interrupt_enabled: [false; 3],
            esp_systimer_raw: 0,
            esp_flash_guard: 0,
            esp_flash: Vec::new(),
            esp_timer_groups,
            flash_storage,
            chip_timers,
            pio,
            wch_timer,
            wch_pfic,
            usb,
            usb_dpram,
            usb_host,
            esp_usb_serial_jtag,
            stop_on_usb_input_complete: false,
            breakpoints: BTreeSet::new(),
            signal_stops: Vec::new(),
        })
    }

    fn service_functional_bootrom(&mut self) -> Result<bool, String> {
        if self.target == TargetId::Esp32c6 {
            let pc = self.cpu.pc();
            let result = (|| {
                if self.service_esp32c6_bootrom_primary(pc)? {
                    return Ok(true);
                }
                self.service_esp32c6_bootrom_secondary(pc)
            })();
            return result
                .map_err(|error| format!("ESP32-C6 functional service at PC {pc:#010x}: {error}"));
        }
        self.service_rp2350_bootrom()
    }

    /// Selected target.
    pub const fn target(&self) -> TargetId {
        self.target
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
    pub fn debug_snapshot(&self) -> CpuSnapshot {
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
    pub fn add_signal_stop(&mut self, path: &str, edge: SignalEdge) -> Result<(), MachineError> {
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

    /// Drives or releases one compiler-facade GPIO input.
    pub fn set_pin(&self, pin: u8, value: Logic) -> Result<(), MachineError> {
        self.gpio.set_input(pin, value, self.now)?;
        for gpio in &self.chip_gpio {
            if usize::from(pin) < gpio.pin_count() {
                gpio.set_input(pin, value, self.now)?;
            }
        }
        Ok(())
    }

    /// Applies a power-on reset to the CPU and devices.
    pub fn reset(&mut self) -> Result<(), MachineError> {
        self.bus.reset_devices(ResetKind::PowerOn);
        self.cpu.reset(ResetKind::PowerOn, &mut self.bus)?;
        self.cpu1.reset(ResetKind::PowerOn, &mut self.bus)?;
        self.cpu1_active = false;
        self.now = SimTime::ZERO;
        self.esp_cpu_frequency_mhz = 40;
        self.esp_enabled_watchdogs.clear();
        self.esp_interrupt_routes.clear();
        self.esp_enabled_interrupts.clear();
        self.esp_interrupt_priorities.clear();
        self.esp_interrupt_threshold = 0;
        self.esp_md5_contexts.clear();
        self.esp_sha256_contexts.clear();
        self.esp_heaps.clear();
        self.esp_systimer_offset = 0;
        self.esp_systimer_alarms = [u64::MAX; 3];
        self.esp_systimer_periods = [0; 3];
        self.esp_systimer_next = [u64::MAX; 3];
        self.esp_systimer_interrupt_enabled = [false; 3];
        self.esp_systimer_raw = 0;
        self.esp_flash_guard = 0;
        Ok(())
    }

    /// Runs until a terminal condition and optionally streams signal changes.
    pub fn run(
        &mut self,
        limits: RunLimits,
        trace: Option<&mut dyn TraceSink>,
    ) -> Result<RunResult, MachineError> {
        self.run_with_stimuli(limits, &[], trace)
    }

    /// Runs with timestamped external GPIO stimulus.
    pub fn run_with_stimuli(
        &mut self,
        limits: RunLimits,
        stimuli: &[PinStimulus],
        mut trace: Option<&mut dyn TraceSink>,
    ) -> Result<RunResult, MachineError> {
        if limits.instructions.is_none() && limits.deadline.is_none() {
            return Err(MachineError::MissingRunLimit);
        }

        let mut digest = TraceDigest::new();
        self.signals.with_registry(|registry| {
            digest.begin(registry);
            if let Some(sink) = trace.as_deref_mut() {
                sink.begin(registry)
            } else {
                Ok(())
            }
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
        let mut wch_timer_was_pending = false;
        let mut chip_timer_was_pending = 0_u16;
        let mut esp_crosscore_was_pending = false;
        let mut esp_usb_was_pending = false;
        let mut esp_timer_was_pending = [[false; 2]; 2];
        let reason = loop {
            if let Some(sio) = &self.sio {
                sio.select_core(0);
            }
            while stimuli
                .get(next_stimulus)
                .is_some_and(|stimulus| stimulus.at <= self.now)
            {
                let stimulus = stimuli[next_stimulus];
                self.set_pin(stimulus.pin, stimulus.value)?;
                stats.events = stats.events.saturating_add(1);
                next_stimulus += 1;
            }
            if let Some(code) = self.exit.code() {
                let _ = code;
                break StopReason::Halted;
            }
            if self.stop_on_usb_input_complete
                && (self
                    .usb_host
                    .as_ref()
                    .is_some_and(Rp2040UsbHost::input_complete)
                    || self
                        .esp_usb_serial_jtag
                        .as_ref()
                        .is_some_and(EspUsbSerialJtagHandle::input_complete))
            {
                break StopReason::HostInputComplete;
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

            let timer_pending = self.timer.poll(self.now);
            if timer_pending && !timer_was_pending {
                stats.events = stats.events.saturating_add(1);
            }
            timer_was_pending = timer_pending;
            self.cpu.set_interrupt(TIMER_INTERRUPT, timer_pending)?;
            if let (Some(timer), Some(pfic)) = (&self.wch_timer, &self.wch_pfic) {
                const TIM2_INTERRUPT: u16 = 38;
                let pending = timer.pending(self.now);
                pfic.set_pending(TIM2_INTERRUPT, pending);
                let deliver = pfic.next_pending() == Some(TIM2_INTERRUPT);
                if deliver && !wch_timer_was_pending {
                    stats.events = stats.events.saturating_add(1);
                }
                wch_timer_was_pending = deliver;
                self.cpu
                    .set_qingke_external_interrupt(TIM2_INTERRUPT, deliver)?;
            }
            if self.target == TargetId::Rp2350 {
                let chip_timer_pending =
                    self.chip_timers
                        .iter()
                        .enumerate()
                        .fold(0_u16, |pending, (timer, handle)| {
                            pending | (u16::from(handle.pending(self.now)) << (timer * 4))
                        });
                stats.events = stats.events.saturating_add(u64::from(
                    (chip_timer_pending & !chip_timer_was_pending).count_ones(),
                ));
                chip_timer_was_pending = chip_timer_pending;
                for pio in &self.pio {
                    if pio.poll(self.now)? {
                        stats.events = stats.events.saturating_add(1);
                    }
                }
                for line in 0..self.chip_timers.len() * 4 {
                    self.cpu.set_hazard3_external_interrupt(
                        u16::try_from(line).expect("RP timer IRQ line fits u16"),
                        chip_timer_pending & (1 << line) != 0,
                    )?;
                }
                if let Some(usb) = &self.usb {
                    if let (Some(host), Some(dpram)) = (&mut self.usb_host, &self.usb_dpram) {
                        stats.events = stats.events.saturating_add(host.poll(self.now, usb, dpram));
                    }
                    self.cpu
                        .set_hazard3_external_interrupt(14, usb.interrupt_pending())?;
                }
            }
            if self.target == TargetId::Esp32c6 {
                // ESP-IDF starts the first FreeRTOS task by raising the
                // FROM_CPU_INTR0 software interrupt. The C6 interrupt matrix
                // routes source 22 to a local CPU interrupt configured by the
                // ROM calls retained above.
                let crosscore_pending = self
                    .bus
                    .read(
                        0x600c_5090,
                        remu_core::AccessWidth::Word,
                        remu_core::AccessKind::Read,
                        self.now,
                    )
                    .map_err(MachineError::Bus)?
                    != 0;
                let interrupt = self.esp_interrupt_routes.get(&22).copied().unwrap_or(2);
                let priority = if interrupt < 32 {
                    self.bus
                        .read(
                            u64::from(0x2000_1010_u32 + interrupt * 4),
                            remu_core::AccessWidth::Word,
                            remu_core::AccessKind::Read,
                            self.now,
                        )
                        .map_err(MachineError::Bus)? as u32
                } else {
                    0
                };
                let threshold = self
                    .bus
                    .read(
                        0x2000_1090,
                        remu_core::AccessWidth::Word,
                        remu_core::AccessKind::Read,
                        self.now,
                    )
                    .map_err(MachineError::Bus)? as u32;
                let deliver = crosscore_pending
                    && self.esp_enabled_interrupts.contains(&interrupt)
                    && priority >= threshold;
                if deliver && !esp_crosscore_was_pending {
                    stats.events = stats.events.saturating_add(1);
                }
                esp_crosscore_was_pending = deliver;
                if interrupt < 32 {
                    self.cpu.set_interrupt(interrupt as u16, deliver)?;
                }
                for (group, handle) in self.esp_timer_groups.iter().enumerate() {
                    for (timer, pending) in handle.pending(self.now).into_iter().enumerate() {
                        let source = match (group, timer) {
                            (0, 0) => 51,
                            (1, 0) => 54,
                            _ => continue,
                        };
                        let Some(interrupt) = self.esp_interrupt_routes.get(&source).copied()
                        else {
                            continue;
                        };
                        let priority = self
                            .esp_interrupt_priorities
                            .get(&interrupt)
                            .copied()
                            .unwrap_or(0);
                        let deliver = pending
                            && self.esp_enabled_interrupts.contains(&interrupt)
                            && priority >= self.esp_interrupt_threshold;
                        if deliver && !esp_timer_was_pending[group][timer] {
                            stats.events = stats.events.saturating_add(1);
                        }
                        esp_timer_was_pending[group][timer] = deliver;
                        if interrupt < 32 {
                            self.cpu.set_interrupt(interrupt as u16, deliver)?;
                        }
                    }
                }
                const SYSTIMER_INT_RAW: u64 = 0x6000_a068;
                const SYSTIMER_INT_CLR: u64 = 0x6000_a06c;
                const SYSTIMER_INT_ST: u64 = 0x6000_a070;
                let clear = self
                    .bus
                    .read(
                        SYSTIMER_INT_CLR,
                        AccessWidth::Word,
                        AccessKind::Read,
                        self.now,
                    )
                    .map_err(MachineError::Bus)? as u8;
                if clear != 0 {
                    self.esp_systimer_raw &= !clear;
                    self.bus
                        .write(SYSTIMER_INT_CLR, AccessWidth::Word, 0, self.now)
                        .map_err(MachineError::Bus)?;
                }
                let counter = self.now.ticks().wrapping_add(self.esp_systimer_offset);
                for alarm in 0..self.esp_systimer_next.len() {
                    let due = self.esp_systimer_interrupt_enabled[alarm]
                        && counter >= self.esp_systimer_next[alarm];
                    if due {
                        self.esp_systimer_raw |= 1 << alarm;
                        let period = self.esp_systimer_periods[alarm];
                        self.esp_systimer_next[alarm] = if period == 0 {
                            u64::MAX
                        } else {
                            self.esp_systimer_next[alarm].wrapping_add(period)
                        };
                    }
                    let source = 57 + alarm as u32;
                    let Some(interrupt) = self.esp_interrupt_routes.get(&source).copied() else {
                        continue;
                    };
                    let priority = if interrupt < 32 {
                        self.bus
                            .read(
                                u64::from(0x2000_1010_u32 + interrupt * 4),
                                remu_core::AccessWidth::Word,
                                remu_core::AccessKind::Read,
                                self.now,
                            )
                            .map_err(MachineError::Bus)? as u32
                    } else {
                        0
                    };
                    let threshold = self
                        .bus
                        .read(
                            0x2000_1090,
                            remu_core::AccessWidth::Word,
                            remu_core::AccessKind::Read,
                            self.now,
                        )
                        .map_err(MachineError::Bus)? as u32;
                    let deliver = self.esp_systimer_raw & (1 << alarm) != 0
                        && self.esp_enabled_interrupts.contains(&interrupt)
                        && priority >= threshold;
                    if deliver {
                        stats.events = stats.events.saturating_add(1);
                    }
                    if interrupt < 32 {
                        self.cpu.set_interrupt(interrupt as u16, deliver)?;
                    }
                }
                let enabled_mask = self
                    .esp_systimer_interrupt_enabled
                    .iter()
                    .enumerate()
                    .fold(0_u8, |mask, (alarm, enabled)| {
                        mask | (u8::from(*enabled) << alarm)
                    });
                self.bus
                    .write(
                        SYSTIMER_INT_RAW,
                        AccessWidth::Word,
                        u64::from(self.esp_systimer_raw),
                        self.now,
                    )
                    .map_err(MachineError::Bus)?;
                self.bus
                    .write(
                        SYSTIMER_INT_ST,
                        AccessWidth::Word,
                        u64::from(self.esp_systimer_raw & enabled_mask),
                        self.now,
                    )
                    .map_err(MachineError::Bus)?;
                if let Some(usb) = &self.esp_usb_serial_jtag
                    && let Some(interrupt) = self.esp_interrupt_routes.get(&48).copied()
                {
                    let priority = if interrupt < 32 {
                        self.bus
                            .read(
                                u64::from(0x2000_1010_u32 + interrupt * 4),
                                remu_core::AccessWidth::Word,
                                remu_core::AccessKind::Read,
                                self.now,
                            )
                            .map_err(MachineError::Bus)? as u32
                    } else {
                        0
                    };
                    let threshold = self
                        .bus
                        .read(
                            0x2000_1090,
                            remu_core::AccessWidth::Word,
                            remu_core::AccessKind::Read,
                            self.now,
                        )
                        .map_err(MachineError::Bus)? as u32;
                    let deliver = usb.interrupt_pending()
                        && self.esp_enabled_interrupts.contains(&interrupt)
                        && priority >= threshold;
                    if deliver && !esp_usb_was_pending {
                        stats.events = stats.events.saturating_add(1);
                    }
                    esp_usb_was_pending = deliver;
                    if interrupt < 32 {
                        self.cpu.set_interrupt(interrupt as u16, deliver)?;
                    }
                }
            }
            self.bus.clear_watchpoint_hit();
            match self.service_functional_bootrom() {
                Ok(true) => {
                    stats.instructions = stats.instructions.saturating_add(1);
                    self.now = self
                        .now
                        .checked_add(remu_core::SimDuration::TICK)
                        .map_err(|_| MachineError::TimeOverflow)?;
                    stats.time = self.now;
                    if let Some(hit) = self.bus.take_watchpoint_hit() {
                        break StopReason::Watchpoint {
                            address: hit.address,
                            access: hit.kind,
                        };
                    }
                    continue;
                }
                Ok(false) => {}
                Err(message) => break StopReason::Fault(message),
            }
            let instruction_pc = self.cpu.pc();
            let outcome = match self.cpu.step(&mut self.bus, self.now) {
                Ok(outcome) => outcome,
                Err(error) => {
                    break StopReason::Fault(format!(
                        "RISC-V CPU fault at PC {instruction_pc:#010x}: {error}"
                    ));
                }
            };
            stats.instructions = stats.instructions.saturating_add(1);
            self.now = self
                .now
                .checked_add(outcome.elapsed)
                .map_err(|_| MachineError::TimeOverflow)?;
            stats.time = self.now;

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

            if let Some(launch) = self.sio.as_ref().and_then(RpSioHandle::take_core1_launch) {
                if let Err(error) = self.cpu1.set_trap_vector(launch.vector_table) {
                    break StopReason::Fault(format!("RISC-V hart 1 launch: {error}"));
                }
                if let Err(error) = self
                    .cpu1
                    .set_register(RiscVRegister::Sp, launch.stack_pointer)
                {
                    break StopReason::Fault(format!("RISC-V hart 1 launch: {error}"));
                }
                if let Err(error) = self.cpu1.set_register(RiscVRegister::Ra, 0x80) {
                    break StopReason::Fault(format!("RISC-V hart 1 launch: {error}"));
                }
                if let Err(error) = self.cpu1.set_pc(launch.entry) {
                    break StopReason::Fault(format!("RISC-V hart 1 launch: {error}"));
                }
                self.cpu1_active = true;
                stats.events = stats.events.saturating_add(1);
            }
            if self.cpu1_active {
                if let Some(sio) = &self.sio {
                    sio.select_core(1);
                }
                if self.breakpoints.contains(&self.cpu1.snapshot().pc) {
                    if let Some(sio) = &self.sio {
                        sio.select_core(0);
                    }
                    break StopReason::Breakpoint;
                }
                self.bus.clear_watchpoint_hit();
                std::mem::swap(&mut self.cpu, &mut self.cpu1);
                let hart1_rom = self.service_functional_bootrom();
                std::mem::swap(&mut self.cpu, &mut self.cpu1);
                match hart1_rom {
                    Ok(true) => {
                        stats.instructions = stats.instructions.saturating_add(1);
                        self.now = self
                            .now
                            .checked_add(remu_core::SimDuration::TICK)
                            .map_err(|_| MachineError::TimeOverflow)?;
                        stats.time = self.now;
                        if let Some(hit) = self.bus.take_watchpoint_hit() {
                            if let Some(sio) = &self.sio {
                                sio.select_core(0);
                            }
                            break StopReason::Watchpoint {
                                address: hit.address,
                                access: hit.kind,
                            };
                        }
                    }
                    Ok(false) => {
                        let instruction_pc = self.cpu1.pc();
                        let hart1_outcome = match self.cpu1.step(&mut self.bus, self.now) {
                            Ok(outcome) => outcome,
                            Err(error) => {
                                if let Some(sio) = &self.sio {
                                    sio.select_core(0);
                                }
                                break StopReason::Fault(format!(
                                    "RISC-V hart 1 fault at PC {instruction_pc:#010x}: {error}"
                                ));
                            }
                        };
                        stats.instructions = stats.instructions.saturating_add(1);
                        self.now = self
                            .now
                            .checked_add(hart1_outcome.elapsed)
                            .map_err(|_| MachineError::TimeOverflow)?;
                        stats.time = self.now;
                        if let Some(hit) = self.bus.take_watchpoint_hit() {
                            if let Some(sio) = &self.sio {
                                sio.select_core(0);
                            }
                            break StopReason::Watchpoint {
                                address: hit.address,
                                access: hit.kind,
                            };
                        }
                        match hart1_outcome.reason {
                            StepReason::Advanced | StepReason::WaitForInterrupt => {}
                            StepReason::Halted => self.cpu1_active = false,
                            StepReason::Breakpoint => {
                                if let Some(sio) = &self.sio {
                                    sio.select_core(0);
                                }
                                break StopReason::Breakpoint;
                            }
                        }
                    }
                    Err(message) => {
                        if let Some(sio) = &self.sio {
                            sio.select_core(0);
                        }
                        break StopReason::Fault(format!("RISC-V hart 1 ROM: {message}"));
                    }
                }
                if let Some(sio) = &self.sio {
                    sio.select_core(0);
                }
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
            }
        };

        if let Some(sink) = trace {
            sink.finish()?;
        }
        Ok(RunResult {
            target: self.target,
            reason,
            stats,
            cpu: self.cpu.snapshot(),
            secondary_cpu: self.cpu1_active.then(|| self.cpu1.snapshot()),
            exit_code: self.exit.code(),
            uart: {
                let mut bytes = self.uart.bytes();
                for uart in &self.chip_uarts {
                    bytes.extend(uart.bytes());
                }
                bytes
            },
            usb: self.esp_usb_serial_jtag.as_ref().map_or_else(
                || {
                    self.usb_host
                        .as_ref()
                        .map_or_else(Vec::new, Rp2040UsbHost::output)
                },
                EspUsbSerialJtagHandle::output,
            ),
            trace_digest: digest.finish(),
        })
    }
}

#[cfg(test)]
mod tests;
