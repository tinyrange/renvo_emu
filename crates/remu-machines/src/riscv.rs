use crate::arm::Rp2040UsbHost;
use crate::{
    MemoryKind, PinStimulus, SignalEdge, SignalStop, TargetId, resolve_signal_stop,
    run_control::RunControl, target_manifest,
};
use md5::{Digest, Md5};
use remu_bus::{
    AddressSpace, Endianness, MapError, Permissions, SharedAccessGuard, SharedBusAccessObserver,
    SharedMemory,
};
use remu_core::{
    AccessKind, AccessWidth, Bus, Cpu, CpuFault, CpuSnapshot, ResetKind, RunLimits, RunStats,
    SimTime, StepReason, StopReason,
};
use remu_cpu_riscv::{RiscVCpu, RiscVProfile, RiscVRegister};
use remu_devices::{
    EspC6Clint, EspC6ClintHandle, EspC6Extmem, EspC6ExtmemHandle, EspC6Plic, EspC6PlicHandle,
    EspGpio, EspSpiFlashCommand, EspSpiMem, EspSpiMemMmuHandle, EspTimerGroup, EspTimerGroupHandle,
    EspTimerGroupKind, EspUsbSerialJtagHandle, ExitDevice, ExitHandle, FunctionalGpio,
    FunctionalPwm, FunctionalTimer, FunctionalUart, GpioHandle, PwmHandle, RegisterBank,
    Rp2040Clocks, Rp2040Pll, Rp2040RegisterBank, Rp2040Timer, Rp2040TimerHandle,
    Rp2040UsbController, Rp2040UsbHandle, Rp2040Xosc, Rp2350AccessCtrl, Rp2350AccessCtrlHandle,
    Rp2350AccessMaster, Rp2350BootRam, Rp2350Otp, Rp2350Powman, Rp2350Sha256, Rp2350Spi,
    Rp2350SpiHandle, Rp2350Ticks, Rp2350Trng, Rp2350TrngHandle, Rp2350XipMaintenance, RpAdc,
    RpAdcHandle, RpAdcVariant, RpDma, RpDmaHandle, RpDmaVariant, RpI2cHandle, RpIoBankHandle,
    RpPadsBank, RpPadsHandle, RpPadsVariant, RpPio, RpPioHandle, RpPioVersion, RpPl011Uart,
    RpSioGpio, RpSioHandle, RpTimerLayout, SignalHub, TimerHandle, UartHandle, WchGpio, WchPfic,
    WchPficHandle, WchTimer, WchTimerHandle, WchUsart, new_rp2350_hstx,
};
use remu_image::{
    EspExecutableImage, EspFlashImage, FirmwareArchitecture, FirmwareImage, Uf2Error, Uf2Image,
};
use remu_radio::{
    BdAddress, BleController, CoexistenceArbiter, CoexistenceError, CoexistenceGrantId,
    Ieee802154Mac, MacAddress, MediumError, MediumProfile, RadioChip, RadioLegalityError,
    RadioLegalityValidator, RadioMedium, TransmissionId, WifiEngine,
};
use remu_signals::{Logic, SignalError};
use remu_trace::{TraceError, TraceSink};
use serde::Serialize;
use sha2::{Sha224, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

mod adc;
mod bootrom_support;
mod esp32c6_peripherals;
use esp32c6_peripherals::{Esp32c6PeripheralHandles, map_esp32c6_peripherals};
mod esp_bootrom_primary;
mod esp_bootrom_secondary;
mod functional_bootrom;
mod heap;
use heap::EspFunctionalHeap;
mod image;
mod lp_uart;
mod pio;
mod pwm;
mod radio;
mod rp2350_spi;
mod rp_bootrom;
mod rp_i2c;
use rp_i2c::map_rp2350_i2c;
mod rp_io;
use rp2350_spi::{map_rp2350_spi, set_rp2350_spi_interrupts};
mod runtime;
mod watchdog;

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
const ESP_ROM_PHY_NOOP_STUB: u32 = 0x4004_fe14;
const ESP_ROM_PHY_I2C_STABLE_STUB: u32 = 0x4004_fe18;
const ESP_ROM_COEX_VERSION: u32 = 0x4004_fdc0;
const ESP_ROM_PHY_FUNCTION_TABLE: u32 = 0x4087_f000;
const ESP_ROM_DEFAULT_FLASH: u32 = 0x4087_fa00;
const ESP_ROM_FLASH_DRIVER: u32 = 0x4087_f900;
const ESP_ROM_FLASH_HOST: u32 = 0x4087_f700;
const ESP_FUNCTIONAL_MMAP_BASE: u32 = 0x4280_0000;
const ESP32C6_CACHE_MMU_VADDR_BASE: u32 = 0x4200_0000;
const ESP32C6_SYSTIMER_BASE: u64 = 0x6000_a000;
// The direct-mode core advances one simulation tick per 160 MHz CPU action;
// the ESP32-C6 system timer runs from its 16 MHz clock source.
const ESP32C6_CPU_TICKS_PER_SYSTIMER_TICK: u64 = 10;
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
    /// Deterministic RF-medium operation failed.
    #[error(transparent)]
    Radio(#[from] MediumError),
    /// Shared-RF coexistence arbitration failed.
    #[error(transparent)]
    Coexistence(#[from] CoexistenceError),
    /// Firmware configured a state outside the recovered native-radio contract.
    #[error(transparent)]
    RadioLegality(#[from] RadioLegalityError),
    /// Firmware has not released the selected radio domain from reset and enabled its clocks.
    #[error("{0} radio domain is clock-gated or held in reset")]
    RadioNotReady(&'static str),
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
    boot_rom_loaded: bool,
    sio: Option<RpSioHandle>,
    io_bank: Option<RpIoBankHandle>,
    dma: Option<RpDmaHandle>,
    accessctrl: Option<Rp2350AccessCtrlHandle>,
    pads: Option<RpPadsHandle>,
    security_contexts: [(bool, bool); 2],
    bus: AddressSpace,
    signals: SignalHub,
    gpio: GpioHandle,
    chip_gpio: Vec<GpioHandle>,
    pub(crate) uart: UartHandle,
    pub(crate) chip_uarts: Vec<UartHandle>,
    pub(crate) chip_adc: Option<RpAdcHandle>,
    pub(crate) chip_pwm: Option<PwmHandle>,
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
    esp_reset_reason: u32,
    esp_flash: Vec<u8>,
    esp_application: Option<EspFlashImage>,
    esp_direct_firmware: Option<FirmwareImage>,
    esp32c6_materialized_mmu: [u32; 256],
    esp32c6_flash_dirty: bool,
    esp_timer_groups: Vec<EspTimerGroupHandle>,
    esp_c6_plic: Option<EspC6PlicHandle>,
    esp_c6_clint: Option<EspC6ClintHandle>,
    esp_c6_extmem: Option<EspC6ExtmemHandle>,
    esp_c6_spimem_mmu: Option<EspSpiMemMmuHandle>,
    esp_c6_spimem_flash: Option<EspSpiMemMmuHandle>,
    esp32c6_peripherals: Option<Esp32c6PeripheralHandles>,
    radio_medium: Option<RadioMedium>,
    radio_coexistence: Option<CoexistenceArbiter>,
    radio_ieee802154_mac: Option<Ieee802154Mac>,
    radio_wifi: Option<WifiEngine>,
    radio_ble: Option<BleController>,
    radio_legality: Option<RadioLegalityValidator>,
    radio_pending_ieee802154_tx: Vec<(TransmissionId, CoexistenceGrantId, SimTime, Option<u8>)>,
    radio_pending_ieee802154_ack: Vec<(TransmissionId, CoexistenceGrantId, SimTime)>,
    radio_pending_ieee802154_cca: Option<SimTime>,
    radio_pending_native_wifi: Vec<crate::native_wifi::PendingNativeWifiTransmission>,
    radio_c6_ble_receptions: Vec<radio::PendingNativeBleReception>,
    radio_c6_ble_completion_anchors: BTreeMap<u32, u32>,
    radio_c6_ble_schedule_records: Vec<u32>,
    radio_c6_pending_ble_transmissions: Vec<radio::PendingNativeBleTransmission>,
    radio_c6_ble_link_sequences: BTreeMap<u32, radio::C6BleLinkSequence>,
    radio_c6_reset_generations: [u64; 4],
    radio_c6_wifi_mac_reset_generation: u64,
    radio_c6_interrupt_sources: [bool; 6],
    radio_coexistence_transmission: Option<(CoexistenceGrantId, TransmissionId)>,
    radio_event_cursor: usize,
    flash_storage: Option<SharedMemory>,
    chip_timers: Vec<Rp2040TimerHandle>,
    i2c: Vec<RpI2cHandle>,
    spi: Vec<Rp2350SpiHandle>,
    pio: Vec<RpPioHandle>,
    wch_timer: Option<WchTimerHandle>,
    wch_pfic: Option<WchPficHandle>,
    usb: Option<Rp2040UsbHandle>,
    usb_dpram: Option<SharedMemory>,
    usb_host: Option<Rp2040UsbHost>,
    trng: Option<Rp2350TrngHandle>,
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
        let (mut bus, signals) = (AddressSpace::new(Endianness::Little), SignalHub::new());
        let (mut chip_timers, mut i2c) = (Vec::new(), Vec::new());
        let mut spi = Vec::new();
        let mut pio = Vec::new();
        let mut usb = None;
        let mut usb_dpram = None;
        let mut usb_host = None;
        let mut trng = None;
        let mut esp_usb_serial_jtag = None;
        let mut esp_timer_groups = Vec::new();
        let mut esp_c6_plic = None;
        let mut esp_c6_clint = None;
        let mut esp_c6_extmem = None;
        let mut esp_c6_spimem_mmu = None;
        let mut esp_c6_spimem_flash = None;
        let mut esp32c6_peripherals = None;
        let radio_medium = if target == TargetId::Esp32c6 {
            Some(RadioMedium::new(MediumProfile::default())?)
        } else {
            None
        };
        let radio_coexistence = (target == TargetId::Esp32c6).then(CoexistenceArbiter::new);
        let radio_ieee802154_mac = (target == TargetId::Esp32c6).then(Ieee802154Mac::new);
        let radio_wifi = (target == TargetId::Esp32c6)
            .then(|| WifiEngine::new(MacAddress([0x02, 0, 0, 0, 0xc6, 1])));
        let radio_ble = (target == TargetId::Esp32c6)
            .then(|| BleController::new(BdAddress([1, 0xc6, 0, 0, 0, 0x02]), 0x32c6_5eed));
        let radio_legality =
            (target == TargetId::Esp32c6).then(|| RadioLegalityValidator::new(RadioChip::Esp32C6));
        let mut wch_timer = None;
        let mut wch_pfic = None;
        let mut chip_adc = None;
        let mut chip_pwm = None;
        let mut sio = None;
        let mut io_bank = None;
        let mut dma = None;
        let mut accessctrl = None;
        let mut pads = None;
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
            let (device, handle) = Rp2350AccessCtrl::new_with_handle("rp2350.accessctrl");
            bus.map_device("rp2350.accessctrl", 0x4006_0000, 0x4000, Box::new(device))?;
            let guard_handle = handle.clone();
            let guard: SharedAccessGuard =
                std::rc::Rc::new(std::cell::RefCell::new(move |address, _width, _kind| {
                    guard_handle.check_address(address)
                }));
            bus.set_access_guard(Some(guard));
            accessctrl = Some(handle);
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
            let (pad_device, pad_handle) = RpPadsBank::new(
                "rp2350.pads-bank0",
                manifest.gpio_count,
                RpPadsVariant::Rp2350,
            );
            bus.map_device(
                "rp2350.pads-bank0",
                0x4003_8000,
                0x4000,
                Box::new(pad_device),
            )?;
            pads = Some(pad_handle);
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
                Box::new(Rp2350Ticks::new("rp2350.ticks")),
            )?;
            bus.map_device(
                "rp2350.powman",
                0x4010_0000,
                0x4000,
                Box::new(Rp2350Powman::new("rp2350.powman")),
            )?;
            let (device, handle) = Rp2350Trng::new("rp2350.trng");
            bus.map_device("rp2350.trng", 0x400f_0000, 0x4000, Box::new(device))?;
            trng = Some(handle);
            bus.map_device(
                "rp2350.sha256",
                0x400f_8000,
                0x4000,
                Box::new(Rp2350Sha256::new("rp2350.sha256")),
            )?;
            bus.map_device(
                "rp2350.otp",
                0x4012_0000,
                0x2_0000,
                Box::new(Rp2350Otp::new("rp2350.otp")),
            )?;
            let (device, handle) = RpDma::new_for_variant("rp2350.dma", RpDmaVariant::Rp2350);
            bus.map_device("rp2350.dma", 0x5000_0000, 0x4000, Box::new(device))?;
            dma = Some(handle);
            map_rp2350_spi(&mut bus, &mut spi)?;
            map_rp2350_i2c(&mut bus, &signals, &mut i2c)?;
            let (adc, adc_handle) = RpAdc::new_for_variant("rp2350.adc", RpAdcVariant::FiveChannel);
            bus.map_device("rp2350.adc", 0x400a_0000, 0x1000, Box::new(adc))?;
            chip_adc = Some(adc_handle);
            let (pwm, pwm_handle) = FunctionalPwm::new("rp2350.pwm", 12);
            bus.map_device("rp2350.pwm", 0x400a_8000, 0x4000, Box::new(pwm))?;
            chip_pwm = Some(pwm_handle);
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
                let (plic_machine, plic_user, plic_handle) =
                    EspC6Plic::new_pair("esp32c6.plic-machine", "esp32c6.plic-user");
                bus.map_device(
                    "esp32c6.plic-machine",
                    0x2000_1000,
                    0x400,
                    Box::new(plic_machine),
                )?;
                bus.map_device("esp32c6.plic-user", 0x2000_1400, 0x400, Box::new(plic_user))?;
                esp_c6_plic = Some(plic_handle);
                let (clint, clint_handle) = EspC6Clint::new("esp32c6.clint");
                bus.map_device("esp32c6.clint", 0x2000_1800, 0x800, Box::new(clint))?;
                esp_c6_clint = Some(clint_handle);
                bus.map_device(
                    "esp32c6.assist-debug",
                    0x600c_2000,
                    0x1000,
                    Box::new(remu_devices::EspC6ControlBlock::new(
                        "esp32c6.assist-debug",
                        0x1000,
                        Some(0x3fc),
                        35_656_192,
                    )),
                )?;
                let (extmem, extmem_handle) = EspC6Extmem::new("esp32c6.extmem");
                bus.map_device("esp32c6.extmem", 0x600c_8000, 0x1000, Box::new(extmem))?;
                esp_c6_extmem = Some(extmem_handle);
                let (spimem0, spimem0_mmu) = EspSpiMem::new_observed("esp32c6.spimem0");
                bus.map_device("esp32c6.spimem0", 0x6000_2000, 0x1000, Box::new(spimem0))?;
                esp_c6_spimem_mmu = Some(spimem0_mmu);
                let (spimem1, spimem1_flash) = EspSpiMem::new_observed("esp32c6.spimem1");
                bus.map_device("esp32c6.spimem1", 0x6000_3000, 0x1000, Box::new(spimem1))?;
                esp_c6_spimem_flash = Some(spimem1_flash);
                let (peripherals, usb_serial_jtag) =
                    map_esp32c6_peripherals(&mut bus, &signals, &mut chip_uarts)?;
                esp32c6_peripherals = Some(peripherals);
                esp_usb_serial_jtag = Some(usb_serial_jtag);
                for (name, base) in [
                    ("esp32c6.timer-group0", 0x6000_8000),
                    ("esp32c6.timer-group1", 0x6000_9000),
                ] {
                    let (device, handle) = EspTimerGroup::new(name, EspTimerGroupKind::Esp32C6);
                    bus.map_device(name, base, 0x1000, Box::new(device))?;
                    esp_timer_groups.push(handle);
                }
                // GPIO9 has an internal pull-up on a normal ESP32-C6 reset.
                // The ROM samples it as strap bit 3 and selects SPI flash boot.
                let (device, handle) = EspGpio::new_with_strap(
                    "esp32c6.gpio",
                    31,
                    "board.esp32c6.chip_gpio",
                    signals.clone(),
                    3 << 2,
                )?;
                bus.map_device("esp32c6.gpio", 0x6009_1000, 0x1000, Box::new(device))?;
                chip_gpio.push(handle);
            }
            TargetId::Rp2350 => {
                let (device, handle, multicore) = RpSioGpio::new_rp2350_with_multicore(
                    "rp2350.sio",
                    manifest.gpio_count,
                    "board.rp2350.chip_gpio",
                    signals.clone(),
                )?;
                bus.map_device("rp2350.sio", 0xd000_0000, 0x200, Box::new(device))?;
                chip_gpio.push(handle);
                sio = Some(multicore);
                io_bank = Some(rp_io::map(&mut bus, chip_gpio[0].clone())?);
                let (uart0, handle) =
                    FunctionalUart::new_lenient("rp2350.uart0", 0x00, 0x18, 0x0090);
                bus.map_device("rp2350.uart0", 0x4007_0000, 0x1000, Box::new(uart0))?;
                chip_uarts.push(handle);
                let (uart1, handle) = RpPl011Uart::new("rp2350.uart1");
                bus.map_device("rp2350.uart1", 0x4007_8000, 0x4000, Box::new(uart1))?;
                chip_uarts.push(handle);
                let (pio0, handle) = RpPio::new_with_version(
                    "rp2350.pio0",
                    u16::from(manifest.gpio_count.min(32)),
                    "board.rp2350.pio0.gpio",
                    signals.clone(),
                    RpPioVersion::Rp2350,
                )?;
                bus.map_device("rp2350.pio0", 0x5020_0000, 0x4000, Box::new(pio0))?;
                pio.push(handle);
                let (hstx_ctrl, hstx_fifo, _hstx_handle) =
                    new_rp2350_hstx("rp2350.hstx", "board.rp2350.hstx", signals.clone())?;
                bus.map_device("rp2350.hstx.ctrl", 0x400c_0000, 0x4000, Box::new(hstx_ctrl))?;
                bus.map_device("rp2350.hstx.fifo", 0x5060_0000, 0x1000, Box::new(hstx_fifo))?;
                pio::map_secondary_rp2350_pios(
                    &mut bus,
                    &mut pio,
                    &signals,
                    u16::from(manifest.gpio_count.min(32)),
                )?;
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

        let secondary_profile = if target == TargetId::Esp32c6 {
            RiscVProfile::esp32c6_lp()
        } else {
            profile.clone()
        };
        let radio_c6_reset_generations = esp32c6_peripherals
            .as_ref()
            .map(|handles| handles.modem.reset_generations())
            .unwrap_or([0; 4]);
        let radio_c6_wifi_mac_reset_generation = esp32c6_peripherals
            .as_ref()
            .map(|handles| handles.wifi_mac.reset_generation())
            .unwrap_or(0);
        Ok(Self {
            target,
            cpu: RiscVCpu::new(profile.clone())?,
            cpu1: RiscVCpu::new(secondary_profile)?,
            cpu1_active: false,
            boot_rom_loaded: false,
            sio,
            io_bank,
            dma,
            accessctrl,
            pads,
            security_contexts: [(false, true); 2],
            bus,
            signals,
            gpio,
            chip_gpio,
            uart,
            chip_uarts,
            chip_adc,
            chip_pwm,
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
            esp_reset_reason: 1,
            esp_flash: Vec::new(),
            esp_application: None,
            esp_direct_firmware: None,
            esp32c6_materialized_mmu: [u32::MAX; 256],
            esp32c6_flash_dirty: false,
            esp_timer_groups,
            esp_c6_plic,
            esp_c6_clint,
            esp_c6_extmem,
            esp_c6_spimem_mmu,
            esp_c6_spimem_flash,
            esp32c6_peripherals,
            radio_medium,
            radio_coexistence,
            radio_ieee802154_mac,
            radio_wifi,
            radio_ble,
            radio_legality,
            radio_pending_ieee802154_tx: Vec::new(),
            radio_pending_ieee802154_ack: Vec::new(),
            radio_pending_ieee802154_cca: None,
            radio_pending_native_wifi: Vec::new(),
            radio_c6_ble_receptions: Vec::new(),
            radio_c6_ble_completion_anchors: BTreeMap::new(),
            radio_c6_ble_schedule_records: Vec::new(),
            radio_c6_pending_ble_transmissions: Vec::new(),
            radio_c6_ble_link_sequences: BTreeMap::new(),
            radio_c6_reset_generations,
            radio_c6_wifi_mac_reset_generation,
            radio_c6_interrupt_sources: [false; 6],
            radio_coexistence_transmission: None,
            radio_event_cursor: 0,
            flash_storage,
            chip_timers,
            i2c,
            spi,
            pio,
            wch_timer,
            wch_pfic,
            usb,
            usb_dpram,
            usb_host,
            trng,
            esp_usb_serial_jtag,
            stop_on_usb_input_complete: false,
            breakpoints: BTreeSet::new(),
            signal_stops: Vec::new(),
        })
    }
}
include!("riscv/machine_runtime.rs");
#[cfg(test)]
mod tests;
