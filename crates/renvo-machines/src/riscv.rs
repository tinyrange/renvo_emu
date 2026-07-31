use crate::arm::Rp2040UsbHost;
use crate::{
    MemoryKind, PinStimulus, SignalEdge, SignalStop, TargetId, matching_signal_stop,
    resolve_signal_stop, target_manifest,
};
use md5::{Digest, Md5};
use renvo_bus::{AddressSpace, Endianness, MapError, Permissions, SharedMemory};
use renvo_core::{
    AccessKind, AccessWidth, Bus, Cpu, CpuFault, CpuSnapshot, ResetKind, RunLimits, RunStats,
    SimTime, StepReason, StopReason,
};
use renvo_cpu_riscv::{RiscVCpu, RiscVProfile, RiscVRegister};
use renvo_devices::{
    EspAnalogI2c, EspGpio, EspSpiMem, EspTimerGroup, EspTimerGroupHandle, EspTimerGroupKind,
    EspUsbSerialJtag, EspUsbSerialJtagHandle, ExitDevice, ExitHandle, FunctionalGpio,
    FunctionalTimer, FunctionalUart, GpioHandle, RegisterBank, Rp2040Clocks, Rp2040Pll,
    Rp2040RegisterBank, Rp2040Timer, Rp2040TimerHandle, Rp2040UsbController, Rp2040UsbHandle,
    Rp2040Xosc, Rp2350BootRam, Rp2350XipMaintenance, RpPio, RpPioHandle, RpSioGpio, RpSioHandle,
    RpTimerLayout, SignalHub, TimerHandle, UartHandle, WchGpio, WchPfic, WchPficHandle, WchTimer,
    WchTimerHandle, WchUsart,
};
use renvo_image::{EspFlashImage, FirmwareArchitecture, FirmwareImage, Uf2Error, Uf2Image};
use renvo_signals::{Logic, SignalError};
use renvo_trace::{TraceDigest, TraceError, TraceSink};
use serde::Serialize;
use sha2::{Sha224, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

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
    Device(#[from] renvo_bus::DeviceError),
    /// Memory or MMIO access failed while servicing the machine.
    #[error(transparent)]
    Bus(#[from] renvo_core::BusFault),
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

#[derive(Debug)]
struct EspFunctionalHeap {
    free: BTreeMap<u32, u32>,
    allocations: BTreeMap<u32, u32>,
    minimum_free: u32,
}

impl EspFunctionalHeap {
    fn new(start: u32, size: u32) -> Option<Self> {
        const METADATA_RESERVE: u32 = 64;
        if size <= METADATA_RESERVE {
            return None;
        }
        let first = start.checked_add(METADATA_RESERVE)?;
        let available = size - METADATA_RESERVE;
        Some(Self {
            free: BTreeMap::from([(first, available)]),
            allocations: BTreeMap::new(),
            minimum_free: available,
        })
    }

    fn free_bytes(&self) -> u32 {
        self.free.values().copied().sum()
    }

    fn allocate(&mut self, size: u32, alignment: u32, offset: u32) -> Option<u32> {
        let size = size.max(1).checked_add(3)? & !3;
        let alignment = alignment.max(4);
        if !alignment.is_power_of_two() {
            return None;
        }
        let selected = self.free.iter().find_map(|(&start, &length)| {
            let adjusted = start.checked_add(offset)?;
            let aligned = adjusted.checked_add(alignment - 1)? & !(alignment - 1);
            let allocation = aligned.checked_sub(offset)?;
            let end = allocation.checked_add(size)?;
            (allocation >= start && end <= start.checked_add(length)?)
                .then_some((start, length, allocation, end))
        })?;
        let (free_start, free_length, allocation, allocation_end) = selected;
        self.free.remove(&free_start);
        if allocation > free_start {
            self.free.insert(free_start, allocation - free_start);
        }
        let free_end = free_start + free_length;
        if allocation_end < free_end {
            self.free.insert(allocation_end, free_end - allocation_end);
        }
        self.allocations.insert(allocation, size);
        self.minimum_free = self.minimum_free.min(self.free_bytes());
        Some(allocation)
    }

    fn release(&mut self, pointer: u32) -> bool {
        let Some(size) = self.allocations.remove(&pointer) else {
            return false;
        };
        self.free.insert(pointer, size);
        let ranges = std::mem::take(&mut self.free);
        for (start, length) in ranges {
            if let Some((&previous_start, &previous_length)) = self.free.last_key_value()
                && previous_start + previous_length == start
            {
                self.free.insert(previous_start, previous_length + length);
                continue;
            }
            self.free.insert(start, length);
        }
        true
    }
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
            TargetId::Rp2040 | TargetId::Esp32s3 => {
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
                    bus.map_ram(region.name, region.start, region.size, region.executable)?;
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
                        renvo_bus::Permissions::RX,
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
            "renvo.test.gpio",
            TEST_GPIO,
            TEST_DEVICE_SIZE,
            Box::new(gpio_device),
        )?;
        bus.map_device(
            "renvo.test.uart",
            TEST_UART,
            TEST_DEVICE_SIZE,
            Box::new(uart_device),
        )?;
        bus.map_device(
            "renvo.test.timer",
            TEST_TIMER,
            TEST_DEVICE_SIZE,
            Box::new(timer_device),
        )?;
        bus.map_device(
            "renvo.test.exit",
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
                for (name, base) in [
                    ("esp32c6.i2c0", 0x6000_4000),
                    ("esp32c6.uhci0", 0x6000_5000),
                    ("esp32c6.rmt", 0x6000_6000),
                    ("esp32c6.ledc", 0x6000_7000),
                    ("esp32c6.systimer", 0x6000_a000),
                    ("esp32c6.twai0", 0x6000_b000),
                    ("esp32c6.i2s", 0x6000_c000),
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
            TargetId::Rp2040 | TargetId::Esp32s3 => unreachable!(),
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

    fn complete_host_call(&mut self, result: u32) -> Result<(), String> {
        let return_address = self
            .cpu
            .register(RiscVRegister::Ra)
            .map_err(|error| error.to_string())?;
        self.cpu
            .set_register(RiscVRegister::A0, result)
            .map_err(|error| error.to_string())?;
        self.cpu
            .set_pc(return_address)
            .map_err(|error| error.to_string())
    }

    fn complete_host_call_u64(&mut self, result: u64) -> Result<(), String> {
        self.cpu
            .set_register(RiscVRegister::A1, (result >> 32) as u32)
            .map_err(|error| error.to_string())?;
        self.complete_host_call(result as u32)
    }

    fn read_guest_c_string(&mut self, address: u32, limit: usize) -> Result<Vec<u8>, String> {
        let mut bytes = Vec::new();
        for offset in 0..limit {
            let byte = self
                .bus
                .read(
                    u64::from(address.wrapping_add(offset as u32)),
                    renvo_core::AccessWidth::Byte,
                    renvo_core::AccessKind::Read,
                    self.now,
                )
                .map_err(|error| error.to_string())? as u8;
            if byte == 0 {
                return Ok(bytes);
            }
            bytes.push(byte);
        }
        Err(format!(
            "guest string at {address:#010x} exceeds {limit} bytes"
        ))
    }

    fn read_guest_bytes(&mut self, address: u32, length: usize) -> Result<Vec<u8>, String> {
        let mut bytes = Vec::with_capacity(length);
        for offset in 0..length {
            bytes.push(
                self.bus
                    .read(
                        u64::from(address.wrapping_add(offset as u32)),
                        renvo_core::AccessWidth::Byte,
                        renvo_core::AccessKind::Read,
                        self.now,
                    )
                    .map_err(|error| error.to_string())? as u8,
            );
        }
        Ok(bytes)
    }

    fn write_guest_bytes(&mut self, address: u32, bytes: &[u8]) -> Result<(), String> {
        for (offset, byte) in bytes.iter().copied().enumerate() {
            self.bus
                .write(
                    u64::from(address.wrapping_add(offset as u32)),
                    renvo_core::AccessWidth::Byte,
                    u64::from(byte),
                    self.now,
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn esp_printf_argument(&mut self, slot: u32) -> Result<u32, String> {
        if slot <= 7 {
            let register =
                RiscVRegister::argument(slot as u8).expect("slot range was checked above");
            return self
                .cpu
                .register(register)
                .map_err(|error| error.to_string());
        }
        let stack = self
            .cpu
            .register(RiscVRegister::Sp)
            .map_err(|error| error.to_string())?;
        self.bus
            .read(
                u64::from(stack.wrapping_add((slot - 8) * 4)),
                renvo_core::AccessWidth::Word,
                renvo_core::AccessKind::Read,
                self.now,
            )
            .map(|value| value as u32)
            .map_err(|error| error.to_string())
    }

    fn service_esp_printf(&mut self) -> Result<u32, String> {
        let format_address = self
            .cpu
            .register(RiscVRegister::A0)
            .map_err(|error| error.to_string())?;
        let format = self.read_guest_c_string(format_address, 16 * 1024)?;
        let mut output = Vec::new();
        let mut cursor = 0;
        let mut argument_slot = 1_u32;

        while cursor < format.len() {
            if format[cursor] != b'%' {
                output.push(format[cursor]);
                cursor += 1;
                continue;
            }
            cursor += 1;
            if format.get(cursor) == Some(&b'%') {
                output.push(b'%');
                cursor += 1;
                continue;
            }

            let mut left = false;
            let mut plus = false;
            let mut alternate = false;
            let mut zero = false;
            while let Some(flag) = format.get(cursor).copied() {
                match flag {
                    b'-' => left = true,
                    b'+' => plus = true,
                    b'#' => alternate = true,
                    b'0' => zero = true,
                    b' ' => {}
                    _ => break,
                }
                cursor += 1;
            }

            let mut width = 0_usize;
            if format.get(cursor) == Some(&b'*') {
                width = self.esp_printf_argument(argument_slot)? as usize;
                argument_slot += 1;
                cursor += 1;
            } else {
                while let Some(digit @ b'0'..=b'9') = format.get(cursor).copied() {
                    width = width
                        .saturating_mul(10)
                        .saturating_add(usize::from(digit - b'0'));
                    cursor += 1;
                }
            }

            let mut precision = None;
            if format.get(cursor) == Some(&b'.') {
                cursor += 1;
                let mut value = 0_usize;
                if format.get(cursor) == Some(&b'*') {
                    value = self.esp_printf_argument(argument_slot)? as usize;
                    argument_slot += 1;
                    cursor += 1;
                } else {
                    while let Some(digit @ b'0'..=b'9') = format.get(cursor).copied() {
                        value = value
                            .saturating_mul(10)
                            .saturating_add(usize::from(digit - b'0'));
                        cursor += 1;
                    }
                }
                precision = Some(value);
            }

            let mut bits = 32_u8;
            match format.get(cursor).copied() {
                Some(b'h') => {
                    cursor += 1;
                    if format.get(cursor) == Some(&b'h') {
                        bits = 8;
                        cursor += 1;
                    } else {
                        bits = 16;
                    }
                }
                Some(b'l') => {
                    cursor += 1;
                    if format.get(cursor) == Some(&b'l') {
                        bits = 64;
                        cursor += 1;
                    }
                }
                Some(b'j') => {
                    bits = 64;
                    cursor += 1;
                }
                Some(b'z' | b't') => cursor += 1,
                _ => {}
            }
            let conversion = *format
                .get(cursor)
                .ok_or_else(|| "unterminated ets_printf conversion".to_owned())?;
            cursor += 1;

            let value = if bits == 64 {
                if argument_slot & 1 != 0 {
                    argument_slot += 1;
                }
                let low = u64::from(self.esp_printf_argument(argument_slot)?);
                let high = u64::from(self.esp_printf_argument(argument_slot + 1)?);
                argument_slot += 2;
                low | (high << 32)
            } else {
                let value = u64::from(self.esp_printf_argument(argument_slot)?);
                argument_slot += 1;
                value
            };

            let mut rendered = match conversion {
                b'c' => String::from(char::from(value as u8)),
                b's' => {
                    let mut bytes = self.read_guest_c_string(value as u32, 16 * 1024)?;
                    if let Some(precision) = precision {
                        bytes.truncate(precision);
                    }
                    String::from_utf8_lossy(&bytes).into_owned()
                }
                b'd' | b'i' => {
                    let signed = match bits {
                        8 => i64::from(value as i8),
                        16 => i64::from(value as i16),
                        64 => value as i64,
                        _ => i64::from(value as i32),
                    };
                    if plus && signed >= 0 {
                        format!("+{signed}")
                    } else {
                        signed.to_string()
                    }
                }
                b'u' => value.to_string(),
                b'x' => {
                    if alternate {
                        format!("{value:#x}")
                    } else {
                        format!("{value:x}")
                    }
                }
                b'X' => {
                    if alternate {
                        format!("{value:#X}")
                    } else {
                        format!("{value:X}")
                    }
                }
                b'o' => {
                    if alternate {
                        format!("{value:#o}")
                    } else {
                        format!("{value:o}")
                    }
                }
                b'p' => format!("0x{value:08x}"),
                other => {
                    return Err(format!(
                        "unsupported ets_printf conversion %{other}",
                        other = char::from(other)
                    ));
                }
            };
            if rendered.len() < width {
                let fill = if zero && !left { '0' } else { ' ' };
                let padding: String = std::iter::repeat_n(fill, width - rendered.len()).collect();
                rendered = if left {
                    rendered + &padding
                } else {
                    padding + &rendered
                };
            }
            output.extend_from_slice(rendered.as_bytes());
        }

        if let Some(uart) = self.chip_uarts.first() {
            uart.transmit(&output);
        } else {
            self.uart.transmit(&output);
        }
        Ok(output.len() as u32)
    }

    fn esp_heap_allocate(
        &mut self,
        handle: u32,
        size: u32,
        alignment: u32,
        offset: u32,
    ) -> Result<u32, String> {
        let heap = self
            .esp_heaps
            .get_mut(&handle)
            .ok_or_else(|| format!("unknown ESP heap handle {handle:#010x}"))?;
        Ok(heap.allocate(size, alignment, offset).unwrap_or(0))
    }

    fn esp_heap_reallocate(&mut self, handle: u32, pointer: u32, size: u32) -> Result<u32, String> {
        if pointer == 0 {
            return self.esp_heap_allocate(handle, size, 4, 0);
        }
        if size == 0 {
            if let Some(heap) = self.esp_heaps.get_mut(&handle) {
                heap.release(pointer);
            }
            return Ok(0);
        }
        let old_size = self
            .esp_heaps
            .get(&handle)
            .and_then(|heap| heap.allocations.get(&pointer))
            .copied()
            .ok_or_else(|| format!("unknown allocation {pointer:#010x}"))?;
        if size <= old_size {
            return Ok(pointer);
        }
        let replacement = self.esp_heap_allocate(handle, size, 4, 0)?;
        if replacement == 0 {
            return Ok(0);
        }
        let mut bytes = Vec::with_capacity(old_size as usize);
        for offset in 0..old_size {
            bytes.push(
                self.bus
                    .read(
                        u64::from(pointer + offset),
                        renvo_core::AccessWidth::Byte,
                        renvo_core::AccessKind::Read,
                        self.now,
                    )
                    .map_err(|error| error.to_string())? as u8,
            );
        }
        self.bus
            .load(u64::from(replacement), &bytes)
            .map_err(|error| error.to_string())?;
        self.esp_heaps
            .get_mut(&handle)
            .expect("validated heap")
            .release(pointer);
        Ok(replacement)
    }

    fn write_guest_words(&mut self, address: u32, words: &[u32]) -> Result<(), String> {
        for (index, value) in words.iter().copied().enumerate() {
            self.bus
                .write(
                    u64::from(address.wrapping_add(index as u32 * 4)),
                    renvo_core::AccessWidth::Word,
                    u64::from(value),
                    self.now,
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn service_functional_bootrom(&mut self) -> Result<bool, String> {
        if self.target == TargetId::Esp32c6 {
            let pc = self.cpu.pc();
            let result: Result<bool, String> = (|| {
                match pc {
                    ESP_ROM_FLASH_START_STUB => {
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    ESP_ROM_FLASH_END_STUB => {
                        let result = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        self.complete_host_call(result)?;
                        Ok(true)
                    }
                    ESP_ROM_FLASH_CHIP_CHECK_STUB => {
                        let inout = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let chip = self
                            .bus
                            .read(
                                u64::from(inout),
                                renvo_core::AccessWidth::Word,
                                renvo_core::AccessKind::Read,
                                self.now,
                            )
                            .map_err(|error| error.to_string())?
                            as u32;
                        let chip = if chip == 0 {
                            self.write_guest_words(inout, &[ESP_ROM_DEFAULT_FLASH])?;
                            ESP_ROM_DEFAULT_FLASH
                        } else {
                            chip
                        };
                        let driver = self
                            .bus
                            .read(
                                u64::from(chip + 4),
                                renvo_core::AccessWidth::Word,
                                renvo_core::AccessKind::Read,
                                self.now,
                            )
                            .map_err(|error| error.to_string())?
                            as u32;
                        if driver == 0 {
                            self.write_guest_words(
                                chip,
                                &[
                                    ESP_ROM_FLASH_HOST,
                                    ESP_ROM_FLASH_DRIVER,
                                    0,
                                    0,
                                    2,
                                    4 * 1024 * 1024,
                                    0x0016_40c8,
                                    0,
                                ],
                            )?;
                        }
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    ESP_ROM_FLASH_DETECT_SIZE_STUB => {
                        let output = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        self.write_guest_words(output, &[4 * 1024 * 1024])?;
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    ESP_ROM_FLASH_OK_STUB => {
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    0x4000_0afc => {
                        self.complete_host_call(ESP_ROM_COEX_VERSION)?;
                        Ok(true)
                    }
                    // ESP32-C6 mask-ROM rtc_get_reset_reason /
                    // esp_rom_get_reset_reason. A cold functional boot reports
                    // POWERON_RESET for CPU0.
                    0x4000_0018 => {
                        self.complete_host_call(1)?;
                        Ok(true)
                    }
                    // ets_printf decodes the guest's RISC-V varargs and emits to
                    // the functional ROM console.
                    0x4000_0028 => {
                        let written = self.service_esp_printf()?;
                        self.complete_host_call(written)?;
                        Ok(true)
                    }
                    // ets_delay_us / esp_rom_delay_us. Functional timing is
                    // instruction ordered rather than wall-clock accurate, so
                    // the delay is a deterministic ordering point.
                    0x4000_0040 => {
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    // ets_get_cpu_frequency and ets_update_cpu_frequency expose
                    // the ROM's ticks-per-microsecond state.
                    0x4000_0044 => {
                        self.complete_host_call(self.esp_cpu_frequency_mhz)?;
                        Ok(true)
                    }
                    0x4000_0048 => {
                        self.esp_cpu_frequency_mhz = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    // gpio_pad_select_gpio / esp_rom_gpio_pad_select_gpio.
                    // The ROM helper selects the ordinary digital IO mux for
                    // one pad; the register-level GPIO model applies the
                    // direction and output state written by the IDF driver.
                    0x4000_0700 => {
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    // uart_tx_wait_idle: the functional UART drains immediately.
                    0x4000_0078 => {
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    0x4000_01e4 => {
                        self.esp_flash_guard = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    0x4000_01e8 => {
                        self.complete_host_call(self.esp_flash_guard)?;
                        Ok(true)
                    }
                    // Install the OS hooks used by ROM flash mmap services. The
                    // direct mapped-image path does not need to call them while
                    // boot remains single-threaded.
                    0x4000_0204 | 0x4000_0208 => {
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    0x4000_020c => {
                        let source = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let length = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?
                            as usize;
                        let output = self
                            .cpu
                            .register(RiscVRegister::A3)
                            .map_err(|error| error.to_string())?;
                        let handle = self
                            .cpu
                            .register(RiscVRegister::A4)
                            .map_err(|error| error.to_string())?;
                        let start = source as usize;
                        let requested_end = start
                            .checked_add(length)
                            .filter(|end| *end <= self.esp_flash.len());
                        if let Some(requested_end) = requested_end {
                            let page_start = start & !0xffff;
                            let page_end = requested_end
                                .saturating_add(0xffff)
                                .min(self.esp_flash.len())
                                & !0xffff;
                            let page_end = page_end.max(requested_end);
                            let mapped = ESP_FUNCTIONAL_MMAP_BASE.wrapping_add(source);
                            self.bus
                                .load(
                                    u64::from(
                                        ESP_FUNCTIONAL_MMAP_BASE.wrapping_add(page_start as u32),
                                    ),
                                    &self.esp_flash[page_start..page_end],
                                )
                                .map_err(|error| error.to_string())?;
                            self.write_guest_words(output, &[mapped])?;
                            self.write_guest_words(handle, &[source / 0x1_0000 + 1])?;
                            self.complete_host_call(0)?;
                        } else {
                            self.complete_host_call(0x102)?;
                        }
                        Ok(true)
                    }
                    0x4000_0214 | 0x4000_0218 | 0x4000_021c => {
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    0x4000_0220 => {
                        self.complete_host_call(128)?;
                        Ok(true)
                    }
                    0x4000_0224 => {
                        let cached = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let physical = cached
                            .checked_sub(ESP_FUNCTIONAL_MMAP_BASE)
                            .filter(|offset| (*offset as usize) < self.esp_flash.len())
                            .unwrap_or(u32::MAX);
                        self.complete_host_call(physical)?;
                        Ok(true)
                    }
                    0x4000_0228 => {
                        let physical = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let cached = if (physical as usize) < self.esp_flash.len() {
                            ESP_FUNCTIONAL_MMAP_BASE.wrapping_add(physical)
                        } else {
                            0
                        };
                        self.complete_host_call(cached)?;
                        Ok(true)
                    }
                    0x4000_022c => {
                        self.complete_host_call(1)?;
                        Ok(true)
                    }
                    0x4000_0230 | 0x4000_0270 => {
                        let output = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        self.write_guest_words(output, &[0x0016_40c8])?;
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    0x4000_0234 => {
                        let output = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        self.write_guest_words(output, &[4 * 1024 * 1024])?;
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    0x4000_0238 => {
                        self.esp_flash.fill(0xff);
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    0x4000_023c => {
                        let address = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?
                            as usize;
                        let length = self
                            .cpu
                            .register(RiscVRegister::A2)
                            .map_err(|error| error.to_string())?
                            as usize;
                        let end = address
                            .checked_add(length)
                            .filter(|end| *end <= self.esp_flash.len())
                            .ok_or_else(|| {
                                format!(
                                    "ESP flash erase {address:#x}..{:#x} exceeds image",
                                    address.saturating_add(length)
                                )
                            })?;
                        self.esp_flash[address..end].fill(0xff);
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    0x4000_0254 | 0x4000_0260 => {
                        let output = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        let address = self
                            .cpu
                            .register(RiscVRegister::A2)
                            .map_err(|error| error.to_string())?
                            as usize;
                        let length = self
                            .cpu
                            .register(RiscVRegister::A3)
                            .map_err(|error| error.to_string())?
                            as usize;
                        let end = address
                            .checked_add(length)
                            .filter(|end| *end <= self.esp_flash.len())
                            .ok_or_else(|| {
                                format!(
                                    "ESP flash read {address:#x}..{:#x} exceeds image",
                                    address.saturating_add(length)
                                )
                            })?;
                        let bytes = self.esp_flash[address..end].to_vec();
                        self.write_guest_bytes(output, &bytes)?;
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    0x4000_0258 | 0x4000_025c => {
                        let input = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        let address = self
                            .cpu
                            .register(RiscVRegister::A2)
                            .map_err(|error| error.to_string())?
                            as usize;
                        let length = self
                            .cpu
                            .register(RiscVRegister::A3)
                            .map_err(|error| error.to_string())?
                            as usize;
                        let end = address
                            .checked_add(length)
                            .filter(|end| *end <= self.esp_flash.len())
                            .ok_or_else(|| {
                                format!(
                                    "ESP flash write {address:#x}..{:#x} exceeds image",
                                    address.saturating_add(length)
                                )
                            })?;
                        let bytes = self.read_guest_bytes(input, length)?;
                        for (current, requested) in self.esp_flash[address..end]
                            .iter_mut()
                            .zip(bytes.into_iter())
                        {
                            *current &= requested;
                        }
                        self.bus
                            .load(
                                u64::from(ESP_FUNCTIONAL_MMAP_BASE.wrapping_add(address as u32)),
                                &self.esp_flash[address..end],
                            )
                            .map_err(|error| error.to_string())?;
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    // intr_matrix_set / esp_rom_route_intr_matrix. ESP32-C6 is
                    // single-core; retain the source-to-CPU-line association for
                    // deterministic interrupt delivery as peripheral models are
                    // activated.
                    0x4000_0730 => {
                        let source = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        let cpu_interrupt = self
                            .cpu
                            .register(RiscVRegister::A2)
                            .map_err(|error| error.to_string())?;
                        self.esp_interrupt_routes.insert(source, cpu_interrupt);
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    0x4000_0718 => {
                        let interrupt = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let priority = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        self.esp_interrupt_priorities.insert(interrupt, priority);
                        if interrupt < 32 {
                            self.bus
                                .write(
                                    u64::from(0x2000_1010_u32 + interrupt * 4),
                                    renvo_core::AccessWidth::Word,
                                    u64::from(priority),
                                    self.now,
                                )
                                .map_err(|error| error.to_string())?;
                        }
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    0x4000_071c => {
                        self.esp_interrupt_threshold = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        self.bus
                            .write(
                                0x2000_1090,
                                renvo_core::AccessWidth::Word,
                                u64::from(self.esp_interrupt_threshold),
                                self.now,
                            )
                            .map_err(|error| error.to_string())?;
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    0x4000_0720 => {
                        let interrupt_mask = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let enabled = self
                            .bus
                            .read(
                                0x2000_1000,
                                renvo_core::AccessWidth::Word,
                                renvo_core::AccessKind::Read,
                                self.now,
                            )
                            .map_err(|error| error.to_string())?
                            as u32;
                        self.bus
                            .write(
                                0x2000_1000,
                                renvo_core::AccessWidth::Word,
                                u64::from(enabled | interrupt_mask),
                                self.now,
                            )
                            .map_err(|error| error.to_string())?;
                        for interrupt in 0..32 {
                            if interrupt_mask & (1 << interrupt) != 0 {
                                self.esp_enabled_interrupts.insert(interrupt);
                                self.cpu
                                    .set_machine_interrupt_enabled(interrupt as u16, true)
                                    .map_err(|error| error.to_string())?;
                            }
                        }
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    0x4000_0724 => {
                        let interrupt_mask = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let enabled = self
                            .bus
                            .read(
                                0x2000_1000,
                                renvo_core::AccessWidth::Word,
                                renvo_core::AccessKind::Read,
                                self.now,
                            )
                            .map_err(|error| error.to_string())?
                            as u32;
                        self.bus
                            .write(
                                0x2000_1000,
                                renvo_core::AccessWidth::Word,
                                u64::from(enabled & !interrupt_mask),
                                self.now,
                            )
                            .map_err(|error| error.to_string())?;
                        for interrupt in 0..32 {
                            if interrupt_mask & (1 << interrupt) != 0 {
                                self.esp_enabled_interrupts.remove(&interrupt);
                                self.cpu
                                    .set_machine_interrupt_enabled(interrupt as u16, false)
                                    .map_err(|error| error.to_string())?;
                                self.cpu
                                    .set_interrupt(interrupt as u16, false)
                                    .map_err(|error| error.to_string())?;
                            }
                        }
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    // Trigger type and handler-vector installation are retained
                    // by ESP-IDF's own tables.
                    0x4000_0728 | 0x4000_072c => {
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    // Mask-ROM MD5 API. Keep the accumulated message in the host
                    // model, while also clearing the guest context to preserve
                    // the API's observable initialization behavior.
                    0x4000_074c => {
                        let context = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        self.esp_md5_contexts.insert(context, Vec::new());
                        self.write_guest_bytes(context, &[0; 88])?;
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    0x4000_0750 => {
                        let context = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let input = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        let length = self
                            .cpu
                            .register(RiscVRegister::A2)
                            .map_err(|error| error.to_string())?
                            as usize;
                        let bytes = self.read_guest_bytes(input, length)?;
                        self.esp_md5_contexts
                            .entry(context)
                            .or_default()
                            .extend_from_slice(&bytes);
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    0x4000_0754 => {
                        let digest_address = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let context = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        let message = self.esp_md5_contexts.remove(&context).unwrap_or_default();
                        let digest = Md5::digest(message);
                        self.write_guest_bytes(digest_address, digest.as_slice())?;
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    // Newlib routines exported by the ESP32-C6 mask ROM.
                    0x4000_04a8 => {
                        let destination = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let byte = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?
                            as u8;
                        let length = self
                            .cpu
                            .register(RiscVRegister::A2)
                            .map_err(|error| error.to_string())?;
                        for offset in 0..length {
                            self.bus
                                .write(
                                    u64::from(destination.wrapping_add(offset)),
                                    renvo_core::AccessWidth::Byte,
                                    u64::from(byte),
                                    self.now,
                                )
                                .map_err(|error| error.to_string())?;
                        }
                        self.complete_host_call(destination)?;
                        Ok(true)
                    }
                    0x4000_04ac | 0x4000_04b0 => {
                        let destination = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let source = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        let length = self
                            .cpu
                            .register(RiscVRegister::A2)
                            .map_err(|error| error.to_string())?;
                        let mut bytes = Vec::with_capacity(length as usize);
                        for offset in 0..length {
                            bytes.push(
                                self.bus
                                    .read(
                                        u64::from(source.wrapping_add(offset)),
                                        renvo_core::AccessWidth::Byte,
                                        renvo_core::AccessKind::Read,
                                        self.now,
                                    )
                                    .map_err(|error| error.to_string())?
                                    as u8,
                            );
                        }
                        for (offset, byte) in bytes.into_iter().enumerate() {
                            self.bus
                                .write(
                                    u64::from(destination.wrapping_add(offset as u32)),
                                    renvo_core::AccessWidth::Byte,
                                    u64::from(byte),
                                    self.now,
                                )
                                .map_err(|error| error.to_string())?;
                        }
                        self.complete_host_call(destination)?;
                        Ok(true)
                    }
                    0x4000_04b4 => {
                        let left = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let right = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        let length = self
                            .cpu
                            .register(RiscVRegister::A2)
                            .map_err(|error| error.to_string())?;
                        let mut result = 0_i32;
                        for offset in 0..length {
                            let left_byte = self
                                .bus
                                .read(
                                    u64::from(left.wrapping_add(offset)),
                                    renvo_core::AccessWidth::Byte,
                                    renvo_core::AccessKind::Read,
                                    self.now,
                                )
                                .map_err(|error| error.to_string())?
                                as u8;
                            let right_byte = self
                                .bus
                                .read(
                                    u64::from(right.wrapping_add(offset)),
                                    renvo_core::AccessWidth::Byte,
                                    renvo_core::AccessKind::Read,
                                    self.now,
                                )
                                .map_err(|error| error.to_string())?
                                as u8;
                            if left_byte != right_byte {
                                result = i32::from(left_byte) - i32::from(right_byte);
                                break;
                            }
                        }
                        self.complete_host_call(result as u32)?;
                        Ok(true)
                    }
                    0x4000_04b8 => {
                        let destination = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let source = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        let mut offset = 0_u32;
                        loop {
                            let byte = self
                                .bus
                                .read(
                                    u64::from(source.wrapping_add(offset)),
                                    renvo_core::AccessWidth::Byte,
                                    renvo_core::AccessKind::Read,
                                    self.now,
                                )
                                .map_err(|error| error.to_string())?
                                as u8;
                            self.bus
                                .write(
                                    u64::from(destination.wrapping_add(offset)),
                                    renvo_core::AccessWidth::Byte,
                                    u64::from(byte),
                                    self.now,
                                )
                                .map_err(|error| error.to_string())?;
                            offset = offset.wrapping_add(1);
                            if byte == 0 {
                                break;
                            }
                        }
                        self.complete_host_call(destination)?;
                        Ok(true)
                    }
                    0x4000_04bc => {
                        let destination = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let source = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        let length = self
                            .cpu
                            .register(RiscVRegister::A2)
                            .map_err(|error| error.to_string())?;
                        let mut terminated = false;
                        for offset in 0..length {
                            let byte = if terminated {
                                0
                            } else {
                                let byte = self
                                    .bus
                                    .read(
                                        u64::from(source.wrapping_add(offset)),
                                        renvo_core::AccessWidth::Byte,
                                        renvo_core::AccessKind::Read,
                                        self.now,
                                    )
                                    .map_err(|error| error.to_string())?
                                    as u8;
                                terminated = byte == 0;
                                byte
                            };
                            self.bus
                                .write(
                                    u64::from(destination.wrapping_add(offset)),
                                    renvo_core::AccessWidth::Byte,
                                    u64::from(byte),
                                    self.now,
                                )
                                .map_err(|error| error.to_string())?;
                        }
                        self.complete_host_call(destination)?;
                        Ok(true)
                    }
                    0x4000_04c0 => {
                        let left = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let right = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        let mut offset = 0_u32;
                        let result = loop {
                            let left_byte = self
                                .bus
                                .read(
                                    u64::from(left.wrapping_add(offset)),
                                    renvo_core::AccessWidth::Byte,
                                    renvo_core::AccessKind::Read,
                                    self.now,
                                )
                                .map_err(|error| error.to_string())?
                                as u8;
                            let right_byte = self
                                .bus
                                .read(
                                    u64::from(right.wrapping_add(offset)),
                                    renvo_core::AccessWidth::Byte,
                                    renvo_core::AccessKind::Read,
                                    self.now,
                                )
                                .map_err(|error| error.to_string())?
                                as u8;
                            if left_byte != right_byte || left_byte == 0 {
                                break (i32::from(left_byte) - i32::from(right_byte)) as u32;
                            }
                            offset = offset.wrapping_add(1);
                        };
                        self.complete_host_call(result)?;
                        Ok(true)
                    }
                    0x4000_04c4 => {
                        let left = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let right = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        let length = self
                            .cpu
                            .register(RiscVRegister::A2)
                            .map_err(|error| error.to_string())?;
                        let mut result = 0;
                        for offset in 0..length {
                            let left_byte = self
                                .bus
                                .read(
                                    u64::from(left.wrapping_add(offset)),
                                    renvo_core::AccessWidth::Byte,
                                    renvo_core::AccessKind::Read,
                                    self.now,
                                )
                                .map_err(|error| error.to_string())?
                                as u8;
                            let right_byte = self
                                .bus
                                .read(
                                    u64::from(right.wrapping_add(offset)),
                                    renvo_core::AccessWidth::Byte,
                                    renvo_core::AccessKind::Read,
                                    self.now,
                                )
                                .map_err(|error| error.to_string())?
                                as u8;
                            if left_byte != right_byte || left_byte == 0 {
                                result = (i32::from(left_byte) - i32::from(right_byte)) as u32;
                                break;
                            }
                        }
                        self.complete_host_call(result)?;
                        Ok(true)
                    }
                    0x4000_04c8 => {
                        let source = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let mut length = 0_u32;
                        while self
                            .bus
                            .read(
                                u64::from(source.wrapping_add(length)),
                                renvo_core::AccessWidth::Byte,
                                renvo_core::AccessKind::Read,
                                self.now,
                            )
                            .map_err(|error| error.to_string())?
                            != 0
                        {
                            length = length.wrapping_add(1);
                        }
                        self.complete_host_call(length)?;
                        Ok(true)
                    }
                    0x4000_051c => {
                        let source = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let needle = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?
                            as u8;
                        let length = self
                            .cpu
                            .register(RiscVRegister::A2)
                            .map_err(|error| error.to_string())?;
                        let mut result = 0;
                        for offset in 0..length {
                            let byte = self
                                .bus
                                .read(
                                    u64::from(source + offset),
                                    renvo_core::AccessWidth::Byte,
                                    renvo_core::AccessKind::Read,
                                    self.now,
                                )
                                .map_err(|error| error.to_string())?
                                as u8;
                            if byte == needle {
                                result = source + offset;
                                break;
                            }
                        }
                        self.complete_host_call(result)?;
                        Ok(true)
                    }
                    0x4000_052c => {
                        let destination = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let source = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        let destination_length =
                            self.read_guest_c_string(destination, 1024 * 1024)?.len() as u32;
                        let source_bytes = self.read_guest_c_string(source, 1024 * 1024)?;
                        self.write_guest_bytes(
                            destination.wrapping_add(destination_length),
                            &source_bytes,
                        )?;
                        self.write_guest_bytes(
                            destination
                                .wrapping_add(destination_length)
                                .wrapping_add(source_bytes.len() as u32),
                            &[0],
                        )?;
                        self.complete_host_call(destination)?;
                        Ok(true)
                    }
                    0x4000_0534 | 0x4000_055c => {
                        let source = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let needle = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?
                            as u8;
                        let bytes = self.read_guest_c_string(source, 1024 * 1024)?;
                        let position = if self.cpu.pc() == 0x4000_0534 {
                            bytes.iter().position(|byte| *byte == needle)
                        } else {
                            bytes.iter().rposition(|byte| *byte == needle)
                        };
                        let result = if needle == 0 {
                            source.wrapping_add(bytes.len() as u32)
                        } else {
                            position
                                .map(|offset| source.wrapping_add(offset as u32))
                                .unwrap_or(0)
                        };
                        self.complete_host_call(result)?;
                        Ok(true)
                    }
                    0x4000_0538 | 0x4000_0564 => {
                        let source = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let set_address = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        let bytes = self.read_guest_c_string(source, 1024 * 1024)?;
                        let set = self.read_guest_c_string(set_address, 1024 * 1024)?;
                        let count = if self.cpu.pc() == 0x4000_0564 {
                            bytes.iter().take_while(|byte| set.contains(byte)).count()
                        } else {
                            bytes.iter().take_while(|byte| !set.contains(byte)).count()
                        };
                        self.complete_host_call(count as u32)?;
                        Ok(true)
                    }
                    0x4000_0544 => {
                        let destination = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let source = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        let capacity = self
                            .cpu
                            .register(RiscVRegister::A2)
                            .map_err(|error| error.to_string())?
                            as usize;
                        let bytes = self.read_guest_c_string(source, 1024 * 1024)?;
                        if capacity != 0 {
                            let copied = bytes.len().min(capacity - 1);
                            self.write_guest_bytes(destination, &bytes[..copied])?;
                            self.write_guest_bytes(destination.wrapping_add(copied as u32), &[0])?;
                        }
                        self.complete_host_call(bytes.len() as u32)?;
                        Ok(true)
                    }
                    0x4000_0558 => {
                        let source = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let maximum = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?
                            as usize;
                        let mut length = 0;
                        while length < maximum
                            && self
                                .bus
                                .read(
                                    u64::from(source.wrapping_add(length as u32)),
                                    renvo_core::AccessWidth::Byte,
                                    renvo_core::AccessKind::Read,
                                    self.now,
                                )
                                .map_err(|error| error.to_string())?
                                != 0
                        {
                            length += 1;
                        }
                        self.complete_host_call(length as u32)?;
                        Ok(true)
                    }
                    // qsort used by ESP-IDF's early heap-region preparation. Its
                    // records are `(start, end)` word pairs and the comparator
                    // orders the signed start address.
                    0x4000_0588 => {
                        let base = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let count = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        let size = self
                            .cpu
                            .register(RiscVRegister::A2)
                            .map_err(|error| error.to_string())?;
                        if size != 8 {
                            return Err(format!(
                                "functional ESP qsort currently requires 8-byte region records, got {size}"
                            ));
                        }
                        let mut records = Vec::with_capacity(count as usize);
                        for index in 0..count {
                            let address = base.wrapping_add(index * size);
                            let start = self
                                .bus
                                .read(
                                    u64::from(address),
                                    renvo_core::AccessWidth::Word,
                                    renvo_core::AccessKind::Read,
                                    self.now,
                                )
                                .map_err(|error| error.to_string())?
                                as u32;
                            let end = self
                                .bus
                                .read(
                                    u64::from(address + 4),
                                    renvo_core::AccessWidth::Word,
                                    renvo_core::AccessKind::Read,
                                    self.now,
                                )
                                .map_err(|error| error.to_string())?
                                as u32;
                            records.push((start, end));
                        }
                        records.sort_by_key(|(start, _)| *start as i32);
                        for (index, (start, end)) in records.into_iter().enumerate() {
                            let address = base.wrapping_add(index as u32 * size);
                            for (offset, value) in [(0, start), (4, end)] {
                                self.bus
                                    .write(
                                        u64::from(address + offset),
                                        renvo_core::AccessWidth::Word,
                                        u64::from(value),
                                        self.now,
                                    )
                                    .map_err(|error| error.to_string())?;
                            }
                        }
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    0x4000_0578 | 0x4000_0580 => {
                        let value = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?
                            as i32;
                        self.complete_host_call(value.wrapping_abs() as u32)?;
                        Ok(true)
                    }
                    0x4000_057c | 0x4000_0584 => {
                        let numerator = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?
                            as i32;
                        let denominator = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?
                            as i32;
                        let (quotient, remainder) = if denominator == 0 {
                            (-1, numerator)
                        } else if numerator == i32::MIN && denominator == -1 {
                            (i32::MIN, 0)
                        } else {
                            (numerator / denominator, numerator % denominator)
                        };
                        self.complete_host_call_u64(
                            u64::from(quotient as u32) | (u64::from(remainder as u32) << 32),
                        )?;
                        Ok(true)
                    }
                    // utoa / itoa
                    0x4000_0598 | 0x4000_059c => {
                        let raw = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let destination = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        let radix = self
                            .cpu
                            .register(RiscVRegister::A2)
                            .map_err(|error| error.to_string())?;
                        let signed =
                            self.cpu.pc() == 0x4000_059c && radix == 10 && (raw as i32) < 0;
                        let mut value = if signed {
                            (raw as i32).unsigned_abs()
                        } else {
                            raw
                        };
                        let mut rendered = Vec::new();
                        if (2..=36).contains(&radix) {
                            loop {
                                let digit = (value % radix) as u8;
                                rendered.push(if digit < 10 {
                                    b'0' + digit
                                } else {
                                    b'a' + digit - 10
                                });
                                value /= radix;
                                if value == 0 {
                                    break;
                                }
                            }
                            if signed {
                                rendered.push(b'-');
                            }
                            rendered.reverse();
                        }
                        rendered.push(0);
                        self.write_guest_bytes(destination, &rendered)?;
                        self.complete_host_call(destination)?;
                        Ok(true)
                    }
                    // Mbed TLS SHA-224/SHA-256 backed by deterministic host
                    // hashing, equivalent to the C6 accelerator at functional
                    // fidelity.
                    0x420f_57da => {
                        let context = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        self.esp_sha256_contexts
                            .insert(context, EspFunctionalSha256::default());
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    0x420f_57e8 => {
                        let context = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        self.esp_sha256_contexts.remove(&context);
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    0x420f_57fc => {
                        let destination = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let source = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        let state = self
                            .esp_sha256_contexts
                            .get(&source)
                            .cloned()
                            .unwrap_or_default();
                        self.esp_sha256_contexts.insert(destination, state);
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    0x420f_5812 => {
                        let context = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let sha224 = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?
                            != 0;
                        self.esp_sha256_contexts.insert(
                            context,
                            EspFunctionalSha256 {
                                sha224,
                                input: Vec::new(),
                            },
                        );
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    0x420f_5840 => {
                        let context = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let input = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        let length = self
                            .cpu
                            .register(RiscVRegister::A2)
                            .map_err(|error| error.to_string())?
                            as usize;
                        let bytes = self.read_guest_bytes(input, length).map_err(|error| {
                            format!(
                                "mbedtls_sha256_update(ctx={context:#010x}, input={input:#010x}, length={length:#x}): {error}"
                            )
                        })?;
                        self.esp_sha256_contexts
                            .entry(context)
                            .or_default()
                            .input
                            .extend_from_slice(&bytes);
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    0x420f_5966 => {
                        let context = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let output = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        let state = self
                            .esp_sha256_contexts
                            .get(&context)
                            .cloned()
                            .unwrap_or_default();
                        let digest = if state.sha224 {
                            Sha224::digest(&state.input).to_vec()
                        } else {
                            Sha256::digest(&state.input).to_vec()
                        };
                        self.write_guest_bytes(output, &digest)?;
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    // ESP-IDF has already installed device-backed FILE hooks at
                    // this point. There is no additional host-side stream buffer
                    // to prepare in the functional model.
                    0x4000_05b0 | 0x4000_05b4 | 0x4000_05b8 | 0x4000_05bc | 0x4000_05c0
                    | 0x4000_05c4 | 0x4000_05d0 => {
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    0x4000_05c8 | 0x4000_05cc => {
                        let register = if self.cpu.pc() == 0x4000_05c8 {
                            RiscVRegister::A1
                        } else {
                            RiscVRegister::A0
                        };
                        let character = self
                            .cpu
                            .register(register)
                            .map_err(|error| error.to_string())?
                            as u8;
                        self.uart.transmit(&[character]);
                        self.complete_host_call(u32::from(character))?;
                        Ok(true)
                    }
                    // The mapped application segments are already visible through
                    // Renvo's deterministic flash view, so enabling the ROM
                    // instruction cache is an ordering point.
                    0x4000_0694 | 0x4000_06a8 => {
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    // ESP-IDF's ROM-resident watchdog HAL. Renvo does not advance
                    // a watchdog countdown in functional mode, but preserves
                    // enable state per HAL context so driver probes remain
                    // coherent.
                    0x4000_039c | 0x4000_03a0 | 0x4000_03a4 | 0x4000_03b0 | 0x4000_03b4
                    | 0x4000_03b8 => {
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    0x4000_03a8 => {
                        let context = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        self.esp_enabled_watchdogs.insert(context);
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    0x4000_03ac => {
                        let context = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        self.esp_enabled_watchdogs.remove(&context);
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    0x4000_03bc => {
                        let context = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let enabled = u32::from(self.esp_enabled_watchdogs.contains(&context));
                        self.complete_host_call(enabled)?;
                        Ok(true)
                    }
                    // Copy the two tick-rate conversion callbacks into the
                    // caller-owned systimer HAL context.
                    0x4000_03c8 => {
                        let context = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let operations = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        let mut callbacks = [0_u32; 2];
                        for (index, callback) in callbacks.iter_mut().enumerate() {
                            *callback = self
                                .bus
                                .read(
                                    u64::from(operations + index as u32 * 4),
                                    renvo_core::AccessWidth::Word,
                                    renvo_core::AccessKind::Read,
                                    self.now,
                                )
                                .map_err(|error| error.to_string())?
                                as u32;
                        }
                        self.write_guest_words(context + 4, &callbacks)?;
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    0x4000_03cc | 0x4000_03d0 => {
                        let counter = self.now.ticks().wrapping_add(self.esp_systimer_offset);
                        self.complete_host_call_u64(counter)?;
                        Ok(true)
                    }
                    0x4000_03d4 | 0x4000_03d8 => {
                        let alarm = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?
                            as usize;
                        if alarm >= self.esp_systimer_alarms.len() {
                            return Err(format!("invalid ESP systimer alarm {alarm}"));
                        }
                        if self.cpu.pc() == 0x4000_03d4 {
                            let value = u64::from(
                                self.cpu
                                    .register(RiscVRegister::A2)
                                    .map_err(|error| error.to_string())?,
                            ) | (u64::from(
                                self.cpu
                                    .register(RiscVRegister::A3)
                                    .map_err(|error| error.to_string())?,
                            ) << 32);
                            self.esp_systimer_alarms[alarm] = value;
                            self.esp_systimer_next[alarm] = value;
                            let address = ESP32C6_SYSTIMER_TARGET_VALUE + alarm as u64 * 8;
                            self.bus
                                .write(address, AccessWidth::Word, value >> 32, self.now)
                                .map_err(|error| error.to_string())?;
                            self.bus
                                .write(
                                    address + 4,
                                    AccessWidth::Word,
                                    value & u64::from(u32::MAX),
                                    self.now,
                                )
                                .map_err(|error| error.to_string())?;
                        } else {
                            // Unlike set_alarm_target(), the period argument is
                            // a 32-bit value in A2. Mirror it into TARGET_CONF:
                            // the inlined ISR reads this register directly.
                            let value = self
                                .cpu
                                .register(RiscVRegister::A2)
                                .map_err(|error| error.to_string())?
                                & ((1 << 26) - 1);
                            self.esp_systimer_periods[alarm] = u64::from(value);
                            let address = ESP32C6_SYSTIMER_TARGET_CONF + alarm as u64 * 4;
                            let current = self
                                .bus
                                .read(address, AccessWidth::Word, AccessKind::Read, self.now)
                                .map_err(|error| error.to_string())?
                                as u32;
                            self.bus
                                .write(
                                    address,
                                    AccessWidth::Word,
                                    u64::from((current & !((1 << 26) - 1)) | value),
                                    self.now,
                                )
                                .map_err(|error| error.to_string())?;
                            if value != 0 {
                                self.esp_systimer_next[alarm] = self
                                    .now
                                    .ticks()
                                    .wrapping_add(self.esp_systimer_offset)
                                    .wrapping_add(u64::from(value));
                            }
                        }
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    0x4000_03dc => {
                        let alarm = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?
                            as usize;
                        let value = *self
                            .esp_systimer_alarms
                            .get(alarm)
                            .ok_or_else(|| format!("invalid ESP systimer alarm {alarm}"))?;
                        self.complete_host_call_u64(value)?;
                        Ok(true)
                    }
                    0x4000_03e8 => {
                        let advance = u64::from(
                            self.cpu
                                .register(RiscVRegister::A2)
                                .map_err(|error| error.to_string())?,
                        ) | (u64::from(
                            self.cpu
                                .register(RiscVRegister::A3)
                                .map_err(|error| error.to_string())?,
                        ) << 32);
                        self.esp_systimer_offset = self.esp_systimer_offset.wrapping_add(advance);
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    0x4000_03e0 => {
                        let alarm = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?
                            as usize;
                        let enabled = self
                            .esp_systimer_interrupt_enabled
                            .get_mut(alarm)
                            .ok_or_else(|| format!("invalid ESP systimer alarm {alarm}"))?;
                        *enabled = true;
                        let interrupt_enable = self
                            .bus
                            .read(
                                ESP32C6_SYSTIMER_INT_ENA,
                                AccessWidth::Word,
                                AccessKind::Read,
                                self.now,
                            )
                            .map_err(|error| error.to_string())?
                            | (1_u64 << alarm);
                        self.bus
                            .write(
                                ESP32C6_SYSTIMER_INT_ENA,
                                AccessWidth::Word,
                                interrupt_enable,
                                self.now,
                            )
                            .map_err(|error| error.to_string())?;
                        if self.esp_systimer_next[alarm] == u64::MAX
                            && self.esp_systimer_periods[alarm] != 0
                        {
                            self.esp_systimer_next[alarm] = self
                                .now
                                .ticks()
                                .wrapping_add(self.esp_systimer_offset)
                                .wrapping_add(self.esp_systimer_periods[alarm]);
                        }
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    // Counter enable mirrors the work-enable bit used by
                    // direct low-level reads.
                    0x4000_03ec => {
                        let counter = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        if counter > 1 {
                            return Err(format!("invalid ESP systimer counter {counter}"));
                        }
                        let current = self
                            .bus
                            .read(
                                ESP32C6_SYSTIMER_BASE,
                                AccessWidth::Word,
                                AccessKind::Read,
                                self.now,
                            )
                            .map_err(|error| error.to_string())?
                            as u32;
                        self.bus
                            .write(
                                ESP32C6_SYSTIMER_BASE,
                                AccessWidth::Word,
                                u64::from(current | (1 << (30 - counter))),
                                self.now,
                            )
                            .map_err(|error| error.to_string())?;
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    0x4000_03f0 | 0x4000_03f4 => {
                        let alarm = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?
                            as usize;
                        if alarm >= self.esp_systimer_alarms.len() {
                            return Err(format!("invalid ESP systimer alarm {alarm}"));
                        }
                        let value = self
                            .cpu
                            .register(RiscVRegister::A2)
                            .map_err(|error| error.to_string())?;
                        let address = ESP32C6_SYSTIMER_TARGET_CONF + alarm as u64 * 4;
                        let current = self
                            .bus
                            .read(address, AccessWidth::Word, AccessKind::Read, self.now)
                            .map_err(|error| error.to_string())?
                            as u32;
                        let updated = if self.cpu.pc() == 0x4000_03f0 {
                            (current & !(1 << 30)) | ((value & 1) << 30)
                        } else {
                            (current & !(1 << 31)) | ((value & 1) << 31)
                        };
                        self.bus
                            .write(address, AccessWidth::Word, u64::from(updated), self.now)
                            .map_err(|error| error.to_string())?;
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    // APB update and stall controls do not alter the
                    // functional counter value.
                    0x4000_03e4 | 0x4000_03f8 => {
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    // ROM TLSF lock setup/entry. The functional machine executes
                    // one guest thread at a time, making these ordering points.
                    0x4000_0460 | 0x4000_0464 | 0x4000_0468 | 0x4000_046c => {
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    0x4000_0458 => {
                        let handle = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let pointer = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        let size = self
                            .esp_heaps
                            .get(&handle)
                            .and_then(|heap| heap.allocations.get(&pointer))
                            .copied()
                            .unwrap_or(0);
                        self.complete_host_call(size)?;
                        Ok(true)
                    }
                    0x4000_045c => {
                        let start = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let size = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        let result = EspFunctionalHeap::new(start, size).map_or(0, |heap| {
                            self.esp_heaps.insert(start, heap);
                            start
                        });
                        self.complete_host_call(result)?;
                        Ok(true)
                    }
                    0x4000_047c => {
                        let handle = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let size = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        let result = self.esp_heap_allocate(handle, size, 4, 0)?;
                        self.complete_host_call(result)?;
                        Ok(true)
                    }
                    0x4000_0480 => {
                        let handle = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let pointer = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        if pointer != 0 {
                            self.esp_heaps
                                .get_mut(&handle)
                                .ok_or_else(|| format!("unknown ESP heap handle {handle:#010x}"))?
                                .release(pointer);
                        }
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    0x4000_0484 => {
                        let handle = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let pointer = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        let size = self
                            .cpu
                            .register(RiscVRegister::A2)
                            .map_err(|error| error.to_string())?;
                        let result = self.esp_heap_reallocate(handle, pointer, size)?;
                        self.complete_host_call(result)?;
                        Ok(true)
                    }
                    0x4000_0488 | 0x4000_048c => {
                        let handle = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let size = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        let alignment = self
                            .cpu
                            .register(RiscVRegister::A2)
                            .map_err(|error| error.to_string())?;
                        let offset = if self.cpu.pc() == 0x4000_0488 {
                            self.cpu
                                .register(RiscVRegister::A3)
                                .map_err(|error| error.to_string())?
                        } else {
                            0
                        };
                        let result = self.esp_heap_allocate(handle, size, alignment, offset)?;
                        self.complete_host_call(result)?;
                        Ok(true)
                    }
                    0x4000_0490 => {
                        self.complete_host_call(1)?;
                        Ok(true)
                    }
                    0x4000_0498 | 0x4000_049c => {
                        let handle = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let heap = self
                            .esp_heaps
                            .get(&handle)
                            .ok_or_else(|| format!("unknown ESP heap handle {handle:#010x}"))?;
                        let value = if self.cpu.pc() == 0x4000_0498 {
                            heap.free_bytes()
                        } else {
                            heap.minimum_free
                        };
                        self.complete_host_call(value)?;
                        Ok(true)
                    }
                    0x4000_04a0 => {
                        let handle = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let info = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        let heap = self
                            .esp_heaps
                            .get(&handle)
                            .ok_or_else(|| format!("unknown ESP heap handle {handle:#010x}"))?;
                        let free = heap.free_bytes();
                        let allocated: u32 = heap.allocations.values().copied().sum();
                        let largest = heap.free.values().copied().max().unwrap_or(0);
                        let words = [
                            free,
                            allocated,
                            largest,
                            heap.minimum_free,
                            heap.allocations.len() as u32,
                            heap.free.len() as u32,
                            (heap.allocations.len() + heap.free.len()) as u32,
                        ];
                        self.write_guest_words(info, &words)?;
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    // Newlib's ROM lock objects are unnecessary in the
                    // single-threaded functional boot phase.
                    0x4000_04a4 => {
                        self.complete_host_call(0)?;
                        Ok(true)
                    }
                    // ESP32-C6 ROM RVFP entry points. The C ABI passes soft-float
                    // payloads through a0-a3, with 64-bit values split low word
                    // first. Keeping these calls at the ROM boundary makes the
                    // implementation deterministic while still executing the
                    // unmodified vendor firmware around them.
                    0x4000_08d0 | 0x4000_09f4 | 0x4000_0a64 | 0x4000_0a74 => {
                        let left_bits = u64::from(
                            self.cpu
                                .register(RiscVRegister::A0)
                                .map_err(|error| error.to_string())?,
                        ) | (u64::from(
                            self.cpu
                                .register(RiscVRegister::A1)
                                .map_err(|error| error.to_string())?,
                        ) << 32);
                        let right_bits = u64::from(
                            self.cpu
                                .register(RiscVRegister::A2)
                                .map_err(|error| error.to_string())?,
                        ) | (u64::from(
                            self.cpu
                                .register(RiscVRegister::A3)
                                .map_err(|error| error.to_string())?,
                        ) << 32);
                        let left = f64::from_bits(left_bits);
                        let right = f64::from_bits(right_bits);
                        let result = match self.cpu.pc() {
                            0x4000_08d0 => left / right,
                            0x4000_09f4 => left + right,
                            0x4000_0a64 => left * right,
                            0x4000_0a74 => left - right,
                            _ => unreachable!(),
                        };
                        self.complete_host_call_u64(result.to_bits())?;
                        Ok(true)
                    }
                    0x4000_08dc | 0x4000_09f8 | 0x4000_0a68 | 0x4000_0a78 => {
                        let left = f32::from_bits(
                            self.cpu
                                .register(RiscVRegister::A0)
                                .map_err(|error| error.to_string())?,
                        );
                        let right = f32::from_bits(
                            self.cpu
                                .register(RiscVRegister::A1)
                                .map_err(|error| error.to_string())?,
                        );
                        let result = match self.cpu.pc() {
                            0x4000_08dc => left / right,
                            0x4000_09f8 => left + right,
                            0x4000_0a68 => left * right,
                            0x4000_0a78 => left - right,
                            _ => unreachable!(),
                        };
                        self.complete_host_call(result.to_bits())?;
                        Ok(true)
                    }
                    0x4000_0988 => {
                        let bits = u64::from(
                            self.cpu
                                .register(RiscVRegister::A0)
                                .map_err(|error| error.to_string())?,
                        ) | (u64::from(
                            self.cpu
                                .register(RiscVRegister::A1)
                                .map_err(|error| error.to_string())?,
                        ) << 32);
                        self.complete_host_call_u64(bits ^ (1_u64 << 63))?;
                        Ok(true)
                    }
                    0x4000_0990 => {
                        let bits = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        self.complete_host_call(bits ^ (1_u32 << 31))?;
                        Ok(true)
                    }
                    0x4000_09ac => {
                        let bits = u64::from(
                            self.cpu
                                .register(RiscVRegister::A0)
                                .map_err(|error| error.to_string())?,
                        ) | (u64::from(
                            self.cpu
                                .register(RiscVRegister::A1)
                                .map_err(|error| error.to_string())?,
                        ) << 32);
                        let exponent = self
                            .cpu
                            .register(RiscVRegister::A2)
                            .map_err(|error| error.to_string())?
                            as i32;
                        self.complete_host_call_u64(f64::from_bits(bits).powi(exponent).to_bits())?;
                        Ok(true)
                    }
                    0x4000_09b0 => {
                        let value = f32::from_bits(
                            self.cpu
                                .register(RiscVRegister::A0)
                                .map_err(|error| error.to_string())?,
                        );
                        let exponent = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?
                            as i32;
                        self.complete_host_call(value.powi(exponent).to_bits())?;
                        Ok(true)
                    }
                    0x4000_09e4 => {
                        let left_bits = u64::from(
                            self.cpu
                                .register(RiscVRegister::A0)
                                .map_err(|error| error.to_string())?,
                        ) | (u64::from(
                            self.cpu
                                .register(RiscVRegister::A1)
                                .map_err(|error| error.to_string())?,
                        ) << 32);
                        let right_bits = u64::from(
                            self.cpu
                                .register(RiscVRegister::A2)
                                .map_err(|error| error.to_string())?,
                        ) | (u64::from(
                            self.cpu
                                .register(RiscVRegister::A3)
                                .map_err(|error| error.to_string())?,
                        ) << 32);
                        self.complete_host_call(u32::from(
                            f64::from_bits(left_bits).is_nan()
                                || f64::from_bits(right_bits).is_nan(),
                        ))?;
                        Ok(true)
                    }
                    0x4000_09e8 => {
                        let left = f32::from_bits(
                            self.cpu
                                .register(RiscVRegister::A0)
                                .map_err(|error| error.to_string())?,
                        );
                        let right = f32::from_bits(
                            self.cpu
                                .register(RiscVRegister::A1)
                                .map_err(|error| error.to_string())?,
                        );
                        self.complete_host_call(u32::from(left.is_nan() || right.is_nan()))?;
                        Ok(true)
                    }
                    0x4000_09fc | 0x4000_0a6c => {
                        let left_bits = u64::from(
                            self.cpu
                                .register(RiscVRegister::A0)
                                .map_err(|error| error.to_string())?,
                        ) | (u64::from(
                            self.cpu
                                .register(RiscVRegister::A1)
                                .map_err(|error| error.to_string())?,
                        ) << 32);
                        let right_bits = u64::from(
                            self.cpu
                                .register(RiscVRegister::A2)
                                .map_err(|error| error.to_string())?,
                        ) | (u64::from(
                            self.cpu
                                .register(RiscVRegister::A3)
                                .map_err(|error| error.to_string())?,
                        ) << 32);
                        let left = f64::from_bits(left_bits);
                        let right = f64::from_bits(right_bits);
                        self.complete_host_call(u32::from(left != right))?;
                        Ok(true)
                    }
                    0x4000_0a00 | 0x4000_0a70 => {
                        let left = f32::from_bits(
                            self.cpu
                                .register(RiscVRegister::A0)
                                .map_err(|error| error.to_string())?,
                        );
                        let right = f32::from_bits(
                            self.cpu
                                .register(RiscVRegister::A1)
                                .map_err(|error| error.to_string())?,
                        );
                        self.complete_host_call(u32::from(left != right))?;
                        Ok(true)
                    }
                    0x4000_0a04 => {
                        let value = f32::from_bits(
                            self.cpu
                                .register(RiscVRegister::A0)
                                .map_err(|error| error.to_string())?,
                        );
                        self.complete_host_call_u64((value as f64).to_bits())?;
                        Ok(true)
                    }
                    0x4000_0a7c => {
                        let bits = u64::from(
                            self.cpu
                                .register(RiscVRegister::A0)
                                .map_err(|error| error.to_string())?,
                        ) | (u64::from(
                            self.cpu
                                .register(RiscVRegister::A1)
                                .map_err(|error| error.to_string())?,
                        ) << 32);
                        self.complete_host_call((f64::from_bits(bits) as f32).to_bits())?;
                        Ok(true)
                    }
                    0x4000_0a08 | 0x4000_0a0c | 0x4000_0a18 => {
                        let bits = u64::from(
                            self.cpu
                                .register(RiscVRegister::A0)
                                .map_err(|error| error.to_string())?,
                        ) | (u64::from(
                            self.cpu
                                .register(RiscVRegister::A1)
                                .map_err(|error| error.to_string())?,
                        ) << 32);
                        let value = f64::from_bits(bits);
                        match self.cpu.pc() {
                            0x4000_0a08 => self.complete_host_call_u64((value as i64) as u64)?,
                            0x4000_0a0c => self.complete_host_call((value as i32) as u32)?,
                            0x4000_0a18 => self.complete_host_call(value as u32)?,
                            _ => unreachable!(),
                        }
                        Ok(true)
                    }
                    0x4000_0a10 | 0x4000_0a14 | 0x4000_0a1c | 0x4000_0a20 => {
                        let value = f32::from_bits(
                            self.cpu
                                .register(RiscVRegister::A0)
                                .map_err(|error| error.to_string())?,
                        );
                        match self.cpu.pc() {
                            0x4000_0a10 => self.complete_host_call_u64((value as i64) as u64)?,
                            0x4000_0a14 => self.complete_host_call((value as i32) as u32)?,
                            0x4000_0a1c => self.complete_host_call_u64(value as u64)?,
                            0x4000_0a20 => self.complete_host_call(value as u32)?,
                            _ => unreachable!(),
                        }
                        Ok(true)
                    }
                    0x4000_0a24 | 0x4000_0a28 | 0x4000_0a34 | 0x4000_0a38 => {
                        let bits = u64::from(
                            self.cpu
                                .register(RiscVRegister::A0)
                                .map_err(|error| error.to_string())?,
                        ) | (u64::from(
                            self.cpu
                                .register(RiscVRegister::A1)
                                .map_err(|error| error.to_string())?,
                        ) << 32);
                        match self.cpu.pc() {
                            0x4000_0a24 => {
                                self.complete_host_call_u64(((bits as i64) as f64).to_bits())?
                            }
                            0x4000_0a28 => {
                                self.complete_host_call(((bits as i64) as f32).to_bits())?
                            }
                            0x4000_0a34 => self.complete_host_call_u64((bits as f64).to_bits())?,
                            0x4000_0a38 => self.complete_host_call((bits as f32).to_bits())?,
                            _ => unreachable!(),
                        }
                        Ok(true)
                    }
                    0x4000_0a2c | 0x4000_0a30 | 0x4000_0a3c | 0x4000_0a40 => {
                        let bits = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        match self.cpu.pc() {
                            0x4000_0a2c => {
                                self.complete_host_call_u64(((bits as i32) as f64).to_bits())?
                            }
                            0x4000_0a30 => {
                                self.complete_host_call(((bits as i32) as f32).to_bits())?
                            }
                            0x4000_0a3c => {
                                self.complete_host_call_u64((f64::from(bits)).to_bits())?
                            }
                            0x4000_0a40 => self.complete_host_call((bits as f32).to_bits())?,
                            _ => unreachable!(),
                        }
                        Ok(true)
                    }
                    0x4000_0a44 | 0x4000_0a4c | 0x4000_0a54 | 0x4000_0a5c => {
                        let left_bits = u64::from(
                            self.cpu
                                .register(RiscVRegister::A0)
                                .map_err(|error| error.to_string())?,
                        ) | (u64::from(
                            self.cpu
                                .register(RiscVRegister::A1)
                                .map_err(|error| error.to_string())?,
                        ) << 32);
                        let right_bits = u64::from(
                            self.cpu
                                .register(RiscVRegister::A2)
                                .map_err(|error| error.to_string())?,
                        ) | (u64::from(
                            self.cpu
                                .register(RiscVRegister::A3)
                                .map_err(|error| error.to_string())?,
                        ) << 32);
                        let left = f64::from_bits(left_bits);
                        let right = f64::from_bits(right_bits);
                        let nan_result = if matches!(self.cpu.pc(), 0x4000_0a44 | 0x4000_0a4c) {
                            -1_i32
                        } else {
                            1_i32
                        };
                        let result = if left.is_nan() || right.is_nan() {
                            nan_result
                        } else if left < right {
                            -1
                        } else if left > right {
                            1
                        } else {
                            0
                        };
                        self.complete_host_call(result as u32)?;
                        Ok(true)
                    }
                    0x4000_0a48 | 0x4000_0a50 | 0x4000_0a58 | 0x4000_0a60 => {
                        let left = f32::from_bits(
                            self.cpu
                                .register(RiscVRegister::A0)
                                .map_err(|error| error.to_string())?,
                        );
                        let right = f32::from_bits(
                            self.cpu
                                .register(RiscVRegister::A1)
                                .map_err(|error| error.to_string())?,
                        );
                        let nan_result = if matches!(self.cpu.pc(), 0x4000_0a48 | 0x4000_0a50) {
                            -1_i32
                        } else {
                            1_i32
                        };
                        let result = if left.is_nan() || right.is_nan() {
                            nan_result
                        } else if left < right {
                            -1
                        } else if left > right {
                            1
                        } else {
                            0
                        };
                        self.complete_host_call(result as u32)?;
                        Ok(true)
                    }
                    0x4000_089c | 0x4000_08a0 | 0x4000_0950 => {
                        let value = u64::from(
                            self.cpu
                                .register(RiscVRegister::A0)
                                .map_err(|error| error.to_string())?,
                        ) | (u64::from(
                            self.cpu
                                .register(RiscVRegister::A1)
                                .map_err(|error| error.to_string())?,
                        ) << 32);
                        let shift = self
                            .cpu
                            .register(RiscVRegister::A2)
                            .map_err(|error| error.to_string())?
                            & 63;
                        let result = match self.cpu.pc() {
                            0x4000_089c => value.wrapping_shl(shift),
                            0x4000_08a0 => ((value as i64) >> shift) as u64,
                            0x4000_0950 => value >> shift,
                            _ => unreachable!(),
                        };
                        self.cpu
                            .set_register(RiscVRegister::A1, (result >> 32) as u32)
                            .map_err(|error| error.to_string())?;
                        self.complete_host_call(result as u32)?;
                        Ok(true)
                    }
                    0x4000_08a4 => {
                        let value = u64::from(
                            self.cpu
                                .register(RiscVRegister::A0)
                                .map_err(|error| error.to_string())?,
                        ) | (u64::from(
                            self.cpu
                                .register(RiscVRegister::A1)
                                .map_err(|error| error.to_string())?,
                        ) << 32);
                        self.complete_host_call_u64(value.swap_bytes())?;
                        Ok(true)
                    }
                    0x4000_08a8 => {
                        let value = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        self.complete_host_call(value.swap_bytes())?;
                        Ok(true)
                    }
                    0x4000_08b8 | 0x4000_08c4 | 0x4000_08f0 | 0x4000_09a4 => {
                        let value = u64::from(
                            self.cpu
                                .register(RiscVRegister::A0)
                                .map_err(|error| error.to_string())?,
                        ) | (u64::from(
                            self.cpu
                                .register(RiscVRegister::A1)
                                .map_err(|error| error.to_string())?,
                        ) << 32);
                        let result = match self.cpu.pc() {
                            0x4000_08b8 => value.leading_zeros(),
                            0x4000_08c4 => value.trailing_zeros(),
                            0x4000_08f0 => {
                                if value == 0 {
                                    0
                                } else {
                                    value.trailing_zeros() + 1
                                }
                            }
                            0x4000_09a4 => value.count_ones(),
                            _ => unreachable!(),
                        };
                        self.complete_host_call(result)?;
                        Ok(true)
                    }
                    0x4000_08bc | 0x4000_08c8 | 0x4000_08f4 | 0x4000_09a0 | 0x4000_09a8 => {
                        let value = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let result = match self.cpu.pc() {
                            0x4000_08bc => value.leading_zeros(),
                            0x4000_08c8 => value.trailing_zeros(),
                            0x4000_08f4 => {
                                if value == 0 {
                                    0
                                } else {
                                    value.trailing_zeros() + 1
                                }
                            }
                            0x4000_09a0 => value.count_ones() & 1,
                            0x4000_09a8 => value.count_ones(),
                            _ => unreachable!(),
                        };
                        self.complete_host_call(result)?;
                        Ok(true)
                    }
                    0x4000_08c0 | 0x4000_09c8 => {
                        let left = u64::from(
                            self.cpu
                                .register(RiscVRegister::A0)
                                .map_err(|error| error.to_string())?,
                        ) | (u64::from(
                            self.cpu
                                .register(RiscVRegister::A1)
                                .map_err(|error| error.to_string())?,
                        ) << 32);
                        let right = u64::from(
                            self.cpu
                                .register(RiscVRegister::A2)
                                .map_err(|error| error.to_string())?,
                        ) | (u64::from(
                            self.cpu
                                .register(RiscVRegister::A3)
                                .map_err(|error| error.to_string())?,
                        ) << 32);
                        let ordering = if self.cpu.pc() == 0x4000_08c0 {
                            (left as i64).cmp(&(right as i64))
                        } else {
                            left.cmp(&right)
                        };
                        let result = match ordering {
                            std::cmp::Ordering::Less => 0,
                            std::cmp::Ordering::Equal => 1,
                            std::cmp::Ordering::Greater => 2,
                        };
                        self.complete_host_call(result)?;
                        Ok(true)
                    }
                    0x4000_08d4 | 0x4000_095c => {
                        let numerator = (u64::from(
                            self.cpu
                                .register(RiscVRegister::A0)
                                .map_err(|error| error.to_string())?,
                        ) | (u64::from(
                            self.cpu
                                .register(RiscVRegister::A1)
                                .map_err(|error| error.to_string())?,
                        ) << 32)) as i64;
                        let denominator = (u64::from(
                            self.cpu
                                .register(RiscVRegister::A2)
                                .map_err(|error| error.to_string())?,
                        ) | (u64::from(
                            self.cpu
                                .register(RiscVRegister::A3)
                                .map_err(|error| error.to_string())?,
                        ) << 32)) as i64;
                        let result = if denominator == 0 {
                            if self.cpu.pc() == 0x4000_08d4 {
                                -1
                            } else {
                                numerator
                            }
                        } else if numerator == i64::MIN && denominator == -1 {
                            if self.cpu.pc() == 0x4000_08d4 {
                                i64::MIN
                            } else {
                                0
                            }
                        } else if self.cpu.pc() == 0x4000_08d4 {
                            numerator / denominator
                        } else {
                            numerator % denominator
                        };
                        self.complete_host_call_u64(result as u64)?;
                        Ok(true)
                    }
                    0x4000_08e0 | 0x4000_0960 => {
                        let numerator = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?
                            as i32;
                        let denominator = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?
                            as i32;
                        let result = if denominator == 0 {
                            if self.cpu.pc() == 0x4000_08e0 {
                                -1
                            } else {
                                numerator
                            }
                        } else if numerator == i32::MIN && denominator == -1 {
                            if self.cpu.pc() == 0x4000_08e0 {
                                i32::MIN
                            } else {
                                0
                            }
                        } else if self.cpu.pc() == 0x4000_08e0 {
                            numerator / denominator
                        } else {
                            numerator % denominator
                        };
                        self.complete_host_call(result as u32)?;
                        Ok(true)
                    }
                    0x4000_098c => {
                        let value = u64::from(
                            self.cpu
                                .register(RiscVRegister::A0)
                                .map_err(|error| error.to_string())?,
                        ) | (u64::from(
                            self.cpu
                                .register(RiscVRegister::A1)
                                .map_err(|error| error.to_string())?,
                        ) << 32);
                        self.complete_host_call_u64(value.wrapping_neg())?;
                        Ok(true)
                    }
                    0x4000_096c => {
                        let left = u64::from(
                            self.cpu
                                .register(RiscVRegister::A0)
                                .map_err(|error| error.to_string())?,
                        ) | (u64::from(
                            self.cpu
                                .register(RiscVRegister::A1)
                                .map_err(|error| error.to_string())?,
                        ) << 32);
                        let right = u64::from(
                            self.cpu
                                .register(RiscVRegister::A2)
                                .map_err(|error| error.to_string())?,
                        ) | (u64::from(
                            self.cpu
                                .register(RiscVRegister::A3)
                                .map_err(|error| error.to_string())?,
                        ) << 32);
                        let result = left.wrapping_mul(right);
                        self.cpu
                            .set_register(RiscVRegister::A1, (result >> 32) as u32)
                            .map_err(|error| error.to_string())?;
                        self.complete_host_call(result as u32)?;
                        Ok(true)
                    }
                    0x4000_09cc | 0x4000_09dc => {
                        let numerator = u64::from(
                            self.cpu
                                .register(RiscVRegister::A0)
                                .map_err(|error| error.to_string())?,
                        ) | (u64::from(
                            self.cpu
                                .register(RiscVRegister::A1)
                                .map_err(|error| error.to_string())?,
                        ) << 32);
                        let denominator = u64::from(
                            self.cpu
                                .register(RiscVRegister::A2)
                                .map_err(|error| error.to_string())?,
                        ) | (u64::from(
                            self.cpu
                                .register(RiscVRegister::A3)
                                .map_err(|error| error.to_string())?,
                        ) << 32);
                        let result = if denominator == 0 {
                            u64::MAX
                        } else if self.cpu.pc() == 0x4000_09cc {
                            numerator / denominator
                        } else {
                            numerator % denominator
                        };
                        self.cpu
                            .set_register(RiscVRegister::A1, (result >> 32) as u32)
                            .map_err(|error| error.to_string())?;
                        self.complete_host_call(result as u32)?;
                        Ok(true)
                    }
                    0x4000_09d4 | 0x4000_09e0 => {
                        let numerator = self
                            .cpu
                            .register(RiscVRegister::A0)
                            .map_err(|error| error.to_string())?;
                        let denominator = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        let result = if denominator == 0 {
                            u32::MAX
                        } else if self.cpu.pc() == 0x4000_09d4 {
                            numerator / denominator
                        } else {
                            numerator % denominator
                        };
                        self.complete_host_call(result)?;
                        Ok(true)
                    }
                    _ => Ok(false),
                }
            })();
            return result
                .map_err(|error| format!("ESP32-C6 functional service at PC {pc:#010x}: {error}"));
        }
        if self.target != TargetId::Rp2350 {
            return Ok(false);
        }
        let pc = self.cpu.pc();
        match pc {
            0x20 => {
                let code = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let address = self
                    .bootrom_services
                    .iter()
                    .find_map(|(address, stored)| (*stored == code).then_some(*address))
                    .unwrap_or_else(|| {
                        let address = 0x100
                            + u32::try_from(self.bootrom_services.len())
                                .expect("RP ROM service count fits u32")
                                * 4;
                        self.bootrom_services.insert(address, code);
                        address
                    });
                self.complete_host_call(address)?;
                Ok(true)
            }
            address if self.bootrom_services.contains_key(&address) => {
                let code = self.bootrom_services[&address];
                let argument = self
                    .cpu
                    .register(RiscVRegister::A0)
                    .map_err(|error| error.to_string())?;
                let result = match code {
                    0x334c => argument.leading_zeros(),
                    0x3350 => argument.count_ones(),
                    0x3352 => argument.reverse_bits(),
                    0x3354 => argument.trailing_zeros(),
                    0x4649 | 0x5845 | 0x4346 | 0x5843 => argument,
                    0x4552 => {
                        let length = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        self.bus
                            .load(
                                u64::from(0x1000_0000_u32.wrapping_add(argument)),
                                &vec![
                                    0xff;
                                    usize::try_from(length)
                                        .map_err(|_| "flash erase length overflow")?
                                ],
                            )
                            .map_err(|error| error.to_string())?;
                        argument
                    }
                    0x5052 => {
                        let source = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        let length = self
                            .cpu
                            .register(RiscVRegister::A2)
                            .map_err(|error| error.to_string())?;
                        let mut bytes = Vec::with_capacity(
                            usize::try_from(length).map_err(|_| "flash program length overflow")?,
                        );
                        for index in 0..length {
                            bytes.push(
                                self.bus
                                    .read(
                                        u64::from(source.wrapping_add(index)),
                                        renvo_core::AccessWidth::Byte,
                                        renvo_core::AccessKind::Read,
                                        self.now,
                                    )
                                    .map_err(|error| error.to_string())?
                                    as u8,
                            );
                        }
                        self.bus
                            .load(u64::from(0x1000_0000_u32.wrapping_add(argument)), &bytes)
                            .map_err(|error| error.to_string())?;
                        argument
                    }
                    0x5347 => {
                        let capacity = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        let flags = self
                            .cpu
                            .register(RiscVRegister::A2)
                            .map_err(|error| error.to_string())?;
                        let words: Vec<u32> = if flags & 0x0001 != 0 {
                            vec![0x0001, 0x2350_0001, 0x5245_4e56, 0x4f52_5032]
                        } else if flags & 0x0010 != 0 {
                            vec![0x0010, 0x7265_6e76, 0x6f2d_7270, 0x3233_3530, 0x5eed_2026]
                        } else {
                            vec![0]
                        };
                        if capacity
                            < u32::try_from(words.len()).expect("sys-info response is small")
                        {
                            u32::MAX - 12
                        } else {
                            for (index, word) in words.iter().copied().enumerate() {
                                self.bus
                                    .write(
                                        u64::from(argument.wrapping_add(
                                            u32::try_from(index).expect("small index fits u32") * 4,
                                        )),
                                        renvo_core::AccessWidth::Word,
                                        u64::from(word),
                                        self.now,
                                    )
                                    .map_err(|error| error.to_string())?;
                            }
                            u32::try_from(words.len()).expect("sys-info response is small")
                        }
                    }
                    0x434d | 0x3443 => {
                        let source = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?;
                        let length = self
                            .cpu
                            .register(RiscVRegister::A2)
                            .map_err(|error| error.to_string())?;
                        for index in 0..length {
                            let byte = self
                                .bus
                                .read(
                                    u64::from(source.wrapping_add(index)),
                                    renvo_core::AccessWidth::Byte,
                                    renvo_core::AccessKind::Read,
                                    self.now,
                                )
                                .map_err(|error| error.to_string())?;
                            self.bus
                                .write(
                                    u64::from(argument.wrapping_add(index)),
                                    renvo_core::AccessWidth::Byte,
                                    byte,
                                    self.now,
                                )
                                .map_err(|error| error.to_string())?;
                        }
                        argument
                    }
                    0x534d | 0x3453 => {
                        let byte = self
                            .cpu
                            .register(RiscVRegister::A1)
                            .map_err(|error| error.to_string())?
                            & 0xff;
                        let length = self
                            .cpu
                            .register(RiscVRegister::A2)
                            .map_err(|error| error.to_string())?;
                        for index in 0..length {
                            self.bus
                                .write(
                                    u64::from(argument.wrapping_add(index)),
                                    renvo_core::AccessWidth::Byte,
                                    u64::from(byte),
                                    self.now,
                                )
                                .map_err(|error| error.to_string())?;
                        }
                        argument
                    }
                    // BOOTROM_STATE_RESET and other lifecycle operations are deterministic
                    // ordering points in the functional single-core model.
                    0x5253 | 0x4252 | 0x5353 | 0x4152 => 0,
                    _ => {
                        return Err(format!(
                            "unsupported RP2350 RISC-V boot-ROM service code {code:#06x}"
                        ));
                    }
                };
                self.complete_host_call(result)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Selected target.
    pub const fn target(&self) -> TargetId {
        self.target
    }

    /// Enables or disables completed bus-access recording.
    pub fn set_access_recording(&mut self, enabled: bool) {
        self.bus.set_access_recording(enabled);
    }

    /// Returns completed bus operations when recording is enabled.
    pub fn access_log(&self) -> &[renvo_bus::BusAccessRecord] {
        self.bus.access_log()
    }

    /// Stops before executing an instruction at `address`.
    pub fn add_breakpoint(&mut self, address: u64) {
        self.breakpoints.insert(address);
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

    /// Loads a parsed direct-mode ELF and sets its entry point.
    pub fn load_firmware(&mut self, image: &FirmwareImage) -> Result<(), MachineError> {
        if image.architecture != FirmwareArchitecture::RiscV32 {
            return Err(MachineError::Architecture {
                target: self.target,
                actual: image.architecture,
            });
        }
        for segment in &image.segments {
            self.bus
                .load(segment.address, &segment.data)
                .map_err(|error| MachineError::Load {
                    address: segment.address,
                    message: error.to_string(),
                })?;
        }
        let entry =
            u32::try_from(image.entry).map_err(|_| MachineError::EntryRange(image.entry))?;
        self.cpu.set_pc(entry)?;
        Ok(())
    }

    /// Retains the complete merged flash artifact for ROM flash and mmap APIs.
    pub fn set_esp_flash_image(&mut self, bytes: &[u8]) {
        self.esp_flash.clear();
        self.esp_flash.extend_from_slice(bytes);
        self.esp_flash.resize(4 * 1024 * 1024, 0xff);
    }

    /// Returns the complete mutable SPI-flash state for persistence.
    pub fn esp_flash_image(&self) -> &[u8] {
        &self.esp_flash
    }

    /// Performs the documented ESP ROM verified-image handoff to an ESP32-C6
    /// application. Mapped flash segments remain backed by the official image,
    /// while load segments are copied to their declared RAM addresses.
    pub fn load_esp_application(&mut self, image: &EspFlashImage) -> Result<(), MachineError> {
        if self.target != TargetId::Esp32c6 {
            return Err(MachineError::UnsupportedTarget(self.target));
        }
        for segment in &image.application.segments {
            self.bus
                .load(u64::from(segment.address), &segment.data)
                .map_err(|error| MachineError::Load {
                    address: u64::from(segment.address),
                    message: error.to_string(),
                })?;
            if (0x4200_0000..0x4400_0000).contains(&segment.address) {
                let mut segment_header = Vec::with_capacity(8);
                segment_header.extend_from_slice(&segment.address.to_le_bytes());
                segment_header.extend_from_slice(&(segment.data.len() as u32).to_le_bytes());
                self.bus
                    .load(u64::from(segment.address - 8), &segment_header)
                    .map_err(|error| MachineError::Load {
                        address: u64::from(segment.address - 8),
                        message: error.to_string(),
                    })?;
            }
        }
        // ESP-IDF deliberately reads the application header through the
        // bytes immediately preceding the first mapped DROM segment. Preserve
        // that cache-window relationship for the direct verified-image
        // handoff.
        if let Some(first) = image.application.segments.first() {
            let header = &image.application.header;
            let mut encoded = vec![
                0xe9,
                header.segment_count,
                header.flash_mode,
                header.flash_size_frequency,
            ];
            encoded.extend_from_slice(&header.entry.to_le_bytes());
            encoded.push(header.write_protect_pin);
            encoded.extend_from_slice(&header.drive_settings);
            encoded.extend_from_slice(&header.chip_id.to_le_bytes());
            encoded.push(header.minimum_revision_legacy);
            encoded.extend_from_slice(&header.minimum_revision.to_le_bytes());
            encoded.extend_from_slice(&header.maximum_revision.to_le_bytes());
            encoded.extend_from_slice(&[0; 4]);
            encoded.push(u8::from(header.hash_appended));
            self.bus
                .load(u64::from(first.address - 32), &encoded)
                .map_err(|error| MachineError::Load {
                    address: u64::from(first.address - 32),
                    message: error.to_string(),
                })?;
        }
        // The mask ROM leaves a small flash descriptor behind its fixed
        // compatibility pointer. The second-stage loader starts the
        // application on the ROM stack below the interface-data window.
        const ROM_FLASH_DATA: u64 = 0x4087_fb00;
        const ROM_FLASH_DATA_POINTER: u64 = 0x4087_ffec;
        let mut descriptor = Vec::with_capacity(28);
        for word in [
            0x0016_40c8_u32,
            4 * 1024 * 1024,
            64 * 1024,
            4 * 1024,
            256,
            0xffff,
            0,
        ] {
            descriptor.extend_from_slice(&word.to_le_bytes());
        }
        self.bus
            .load(ROM_FLASH_DATA, &descriptor)
            .map_err(|error| MachineError::Load {
                address: ROM_FLASH_DATA,
                message: error.to_string(),
            })?;
        self.bus
            .load(
                ROM_FLASH_DATA_POINTER,
                &(ROM_FLASH_DATA as u32).to_le_bytes(),
            )
            .map_err(|error| MachineError::Load {
                address: ROM_FLASH_DATA_POINTER,
                message: error.to_string(),
            })?;
        // The mask ROM publishes its retained-DRAM reservation through a
        // fixed pointer. ESP-IDF consumes this while constructing the heap
        // region table. Keep the layout record in an otherwise unused tail of
        // the mask-ROM data window.
        const ROM_LAYOUT: u64 = 0x4004_ff00;
        const ROM_LAYOUT_POINTER: u64 = 0x4004_fffc;
        const ROM_RESERVED_DRAM_START: u32 = 0x4087_e000;
        let mut layout = vec![0_u8; 30 * 4];
        for (index, word) in [
            ROM_RESERVED_DRAM_START,
            ROM_RESERVED_DRAM_START,
            0x4087_e600,
            0x4087_e610,
        ]
        .into_iter()
        .enumerate()
        {
            layout[index * 4..index * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        self.bus
            .load(ROM_LAYOUT, &layout)
            .map_err(|error| MachineError::Load {
                address: ROM_LAYOUT,
                message: error.to_string(),
            })?;
        self.bus
            .load(ROM_LAYOUT_POINTER, &(ROM_LAYOUT as u32).to_le_bytes())
            .map_err(|error| MachineError::Load {
                address: ROM_LAYOUT_POINTER,
                message: error.to_string(),
            })?;
        self.bus
            .load(u64::from(ESP_ROM_COEX_VERSION), b"renvo-c6-functional\0")
            .map_err(|error| MachineError::Load {
                address: u64::from(ESP_ROM_COEX_VERSION),
                message: error.to_string(),
            })?;
        const ROM_FLASH_API: u64 = 0x4087_f800;
        const ROM_FLASH_NAME: u64 = 0x4087_f6e0;
        self.bus
            .load(ROM_FLASH_NAME, b"GD25Q32-functional\0")
            .map_err(|error| MachineError::Load {
                address: ROM_FLASH_NAME,
                message: error.to_string(),
            })?;
        self.write_guest_words(
            ROM_FLASH_API as u32,
            &[
                ESP_ROM_FLASH_START_STUB,
                ESP_ROM_FLASH_END_STUB,
                ESP_ROM_FLASH_CHIP_CHECK_STUB,
            ],
        )
        .map_err(MachineError::BootBlock)?;
        let mut driver = vec![0_u8; 128];
        driver[0..4].copy_from_slice(&(ROM_FLASH_NAME as u32).to_le_bytes());
        driver[16..20].copy_from_slice(&ESP_ROM_FLASH_DETECT_SIZE_STUB.to_le_bytes());
        driver[0x58..0x5c].copy_from_slice(&ESP_ROM_FLASH_OK_STUB.to_le_bytes());
        self.bus
            .load(u64::from(ESP_ROM_FLASH_DRIVER), &driver)
            .map_err(|error| MachineError::Load {
                address: u64::from(ESP_ROM_FLASH_DRIVER),
                message: error.to_string(),
            })?;
        let default_chip = [
            ESP_ROM_FLASH_HOST,
            ESP_ROM_FLASH_DRIVER,
            0,
            0,
            2,
            4 * 1024 * 1024,
            0x0016_40c8,
            0,
        ];
        self.write_guest_words(ESP_ROM_DEFAULT_FLASH, &default_chip)
            .map_err(MachineError::BootBlock)?;
        self.write_guest_words(0x4087_ffe4, &[ROM_FLASH_API as u32, ESP_ROM_DEFAULT_FLASH])
            .map_err(MachineError::BootBlock)?;
        const ROM_TLSF_TABLE: u64 = 0x4087_f600;
        self.bus
            .load(ROM_TLSF_TABLE, &[0; 20 * 4])
            .map_err(|error| MachineError::Load {
                address: ROM_TLSF_TABLE,
                message: error.to_string(),
            })?;
        self.write_guest_words(0x4087_ffd8, &[ROM_TLSF_TABLE as u32])
            .map_err(MachineError::BootBlock)?;
        self.cpu.set_register(RiscVRegister::Sp, 0x4087_e610)?;
        self.cpu.set_pc(image.application.header.entry)?;
        Ok(())
    }

    /// Loads raw instructions/data into an already mapped memory range.
    pub fn load_bytes(&mut self, address: u64, bytes: &[u8]) -> Result<(), MachineError> {
        self.bus
            .load(address, bytes)
            .map_err(|error| MachineError::Load {
                address,
                message: error.to_string(),
            })
    }

    /// Sets a direct-mode entry without parsing an ELF.
    pub fn set_entry(&mut self, entry: u32) -> Result<(), MachineError> {
        self.cpu.set_pc(entry)?;
        Ok(())
    }

    /// Queues bytes for delivery after the functional USB host enumerates CDC.
    pub fn queue_usb_input(&mut self, bytes: &[u8]) {
        if let Some(host) = &mut self.usb_host {
            host.queue_input(bytes);
        }
        if let Some(usb) = &self.esp_usb_serial_jtag {
            usb.queue_input(bytes);
        }
    }

    /// Stops a bounded run once all queued USB input returns to the raw-REPL prompt.
    pub fn stop_on_usb_input_complete(&mut self, enabled: bool) {
        self.stop_on_usb_input_complete = enabled;
    }

    /// Loads the official RP2350 RISC-V UF2 and performs its image-definition handoff.
    pub fn load_rp2350_riscv_uf2(&mut self, image: &Uf2Image) -> Result<(), MachineError> {
        const FAMILY: u32 = 0xe48b_ff5a;
        if self.target != TargetId::Rp2350 {
            return Err(MachineError::UnsupportedTarget(self.target));
        }
        let actual = image.family_id.unwrap_or_default();
        if actual != FAMILY {
            return Err(MachineError::Uf2Family {
                expected: FAMILY,
                actual,
            });
        }
        let materialized = image.materialize(0x1000_0000, 16 * 1024 * 1024, 0xff)?;
        for segment in &image.segments {
            self.bus
                .load(u64::from(segment.address), &segment.data)
                .map_err(|error| MachineError::Load {
                    address: u64::from(segment.address),
                    message: error.to_string(),
                })?;
        }
        let entry = materialized
            .get(0x20..0x24)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u32::from_le_bytes)
            .ok_or_else(|| MachineError::BootBlock("missing entry point at offset 0x20".into()))?;
        let stack = materialized
            .get(0x24..0x28)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u32::from_le_bytes)
            .ok_or_else(|| {
                MachineError::BootBlock("missing initial stack at offset 0x24".into())
            })?;
        if !(0x1000_0000..0x1100_0000).contains(&entry) || entry & 1 != 0 {
            return Err(MachineError::BootBlock(format!(
                "entry {entry:#010x} is not aligned XIP code"
            )));
        }
        if !(0x2000_0000..=0x2008_2000).contains(&stack) || stack & 3 != 0 {
            return Err(MachineError::BootBlock(format!(
                "stack {stack:#010x} is outside SRAM"
            )));
        }
        self.cpu.set_register(RiscVRegister::Sp, stack)?;
        self.cpu.set_pc(entry)?;
        Ok(())
    }

    /// Replaces the complete persistent RP2350 XIP flash backing before the
    /// official UF2 overlay is applied.
    pub fn set_rp_flash_image(&self, bytes: &[u8]) -> Result<(), MachineError> {
        if self.target != TargetId::Rp2350 {
            return Err(MachineError::UnsupportedTarget(self.target));
        }
        let storage = self
            .flash_storage
            .as_ref()
            .expect("RP2350 target has XIP flash storage");
        if bytes.len() != storage.len() {
            return Err(MachineError::BootBlock(format!(
                "persistent flash image is {} bytes; expected {}",
                bytes.len(),
                storage.len()
            )));
        }
        if !storage.write_range(0, bytes) {
            return Err(MachineError::BootBlock(
                "persistent flash backing rejected a full-image update".to_owned(),
            ));
        }
        Ok(())
    }

    /// Copies the complete mutable RP2350 XIP flash state for persistence.
    pub fn rp_flash_image(&self) -> Result<Vec<u8>, MachineError> {
        if self.target != TargetId::Rp2350 {
            return Err(MachineError::UnsupportedTarget(self.target));
        }
        Ok(self
            .flash_storage
            .as_ref()
            .expect("RP2350 target has XIP flash storage")
            .to_vec())
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
                        renvo_core::AccessWidth::Word,
                        renvo_core::AccessKind::Read,
                        self.now,
                    )
                    .map_err(MachineError::Bus)?
                    != 0;
                let interrupt = self.esp_interrupt_routes.get(&22).copied().unwrap_or(2);
                let priority = if interrupt < 32 {
                    self.bus
                        .read(
                            u64::from(0x2000_1010_u32 + interrupt * 4),
                            renvo_core::AccessWidth::Word,
                            renvo_core::AccessKind::Read,
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
                        renvo_core::AccessWidth::Word,
                        renvo_core::AccessKind::Read,
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
                                renvo_core::AccessWidth::Word,
                                renvo_core::AccessKind::Read,
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
                            renvo_core::AccessWidth::Word,
                            renvo_core::AccessKind::Read,
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
                                renvo_core::AccessWidth::Word,
                                renvo_core::AccessKind::Read,
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
                            renvo_core::AccessWidth::Word,
                            renvo_core::AccessKind::Read,
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
                        .checked_add(renvo_core::SimDuration::TICK)
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
                            .checked_add(renvo_core::SimDuration::TICK)
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
mod tests {
    use super::*;
    use renvo_trace::{Timescale, VcdWriter};

    #[test]
    fn esp32c6_rom_systimer_period_is_visible_to_inlined_isr_reads() {
        let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
        machine.cpu.set_pc(0x4000_03d8).unwrap();
        machine.cpu.set_register(RiscVRegister::A1, 0).unwrap();
        machine.cpu.set_register(RiscVRegister::A2, 1_000).unwrap();
        // set_alarm_period() has a 32-bit third argument; A3 must not leak
        // into the host-side period even if it contains unrelated state.
        machine
            .cpu
            .set_register(RiscVRegister::A3, u32::MAX)
            .unwrap();

        assert!(machine.service_functional_bootrom().unwrap());

        assert_eq!(machine.esp_systimer_periods[0], 1_000);
        assert_eq!(
            machine
                .bus
                .read(
                    ESP32C6_SYSTIMER_TARGET_CONF,
                    AccessWidth::Word,
                    AccessKind::Read,
                    SimTime::ZERO,
                )
                .unwrap()
                & ((1 << 26) - 1),
            1_000
        );
    }

    #[test]
    fn all_initial_riscv_modes_execute_and_halt_deterministically() {
        // addi x1,x0,7; addi x2,x0,5; add x3,x1,x2; ebreak
        let program = [0x0070_0093_u32, 0x0050_0113, 0x0020_81b3, 0x0010_0073]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>();
        for target in [
            TargetId::Ch32v003,
            TargetId::Ch32v006,
            TargetId::Esp32c6,
            TargetId::Rp2350,
        ] {
            let entry = target_manifest(target).memory[0].start;
            let mut machine = RiscVMachine::new(target).unwrap();
            machine.load_bytes(entry, &program).unwrap();
            machine
                .set_entry(u32::try_from(entry).expect("initial addresses fit RV32"))
                .unwrap();
            let result = machine
                .run(
                    RunLimits {
                        instructions: Some(16),
                        deadline: None,
                    },
                    None,
                )
                .unwrap();
            assert_eq!(result.reason, StopReason::Halted, "{target}");
            assert_eq!(result.cpu.registers[3].value, 12, "{target}");
        }
    }

    #[test]
    fn gpio_facade_streams_valid_vcd() {
        // lui x1,0xffff0; addi x2,x0,1; sw x2,0(x1); sw x2,4(x1); ebreak
        let program = [
            0xffff_00b7_u32,
            0x0010_0113,
            0x0020_a023,
            0x0020_a223,
            0x0010_0073,
        ]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect::<Vec<_>>();
        let mut machine = RiscVMachine::new(TargetId::Ch32v003).unwrap();
        machine.load_bytes(0, &program).unwrap();
        machine.set_entry(0).unwrap();
        let mut vcd = VcdWriter::new(Vec::new(), Timescale::Nanosecond);
        let result = machine
            .run(
                RunLimits {
                    instructions: Some(16),
                    deadline: None,
                },
                Some(&mut vcd),
            )
            .unwrap();
        assert_eq!(result.reason, StopReason::Halted);
        let output = String::from_utf8(vcd.into_inner()).unwrap();
        assert!(output.contains("$enddefinitions $end"));
        assert!(output.contains("#3"));
    }

    #[test]
    fn unsupported_targets_fail_explicitly() {
        assert!(matches!(
            RiscVMachine::new(TargetId::Rp2040),
            Err(MachineError::UnsupportedTarget(TargetId::Rp2040))
        ));
    }
}
