use crate::riscv::{TEST_DEVICE_SIZE, TEST_EXIT_SIZE};
use crate::{
    MemoryKind, PinStimulus, RunResult, TEST_EXIT, TEST_GPIO, TEST_TIMER, TEST_UART, TargetId,
    target_manifest,
};
use md5::{Digest, Md5};
use renvo_bus::{AddressSpace, Endianness, MapError, Permissions, SharedMemory};
use renvo_core::{
    AccessKind, AccessWidth, Bus, Cpu, CpuFault, RunLimits, RunStats, SimTime, StepReason,
    StopReason,
};
use renvo_cpu_xtensa::{XtensaCpu, XtensaRegister};
use renvo_devices::{
    DeterministicRng, EspGpio, EspMmuTable, EspMmuTableHandle, EspRtcControl, EspSpiMem, EspSystem,
    EspSystemHandle, EspSystimer, EspSystimerHandle, EspTimerGroup, EspTimerGroupHandle,
    EspTimerGroupKind, EspUsbOtg, EspUsbOtgHandle, EspUsbSerialJtag, EspUsbSerialJtagHandle,
    ExitDevice, ExitHandle, FunctionalGpio, FunctionalTimer, FunctionalUart, GpioHandle,
    Rp2040RegisterBank, SignalHub, TimerHandle, UartHandle,
};
use renvo_image::{EspFlashImage, FirmwareArchitecture, FirmwareImage};
use renvo_signals::{Logic, SignalError};
use renvo_trace::{TraceDigest, TraceError, TraceSink};
use sha2::{Sha224, Sha256};
use std::collections::{BTreeMap, VecDeque};
use thiserror::Error;

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
    Device(#[from] renvo_bus::DeviceError),
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
    raw_chunks_queued: usize,
    raw_chunks_completed: usize,
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
            raw_chunks_queued: 0,
            raw_chunks_completed: 0,
            output: Vec::new(),
            input_started: false,
            sending_raw_chunk: false,
            raw_prompt_ready: false,
        }
    }

    fn queue_input(&mut self, bytes: &[u8]) {
        self.input.extend(bytes.iter().copied());
        self.input_queued |= !bytes.is_empty();
        self.raw_chunks_queued = self
            .raw_chunks_queued
            .saturating_add(bytes.iter().filter(|byte| **byte == 0x04).count());
        self.sending_raw_chunk |= !bytes.is_empty();
    }

    fn output(&self) -> Vec<u8> {
        self.output.clone()
    }

    fn input_complete(&self) -> bool {
        self.input_queued
            && self.raw_chunks_queued != 0
            && self.raw_chunks_completed >= self.raw_chunks_queued
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
        if std::env::var_os("RENVO_DEBUG_USB").is_some() {
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
        if std::env::var_os("RENVO_DEBUG_USB").is_some() && now.ticks().is_multiple_of(100_000) {
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
                    if std::env::var_os("RENVO_DEBUG_USB").is_some() {
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
                if std::env::var_os("RENVO_DEBUG_USB").is_some() {
                    eprintln!("dwc2 ep0 status OUT at={}", now.ticks());
                }
                usb.inject_output(0, &[]);
                self.finish_control(now);
                return 1;
            }
        } else if let Some(packet) = usb.take_input(0) {
            if std::env::var_os("RENVO_DEBUG_USB").is_some() {
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
                if std::env::var_os("RENVO_DEBUG_USB").is_some() {
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
        if std::env::var_os("RENVO_DEBUG_USB").is_some()
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
            if std::env::var_os("RENVO_DEBUG_USB").is_some() {
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
                    self.raw_chunks_completed = self
                        .output
                        .windows(3)
                        .filter(|window| *window == b"\x04\x04>")
                        .count();
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
    uart: UartHandle,
    chip_uart: UartHandle,
    timer: TimerHandle,
    exit: ExitHandle,
    usb_serial_jtag: EspUsbSerialJtagHandle,
    usb_otg: EspUsbOtgHandle,
    usb_host: EspDwc2Host,
    system: EspSystemHandle,
    systimer: EspSystimerHandle,
    timer_groups: Vec<EspTimerGroupHandle>,
    mmu_table: EspMmuTableHandle,
    now: SimTime,
    stack: u32,
    appcpu_boot_address: Option<u32>,
    interrupt_routes: BTreeMap<(u32, u32), u32>,
    md5_contexts: BTreeMap<u32, Vec<u8>>,
    sha256_contexts: BTreeMap<u32, FunctionalSha256>,
    setjmp_contexts: BTreeMap<u32, XtensaCpu>,
    flash: Vec<u8>,
    stop_on_usb_input_complete: bool,
}

impl XtensaMachine {
    /// Creates the ESP32-S3 direct-mode map.
    pub fn new(target: TargetId) -> Result<Self, XtensaMachineError> {
        if target != TargetId::Esp32s3 {
            return Err(XtensaMachineError::UnsupportedTarget(target));
        }
        let manifest = target_manifest(target);
        let mut bus = AddressSpace::new(Endianness::Little);
        let mut stack = None;
        for region in manifest.memory {
            match region.kind {
                MemoryKind::Ram => {
                    bus.map_ram(region.name, region.start, region.size, region.executable)?;
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
        rom_service_data[..20].copy_from_slice(b"renvo-coex-rom-v0.0\0");
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
        for (name, base) in [
            ("radio-fe2", 0x6000_5000),
            ("radio-fe", 0x6000_6000),
            ("efuse", 0x6000_7000),
            ("io-mux", 0x6000_9000),
            ("hinf", 0x6000_b000),
            ("uhci1", 0x6000_c000),
            ("i2s0", 0x6000_f000),
            ("uart1", 0x6001_0000),
            ("bluetooth", 0x6001_1000),
            ("i2c0", 0x6001_3000),
            ("uhci0", 0x6001_4000),
            ("slchost", 0x6001_5000),
            ("rmt", 0x6001_6000),
            ("pcnt", 0x6001_7000),
            ("slc", 0x6001_8000),
            ("ledc", 0x6001_9000),
            ("radio-nrx", 0x6001_c000),
            ("radio-bb", 0x6001_d000),
            ("pwm0", 0x6001_e000),
            ("rtc-slowmem-controller", 0x6002_1000),
            ("spi2", 0x6002_4000),
            ("spi3", 0x6002_5000),
            ("syscon", 0x6002_6000),
            ("i2c1", 0x6002_7000),
            ("sdmmc", 0x6002_8000),
            ("peripheral-backup", 0x6002_a000),
            ("twai", 0x6002_b000),
            ("pwm1", 0x6002_c000),
            ("i2s1", 0x6002_d000),
            ("uart2", 0x6002_e000),
            ("usb-wrap", 0x6003_9000),
            ("aes", 0x6003_a000),
            ("sha", 0x6003_b000),
            ("rsa", 0x6003_c000),
            ("digital-signature", 0x6003_d000),
            ("hmac", 0x6003_e000),
            ("gdma", 0x6003_f000),
            ("saradc", 0x6004_0000),
            ("lcd-cam", 0x6004_1000),
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
        bus.map_device(
            "esp32s3.rtc-control",
            0x6000_8000,
            0x1000,
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
        let mut cache_registers = vec![0; 0x1000 / 4];
        // CACHE_STATE reports both I-cache and D-cache idle/enabled state.
        // The direct verified-image handoff starts with coherent caches.
        cache_registers[0x130 / 4] = 0x0000_1001;
        bus.map_device(
            "esp32s3.cache",
            0x600c_4000,
            0x1000,
            Box::new(Rp2040RegisterBank::new("esp32s3.cache", cache_registers)),
        )?;
        let signals = SignalHub::new();
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
        let (chip_gpio_device, chip_gpio) = EspGpio::new(
            "esp32s3.gpio",
            32,
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
            timer,
            exit,
            usb_serial_jtag,
            usb_otg,
            usb_host: EspDwc2Host::new(),
            system,
            systimer,
            timer_groups,
            mmu_table,
            now: SimTime::ZERO,
            stack: stack.expect("ESP32-S3 manifest includes DRAM"),
            appcpu_boot_address: None,
            interrupt_routes: BTreeMap::new(),
            md5_contexts: BTreeMap::new(),
            sha256_contexts: BTreeMap::new(),
            setjmp_contexts: BTreeMap::new(),
            flash: Vec::new(),
            stop_on_usb_input_complete: false,
        })
    }

    /// Loads an Xtensa ELF and establishes CPU0 direct state.
    pub fn load_firmware(&mut self, image: &FirmwareImage) -> Result<(), XtensaMachineError> {
        if image.architecture != FirmwareArchitecture::Xtensa {
            return Err(XtensaMachineError::Architecture(image.architecture));
        }
        for segment in &image.segments {
            self.bus
                .load(segment.address, &segment.data)
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
        for segment in &image.application.segments {
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
            .set_direct_state(self.stack, image.application.header.entry);
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

    /// Returns completed bus operations when recording is enabled.
    pub fn access_log(&self) -> &[renvo_bus::BusAccessRecord] {
        self.bus.access_log()
    }

    /// Queues deterministic CDC-ACM input for the native USB console.
    pub fn queue_usb_input(&mut self, bytes: &[u8]) {
        self.usb_serial_jtag.queue_input(bytes);
        self.usb_host.queue_input(bytes);
    }

    /// Stops a bounded run once all queued USB input returns to the raw-REPL prompt.
    pub fn stop_on_usb_input_complete(&mut self, enabled: bool) {
        self.stop_on_usb_input_complete = enabled;
    }

    fn complete_functional_rom_call(&mut self, result: u32) -> Result<(), String> {
        self.cpu
            .complete_functional_call(result)
            .map_err(|error| error.to_string())
    }

    fn complete_functional_rom_call_u64(&mut self, result: u64) -> Result<(), String> {
        self.cpu
            .set_register(XtensaRegister::A3, (result >> 32) as u32);
        self.complete_functional_rom_call(result as u32)
    }

    fn read_guest_bytes(&mut self, address: u32, length: usize) -> Result<Vec<u8>, String> {
        let mut bytes = Vec::with_capacity(length);
        for offset in 0..length {
            bytes.push(
                self.bus
                    .read(
                        u64::from(address.wrapping_add(offset as u32)),
                        AccessWidth::Byte,
                        AccessKind::Read,
                        self.now,
                    )
                    .map_err(|error| error.to_string())? as u8,
            );
        }
        Ok(bytes)
    }

    fn read_guest_c_string(&mut self, address: u32, limit: usize) -> Result<Vec<u8>, String> {
        let mut bytes = Vec::new();
        for offset in 0..limit {
            let byte = self
                .bus
                .read(
                    u64::from(address.wrapping_add(offset as u32)),
                    AccessWidth::Byte,
                    AccessKind::Read,
                    self.now,
                )
                .map_err(|error| error.to_string())? as u8;
            if byte == 0 {
                break;
            }
            bytes.push(byte);
        }
        Ok(bytes)
    }

    fn functional_rom_printf(&mut self) -> Result<u32, String> {
        let format_address = self.cpu.register(XtensaRegister::A2);
        let format = self.read_guest_c_string(format_address, 4096)?;
        let mut arguments = Vec::with_capacity(5);
        for register in [
            XtensaRegister::A3,
            XtensaRegister::A4,
            XtensaRegister::A5,
            XtensaRegister::A6,
            XtensaRegister::A7,
        ] {
            arguments.push(self.cpu.register(register));
        }
        let mut next_argument = 0;
        let mut output = Vec::new();
        let mut index = 0;
        while index < format.len() {
            if format[index] != b'%' {
                output.push(format[index]);
                index += 1;
                continue;
            }
            index += 1;
            if format.get(index) == Some(&b'%') {
                output.push(b'%');
                index += 1;
                continue;
            }
            let mut zero_pad = false;
            if format.get(index) == Some(&b'0') {
                zero_pad = true;
                index += 1;
            }
            let mut width = 0_usize;
            while let Some(byte @ b'0'..=b'9') = format.get(index).copied() {
                width = width
                    .saturating_mul(10)
                    .saturating_add(usize::from(byte - b'0'));
                index += 1;
            }
            while matches!(format.get(index), Some(b'l' | b'h' | b'z')) {
                index += 1;
            }
            let conversion = format.get(index).copied().unwrap_or_default();
            index += usize::from(index < format.len());
            let argument = arguments.get(next_argument).copied().unwrap_or_default();
            next_argument += 1;
            let rendered = match conversion {
                b's' => self.read_guest_c_string(argument, 4096)?,
                b'c' => vec![argument as u8],
                b'd' | b'i' => (argument as i32).to_string().into_bytes(),
                b'u' => argument.to_string().into_bytes(),
                b'x' => format!("{argument:x}").into_bytes(),
                b'X' => format!("{argument:X}").into_bytes(),
                b'p' => format!("0x{argument:08x}").into_bytes(),
                unknown => vec![b'%', unknown],
            };
            if width > rendered.len() {
                output.extend(std::iter::repeat_n(
                    if zero_pad { b'0' } else { b' ' },
                    width - rendered.len(),
                ));
            }
            output.extend(rendered);
        }
        self.chip_uart.transmit(&output);
        Ok(output.len() as u32)
    }

    fn write_guest_bytes(&mut self, address: u32, bytes: &[u8]) -> Result<(), String> {
        for (offset, byte) in bytes.iter().copied().enumerate() {
            self.bus
                .write(
                    u64::from(address.wrapping_add(offset as u32)),
                    AccessWidth::Byte,
                    u64::from(byte),
                    self.now,
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn service_functional_rom(&mut self) -> Result<bool, String> {
        let pc = self.cpu.pc();
        // The verified-image handoff has the same externally visible flash
        // state as a completed second-stage bootloader. IDF's early probe has
        // already initialized the static host/chip-driver fields; publish the
        // default chip before FreeRTOS launches application tasks.
        if pc == 0x4213_8764 {
            self.write_guest_bytes(0x3fca_6850, &0x3fca_1dfc_u32.to_le_bytes())?;
            self.write_guest_bytes(0x3fca_1dfc + 20, &(16_u32 * 1024 * 1024).to_le_bytes())?;
            self.write_guest_bytes(0x3fca_1dfc + 24, &0x0016_40c8_u32.to_le_bytes())?;
            return Ok(false);
        }
        // CPU1 has accepted the nonblocking IPC request and entered
        // spi_flash_op_block_func, which publishes s_flash_op_can_start.
        if pc == 0x4037_f1d8 && self.appcpu_boot_address.is_some() {
            self.write_guest_bytes(0x3fca_6847, &[1])?;
            return Ok(false);
        }
        // Execute CPU1 IPC requests synchronously in the functional dual-core
        // model. The cache-block callback's externally visible action is to
        // acknowledge that CPU1 has disabled its scheduler and caches.
        if pc == 0x4200_8718
            && self.cpu.register(XtensaRegister::A2) == 1
            && self.appcpu_boot_address.is_some()
        {
            let callback = self.cpu.register(XtensaRegister::A3);
            if callback == 0x4037_f154 {
                self.write_guest_bytes(0x3fca_6847, &[1])?;
            }
            self.complete_functional_rom_call(0)?;
            return Ok(true);
        }
        // Surface IDF abort diagnostics as stable emulator faults instead of
        // executing the noreturn panic tail into its deliberate trap.
        if pc == 0x4038_5d7c {
            let details = self.cpu.register(XtensaRegister::A2);
            let message = self
                .read_guest_c_string(details, 16 * 1024)
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                .unwrap_or_else(|_| format!("details at {details:#010x}"));
            return Err(format!("ESP-IDF panic: {message}"));
        }
        match pc {
            // ESP-IDF flash API. The functional controller operates on the
            // exact merged image installed by the loader and preserves NOR
            // program/erase behavior for partitions and filesystems.
            0x4038_c624 => {
                let size_out = self.cpu.register(XtensaRegister::A3);
                self.write_guest_bytes(size_out, &(self.flash.len() as u32).to_le_bytes())?;
                self.complete_functional_rom_call(0)?;
            }
            0x4038_c824 => {
                let destination = self.cpu.register(XtensaRegister::A3);
                let offset = self.cpu.register(XtensaRegister::A4) as usize;
                let length = self.cpu.register(XtensaRegister::A5) as usize;
                let end = offset
                    .checked_add(length)
                    .filter(|end| *end <= self.flash.len())
                    .ok_or_else(|| {
                        format!(
                            "ESP flash read {offset:#x}+{length:#x} exceeds {:#x}",
                            self.flash.len()
                        )
                    })?;
                let bytes = self.flash[offset..end].to_vec();
                self.write_guest_bytes(destination, &bytes)?;
                self.complete_functional_rom_call(0)?;
            }
            0x4038_c9d4 => {
                let source = self.cpu.register(XtensaRegister::A3);
                let offset = self.cpu.register(XtensaRegister::A4) as usize;
                let length = self.cpu.register(XtensaRegister::A5) as usize;
                let end = offset
                    .checked_add(length)
                    .filter(|end| *end <= self.flash.len())
                    .ok_or_else(|| {
                        format!(
                            "ESP flash write {offset:#x}+{length:#x} exceeds {:#x}",
                            self.flash.len()
                        )
                    })?;
                let bytes = self.read_guest_bytes(source, length)?;
                for (destination, requested) in
                    self.flash[offset..end].iter_mut().zip(bytes.into_iter())
                {
                    *destination &= requested;
                }
                self.complete_functional_rom_call(0)?;
            }
            0x4038_c418 => {
                let offset = self.cpu.register(XtensaRegister::A3) as usize;
                let length = self.cpu.register(XtensaRegister::A4) as usize;
                let end = offset
                    .checked_add(length)
                    .filter(|end| *end <= self.flash.len())
                    .ok_or_else(|| {
                        format!(
                            "ESP flash erase {offset:#x}+{length:#x} exceeds {:#x}",
                            self.flash.len()
                        )
                    })?;
                self.flash[offset..end].fill(0xff);
                self.complete_functional_rom_call(0)?;
            }
            // rtc_get_reset_reason(cpu): deterministic power-on reset.
            0x4000_057c => self.complete_functional_rom_call(1)?,
            // ets_delay_us: virtual time advances at instruction granularity,
            // so the functional delay completes deterministically.
            0x4000_0600 => self.complete_functional_rom_call(0)?,
            // ets_printf, with the integer/string subset used by ROM and IDF
            // startup diagnostics.
            0x4000_05d0 => {
                let written = self.functional_rom_printf()?;
                self.complete_functional_rom_call(written)?;
            }
            // ROM console byte writers used before the IDF UART driver starts.
            0x4000_0648 | 0x4000_0654 | 0x4000_06b4 => {
                let byte = self.cpu.register(XtensaRegister::A2) as u8;
                self.chip_uart.transmit(&[byte]);
                self.complete_functional_rom_call(0)?;
            }
            // Flush/wait/divisor/console-selection calls complete immediately
            // against the host-drained functional UART.
            0x4000_05e8 | 0x4000_0630 | 0x4000_0690 | 0x4000_069c | 0x4000_06a8 | 0x4000_06c0 => {
                self.complete_functional_rom_call(0)?;
            }
            // Watchdog disable.
            0x4000_0714 => self.complete_functional_rom_call(0)?,
            // ets_set_appcpu_boot_addr(entry). The mask ROM releases CPU1 at
            // this address; Renvo then interprets both cores over the shared
            // address space.
            0x4000_0720 => {
                let entry = self.cpu.register(XtensaRegister::A2);
                if std::env::var_os("RENVO_DEBUG_INTERRUPTS").is_some() {
                    eprintln!(
                        "set appcpu boot entry={entry:#010x} stack={:#010x}",
                        self.stack.wrapping_sub(0x1000)
                    );
                }
                if entry != 0 && self.appcpu_boot_address.is_none() {
                    self.appcpu_boot_address = Some(entry);
                    self.cpu1
                        .set_direct_state(self.stack.wrapping_sub(0x1000), entry);
                    self.cpu1.set_processor_id(1);
                }
                self.complete_functional_rom_call(0)?;
            }
            // ROM watchdog HAL. The emulator has no wall-clock watchdog;
            // configuration, feed, and write-protect operations are therefore
            // deterministic no-ops, while `is_enabled` reports disabled.
            0x4000_0dbc..=0x4000_0e34 => self.complete_functional_rom_call(0)?,
            // Initializes ROM newlib lock indirections. Renvo's deterministic
            // single-host-thread execution does not require host mutexes.
            0x4000_11dc => self.complete_functional_rom_call(0)?,
            // memset(destination, byte, length)
            0x4000_11e8 => {
                let destination = self.cpu.register(XtensaRegister::A2);
                let byte = self.cpu.register(XtensaRegister::A3) as u8;
                let length = self.cpu.register(XtensaRegister::A4);
                for index in 0..length {
                    self.bus
                        .write(
                            u64::from(destination.wrapping_add(index)),
                            AccessWidth::Byte,
                            u64::from(byte),
                            self.now,
                        )
                        .map_err(|error| error.to_string())?;
                }
                self.complete_functional_rom_call(destination)?;
            }
            // bzero(destination, length)
            0x4000_1260 => {
                let destination = self.cpu.register(XtensaRegister::A2);
                let length = self.cpu.register(XtensaRegister::A3);
                for index in 0..length {
                    self.bus
                        .write(
                            u64::from(destination.wrapping_add(index)),
                            AccessWidth::Byte,
                            0,
                            self.now,
                        )
                        .map_err(|error| error.to_string())?;
                }
                self.complete_functional_rom_call(0)?;
            }
            0x4000_11f4 | 0x4000_1200 => {
                let destination = self.cpu.register(XtensaRegister::A2);
                let source = self.cpu.register(XtensaRegister::A3);
                let length = self.cpu.register(XtensaRegister::A4) as usize;
                let bytes = self.read_guest_bytes(source, length)?;
                self.write_guest_bytes(destination, &bytes)?;
                self.complete_functional_rom_call(destination)?;
            }
            0x4000_120c => {
                let left = self.cpu.register(XtensaRegister::A2);
                let right = self.cpu.register(XtensaRegister::A3);
                let length = self.cpu.register(XtensaRegister::A4) as usize;
                let left = self.read_guest_bytes(left, length)?;
                let right = self.read_guest_bytes(right, length)?;
                let result = left
                    .iter()
                    .zip(right.iter())
                    .find_map(|(left, right)| {
                        (left != right).then(|| i32::from(*left) - i32::from(*right))
                    })
                    .unwrap_or_default();
                self.complete_functional_rom_call(result as u32)?;
            }
            // strcpy/strncpy
            0x4000_1218 | 0x4000_1224 => {
                let destination = self.cpu.register(XtensaRegister::A2);
                let source = self.cpu.register(XtensaRegister::A3);
                let limit = if pc == 0x4000_1224 {
                    self.cpu.register(XtensaRegister::A4) as usize
                } else {
                    1024 * 1024
                };
                let mut terminated = false;
                for offset in 0..limit {
                    let byte = if terminated {
                        0
                    } else {
                        let byte = self
                            .read_guest_bytes(source.wrapping_add(offset as u32), 1)?
                            .into_iter()
                            .next()
                            .unwrap_or_default();
                        terminated = byte == 0;
                        byte
                    };
                    self.write_guest_bytes(destination.wrapping_add(offset as u32), &[byte])?;
                    if pc == 0x4000_1218 && terminated {
                        break;
                    }
                }
                self.complete_functional_rom_call(destination)?;
            }
            // memchr(buffer, byte, length)
            0x4000_1344 => {
                let source = self.cpu.register(XtensaRegister::A2);
                let byte = self.cpu.register(XtensaRegister::A3) as u8;
                let length = self.cpu.register(XtensaRegister::A4);
                let mut found = 0;
                for offset in 0..length {
                    let candidate = self
                        .read_guest_bytes(source.wrapping_add(offset), 1)?
                        .into_iter()
                        .next()
                        .unwrap_or_default();
                    if candidate == byte {
                        found = source.wrapping_add(offset);
                        break;
                    }
                }
                self.complete_functional_rom_call(found)?;
            }
            // strcmp/strncmp. Return the first unsigned-byte difference, as
            // required by newlib, rather than merely a normalized ordering.
            0x4000_1230 | 0x4000_123c => {
                let left = self.cpu.register(XtensaRegister::A2);
                let right = self.cpu.register(XtensaRegister::A3);
                let limit = if pc == 0x4000_123c {
                    self.cpu.register(XtensaRegister::A4) as usize
                } else {
                    1024 * 1024
                };
                let mut result = 0_i32;
                for index in 0..limit {
                    let left_byte = self
                        .read_guest_bytes(left.wrapping_add(index as u32), 1)?
                        .into_iter()
                        .next()
                        .unwrap_or_default();
                    let right_byte = self
                        .read_guest_bytes(right.wrapping_add(index as u32), 1)?
                        .into_iter()
                        .next()
                        .unwrap_or_default();
                    if left_byte != right_byte {
                        result = i32::from(left_byte) - i32::from(right_byte);
                        break;
                    }
                    if left_byte == 0 {
                        break;
                    }
                }
                self.complete_functional_rom_call(result as u32)?;
            }
            // strcat
            0x4000_1374 => {
                let destination = self.cpu.register(XtensaRegister::A2);
                let source = self.cpu.register(XtensaRegister::A3);
                let destination_length =
                    self.read_guest_c_string(destination, 1024 * 1024)?.len() as u32;
                let mut suffix = self.read_guest_c_string(source, 1024 * 1024)?;
                suffix.push(0);
                self.write_guest_bytes(destination.wrapping_add(destination_length), &suffix)?;
                self.complete_functional_rom_call(destination)?;
            }
            // strchr
            0x4000_138c => {
                let string = self.cpu.register(XtensaRegister::A2);
                let needle = (self.cpu.register(XtensaRegister::A3) & 0xff) as u8;
                let bytes = self.read_guest_c_string(string, 1024 * 1024)?;
                let offset = if needle == 0 {
                    Some(bytes.len())
                } else {
                    bytes.iter().position(|byte| *byte == needle)
                };
                let result = offset.map_or(0, |offset| string.wrapping_add(offset as u32));
                self.complete_functional_rom_call(result)?;
            }
            // strcspn
            0x4000_1398 => {
                let string = self.cpu.register(XtensaRegister::A2);
                let reject = self.cpu.register(XtensaRegister::A3);
                let bytes = self.read_guest_c_string(string, 1024 * 1024)?;
                let rejected = self.read_guest_c_string(reject, 1024 * 1024)?;
                let length = bytes
                    .iter()
                    .position(|byte| rejected.contains(byte))
                    .unwrap_or(bytes.len());
                self.complete_functional_rom_call(length as u32)?;
            }
            // strlcpy
            0x4000_13bc => {
                let destination = self.cpu.register(XtensaRegister::A2);
                let source = self.cpu.register(XtensaRegister::A3);
                let capacity = self.cpu.register(XtensaRegister::A4) as usize;
                let bytes = self.read_guest_c_string(source, 1024 * 1024)?;
                if capacity != 0 {
                    let copied = bytes.len().min(capacity - 1);
                    let mut output = bytes[..copied].to_vec();
                    output.push(0);
                    self.write_guest_bytes(destination, &output)?;
                }
                self.complete_functional_rom_call(bytes.len() as u32)?;
            }
            // strrchr
            0x4000_1404 => {
                let string = self.cpu.register(XtensaRegister::A2);
                let needle = (self.cpu.register(XtensaRegister::A3) & 0xff) as u8;
                let bytes = self.read_guest_c_string(string, 1024 * 1024)?;
                let offset = if needle == 0 {
                    Some(bytes.len())
                } else {
                    bytes.iter().rposition(|byte| *byte == needle)
                };
                let result = offset.map_or(0, |offset| string.wrapping_add(offset as u32));
                self.complete_functional_rom_call(result)?;
            }
            // strspn
            0x4000_141c => {
                let string = self.cpu.register(XtensaRegister::A2);
                let accepted = self.cpu.register(XtensaRegister::A3);
                let bytes = self.read_guest_c_string(string, 1024 * 1024)?;
                let accepted = self.read_guest_c_string(accepted, 1024 * 1024)?;
                let length = bytes
                    .iter()
                    .position(|byte| !accepted.contains(byte))
                    .unwrap_or(bytes.len());
                self.complete_functional_rom_call(length as u32)?;
            }
            // strlen
            0x4000_1248 => {
                let string = self.cpu.register(XtensaRegister::A2);
                let length = self.read_guest_c_string(string, 1024 * 1024)?.len();
                self.complete_functional_rom_call(length as u32)?;
            }
            // qsort. IDF startup sorts eight-byte reserved-memory ranges by
            // their first signed address field. Keep that ROM callback
            // contract explicit; singleton arrays are naturally unchanged.
            0x4000_1488 => {
                let base = self.cpu.register(XtensaRegister::A2);
                let count = self.cpu.register(XtensaRegister::A3) as usize;
                let size = self.cpu.register(XtensaRegister::A4) as usize;
                let comparator = self.cpu.register(XtensaRegister::A5);
                if count > 1 {
                    if comparator != 0x4212_82dc || size < 4 {
                        return Err(format!(
                            "unsupported functional qsort comparator {comparator:#010x}, size {size}"
                        ));
                    }
                    let mut records = (0..count)
                        .map(|index| {
                            self.read_guest_bytes(base.wrapping_add((index * size) as u32), size)
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    records.sort_by_key(|record| {
                        i32::from_le_bytes(
                            record[..4].try_into().expect("record is at least 4 bytes"),
                        )
                    });
                    for (index, record) in records.iter().enumerate() {
                        self.write_guest_bytes(base.wrapping_add((index * size) as u32), record)?;
                    }
                }
                self.complete_functional_rom_call(0)?;
            }
            // Newlib non-local control transfer. Save the logical windowed
            // context after setjmp has returned to its caller; longjmp
            // reinstates it and supplies the non-zero call8 return value.
            0x4000_144c => {
                let environment = self.cpu.register(XtensaRegister::A2);
                self.complete_functional_rom_call(0)?;
                self.setjmp_contexts.insert(environment, self.cpu.clone());
            }
            // ESP-IDF wraps the ESP32-S3 ROM longjmp so WINDOWSTART can be
            // repaired in a critical section. The interpreter keeps logical
            // register windows directly, so both entry points perform the
            // same non-local restoration. This must not return through the
            // wrapper: doing so would leave MicroPython's NLR frame active.
            0x4000_1440 | 0x4212_b548 => {
                let environment = self.cpu.register(XtensaRegister::A2);
                let value = self.cpu.register(XtensaRegister::A3).max(1);
                let mut restored = self
                    .setjmp_contexts
                    .get(&environment)
                    .cloned()
                    .ok_or_else(|| format!("longjmp used unknown environment {environment:#x}"))?;
                restored.set_register(XtensaRegister::A10, value);
                self.cpu = restored;
            }
            // Newlib expands sqrtf into the Xtensa coprocessor's reciprocal
            // approximation/refinement sequence. At the platform's declared
            // functional fidelity, evaluate the public helper atomically
            // while retaining IEEE-754 single-precision behavior.
            0x4212_e2c4 => {
                let value = f32::from_bits(self.cpu.register(XtensaRegister::A2));
                self.complete_functional_rom_call(value.sqrt().to_bits())?;
            }
            // Mbed TLS SHA-224/SHA-256 API backed by deterministic host
            // hashing. This is the functional equivalent of the ESP32-S3
            // SHA accelerator, including incremental and cloned contexts.
            0x420d_37a8 => {
                let context = self.cpu.register(XtensaRegister::A2);
                self.sha256_contexts
                    .insert(context, FunctionalSha256::default());
                self.complete_functional_rom_call(0)?;
            }
            0x4212_cfe8 => {
                let context = self.cpu.register(XtensaRegister::A2);
                self.sha256_contexts.remove(&context);
                self.complete_functional_rom_call(0)?;
            }
            0x420d_37bc => {
                let destination = self.cpu.register(XtensaRegister::A2);
                let source = self.cpu.register(XtensaRegister::A3);
                let state = self
                    .sha256_contexts
                    .get(&source)
                    .cloned()
                    .unwrap_or_default();
                self.sha256_contexts.insert(destination, state);
                self.complete_functional_rom_call(0)?;
            }
            0x420d_37d0 => {
                let context = self.cpu.register(XtensaRegister::A2);
                let sha224 = self.cpu.register(XtensaRegister::A3) != 0;
                self.sha256_contexts.insert(
                    context,
                    FunctionalSha256 {
                        sha224,
                        input: Vec::new(),
                    },
                );
                self.complete_functional_rom_call(0)?;
            }
            0x420d_37f0 => {
                let context = self.cpu.register(XtensaRegister::A2);
                let input = self.cpu.register(XtensaRegister::A3);
                let length = self.cpu.register(XtensaRegister::A4) as usize;
                let bytes = self.read_guest_bytes(input, length)?;
                self.sha256_contexts
                    .entry(context)
                    .or_default()
                    .input
                    .extend_from_slice(&bytes);
                self.complete_functional_rom_call(0)?;
            }
            0x420d_3938 => {
                let context = self.cpu.register(XtensaRegister::A2);
                let output = self.cpu.register(XtensaRegister::A3);
                let state = self
                    .sha256_contexts
                    .get(&context)
                    .cloned()
                    .unwrap_or_default();
                let digest = if state.sha224 {
                    Sha224::digest(&state.input).to_vec()
                } else {
                    Sha256::digest(&state.input).to_vec()
                };
                self.write_guest_bytes(output, &digest)?;
                self.complete_functional_rom_call(0)?;
            }
            // Newlib integer helpers.
            0x4000_1458 | 0x4000_1470 => {
                let value = self.cpu.register(XtensaRegister::A2) as i32;
                self.complete_functional_rom_call(value.wrapping_abs() as u32)?;
            }
            0x4000_1464 | 0x4000_147c => {
                let numerator = self.cpu.register(XtensaRegister::A2) as i32;
                let denominator = self.cpu.register(XtensaRegister::A3) as i32;
                let quotient = if denominator == 0 {
                    0
                } else {
                    numerator.checked_div(denominator).unwrap_or(i32::MIN)
                };
                let remainder = if denominator == 0 {
                    numerator
                } else {
                    numerator.checked_rem(denominator).unwrap_or_default()
                };
                self.complete_functional_rom_call_u64(
                    u64::from(quotient as u32) | (u64::from(remainder as u32) << 32),
                )?;
            }
            // utoa/itoa
            0x4000_14b8 | 0x4000_14c4 => {
                let raw = self.cpu.register(XtensaRegister::A2);
                let destination = self.cpu.register(XtensaRegister::A3);
                let radix = self.cpu.register(XtensaRegister::A4);
                let signed = pc == 0x4000_14c4 && radix == 10 && (raw as i32) < 0;
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
                self.complete_functional_rom_call(destination)?;
            }
            // Instruction/data cache configuration and resume routines. Direct
            // application loading has already established coherent mappings.
            0x4000_19b0 => {
                let virtual_address = self.cpu.register(XtensaRegister::A3);
                let physical_address = self.cpu.register(XtensaRegister::A4) as usize;
                let page_size_kib = self.cpu.register(XtensaRegister::A5) as usize;
                let pages = self.cpu.register(XtensaRegister::A6) as usize;
                let fixed = self.cpu.register(XtensaRegister::A7) != 0;
                let page_size = page_size_kib.saturating_mul(1024);
                if page_size != 64 * 1024 {
                    self.complete_functional_rom_call(3)?;
                } else {
                    for page in 0..pages {
                        let source = physical_address
                            .checked_add(if fixed { 0 } else { page * page_size })
                            .ok_or_else(|| "ESP flash MMU source overflow".to_owned())?;
                        let end = source
                            .checked_add(page_size)
                            .filter(|end| *end <= self.flash.len())
                            .ok_or_else(|| {
                                format!(
                                    "ESP flash MMU map {source:#x}..{:#x} exceeds image",
                                    source.saturating_add(page_size)
                                )
                            })?;
                        let destination = virtual_address.wrapping_add((page * page_size) as u32);
                        self.bus
                            .load(u64::from(destination), &self.flash[source..end])
                            .map_err(|error| error.to_string())?;
                    }
                    self.complete_functional_rom_call(0)?;
                }
            }
            0x4000_15fc..=0x4000_1a28 => {
                self.complete_functional_rom_call(0)?;
            }
            // ROM clock query/update services.
            0x4000_1a34 => self.complete_functional_rom_call(80_000_000)?,
            0x4000_1a40 => self.complete_functional_rom_call(240)?,
            0x4000_1a4c => self.complete_functional_rom_call(0)?,
            // ROM GPIO matrix and pad helpers. The register-level GPIO model
            // retains pin state; routing and pad policy complete immediately.
            0x4000_1a58..=0x4000_1b48 => self.complete_functional_rom_call(0)?,
            // Mask-ROM MD5 API used to validate partition metadata.
            0x4000_1c5c => {
                let context = self.cpu.register(XtensaRegister::A2);
                self.md5_contexts.insert(context, Vec::new());
                self.write_guest_bytes(context, &[0; 88])?;
                self.complete_functional_rom_call(0)?;
            }
            0x4000_1c68 => {
                let context = self.cpu.register(XtensaRegister::A2);
                let input = self.cpu.register(XtensaRegister::A3);
                let length = self.cpu.register(XtensaRegister::A4) as usize;
                let bytes = self.read_guest_bytes(input, length)?;
                self.md5_contexts
                    .entry(context)
                    .or_default()
                    .extend_from_slice(&bytes);
                self.complete_functional_rom_call(0)?;
            }
            0x4000_1c74 => {
                let digest_address = self.cpu.register(XtensaRegister::A2);
                let context = self.cpu.register(XtensaRegister::A3);
                let message = self.md5_contexts.remove(&context).unwrap_or_default();
                let digest = Md5::digest(message);
                self.write_guest_bytes(digest_address, digest.as_slice())?;
                self.complete_functional_rom_call(0)?;
            }
            // ROM CRC32 helpers retain the caller-supplied accumulator.
            0x4000_1c98 | 0x4000_1ca4 => {
                let mut crc = self.cpu.register(XtensaRegister::A2);
                let input = self.cpu.register(XtensaRegister::A3);
                let length = self.cpu.register(XtensaRegister::A4) as usize;
                for byte in self.read_guest_bytes(input, length)? {
                    if pc == 0x4000_1c98 {
                        crc ^= u32::from(byte);
                        for _ in 0..8 {
                            crc = (crc >> 1) ^ (0xedb8_8320 & 0_u32.wrapping_sub(crc & 1));
                        }
                    } else {
                        crc ^= u32::from(byte) << 24;
                        for _ in 0..8 {
                            crc = (crc << 1) ^ (0x04c1_1db7 & 0_u32.wrapping_sub(crc >> 31));
                        }
                    }
                }
                self.complete_functional_rom_call(crc)?;
            }
            // Functional blank/default eFuse policy: unsecured boot, default
            // SPI pads, USB enabled, and no burned feature-disable bits.
            0x4000_1ef0..=0x4000_2028 => self.complete_functional_rom_call(0)?,
            // _xtos_set_intlevel(level)
            0x4000_1c38 => {
                let level = self.cpu.register(XtensaRegister::A2);
                let previous = self.cpu.set_interrupt_level(level);
                self.complete_functional_rom_call(previous)?;
            }
            // intr_matrix_set(cpu, source, interrupt): retain deterministic
            // routing state for subsequent peripheral interrupt delivery.
            0x4000_1b54 => {
                let cpu = self.cpu.register(XtensaRegister::A2);
                let source = self.cpu.register(XtensaRegister::A3);
                let interrupt = self.cpu.register(XtensaRegister::A4);
                if std::env::var_os("RENVO_DEBUG_INTERRUPTS").is_some() {
                    eprintln!("interrupt route cpu={cpu} source={source} line={interrupt}");
                }
                self.interrupt_routes.insert((cpu, source), interrupt);
                self.complete_functional_rom_call(0)?;
            }
            // ROM analog-I2C register helpers used during clock/PHY setup.
            0x4000_5cd0 | 0x4000_5cdc | 0x4000_5d48 | 0x4000_5d54 | 0x4000_5d60 | 0x4000_5d6c => {
                self.complete_functional_rom_call(0)?
            }
            // Coexistence-ROM build identifier. The real ROM returns a
            // persistent C string; expose an emulator-owned string in the
            // modeled ROM data window so IDF can copy and report it normally.
            0x4000_5b68 => self.complete_functional_rom_call(0x3ff1_e000)?,
            // ROM libgcc scalar floating-point helpers. Keep arguments and
            // results as raw ABI payloads so NaNs and signed zero survive.
            0x4000_2190 | 0x4000_2274 | 0x4000_243c | 0x4000_2508 => {
                let left = f32::from_bits(self.cpu.register(XtensaRegister::A2));
                let right = f32::from_bits(self.cpu.register(XtensaRegister::A3));
                let result = match pc {
                    0x4000_2190 => left + right,
                    0x4000_2274 => left / right,
                    0x4000_243c => left * right,
                    0x4000_2508 => left - right,
                    _ => unreachable!(),
                };
                self.complete_functional_rom_call(result.to_bits())?;
            }
            0x4000_2184 | 0x4000_2250 | 0x4000_2418 | 0x4000_24fc => {
                let left = f64::from_bits(
                    u64::from(self.cpu.register(XtensaRegister::A2))
                        | (u64::from(self.cpu.register(XtensaRegister::A3)) << 32),
                );
                let right = f64::from_bits(
                    u64::from(self.cpu.register(XtensaRegister::A4))
                        | (u64::from(self.cpu.register(XtensaRegister::A5)) << 32),
                );
                let result = match pc {
                    0x4000_2184 => left + right,
                    0x4000_2250 => left / right,
                    0x4000_2418 => left * right,
                    0x4000_24fc => left - right,
                    _ => unreachable!(),
                };
                self.complete_functional_rom_call_u64(result.to_bits())?;
            }
            0x4000_2490 => {
                let value = self.cpu.register(XtensaRegister::A2);
                self.complete_functional_rom_call(value ^ (1 << 31))?;
            }
            0x4000_2478 => {
                let value = u64::from(self.cpu.register(XtensaRegister::A2))
                    | (u64::from(self.cpu.register(XtensaRegister::A3)) << 32);
                self.complete_functional_rom_call_u64(value ^ (1_u64 << 63))?;
            }
            0x4000_22a4 => {
                let value = f32::from_bits(self.cpu.register(XtensaRegister::A2));
                self.complete_functional_rom_call_u64(f64::from(value).to_bits())?;
            }
            0x4000_252c => {
                let value = f64::from_bits(
                    u64::from(self.cpu.register(XtensaRegister::A2))
                        | (u64::from(self.cpu.register(XtensaRegister::A3)) << 32),
                );
                self.complete_functional_rom_call((value as f32).to_bits())?;
            }
            0x4000_22ec | 0x4000_2310 => {
                let value = f32::from_bits(self.cpu.register(XtensaRegister::A2));
                let result = if pc == 0x4000_22ec {
                    (value as i32) as u32
                } else {
                    value as u32
                };
                self.complete_functional_rom_call(result)?;
            }
            0x4000_22e0 | 0x4000_2304 => {
                let value = f32::from_bits(self.cpu.register(XtensaRegister::A2));
                let result = if pc == 0x4000_22e0 {
                    (value as i64) as u64
                } else {
                    value as u64
                };
                self.complete_functional_rom_call_u64(result)?;
            }
            0x4000_22d4 | 0x4000_22f8 => {
                let value = f64::from_bits(
                    u64::from(self.cpu.register(XtensaRegister::A2))
                        | (u64::from(self.cpu.register(XtensaRegister::A3)) << 32),
                );
                let result = if pc == 0x4000_22d4 {
                    (value as i32) as u32
                } else {
                    value as u32
                };
                self.complete_functional_rom_call(result)?;
            }
            0x4000_2340 | 0x4000_2370 => {
                let value = self.cpu.register(XtensaRegister::A2);
                let result = if pc == 0x4000_2340 {
                    (value as i32) as f32
                } else {
                    value as f32
                };
                self.complete_functional_rom_call(result.to_bits())?;
            }
            0x4000_2334 | 0x4000_2364 => {
                let value = self.cpu.register(XtensaRegister::A2);
                let result = if pc == 0x4000_2334 {
                    f64::from(value as i32)
                } else {
                    f64::from(value)
                };
                self.complete_functional_rom_call_u64(result.to_bits())?;
            }
            0x4000_2328 | 0x4000_2358 => {
                let value = u64::from(self.cpu.register(XtensaRegister::A2))
                    | (u64::from(self.cpu.register(XtensaRegister::A3)) << 32);
                let result = if pc == 0x4000_2328 {
                    (value as i64) as f32
                } else {
                    value as f32
                };
                self.complete_functional_rom_call(result.to_bits())?;
            }
            0x4000_24f0 => {
                let value = f32::from_bits(self.cpu.register(XtensaRegister::A2));
                let exponent = self.cpu.register(XtensaRegister::A3) as i32;
                self.complete_functional_rom_call(value.powi(exponent).to_bits())?;
            }
            0x4000_2298 | 0x4000_2394 | 0x4000_23ac | 0x4000_23c4 | 0x4000_23e8 | 0x4000_24b4
            | 0x4000_2598 => {
                let left = f32::from_bits(self.cpu.register(XtensaRegister::A2));
                let right = f32::from_bits(self.cpu.register(XtensaRegister::A3));
                let unordered = left.is_nan() || right.is_nan();
                let result = match pc {
                    0x4000_2298 | 0x4000_24b4 => i32::from(left != right || unordered),
                    0x4000_2394 => {
                        if unordered || left < right {
                            -1
                        } else {
                            i32::from(left > right)
                        }
                    }
                    0x4000_23ac => {
                        if unordered {
                            -1
                        } else {
                            i32::from(left > right)
                        }
                    }
                    0x4000_23c4 => {
                        if unordered || left > right {
                            1
                        } else {
                            -i32::from(left < right)
                        }
                    }
                    0x4000_23e8 => {
                        if unordered {
                            1
                        } else {
                            -i32::from(left < right)
                        }
                    }
                    0x4000_2598 => i32::from(unordered),
                    _ => unreachable!(),
                };
                self.complete_functional_rom_call(result as u32)?;
            }
            // Compiler-runtime unsigned division and remainder helpers.
            0x4000_225c | 0x4000_23f4 => {
                let numerator = (u64::from(self.cpu.register(XtensaRegister::A2))
                    | (u64::from(self.cpu.register(XtensaRegister::A3)) << 32))
                    as i64;
                let denominator = (u64::from(self.cpu.register(XtensaRegister::A4))
                    | (u64::from(self.cpu.register(XtensaRegister::A5)) << 32))
                    as i64;
                let result = if denominator == 0 {
                    if pc == 0x4000_225c { -1 } else { numerator }
                } else if pc == 0x4000_225c {
                    numerator.checked_div(denominator).unwrap_or(i64::MIN)
                } else {
                    numerator.checked_rem(denominator).unwrap_or_default()
                };
                self.complete_functional_rom_call_u64(result as u64)?;
            }
            0x4000_2280 | 0x4000_2400 => {
                let numerator = self.cpu.register(XtensaRegister::A2) as i32;
                let denominator = self.cpu.register(XtensaRegister::A3) as i32;
                let result = if denominator == 0 {
                    if pc == 0x4000_2280 { -1 } else { numerator }
                } else if pc == 0x4000_2280 {
                    numerator.checked_div(denominator).unwrap_or(i32::MIN)
                } else {
                    numerator.checked_rem(denominator).unwrap_or_default()
                };
                self.complete_functional_rom_call(result as u32)?;
            }
            0x4000_2544 | 0x4000_2574 => {
                let numerator = u64::from(self.cpu.register(XtensaRegister::A2))
                    | (u64::from(self.cpu.register(XtensaRegister::A3)) << 32);
                let denominator = u64::from(self.cpu.register(XtensaRegister::A4))
                    | (u64::from(self.cpu.register(XtensaRegister::A5)) << 32);
                let result = if denominator == 0 {
                    u64::MAX
                } else if pc == 0x4000_2544 {
                    numerator / denominator
                } else {
                    numerator % denominator
                };
                self.complete_functional_rom_call_u64(result)?;
            }
            0x4000_255c | 0x4000_2580 => {
                let numerator = self.cpu.register(XtensaRegister::A2);
                let denominator = self.cpu.register(XtensaRegister::A3);
                let result = if denominator == 0 {
                    u32::MAX
                } else if pc == 0x4000_255c {
                    numerator / denominator
                } else {
                    numerator % denominator
                };
                self.complete_functional_rom_call(result)?;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// Drives or releases one low GPIO bank pin.
    pub fn set_pin(&self, pin: u8, value: Logic) -> Result<(), XtensaMachineError> {
        self.gpio.set_input(pin, value, self.now)?;
        self.chip_gpio.set_input(pin, value, self.now)?;
        Ok(())
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
                && std::env::var_os("RENVO_DEBUG_USB").is_some()
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
                if let Some(interrupt) = self
                    .interrupt_routes
                    .get(&(core, 38))
                    .copied()
                    .filter(|interrupt| *interrupt != 6)
                {
                    if core == 0 {
                        self.cpu.set_interrupt(interrupt as u16, usb_pending)?;
                    } else if self.appcpu_boot_address.is_some() {
                        self.cpu1.set_interrupt(interrupt as u16, usb_pending)?;
                    }
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
                if let Some(interrupt) = self.interrupt_routes.get(&(core, source)).copied() {
                    if newly_pending && std::env::var_os("RENVO_DEBUG_INTERRUPTS").is_some() {
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
                            .set_interrupt(interrupt as u16, crosscore_pending)?;
                    } else if self.appcpu_boot_address.is_some() {
                        self.cpu1
                            .set_interrupt(interrupt as u16, crosscore_pending)?;
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
                if let Some(interrupt) = self.interrupt_routes.get(&(core, source)).copied() {
                    if pending
                        && newly_pending
                        && std::env::var_os("RENVO_DEBUG_INTERRUPTS").is_some()
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
                    self.set_systimer_interrupt(core, interrupt, pending)?;
                } else if pending
                    && newly_pending
                    && std::env::var_os("RENVO_DEBUG_INTERRUPTS").is_some()
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
                        let Some(interrupt) = self
                            .interrupt_routes
                            .get(&(core, source))
                            .copied()
                            .filter(|interrupt| *interrupt != 6)
                        else {
                            continue;
                        };
                        if core == 0 {
                            self.cpu.set_interrupt(interrupt as u16, pending)?;
                        } else if self.appcpu_boot_address.is_some() {
                            self.cpu1.set_interrupt(interrupt as u16, pending)?;
                        }
                    }
                }
            }
            let running_cpu1 = next_core == 1 && self.appcpu_boot_address.is_some();
            if running_cpu1 {
                std::mem::swap(&mut self.cpu, &mut self.cpu1);
            }
            match self.service_functional_rom() {
                Ok(true) => {
                    if running_cpu1 {
                        std::mem::swap(&mut self.cpu, &mut self.cpu1);
                    }
                    stats.instructions = stats.instructions.saturating_add(1);
                    self.now = self
                        .now
                        .checked_add(renvo_core::SimDuration::TICK)
                        .map_err(|_| XtensaMachineError::TimeOverflow)?;
                    stats.time = self.now;
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
            next_core = if self.appcpu_boot_address.is_some() {
                next_core ^ 1
            } else {
                0
            };
            for change in self.signals.drain_changes() {
                digest.change(&change);
                if let Some(sink) = trace.as_deref_mut() {
                    sink.change(&change)?;
                }
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
mod tests {
    use super::*;

    #[test]
    fn appcpu_systimer_defers_to_a_logical_window_safe_point_during_usb_execution() {
        assert!(appcpu_systimer_level(true, false, false));
        assert!(!appcpu_systimer_level(true, true, false));
        assert!(appcpu_systimer_level(true, true, true));
        assert!(!appcpu_systimer_level(false, true, true));
    }

    #[test]
    fn dwc2_host_completes_only_after_the_final_raw_prompt() {
        let mut host = EspDwc2Host::new();
        assert!(!host.input_complete());
        host.queue_input(b"\x01print(1)\n\x04");
        host.input.clear();
        host.sending_raw_chunk = false;
        host.raw_prompt_ready = true;
        host.output.extend_from_slice(b"OK\x04\x04>");
        host.raw_chunks_completed = 1;
        assert!(host.input_complete());
    }
}
