use crate::HOST_SCRIPT_COMPLETE_MARKER;
use crate::riscv::{TEST_DEVICE_SIZE, TEST_EXIT_SIZE};
use crate::{
    MemoryKind, PinStimulus, RunResult, SignalEdge, SignalStop, TEST_EXIT, TEST_GPIO, TEST_TIMER,
    TEST_UART, TargetId, matching_signal_stop, resolve_signal_stop, target_manifest,
};
use md5::{Digest, Md5};
use remu_bus::{AddressSpace, Endianness, Permissions, SharedBusAccessObserver, SharedMemory};
use remu_core::{
    AccessKind, AccessWidth, Bus, Cpu, RunLimits, RunStats, SimTime, StepReason, StopReason,
};
use remu_cpu_xtensa::{XtensaCpu, XtensaRegister};
use remu_devices::{
    Esp32S3Aes, Esp32S3AesHandle, Esp32S3AgcRegisters, Esp32S3BleExchangeMemoryHandle,
    Esp32S3BleExchangeMemoryRegisters, Esp32S3BleLpClock, Esp32S3BleLpClockHandle, Esp32S3Extmem,
    Esp32S3ExtmemHandle, Esp32S3FeRegisters, Esp32S3IoMux, Esp32S3IoMuxHandle, Esp32S3LcdCam,
    Esp32S3LcdCamHandle, Esp32S3Ledc, Esp32S3LedcHandle, Esp32S3Mcpwm, Esp32S3McpwmHandle,
    Esp32S3Pcnt, Esp32S3PcntHandle, Esp32S3PhyRegisters, Esp32S3Pms, Esp32S3PmsHandle,
    Esp32S3SarAdc, Esp32S3SarAdcHandle, Esp32S3Sdmmc, Esp32S3SdmmcHandle, Esp32S3Sha,
    Esp32S3ShaHandle, Esp32S3Syscon, Esp32S3SysconHandle, Esp32S3Tsens, Esp32S3TsensHandle,
    Esp32S3Uhci, Esp32S3UhciHandle, Esp32S3UsbWrap, Esp32S3UsbWrapHandle, Esp32S3WifiMacHandle,
    Esp32S3WifiMacRegisters, Esp32S3WorldController, Esp32S3WorldControllerHandle, Esp32S3XtsAes,
    Esp32s3I2c, Esp32s3I2cHandle, Esp32s3I2s, Esp32s3I2sHandle, Esp32s3Rmt, Esp32s3RmtHandle,
    Esp32s3Spi, Esp32s3SpiHandle, EspC6ControlBlock, EspDigitalSignature, EspEfuse, EspGdma,
    EspGdmaHandle, EspGpio, EspHmac, EspInterruptMatrix, EspInterruptMatrixHandle, EspMmuTable,
    EspMmuTableHandle, EspRsa, EspRtcControl, EspRtcControlHandle, EspSpiMem, EspSystem,
    EspSystemHandle, EspSystimer, EspSystimerHandle, EspTimerGroup, EspTimerGroupHandle,
    EspTimerGroupKind, EspTwai, EspTwaiHandle, EspUsbOtg, EspUsbOtgHandle, EspUsbSerialJtag,
    EspUsbSerialJtagHandle, ExitDevice, ExitHandle, FunctionalGpio, FunctionalTimer,
    FunctionalUart, GpioHandle, SignalHub, TimerHandle, UartHandle,
};
use remu_image::{EspFlashImage, FirmwareArchitecture, FirmwareImage};
use remu_radio::{
    BdAddress, BleController, CoexistenceArbiter, CoexistenceGrantId, MacAddress, MediumProfile,
    RadioChip, RadioLegalityValidator, RadioMedium, TransmissionId, WifiEngine,
};
use remu_signals::Logic;
use remu_trace::{TraceDigest, TraceSink};
use sha2::{Sha224, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
mod error;
mod functional_rom;
mod interrupts;
mod peripheral_dma;
mod peripheral_handles;
mod pms;
mod radio;
pub use error::XtensaMachineError;
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
    i2c: Vec<Esp32s3I2cHandle>,
    spi: Vec<Esp32s3SpiHandle>,
    i2s: Vec<Esp32s3I2sHandle>,
    rmt: Esp32s3RmtHandle,
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
    assist_debug: remu_devices::Esp32S3AssistDebugHandle,
    syscon: Esp32S3SysconHandle,
    wifi_mac: Esp32S3WifiMacHandle,
    ble_exchange_memory: Esp32S3BleExchangeMemoryHandle,
    ble_lp_clock: Esp32S3BleLpClockHandle,
    rtc_control: EspRtcControlHandle,
    rtc_i2c: remu_devices::Esp32S3RtcI2cHandle,
    rtc_io: remu_devices::Esp32S3RtcIoHandle,
    sdm: remu_devices::Esp32S3SdmHandle,
    mmu_table: EspMmuTableHandle,
    radio_medium: RadioMedium,
    radio_coexistence: CoexistenceArbiter,
    radio_wifi: WifiEngine,
    radio_ble: BleController,
    radio_legality: RadioLegalityValidator,
    radio_reset_generation: u64,
    radio_coexistence_transmission: Option<(CoexistenceGrantId, TransmissionId)>,
    radio_event_cursor: usize,
    pending_native_wifi: Vec<crate::native_wifi::PendingNativeWifiTransmission>,
    pending_native_ble_transmissions: VecDeque<radio::PendingNativeBleTransmission>,
    pending_native_ble_receptions: VecDeque<radio::PendingNativeBleReception>,
    pending_native_ble_slot_completions: VecDeque<(u64, u32, u16)>,
    native_ble_link_sequences: BTreeMap<u32, radio::S3BleLinkSequence>,
    now: SimTime,
    stack: u32,
    instruction_cache_configured: bool,
    boot_rom_loaded: bool,
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
    fn release_appcpu_if_requested(&mut self) {
        if self.appcpu_boot_address.is_some() {
            return;
        }
        let entry = self.system.appcpu_boot_address();
        if entry == 0 {
            return;
        }
        self.appcpu_boot_address = Some(entry);
        self.cpu1
            .set_direct_state(self.stack.wrapping_sub(0x1000), entry);
        self.cpu1.set_processor_id(1);
    }

    /// Creates the ESP32-S3 direct-mode map.
    pub fn new(target: TargetId) -> Result<Self, XtensaMachineError> {
        if target != TargetId::Esp32s3 {
            return Err(XtensaMachineError::UnsupportedTarget(target));
        }
        let manifest = target_manifest(target);
        let mut bus = AddressSpace::new(Endianness::Little);
        let signals = SignalHub::new();
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
        bus.map_shared(
            "esp32s3.rom-rodata",
            0x3ff1_8000,
            0x5000,
            Permissions::RO,
            SharedMemory::zeroed(0x5000),
            0,
        )?;
        let mut radio_service_page = vec![0_u8; 0x1000];
        let rom_service_signature = b"remu-coex-rom-v0.0\0";
        radio_service_page[..rom_service_signature.len()].copy_from_slice(rom_service_signature);
        bus.map_shared(
            "esp32s3.radio-service-data",
            0x3ff1_d000,
            0x1000,
            Permissions::RW,
            SharedMemory::from_bytes(radio_service_page),
            0,
        )?;
        let mut rom_service_data = vec![0_u8; 0x1000];
        // ESP32-S3 rev0 mask-ROM Wi-Fi interface constants. These three
        // immutable pointers connect libpp's native LMAC routines to the
        // ROM-reserved state blocks. Values are reproduced from the
        // .rodata.interface section of Espressif's pinned rev0 ROM ELF.
        rom_service_data[0xe50..0xe54].copy_from_slice(&0x3fce_f1ac_u32.to_le_bytes());
        rom_service_data[0xe54..0xe58].copy_from_slice(&0x3fce_f3d1_u32.to_le_bytes());
        rom_service_data[0xe58..0xe5c].copy_from_slice(&0x3fce_f308_u32.to_le_bytes());
        bus.map_shared(
            "esp32s3.rom-service-data",
            0x3ff1_e000,
            0x1000,
            Permissions::RO,
            SharedMemory::from_bytes(rom_service_data),
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
        for (name, base, size, reset_offset, reset_value) in [
            ("esp32s3.fe2-registers", 0x6000_5000, 0x1000, 0, 0),
            ("esp32s3.bt-registers", 0x6001_1000, 0x1000, 0, 0),
            ("esp32s3.nrx-registers", 0x6001_cc00, 0x400, 0, 0),
            ("esp32s3.bb-registers", 0x6001_d000, 0x1000, 0, 0),
        ] {
            let registers = EspC6ControlBlock::new(name, size, None, 0)
                .with_reset_word(reset_offset, reset_value);
            bus.map_device(name, base, size, Box::new(registers))?;
        }
        bus.map_device(
            "esp32s3.fe-registers",
            0x6000_6000,
            0x1000,
            Box::new(Esp32S3FeRegisters::new("esp32s3.fe-registers")),
        )?;
        bus.map_device(
            "esp32s3.phy-registers",
            0x6000_e000,
            0x1000,
            Box::new(Esp32S3PhyRegisters::new("esp32s3.phy-registers")),
        )?;
        bus.map_device(
            "esp32s3.agc-registers",
            0x6001_c000,
            0x0c00,
            Box::new(Esp32S3AgcRegisters::new("esp32s3.agc-registers")),
        )?;
        // The genuine S3 BLE controller programs its exchange-memory mapping
        // table through 0x6003_1204..0x6003_12c8 during
        // `r_emi_em_base_init`. This private page is distinct from the BT
        // baseband page at 0x6001_1000 and the Wi-Fi MAC at 0x6003_3000.
        // Keep ordinary read/modify/write state here; individual strobes are
        // promoted to modeled behavior as the controller reaches them.
        let ble_exchange_memory_device =
            Esp32S3BleExchangeMemoryRegisters::new("esp32s3.ble-exchange-memory-registers");
        let ble_exchange_memory = ble_exchange_memory_device.handle();
        bus.map_device(
            "esp32s3.ble-exchange-memory-registers",
            0x6003_1000,
            0x2000,
            Box::new(ble_exchange_memory_device),
        )?;
        let wifi_mac_device = Esp32S3WifiMacRegisters::new("esp32s3.wifi-mac-registers");
        let wifi_mac = wifi_mac_device.handle();
        bus.map_device(
            "esp32s3.wifi-mac-registers",
            0x6003_3000,
            0x3000,
            Box::new(wifi_mac_device),
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
        let (i2c, spi, i2s, rmt) = Self::map_board_serial_peripherals(&mut bus, &signals)?;
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
        let (assist_debug_device, assist_debug) =
            remu_devices::Esp32S3AssistDebug::new("esp32s3.assist-debug");
        bus.map_device(
            "esp32s3.assist-debug",
            0x600c_e000,
            0x1000,
            Box::new(assist_debug_device),
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
        let (ble_lp_clock_device, ble_lp_clock) = Esp32S3BleLpClock::new("esp32s3.ble-lp-clock");
        bus.map_device(
            "esp32s3.ble-lp-clock",
            0x6004_2000,
            0x1000,
            Box::new(ble_lp_clock_device),
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
            "esp32s3.spi1",
            0x6000_2000,
            0x1000,
            Box::new(EspSpiMem::new_with_jedec_id("esp32s3.spi1", 0x0018_40c8)),
        )?;
        bus.map_device(
            "esp32s3.spi0",
            0x6000_3000,
            0x1000,
            Box::new(EspSpiMem::new_with_jedec_id("esp32s3.spi0", 0x0018_40c8)),
        )?;
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
        // Mask-ROM-owned SRAM is outside the heap and starts zeroed on a real
        // second-stage handoff. In particular, Wi-Fi installs g_osi_funcs_p
        // here before taking its first API lock.
        bus.load(0x3fce_9710, &vec![0; 0x68f0])
            .map_err(|error| XtensaMachineError::Load {
                address: 0x3fce_9710,
                message: error.to_string(),
            })?;
        // Direct handoff keeps the ROM-initialized pointer functional.
        // a zeroed functional legacy-flash data table in unused DRAM.
        bus.write(0x3fce_ffe4, AccessWidth::Word, 0x3fce_0000, SimTime::ZERO)
            .map_err(|error| XtensaMachineError::Load {
                address: 0x3fce_ffe4,
                message: error.to_string(),
            })?;
        // esp_rom_spiflash_legacy_data points at the mask-ROM flash
        // descriptor normally populated by the second-stage bootloader.
        for (offset, value) in [
            (0_u64, 0x0018_40c8_u64),
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
            i2c,
            spi,
            i2s,
            rmt,
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
            assist_debug,
            syscon,
            wifi_mac,
            ble_exchange_memory,
            ble_lp_clock,
            rtc_control,
            rtc_i2c,
            rtc_io,
            sdm,
            mmu_table,
            radio_medium: RadioMedium::new(MediumProfile::default())?,
            radio_coexistence: CoexistenceArbiter::new(),
            radio_wifi: WifiEngine::new(MacAddress([0x02, 0, 0, 0, 0x53, 3])),
            radio_ble: BleController::new(BdAddress([3, 0x53, 0, 0, 0, 0x02]), 0x3253_5eed),
            radio_legality: RadioLegalityValidator::new(RadioChip::Esp32S3),
            radio_reset_generation: 0,
            radio_coexistence_transmission: None,
            radio_event_cursor: 0,
            pending_native_wifi: Vec::new(),
            pending_native_ble_transmissions: VecDeque::new(),
            pending_native_ble_receptions: VecDeque::new(),
            pending_native_ble_slot_completions: VecDeque::new(),
            native_ble_link_sequences: BTreeMap::new(),
            now: SimTime::ZERO,
            // ESP32-S3 reserves an 8-KiB ROM startup stack ending here. CPU0
            // uses the upper half and the APP CPU starts one 4-KiB half below;
            // the generic DRAM end lies in SRAM2 that may belong to D-cache.
            stack: 0x3fce_b710,
            instruction_cache_configured: false,
            boot_rom_loaded: false,
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
}
include!("xtensa/machine_runtime.rs");

#[cfg(test)]
mod aux_tests;
#[cfg(test)]
mod tests;
