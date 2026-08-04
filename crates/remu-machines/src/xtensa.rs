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
    DeterministicRng, Esp32s3I2c, Esp32s3I2s, Esp32s3Rmt, Esp32s3Spi, EspGpio, EspMmuTable,
    EspMmuTableHandle, EspRtcControl, EspSpiMem, EspSystem, EspSystemHandle, EspSystimer,
    EspSystimerHandle, EspTimerGroup, EspTimerGroupHandle, EspTimerGroupKind, EspUsbOtg,
    EspUsbOtgHandle, EspUsbSerialJtag, EspUsbSerialJtagHandle, ExitDevice, ExitHandle,
    FunctionalGpio, FunctionalTimer, FunctionalUart, GpioHandle, Rp2040RegisterBank, SignalHub,
    TimerHandle, UartHandle,
};
use remu_image::{EspFlashImage, FirmwareArchitecture, FirmwareImage};
use remu_signals::{Logic, SignalError};
use remu_trace::{TraceDigest, TraceError, TraceSink};
use sha2::{Sha224, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use thiserror::Error;

mod functional_rom;

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

#[derive(Clone, Copy)]
enum Dwc2ControlResponse {
    DeviceDescriptor,
    ConfigurationDescriptor,
    None,
}

fn appcpu_systimer_level(pending: bool, usb_input_started: bool, safe_point: bool) -> bool {
    pending && (!usb_input_started || safe_point)
}

struct Dwc2ControlRequest {
    setup: [u8; 8],
    response: Dwc2ControlResponse,
}

struct Dwc2ControlTransfer {
    request: Dwc2ControlRequest,
    response: Vec<u8>,
    data_complete: bool,
}

#[derive(Clone, Debug, Default)]
struct FunctionalSha256 {
    sha224: bool,
    input: Vec<u8>,
}

struct EspDwc2Host {
    reset_sent: bool,
    next_setup_at: u64,
    requests: VecDeque<Dwc2ControlRequest>,
    active: Option<Dwc2ControlTransfer>,
    bulk_in: Option<u8>,
    bulk_out: Option<u8>,
    input: VecDeque<u8>,
    input_queued: bool,
    output: Vec<u8>,
    input_started: bool,
    sending_raw_chunk: bool,
    raw_prompt_ready: bool,
}

impl EspDwc2Host {
    fn new() -> Self {
        Self {
            reset_sent: false,
            next_setup_at: 0,
            requests: VecDeque::from([
                Dwc2ControlRequest {
                    setup: [0x80, 6, 0, 1, 0, 0, 18, 0],
                    response: Dwc2ControlResponse::DeviceDescriptor,
                },
                Dwc2ControlRequest {
                    setup: [0x00, 5, 1, 0, 0, 0, 0, 0],
                    response: Dwc2ControlResponse::None,
                },
                Dwc2ControlRequest {
                    setup: [0x80, 6, 0, 2, 0, 0, 255, 0],
                    response: Dwc2ControlResponse::ConfigurationDescriptor,
                },
                Dwc2ControlRequest {
                    setup: [0x00, 9, 1, 0, 0, 0, 0, 0],
                    response: Dwc2ControlResponse::None,
                },
                Dwc2ControlRequest {
                    setup: [0x21, 0x22, 3, 0, 0, 0, 0, 0],
                    response: Dwc2ControlResponse::None,
                },
            ]),
            active: None,
            bulk_in: None,
            bulk_out: None,
            input: VecDeque::new(),
            input_queued: false,
            output: Vec::new(),
            input_started: false,
            sending_raw_chunk: false,
            raw_prompt_ready: false,
        }
    }

    fn queue_input(&mut self, bytes: &[u8]) {
        self.input.extend(bytes.iter().copied());
        self.input_queued |= !bytes.is_empty();
        self.sending_raw_chunk |= !bytes.is_empty();
    }

    fn output(&self) -> Vec<u8> {
        self.output.clone()
    }

    fn input_complete(&self) -> bool {
        self.input_queued
            && self
                .output
                .windows(HOST_SCRIPT_COMPLETE_MARKER.len())
                .any(|window| window == HOST_SCRIPT_COMPLETE_MARKER.as_bytes())
            && self.output.ends_with(b"\x04\x04>")
    }

    fn input_started(&self) -> bool {
        self.input_started
    }

    fn can_poll(&self) -> bool {
        self.sending_raw_chunk || self.raw_prompt_ready
    }

    fn discover_bulk_endpoints(&mut self, descriptor: &[u8]) {
        let mut offset = 0;
        while offset + 2 <= descriptor.len() {
            let length = usize::from(descriptor[offset]);
            if length < 2 || offset + length > descriptor.len() {
                break;
            }
            if descriptor[offset + 1] == 5 && length >= 7 && descriptor[offset + 3] & 3 == 2 {
                let address = descriptor[offset + 2];
                if address & 0x80 != 0 {
                    self.bulk_in = Some(address & 0x0f);
                } else {
                    self.bulk_out = Some(address & 0x0f);
                }
            }
            offset += length;
        }
    }

    fn finish_control(&mut self, now: SimTime) {
        let transfer = self.active.take().expect("active DWC2 control transfer");
        if std::env::var_os("REMU_DEBUG_USB").is_some() {
            eprintln!(
                "dwc2 control done setup={:02x?} response={} at={}",
                transfer.request.setup,
                transfer.response.len(),
                now.ticks()
            );
        }
        if matches!(
            transfer.request.response,
            Dwc2ControlResponse::ConfigurationDescriptor
        ) {
            self.discover_bulk_endpoints(&transfer.response);
        }
        self.next_setup_at = now.ticks().saturating_add(256);
    }

    fn poll_control(&mut self, now: SimTime, usb: &EspUsbOtgHandle) -> u64 {
        if std::env::var_os("REMU_DEBUG_USB").is_some() && now.ticks().is_multiple_of(100_000) {
            let (ahb, status, mask, daint, daint_mask) = usb.interrupt_diagnostic();
            let (ictl, iint, itsiz, octl, oint, otsiz, empty) = usb.endpoint_diagnostic(0);
            eprintln!(
                "dwc2 active at={} ahb={ahb:#x} status={status:#x} mask={mask:#x} daint={daint:#x} daint_mask={daint_mask:#x} ep0={ictl:#x}/{iint:#x}/{itsiz:#x} {octl:#x}/{oint:#x}/{otsiz:#x} empty={empty:#x}",
                now.ticks()
            );
        }
        let Some(transfer) = &mut self.active else {
            return 0;
        };
        if transfer.request.setup[0] & 0x80 != 0 {
            if !transfer.data_complete {
                if let Some(packet) = usb.take_input(0) {
                    if std::env::var_os("REMU_DEBUG_USB").is_some() {
                        eprintln!("dwc2 ep0 IN {} bytes at={}", packet.len(), now.ticks());
                    }
                    transfer.response.extend_from_slice(&packet);
                    let requested = usize::from(u16::from_le_bytes([
                        transfer.request.setup[6],
                        transfer.request.setup[7],
                    ]));
                    transfer.data_complete =
                        packet.len() < 64 || transfer.response.len() >= requested;
                    return 1;
                }
            } else if usb.output_ready(0) && !usb.interrupt_pending() {
                if std::env::var_os("REMU_DEBUG_USB").is_some() {
                    eprintln!("dwc2 ep0 status OUT at={}", now.ticks());
                }
                usb.inject_output(0, &[]);
                self.finish_control(now);
                return 1;
            }
        } else if let Some(packet) = usb.take_input(0) {
            if std::env::var_os("REMU_DEBUG_USB").is_some() {
                eprintln!(
                    "dwc2 ep0 status IN {} bytes at={}",
                    packet.len(),
                    now.ticks()
                );
            }
            if packet.is_empty() {
                self.finish_control(now);
            }
            return 1;
        }
        0
    }

    fn poll(&mut self, now: SimTime, usb: &EspUsbOtgHandle) -> u64 {
        if !self.reset_sent {
            if usb.device_connected() {
                if std::env::var_os("REMU_DEBUG_USB").is_some() {
                    eprintln!("dwc2 bus reset at={}", now.ticks());
                }
                usb.inject_bus_reset();
                self.reset_sent = true;
                self.next_setup_at = now.ticks().saturating_add(1024);
                return 1;
            }
            return 0;
        }
        if self.active.is_some() {
            return self.poll_control(now, usb);
        }
        if std::env::var_os("REMU_DEBUG_USB").is_some()
            && now.ticks().is_multiple_of(100_000)
            && usb.interrupt_pending()
        {
            let (ahb, status, mask, daint, daint_mask) = usb.interrupt_diagnostic();
            let (ictl, iint, itsiz, octl, oint, otsiz, empty) = usb.endpoint_diagnostic(2);
            eprintln!(
                "dwc2 pending at={} ahb={ahb:#x} status={status:#x} mask={mask:#x} daint={daint:#x} daint_mask={daint_mask:#x} ep2={ictl:#x}/{iint:#x}/{itsiz:#x} {octl:#x}/{oint:#x}/{otsiz:#x} empty={empty:#x}",
                now.ticks()
            );
        }
        if now.ticks() >= self.next_setup_at
            && !usb.interrupt_pending()
            && let Some(request) = self.requests.pop_front()
        {
            if std::env::var_os("REMU_DEBUG_USB").is_some() {
                eprintln!("dwc2 setup {:02x?} at={}", request.setup, now.ticks());
            }
            usb.inject_setup(request.setup);
            self.active = Some(Dwc2ControlTransfer {
                request,
                response: Vec::new(),
                data_complete: false,
            });
            return 1;
        }

        let mut events = 0;
        for endpoint in 1..7_u8 {
            if let Some(packet) = usb.take_input(endpoint) {
                if self.bulk_in == Some(endpoint) {
                    self.output.extend_from_slice(&packet);
                    if self.output.ends_with(b"\x04\x04>")
                        || self.output.ends_with(b"raw REPL; CTRL-B to exit\r\n>")
                    {
                        self.raw_prompt_ready = true;
                    }
                }
                events += 1;
            }
        }
        if !self.sending_raw_chunk && self.raw_prompt_ready && !self.input.is_empty() {
            self.sending_raw_chunk = true;
            self.raw_prompt_ready = false;
        }
        if let Some(endpoint) = self.bulk_out
            && !self.input.is_empty()
            && self.sending_raw_chunk
            && usb.output_ready(endpoint)
            && !usb.interrupt_pending()
        {
            let mut length = self.input.len().min(64).min(usb.output_capacity(endpoint));
            if let Some(end) = self
                .input
                .iter()
                .take(length)
                .position(|byte| *byte == 0x04)
            {
                length = end + 1;
            }
            if length == 0 {
                return events;
            }
            let packet = self.input.drain(..length).collect::<Vec<_>>();
            usb.inject_output(endpoint, &packet);
            self.input_started = true;
            if packet.contains(&0x04) {
                self.sending_raw_chunk = false;
            }
            events += 1;
        }
        events
    }
}

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
    usb_host: EspDwc2Host,
    saradc: Esp32S3SarAdcHandle,
    system: EspSystemHandle,
    systimer: EspSystimerHandle,
    timer_groups: Vec<EspTimerGroupHandle>,
    ledc: Esp32S3LedcHandle,
    mmu_table: EspMmuTableHandle,
    now: SimTime,
    stack: u32,
    instruction_cache_configured: bool,
    windowed_handoff_pending: bool,
    appcpu_boot_address: Option<u32>,
    interrupt_routes: [[u8; 128]; 2],
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
        bus.map_ram("rtc-slow-memory", 0x5000_0000, 0x2000, true)?;
        let signals = SignalHub::new();
        for (name, base) in [
            ("radio-fe2", 0x6000_5000),
            ("radio-fe", 0x6000_6000),
            ("io-mux", 0x6000_9000),
            ("hinf", 0x6000_b000),
            ("uhci1", 0x6000_c000),
            ("bluetooth", 0x6001_1000),
            ("uhci0", 0x6001_4000),
            ("slchost", 0x6001_5000),
            ("pcnt", 0x6001_7000),
            ("slc", 0x6001_8000),
            ("radio-nrx", 0x6001_c000),
            ("radio-bb", 0x6001_d000),
            ("rtc-slowmem-controller", 0x6002_1000),
            ("syscon", 0x6002_6000),
            ("sdmmc", 0x6002_8000),
            ("peripheral-backup", 0x6002_a000),
            ("pwm1", 0x6002_c000),
            ("usb-wrap", 0x6003_9000),
            ("aes", 0x6003_a000),
            ("rsa", 0x6003_c000),
            ("digital-signature", 0x6003_d000),
            ("hmac", 0x6003_e000),
            ("saradc", 0x6004_0000),
            ("sensitive", 0x600c_1000),
            ("interrupt-matrix", 0x600c_2000),
            ("assist-debug", 0x600c_e000),
            ("world-controller", 0x600d_0000),
        ] {
            bus.map_device(
                format!("esp32s3.{name}"),
                base,
                0x1000,
                Box::new(Rp2040RegisterBank::new(
                    format!("esp32s3.{name}"),
                    vec![0; 0x1000 / 4],
                )),
            )?;
        }
        for (name, base) in [("i2c0", 0x6001_3000), ("i2c1", 0x6002_7000)] {
            let device = Esp32s3I2c::new(format!("esp32s3.{name}"), signals.clone())?;
            bus.map_device(format!("esp32s3.{name}"), base, 0x1000, Box::new(device))?;
        }
        let rmt_device = Esp32s3Rmt::new("esp32s3.rmt", signals.clone())?;
        bus.map_device("esp32s3.rmt", 0x6001_6000, 0x1000, Box::new(rmt_device))?;
        bus.map_device(
            "esp32s3.efuse",
            0x6000_7000,
            0x1000,
            Box::new(EspEfuse::new("esp32s3.efuse")),
        )?;
        bus.map_device(
            "esp32s3.rng",
            0x6003_5000,
            0x1000,
            Box::new(DeterministicRng::new("esp32s3.rng", 0x7c, 0x32f3_0001)),
        )?;
        let mut analog_i2c_registers = vec![0; 0x1000 / 4];
        // I2C_MST_ANA_CONF0.BBPLL_CAL_DONE. The functional clock tree locks
        // immediately and retains the completion status through RMW setup.
        analog_i2c_registers[0x40 / 4] = 1 << 24;
        bus.map_device(
            "esp32s3.analog-i2c",
            0x6000_e000,
            0x1000,
            Box::new(Rp2040RegisterBank::new(
                "esp32s3.analog-i2c",
                analog_i2c_registers,
            )),
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
        bus.map_device(
            "esp32s3.rtc-control",
            0x6000_8000,
            0x800,
            Box::new(EspRtcControl::new("esp32s3.rtc-control")),
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
        let (mmu_table_device, mmu_table) = EspMmuTable::new("esp32s3.mmu-table");
        bus.map_device(
            "esp32s3.mmu-table",
            0x600c_5000,
            0x1000,
            Box::new(mmu_table_device),
        )?;
        let cache_registers = vec![0; 0x1000 / 4];
        // CACHE_STATE reports both I-cache and D-cache idle/enabled state.
        // A verified image starts before the application cache-mode call.
        bus.map_device(
            "esp32s3.cache",
            0x600c_4000,
            0x1000,
            Box::new(Rp2040RegisterBank::new("esp32s3.cache", cache_registers)),
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
            0x1000,
            Box::new(chip_gpio_device),
        )?;
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
            usb_host: EspDwc2Host::new(),
            saradc,
            system,
            systimer,
            timer_groups,
            ledc,
            mmu_table,
            now: SimTime::ZERO,
            stack: stack.expect("ESP32-S3 manifest includes DRAM"),
            instruction_cache_configured: false,
            windowed_handoff_pending: false,
            appcpu_boot_address: None,
            interrupt_routes: [[u8::MAX; 128]; 2],
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

    fn set_systimer_interrupt(
        &mut self,
        core: u32,
        interrupt: u32,
        pending: bool,
    ) -> Result<(), XtensaMachineError> {
        if core == 0 {
            self.cpu.set_interrupt(interrupt as u16, pending)?;
        } else if core == 1 && self.appcpu_boot_address.is_some() {
            // Once an external script is running, retain a pending CPU1 tick
            // until the application core reaches WAITI or another shallow
            // logical-window safe point. This gives the functional model a
            // deterministic preemption boundary while still waking delayed
            // and newly runnable tasks.
            let asserted = appcpu_systimer_level(
                pending,
                self.usb_host.input_started(),
                self.cpu1.functional_interrupt_safe_point(),
            );
            self.cpu1.set_interrupt(interrupt as u16, asserted)?;
        }
        Ok(())
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
                stats.events = stats
                    .events
                    .saturating_add(self.usb_host.poll(self.now, &self.usb_otg));
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
                // IDF maps every unused source to CPU interrupt 6, the
                // architecture's disabled/reserved matrix sink.
                let interrupt = self.interrupt_routes[core as usize][38];
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
                let interrupt = self.interrupt_routes[core as usize][48];
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
            for core in 0..2_u32 {
                let crosscore_pending = self.system.from_cpu_pending(core as usize);
                let newly_pending = crosscore_pending && !crosscore_was_pending[core as usize];
                if newly_pending {
                    stats.events = stats.events.saturating_add(1);
                }
                crosscore_was_pending[core as usize] = crosscore_pending;
                let source = 79 + core;
                let interrupt = self.interrupt_routes[core as usize][source as usize];
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
                let interrupt = self
                    .interrupt_routes
                    .get(core as usize)
                    .map_or(u8::MAX, |routes| routes[source as usize]);
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
                        let interrupt = self.interrupt_routes[core as usize][source as usize];
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
            let outcome = match self.cpu.step(&mut self.bus, self.now) {
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
mod tests;
