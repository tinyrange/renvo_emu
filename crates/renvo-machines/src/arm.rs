use crate::riscv::{TEST_DEVICE_SIZE, TEST_EXIT_SIZE};
use crate::{
    MemoryKind, PinStimulus, RunResult, TEST_EXIT, TEST_GPIO, TEST_TIMER, TEST_UART, TargetId,
    target_manifest,
};
use renvo_bus::{AddressSpace, BusAccessRecord, Endianness, MapError, Permissions, SharedMemory};
use renvo_core::{
    AccessKind, AccessWidth, Bus, Cpu, CpuFault, RunLimits, RunStats, SimTime, StepReason,
    StopReason,
};
use renvo_cpu_arm::{ArmCpu, ArmProfile, ArmRegister};
use renvo_devices::{
    ArmPpbHandle, ArmPrivatePeripheralBus, ExitDevice, ExitHandle, FunctionalGpio, FunctionalTimer,
    FunctionalUart, GpioHandle, Rp2040Clocks, Rp2040Pll, Rp2040RegisterBank, Rp2040Resets,
    Rp2040Rtc, Rp2040Ssi, Rp2040Timer, Rp2040TimerHandle, Rp2040UsbController, Rp2040UsbHandle,
    Rp2040Watchdog, Rp2040Xosc, Rp2350BootRam, Rp2350XipMaintenance, RpSioGpio, RpSioHandle,
    RpTimerLayout, SignalHub, TimerHandle, UartHandle,
};
use renvo_image::{FirmwareArchitecture, FirmwareImage, Uf2Error, Uf2Image};
use renvo_signals::{Logic, SignalError};
use renvo_trace::{TraceDigest, TraceError, TraceSink};
use std::collections::{BTreeMap, VecDeque};
use thiserror::Error;

/// Arm machine construction or execution failure.
#[derive(Debug, Error)]
pub enum ArmMachineError {
    /// Target does not have an implemented Arm mode.
    #[error("target {0} does not currently have a runnable Arm M-profile")]
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
    /// Firmware has the wrong ELF architecture.
    #[error("firmware architecture {actual:?} does not match Arm target {target}")]
    Architecture {
        /// Target being loaded.
        target: TargetId,
        /// ELF architecture.
        actual: FirmwareArchitecture,
    },
    /// Entry does not fit a 32-bit address.
    #[error("firmware entry {0:#x} exceeds the Arm address space")]
    EntryRange(u64),
    /// Segment does not fall within the target map.
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
    /// UF2 parsing or flash reconstruction failed.
    #[error(transparent)]
    Uf2(#[from] Uf2Error),
    /// The UF2 target family does not match the selected processor.
    #[error("UF2 family {actual:#010x} does not match {target}; expected {expected:#010x}")]
    Uf2Family {
        /// Selected machine.
        target: TargetId,
        /// Family identifier required by that machine.
        expected: u32,
        /// Family identifier present in the artifact.
        actual: u32,
    },
    /// A boot-ROM handoff is not implemented for the selected target.
    #[error("boot-ROM handoff is not implemented for {0}")]
    BootHandoffUnsupported(TargetId),
    /// An official image contains an invalid post-boot vector table.
    #[error("invalid {target} vector table at {vector_base:#010x}: {message}")]
    BootVector {
        /// Selected machine.
        target: TargetId,
        /// Address of the vector table.
        vector_base: u32,
        /// Validation diagnostic.
        message: String,
    },
}

const USB_BUF_FULL: u16 = 0x8000;
const USB_BUF_AVAIL: u16 = 0x0400;
const USB_BUF_LEN: u16 = 0x03ff;

#[derive(Clone, Copy)]
enum UsbControlResponse {
    DeviceDescriptor,
    ConfigurationDescriptor,
    None,
}

struct UsbControlRequest {
    setup: [u8; 8],
    response: UsbControlResponse,
}

struct UsbControlTransfer {
    request: UsbControlRequest,
    response: Vec<u8>,
    data_complete: bool,
}

pub(crate) struct Rp2040UsbHost {
    reset_sent: bool,
    next_setup_at: u64,
    requests: VecDeque<UsbControlRequest>,
    active: Option<UsbControlTransfer>,
    bulk_in: Option<u8>,
    bulk_out: Option<u8>,
    input: VecDeque<u8>,
    output: Vec<u8>,
    sending_raw_chunk: bool,
    raw_prompt_ready: bool,
}

impl Rp2040UsbHost {
    pub(crate) fn new() -> Self {
        let requests = VecDeque::from([
            UsbControlRequest {
                setup: [0x80, 6, 0, 1, 0, 0, 18, 0],
                response: UsbControlResponse::DeviceDescriptor,
            },
            UsbControlRequest {
                setup: [0x00, 5, 1, 0, 0, 0, 0, 0],
                response: UsbControlResponse::None,
            },
            UsbControlRequest {
                setup: [0x80, 6, 0, 2, 0, 0, 255, 0],
                response: UsbControlResponse::ConfigurationDescriptor,
            },
            UsbControlRequest {
                setup: [0x00, 9, 1, 0, 0, 0, 0, 0],
                response: UsbControlResponse::None,
            },
            // CDC ACM SET_CONTROL_LINE_STATE with DTR and RTS asserted.
            UsbControlRequest {
                setup: [0x21, 0x22, 3, 0, 0, 0, 0, 0],
                response: UsbControlResponse::None,
            },
        ]);
        Self {
            reset_sent: false,
            next_setup_at: 0,
            requests,
            active: None,
            bulk_in: None,
            bulk_out: None,
            input: VecDeque::new(),
            output: Vec::new(),
            sending_raw_chunk: false,
            raw_prompt_ready: false,
        }
    }

    pub(crate) fn queue_input(&mut self, bytes: &[u8]) {
        self.input.extend(bytes.iter().copied());
        self.sending_raw_chunk |= !bytes.is_empty();
    }

    pub(crate) fn output(&self) -> Vec<u8> {
        self.output.clone()
    }

    fn endpoint_buffer_control_offset(endpoint: u8, input: bool) -> usize {
        0x80 + usize::from(endpoint) * 8 + usize::from(!input) * 4
    }

    fn endpoint_control_offset(endpoint: u8, input: bool) -> usize {
        0x08 + (usize::from(endpoint) - 1) * 8 + usize::from(!input) * 4
    }

    fn finish_control(&mut self, now: u64) {
        let transfer = self.active.take().expect("active USB control transfer");
        if matches!(
            transfer.request.response,
            UsbControlResponse::ConfigurationDescriptor
        ) {
            self.discover_bulk_endpoints(&transfer.response);
        }
        self.next_setup_at = now.saturating_add(256);
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
                let endpoint = address & 0x0f;
                if address & 0x80 != 0 {
                    self.bulk_in = Some(endpoint);
                } else {
                    self.bulk_out = Some(endpoint);
                }
            }
            offset += length;
        }
    }

    fn complete_control_in(
        &mut self,
        now: u64,
        usb: &Rp2040UsbHandle,
        dpram: &SharedMemory,
    ) -> u64 {
        let Some(transfer) = &mut self.active else {
            return 0;
        };
        let control = dpram.read_u32(0x80).unwrap_or(0);
        let buffer = control as u16;
        if !transfer.data_complete
            && buffer & (USB_BUF_AVAIL | USB_BUF_FULL) == (USB_BUF_AVAIL | USB_BUF_FULL)
        {
            let length = usize::from(buffer & USB_BUF_LEN);
            if let Some(bytes) = dpram.read_range(0x100, length) {
                transfer.response.extend(bytes);
            }
            dpram.write_u32(0x80, control & !u32::from(USB_BUF_AVAIL | USB_BUF_FULL));
            usb.complete_buffer(0, true);
            let requested = usize::from(u16::from_le_bytes([
                transfer.request.setup[6],
                transfer.request.setup[7],
            ]));
            transfer.data_complete = length < 64 || transfer.response.len() >= requested;
            return 1;
        }
        if transfer.data_complete {
            let control = dpram.read_u32(0x84).unwrap_or(0);
            let buffer = control as u16;
            if buffer & USB_BUF_AVAIL != 0 && buffer & USB_BUF_FULL == 0 {
                let completed =
                    (control & !u32::from(USB_BUF_AVAIL | USB_BUF_LEN)) | u32::from(USB_BUF_FULL);
                dpram.write_u32(0x84, completed);
                usb.complete_buffer(0, false);
                self.finish_control(now);
                return 1;
            }
        }
        0
    }

    fn complete_control_out(
        &mut self,
        now: u64,
        usb: &Rp2040UsbHandle,
        dpram: &SharedMemory,
    ) -> u64 {
        let control = dpram.read_u32(0x80).unwrap_or(0);
        let buffer = control as u16;
        if buffer & (USB_BUF_AVAIL | USB_BUF_FULL) == (USB_BUF_AVAIL | USB_BUF_FULL) {
            dpram.write_u32(0x80, control & !u32::from(USB_BUF_AVAIL | USB_BUF_FULL));
            usb.complete_buffer(0, true);
            self.finish_control(now);
            return 1;
        }
        0
    }

    fn service_bulk_in(
        &mut self,
        usb: &Rp2040UsbHandle,
        dpram: &SharedMemory,
        endpoint: u8,
    ) -> u64 {
        let endpoint_control = dpram
            .read_u32(Self::endpoint_control_offset(endpoint, true))
            .unwrap_or(0);
        if endpoint_control & (1 << 31) == 0 {
            return 0;
        }
        let buffer_offset = usize::from(endpoint_control as u16);
        let control_offset = Self::endpoint_buffer_control_offset(endpoint, true);
        let mut control = dpram.read_u32(control_offset).unwrap_or(0);
        let double_buffered = endpoint_control & (1 << 30) != 0;
        let mut completed = false;
        for buffer_index in 0..=usize::from(double_buffered) {
            let shift = buffer_index * 16;
            let half = (control >> shift) as u16;
            if half & (USB_BUF_AVAIL | USB_BUF_FULL) != (USB_BUF_AVAIL | USB_BUF_FULL) {
                continue;
            }
            let length = usize::from(half & USB_BUF_LEN);
            if let Some(bytes) = dpram.read_range(buffer_offset + buffer_index * 64, length) {
                self.output.extend(bytes);
                if self.output.ends_with(b"\x04\x04>")
                    || self.output.ends_with(b"raw REPL; CTRL-B to exit\r\n>")
                {
                    self.raw_prompt_ready = true;
                }
            }
            let cleared = half & !(USB_BUF_AVAIL | USB_BUF_FULL);
            control = (control & !(0xffff << shift)) | (u32::from(cleared) << shift);
            completed = true;
        }
        if completed {
            dpram.write_u32(control_offset, control);
            usb.complete_buffer(endpoint, true);
            1
        } else {
            0
        }
    }

    fn service_bulk_out(
        &mut self,
        usb: &Rp2040UsbHandle,
        dpram: &SharedMemory,
        endpoint: u8,
    ) -> u64 {
        if self.input.is_empty() {
            return 0;
        }
        if !self.sending_raw_chunk {
            if !self.raw_prompt_ready {
                return 0;
            }
            self.sending_raw_chunk = true;
            self.raw_prompt_ready = false;
        }
        let endpoint_control = dpram
            .read_u32(Self::endpoint_control_offset(endpoint, false))
            .unwrap_or(0);
        if endpoint_control & (1 << 31) == 0 {
            return 0;
        }
        let control_offset = Self::endpoint_buffer_control_offset(endpoint, false);
        let control = dpram.read_u32(control_offset).unwrap_or(0);
        let buffer = control as u16;
        if buffer & USB_BUF_AVAIL == 0 || buffer & USB_BUF_FULL != 0 {
            return 0;
        }
        let mut length = self.input.len().min(64);
        if let Some(end) = self
            .input
            .iter()
            .take(length)
            .position(|byte| *byte == 0x04)
        {
            length = end + 1;
        }
        let bytes = self.input.drain(..length).collect::<Vec<_>>();
        if bytes.contains(&0x04) {
            self.sending_raw_chunk = false;
        }
        let buffer_offset = usize::from(endpoint_control as u16);
        dpram.write_range(buffer_offset, &bytes);
        let completed = (control & !u32::from(USB_BUF_AVAIL | USB_BUF_LEN))
            | u32::from(USB_BUF_FULL)
            | u32::try_from(length).expect("USB packet length fits u32");
        dpram.write_u32(control_offset, completed);
        usb.complete_buffer(endpoint, false);
        1
    }

    pub(crate) fn poll(
        &mut self,
        now: SimTime,
        usb: &Rp2040UsbHandle,
        dpram: &SharedMemory,
    ) -> u64 {
        if !self.reset_sent {
            if usb.device_connected() {
                usb.inject_bus_reset();
                self.reset_sent = true;
                self.next_setup_at = now.ticks().saturating_add(1024);
                return 1;
            }
            return 0;
        }

        if let Some(transfer) = &self.active {
            return if transfer.request.setup[0] & 0x80 != 0 {
                self.complete_control_in(now.ticks(), usb, dpram)
            } else {
                self.complete_control_out(now.ticks(), usb, dpram)
            };
        }

        if now.ticks() >= self.next_setup_at
            && !usb.interrupt_pending()
            && let Some(request) = self.requests.pop_front()
        {
            dpram.write_range(0, &request.setup);
            usb.inject_setup();
            self.active = Some(UsbControlTransfer {
                request,
                response: Vec::new(),
                data_complete: false,
            });
            return 1;
        }

        let mut events = 0;
        if let Some(endpoint) = self.bulk_in {
            events += self.service_bulk_in(usb, dpram, endpoint);
        }
        if let Some(endpoint) = self.bulk_out {
            events += self.service_bulk_out(usb, dpram, endpoint);
        }
        events
    }
}

/// Runnable direct-ELF Arm vertical slice for RP2040 and RP2350.
pub struct ArmMachine {
    target: TargetId,
    cpu: ArmCpu,
    cpu1: ArmCpu,
    cpu1_active: bool,
    bus: AddressSpace,
    signals: SignalHub,
    gpio: GpioHandle,
    chip_gpio: GpioHandle,
    sio: RpSioHandle,
    uart: UartHandle,
    chip_uart: UartHandle,
    timer: TimerHandle,
    exit: ExitHandle,
    now: SimTime,
    default_stack: u32,
    flash_base: u32,
    flash_size: usize,
    flash_storage: SharedMemory,
    bootrom_services: BTreeMap<u32, u32>,
    native_bootrom: bool,
    ppb: ArmPpbHandle,
    chip_timers: Vec<Rp2040TimerHandle>,
    usb: Option<Rp2040UsbHandle>,
    usb_dpram: Option<SharedMemory>,
    usb_host: Option<Rp2040UsbHost>,
}

impl ArmMachine {
    /// Creates the selected Raspberry Pi Arm mode.
    pub fn new(target: TargetId) -> Result<Self, ArmMachineError> {
        let profile = match target {
            TargetId::Rp2040 => ArmProfile::CortexM0Plus,
            TargetId::Rp2350 => ArmProfile::CortexM33,
            _ => return Err(ArmMachineError::UnsupportedTarget(target)),
        };
        let manifest = target_manifest(target);
        let mut bus = AddressSpace::new(Endianness::Little);
        let cpuid = match target {
            TargetId::Rp2040 => 0x410c_c601,
            TargetId::Rp2350 => 0x410f_d210,
            _ => unreachable!(),
        };
        let (ppb_device, ppb) = ArmPrivatePeripheralBus::new(format!("{target}.ppb"), cpuid);
        bus.map_device(
            format!("{target}.ppb"),
            0xe000_e000,
            0x1000,
            Box::new(ppb_device),
        )?;
        if target == TargetId::Rp2040 {
            // The public B2 ROM exposes a function-table pointer at 0x14 and its lookup routine at
            // 0x18. The SDK also copies the B2 single-precision table into RAM during startup, so
            // publish functional entries rather than allowing those indirect calls to be null.
            let mut rom = vec![0_u8; 16 * 1024];
            rom[0x13] = 3;
            // Functional return point for core-1 entry functions. The physical
            // boot ROM parks a returned core; WFI provides the same visible
            // behavior without interpreting undocumented ROM implementation.
            rom[0x80..0x82].copy_from_slice(&0xbf30_u16.to_le_bytes());
            rom[0x14..0x16].copy_from_slice(&0x0100_u16.to_le_bytes());
            rom[0x16..0x18].copy_from_slice(&0x0300_u16.to_le_bytes());
            rom[0x18..0x1a].copy_from_slice(&0x0021_u16.to_le_bytes());
            for offset in (0_u32..0x80).step_by(4) {
                let pointer = (0x0500_u32 + offset) | 1;
                let table_index = 0x0400_usize + usize::try_from(offset).expect("small offset");
                rom[table_index..table_index + 4].copy_from_slice(&pointer.to_le_bytes());
            }
            bus.map_write_ignored_rom("rp2040.bootrom-functional", 0, rom)?;
        } else {
            // RP2350 Arm images obtain ROM services through the documented halfword pointer at
            // 0x16. The actual mask-aware lookup and returned functions are machine host calls;
            // all other bytes retain deterministic erased-ROM semantics.
            let mut rom = vec![0_u8; 32 * 1024];
            rom[0x80..0x82].copy_from_slice(&0xbf30_u16.to_le_bytes());
            rom[0x16..0x18].copy_from_slice(&0x0021_u16.to_le_bytes());
            bus.map_write_ignored_rom("rp2350.bootrom-functional", 0, rom)?;
        }
        let mut default_stack = None;
        let mut flash = None;
        let mut flash_storage = None;
        let mut chip_timers = Vec::new();
        let mut usb = None;
        let mut usb_dpram = None;
        let mut usb_host = None;
        for region in manifest.memory {
            match region.kind {
                MemoryKind::Ram => {
                    bus.map_ram(region.name, region.start, region.size, region.executable)?;
                    let end = region.start + u64::try_from(region.size).expect("size fits u64");
                    default_stack = Some(
                        u32::try_from(end).expect("initial Arm address ranges fit in 32 bits"),
                    );
                }
                MemoryKind::Flash | MemoryKind::Rom => {
                    if region.kind == MemoryKind::Flash {
                        flash = Some((
                            u32::try_from(region.start)
                                .expect("Arm target flash address fits in 32 bits"),
                            region.size,
                        ));
                    }
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
                        Permissions::RX,
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
            let size = flash.expect("RP2350 manifest includes XIP flash").1;
            // RP2350 exposes the same external address space through cached, uncached, and
            // untranslated XIP windows. Timing/cache allocation differs in hardware, but all
            // three read coherently in the functional model.
            bus.map_shared(
                "rp2350.xip-nocache-noalloc",
                0x1400_0000,
                size,
                Permissions::RX,
                storage.clone(),
                0,
            )?;
            bus.map_shared(
                "rp2350.xip-nocache-noalloc-notranslate",
                0x1c00_0000,
                size,
                Permissions::RX,
                storage.clone(),
                0,
            )?;
        }
        let signals = SignalHub::new();
        let (gpio_device, gpio) = FunctionalGpio::new(
            format!("{target}.compiler-gpio"),
            manifest.gpio_count.min(32),
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
        let (sio_device, chip_gpio, sio) = if target == TargetId::Rp2350 {
            RpSioGpio::new_rp2350_with_multicore(
                format!("{target}.sio"),
                manifest.gpio_count.min(32),
                &format!("board.{target}.chip_gpio"),
                signals.clone(),
            )?
        } else {
            RpSioGpio::new_with_multicore(
                format!("{target}.sio"),
                manifest.gpio_count.min(32),
                &format!("board.{target}.chip_gpio"),
                signals.clone(),
            )?
        };
        bus.map_device(
            format!("{target}.sio"),
            0xd000_0000,
            0x200,
            Box::new(sio_device),
        )?;
        if target == TargetId::Rp2040 {
            let mut sysinfo_reset = vec![0; 8];
            // Production RP2040 B2: revision 2, RP2 part 2, Raspberry Pi
            // JEP-106 manufacturer 0x927, running on ASIC.
            sysinfo_reset[0] = 0x2000_2927;
            sysinfo_reset[1] = 2;
            bus.map_device(
                "rp2040.sysinfo",
                0x4000_0000,
                0x4000,
                Box::new(Rp2040RegisterBank::new("rp2040.sysinfo", sysinfo_reset)),
            )?;
            bus.map_device(
                "rp2040.clocks",
                0x4000_8000,
                0x4000,
                Box::new(Rp2040Clocks::new("rp2040.clocks")),
            )?;
            bus.map_device(
                "rp2040.resets",
                0x4000_c000,
                0x4000,
                Box::new(Rp2040Resets::new("rp2040.resets")),
            )?;
            let mut psm_reset = vec![0; 4];
            psm_reset[3] = 0x0001_ffff;
            bus.map_device(
                "rp2040.psm",
                0x4001_0000,
                0x4000,
                Box::new(Rp2040RegisterBank::new("rp2040.psm", psm_reset)),
            )?;
            bus.map_device(
                "rp2040.xosc",
                0x4002_4000,
                0x4000,
                Box::new(Rp2040Xosc::new("rp2040.xosc")),
            )?;
            bus.map_device(
                "rp2040.io-bank0",
                0x4001_4000,
                0x4000,
                Box::new(Rp2040RegisterBank::new("rp2040.io-bank0", vec![0; 256])),
            )?;
            let mut pad_reset = vec![0x56; 64];
            pad_reset[0] = 0;
            bus.map_device(
                "rp2040.pads-bank0",
                0x4001_c000,
                0x4000,
                Box::new(Rp2040RegisterBank::new("rp2040.pads-bank0", pad_reset)),
            )?;
            bus.map_device(
                "rp2040.io-qspi",
                0x4001_8000,
                0x4000,
                Box::new(Rp2040RegisterBank::new("rp2040.io-qspi", vec![0; 64])),
            )?;
            for (name, base) in [
                ("rp2040.uart1", 0x4003_8000),
                ("rp2040.spi0", 0x4003_c000),
                ("rp2040.spi1", 0x4004_0000),
                ("rp2040.i2c0", 0x4004_4000),
                ("rp2040.i2c1", 0x4004_8000),
                ("rp2040.adc", 0x4004_c000),
                ("rp2040.pwm", 0x4005_0000),
                ("rp2040.dma", 0x5000_0000),
                ("rp2040.pio0", 0x5020_0000),
                ("rp2040.pio1", 0x5030_0000),
            ] {
                bus.map_device(
                    name,
                    base,
                    0x4000,
                    Box::new(Rp2040RegisterBank::new(name, vec![0; 0x1000 / 4])),
                )?;
            }
            let mut qspi_pad_reset = vec![0x56; 8];
            qspi_pad_reset[0] = 0;
            bus.map_device(
                "rp2040.pads-qspi",
                0x4002_0000,
                0x4000,
                Box::new(Rp2040RegisterBank::new("rp2040.pads-qspi", qspi_pad_reset)),
            )?;
            bus.map_device(
                "rp2040.pll-sys",
                0x4002_8000,
                0x4000,
                Box::new(Rp2040Pll::new("rp2040.pll-sys")),
            )?;
            bus.map_device(
                "rp2040.pll-usb",
                0x4002_c000,
                0x4000,
                Box::new(Rp2040Pll::new("rp2040.pll-usb")),
            )?;
            bus.map_device(
                "rp2040.watchdog",
                0x4005_8000,
                0x4000,
                Box::new(Rp2040Watchdog::new("rp2040.watchdog")),
            )?;
            bus.map_device(
                "rp2040.rtc",
                0x4005_c000,
                0x4000,
                Box::new(Rp2040Rtc::new("rp2040.rtc")),
            )?;
            let mut rosc_reset = vec![0; 16];
            rosc_reset[0x18 / 4] = 0x8000_1000;
            rosc_reset[0x1c / 4] = 1;
            bus.map_device(
                "rp2040.rosc",
                0x4006_0000,
                0x4000,
                Box::new(Rp2040RegisterBank::new("rp2040.rosc", rosc_reset)),
            )?;
            bus.map_device(
                "rp2040.vreg-and-chip-reset",
                0x4006_4000,
                0x4000,
                Box::new(Rp2040RegisterBank::new(
                    "rp2040.vreg-and-chip-reset",
                    vec![0; 8],
                )),
            )?;
            let (timer_device, timer_handle) =
                Rp2040Timer::new("rp2040.timer", RpTimerLayout::Rp2040);
            bus.map_device("rp2040.timer", 0x4005_4000, 0x4000, Box::new(timer_device))?;
            chip_timers.push(timer_handle);
            let dpram = SharedMemory::zeroed(0x1000);
            bus.map_shared(
                "rp2040.usb-dpram",
                0x5010_0000,
                0x1000,
                Permissions::RW,
                dpram.clone(),
                0,
            )?;
            usb_dpram = Some(dpram);
            let (usb_device, usb_handle) = Rp2040UsbController::new_with_handle("rp2040.usbctrl");
            bus.map_device("rp2040.usbctrl", 0x5011_0000, 0x4000, Box::new(usb_device))?;
            usb = Some(usb_handle);
            usb_host = Some(Rp2040UsbHost::new());
            bus.map_device(
                "rp2040.xip-ssi",
                0x1800_0000,
                0x4000,
                Box::new(Rp2040Ssi::new("rp2040.xip-ssi")),
            )?;
            bus.map_device(
                "rp2040.xip-control",
                0x1400_0000,
                0x4000,
                Box::new(Rp2040RegisterBank::new("rp2040.xip-control", vec![0; 16])),
            )?;
        } else {
            let mut psm_reset = vec![0; 4];
            psm_reset[3] = 0x0fff_ffff;
            bus.map_device(
                "rp2350.psm",
                0x4001_8000,
                0x4000,
                Box::new(Rp2040RegisterBank::new("rp2350.psm", psm_reset)),
            )?;
            bus.map_device(
                "rp2350.clocks",
                0x4001_0000,
                0x4000,
                Box::new(Rp2040Clocks::new("rp2350.clocks")),
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
                ("rp2350.pio0", 0x5020_0000),
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
            // The RP2350 second-stage flash bootstrap executes from scratch SRAM and configures
            // the QSPI pads before touching the QMI. Retain the pad state and honor the RP atomic
            // aliases; the electrical slew/drive details do not affect functional execution.
            let mut qspi_pad_reset = vec![0x56; 0x1000 / 4];
            qspi_pad_reset[0] = 0;
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
                // The functional flash path begins with each QSPI output deasserted high.
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
            // The 0x1800_0000 alias addresses cache maintenance rather than external flash.
            // Functional execution has no cache timing, so acknowledge every maintenance access.
            bus.map_device(
                "rp2350.xip-maintenance",
                0x1800_0000,
                0x0400_0000,
                Box::new(Rp2350XipMaintenance::new("rp2350.xip-maintenance")),
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
                ("rp2350.timer0", 0x400b_0000),
                ("rp2350.timer1", 0x400b_8000),
            ] {
                let (timer_device, timer_handle) = Rp2040Timer::new(name, RpTimerLayout::Rp2350);
                bus.map_device(name, base, 0x4000, Box::new(timer_device))?;
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
            let (usb_device, usb_handle) = Rp2040UsbController::new_with_handle("rp2350.usbctrl");
            bus.map_device("rp2350.usbctrl", 0x5011_0000, 0x4000, Box::new(usb_device))?;
            usb = Some(usb_handle);
            usb_host = Some(Rp2040UsbHost::new());
        }
        let uart_base = match target {
            TargetId::Rp2040 => 0x4003_4000,
            TargetId::Rp2350 => 0x4007_0000,
            _ => unreachable!(),
        };
        let (uart_device, chip_uart) =
            FunctionalUart::new_lenient(format!("{target}.uart0"), 0x00, 0x18, 0x0090);
        bus.map_device(
            format!("{target}.uart0"),
            uart_base,
            0x1000,
            Box::new(uart_device),
        )?;
        Ok(Self {
            target,
            cpu: ArmCpu::new(profile),
            cpu1: ArmCpu::new(profile),
            cpu1_active: false,
            bus,
            signals,
            gpio,
            chip_gpio,
            sio,
            uart,
            chip_uart,
            timer,
            exit,
            now: SimTime::ZERO,
            default_stack: default_stack.expect("Arm target manifests include RAM"),
            flash_base: flash.expect("Arm target manifests include flash").0,
            flash_size: flash.expect("Arm target manifests include flash").1,
            flash_storage: flash_storage.expect("Arm target manifests include flash storage"),
            bootrom_services: BTreeMap::new(),
            native_bootrom: false,
            ppb,
            chip_timers,
            usb,
            usb_dpram,
            usb_host,
        })
    }

    fn service_functional_bootrom(&mut self) -> Result<bool, String> {
        if self.target == TargetId::Rp2040 && self.native_bootrom {
            return Ok(false);
        }
        let pc = self
            .cpu
            .register(ArmRegister::Pc)
            .map_err(|error| error.to_string())?;
        match pc {
            0x20 => {
                let code_register = if self.target == TargetId::Rp2040 {
                    ArmRegister::R1
                } else {
                    ArmRegister::R0
                };
                let code = self
                    .cpu
                    .register(code_register)
                    .map_err(|error| error.to_string())?;
                if self.target == TargetId::Rp2040 && code == 0x4653 {
                    // RP2040 ROM data code "SF". The SDK copies this table and tail-calls its
                    // entries for single-precision scalar operations.
                    self.cpu
                        .complete_host_call(0x0400)
                        .map_err(|error| error.to_string())?;
                    return Ok(true);
                }
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
                self.cpu
                    .complete_host_call(address | 1)
                    .map_err(|error| error.to_string())?;
                Ok(true)
            }
            0x0500..=0x057c if self.target == TargetId::Rp2040 => {
                let left_bits = self
                    .cpu
                    .register(ArmRegister::R0)
                    .map_err(|error| error.to_string())?;
                let right_bits = self
                    .cpu
                    .register(ArmRegister::R1)
                    .map_err(|error| error.to_string())?;
                let left = f32::from_bits(left_bits);
                let right = f32::from_bits(right_bits);
                let offset = pc - 0x0500;
                let result = match offset {
                    0x00 => (left + right).to_bits(),
                    0x04 => (left - right).to_bits(),
                    0x08 => (left * right).to_bits(),
                    0x0c => (left / right).to_bits(),
                    0x18 => left.sqrt().to_bits(),
                    0x1c | 0x20 => (left as i32) as u32,
                    0x24 | 0x28 => left as u32,
                    0x2c | 0x30 => (left_bits as i32 as f32).to_bits(),
                    0x34 | 0x38 => (left_bits as f32).to_bits(),
                    0x3c => left.cos().to_bits(),
                    0x40 => left.sin().to_bits(),
                    0x44 => left.tan().to_bits(),
                    0x4c => left.exp().to_bits(),
                    0x50 => left.ln().to_bits(),
                    0x58 => left.atan2(right).to_bits(),
                    0x7c => {
                        let bits = f64::from(left).to_bits();
                        self.cpu
                            .complete_host_call_with_high(bits as u32, (bits >> 32) as u32)
                            .map_err(|error| error.to_string())?;
                        return Ok(true);
                    }
                    _ => {
                        return Err(format!(
                            "unsupported RP2040 single-precision ROM table offset {offset:#04x}"
                        ));
                    }
                };
                self.cpu
                    .complete_host_call(result)
                    .map_err(|error| error.to_string())?;
                Ok(true)
            }
            address if self.bootrom_services.contains_key(&address) => {
                let code = self.bootrom_services[&address];
                let argument = self
                    .cpu
                    .register(ArmRegister::R0)
                    .map_err(|error| error.to_string())?;
                let result = match code {
                    0x334c => argument.leading_zeros(),
                    0x3350 => argument.count_ones(),
                    0x3352 => argument.reverse_bits(),
                    0x3354 => argument.trailing_zeros(),
                    // CONNECT_INTERNAL_FLASH, FLASH_EXIT_XIP, FLASH_FLUSH_CACHE, and
                    // FLASH_ENTER_CMD_XIP manipulate the physical QSPI path. The functional
                    // model keeps one coherent flash/XIP mapping, so these are ordering points.
                    0x4649 | 0x5845 | 0x4346 | 0x5843 => argument,
                    0x4552 => {
                        let length = self
                            .cpu
                            .register(ArmRegister::R1)
                            .map_err(|error| error.to_string())?;
                        let address = self
                            .flash_base
                            .checked_add(argument)
                            .ok_or_else(|| "RP2040 flash erase address overflow".to_owned())?;
                        self.bus
                            .load(
                                u64::from(address),
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
                            .register(ArmRegister::R1)
                            .map_err(|error| error.to_string())?;
                        let length = self
                            .cpu
                            .register(ArmRegister::R2)
                            .map_err(|error| error.to_string())?;
                        let mut bytes = Vec::with_capacity(
                            usize::try_from(length).map_err(|_| "flash program length overflow")?,
                        );
                        for index in 0..length {
                            bytes.push(
                                self.bus
                                    .read(
                                        u64::from(source.wrapping_add(index)),
                                        AccessWidth::Byte,
                                        AccessKind::Read,
                                        self.now,
                                    )
                                    .map_err(|error| error.to_string())?
                                    as u8,
                            );
                        }
                        let address = self
                            .flash_base
                            .checked_add(argument)
                            .ok_or_else(|| "RP2040 flash program address overflow".to_owned())?;
                        self.bus
                            .load(u64::from(address), &bytes)
                            .map_err(|error| error.to_string())?;
                        argument
                    }
                    0x5347 if self.target == TargetId::Rp2350 => {
                        let capacity = self
                            .cpu
                            .register(ArmRegister::R1)
                            .map_err(|error| error.to_string())?;
                        let flags = self
                            .cpu
                            .register(ArmRegister::R2)
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
                            u32::MAX - 12 // BOOTROM_ERROR_BUFFER_TOO_SMALL (-13)
                        } else {
                            for (index, word) in words.iter().copied().enumerate() {
                                self.bus
                                    .write(
                                        u64::from(argument.wrapping_add(
                                            u32::try_from(index).expect("sys-info index fits u32")
                                                * 4,
                                        )),
                                        AccessWidth::Word,
                                        u64::from(word),
                                        self.now,
                                    )
                                    .map_err(|error| error.to_string())?;
                            }
                            u32::try_from(words.len()).expect("sys-info response is small")
                        }
                    }
                    // The SDK routes its Arm EABI memory helpers through these ROM services.
                    // Each lookup must retain its own identity; one shared trap would make
                    // memcpy behave like whichever ROM function was looked up most recently.
                    0x434d | 0x3443 => {
                        let source = self
                            .cpu
                            .register(ArmRegister::R1)
                            .map_err(|error| error.to_string())?;
                        let length = self
                            .cpu
                            .register(ArmRegister::R2)
                            .map_err(|error| error.to_string())?;
                        for index in 0..length {
                            let byte = self
                                .bus
                                .read(
                                    u64::from(source.wrapping_add(index)),
                                    AccessWidth::Byte,
                                    AccessKind::Read,
                                    self.now,
                                )
                                .map_err(|error| error.to_string())?;
                            self.bus
                                .write(
                                    u64::from(argument.wrapping_add(index)),
                                    AccessWidth::Byte,
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
                            .register(ArmRegister::R1)
                            .map_err(|error| error.to_string())?
                            & 0xff;
                        let length = self
                            .cpu
                            .register(ArmRegister::R2)
                            .map_err(|error| error.to_string())?;
                        for index in 0..length {
                            self.bus
                                .write(
                                    u64::from(argument.wrapping_add(index)),
                                    AccessWidth::Byte,
                                    u64::from(byte),
                                    self.now,
                                )
                                .map_err(|error| error.to_string())?;
                        }
                        argument
                    }
                    // RP2350 REBOOT, BOOTROM_STATE_RESET, SET_BOOTROM_STACK and
                    // FLASH_RESET_ADDRESS_TRANS are lifecycle/ordering operations in the
                    // functional model. A real reset request is surfaced by higher-level run
                    // control rather than destructively replacing the active interpreter.
                    0x4252 | 0x5253 | 0x5353 | 0x4152 if self.target == TargetId::Rp2350 => 0,
                    _ => {
                        return Err(format!(
                            "unsupported {} boot-ROM service code {code:#06x}",
                            self.target
                        ));
                    }
                };
                self.cpu
                    .complete_host_call(result)
                    .map_err(|error| error.to_string())?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Loads an Arm ELF and establishes direct-mode entry state.
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
        let entry =
            u32::try_from(image.entry).map_err(|_| ArmMachineError::EntryRange(image.entry))?;
        if let Some(segment) = image.segments.iter().find(|segment| segment.executable) {
            self.cpu.set_vector_base(
                u32::try_from(segment.address)
                    .map_err(|_| ArmMachineError::EntryRange(segment.address))?,
            );
        }
        self.cpu.set_direct_state(self.default_stack, entry | 1)?;
        Ok(())
    }

    /// Loads a validated Raspberry Pi UF2 image into the target's XIP flash.
    pub fn load_uf2(&mut self, image: &Uf2Image) -> Result<(), ArmMachineError> {
        let expected = match self.target {
            TargetId::Rp2040 => 0xe48b_ff56,
            TargetId::Rp2350 => 0xe48b_ff59,
            _ => return Err(ArmMachineError::UnsupportedTarget(self.target)),
        };
        let actual = image.family_id.unwrap_or_default();
        if actual != expected {
            return Err(ArmMachineError::Uf2Family {
                target: self.target,
                expected,
                actual,
            });
        }
        // Materialization is deliberately performed before mutating the machine. Besides
        // reconstructing erased gaps, this validates every UF2 range against the actual target.
        let _validated_layout = image.materialize(self.flash_base, self.flash_size, 0xff)?;
        for segment in &image.segments {
            self.bus
                .load(u64::from(segment.address), &segment.data)
                .map_err(|error| ArmMachineError::Load {
                    address: u64::from(segment.address),
                    message: error.to_string(),
                })?;
        }
        Ok(())
    }

    /// Replaces the complete persistent XIP flash backing before firmware is
    /// overlaid and booted.
    pub fn set_flash_image(&self, bytes: &[u8]) -> Result<(), ArmMachineError> {
        if bytes.len() != self.flash_size {
            return Err(ArmMachineError::BootVector {
                target: self.target,
                vector_base: self.flash_base,
                message: format!(
                    "persistent flash image is {} bytes; expected {}",
                    bytes.len(),
                    self.flash_size
                ),
            });
        }
        if !self.flash_storage.write_range(0, bytes) {
            return Err(ArmMachineError::BootVector {
                target: self.target,
                vector_base: self.flash_base,
                message: "persistent flash backing rejected a full-image update".to_owned(),
            });
        }
        Ok(())
    }

    /// Copies the complete mutable XIP flash state for persistence.
    pub fn flash_image(&self) -> Vec<u8> {
        self.flash_storage.to_vec()
    }

    /// Installs a complete 16 KiB RP2040 boot-ROM image.
    pub fn load_rp2040_boot_rom(&mut self, image: &[u8]) -> Result<(), ArmMachineError> {
        if self.target != TargetId::Rp2040 || image.len() != 16 * 1024 {
            return Err(ArmMachineError::BootVector {
                target: self.target,
                vector_base: 0,
                message: format!(
                    "RP2040 boot ROM must be exactly 16384 bytes, got {}",
                    image.len()
                ),
            });
        }
        self.bus
            .load(0, image)
            .map_err(|error| ArmMachineError::Load {
                address: 0,
                message: error.to_string(),
            })?;
        self.native_bootrom = true;
        Ok(())
    }

    /// Applies the documented functional RP2040 boot-ROM boundary and starts at the SDK vector
    /// table after the 256-byte second-stage flash bootloader.
    pub fn rp2040_bootrom_handoff(&mut self) -> Result<(), ArmMachineError> {
        if self.target != TargetId::Rp2040 {
            return Err(ArmMachineError::BootHandoffUnsupported(self.target));
        }
        self.boot_vector_at(self.flash_base + 0x100)
    }

    /// Applies the RP2350 authenticated-image boundary after the UF2 has been validated by the
    /// host loader. Official Arm images place their vector table at the start of XIP flash.
    pub fn rp2350_arm_bootrom_handoff(&mut self) -> Result<(), ArmMachineError> {
        if self.target != TargetId::Rp2350 {
            return Err(ArmMachineError::BootHandoffUnsupported(self.target));
        }
        self.boot_vector_at(self.flash_base)
    }

    fn boot_vector_at(&mut self, vector_base: u32) -> Result<(), ArmMachineError> {
        let stack = self
            .bus
            .read(
                u64::from(vector_base),
                AccessWidth::Word,
                AccessKind::Read,
                self.now,
            )
            .map_err(|error| ArmMachineError::BootVector {
                target: self.target,
                vector_base,
                message: error.to_string(),
            })? as u32;
        let entry = self
            .bus
            .read(
                u64::from(vector_base + 4),
                AccessWidth::Word,
                AccessKind::Read,
                self.now,
            )
            .map_err(|error| ArmMachineError::BootVector {
                target: self.target,
                vector_base,
                message: error.to_string(),
            })? as u32;
        let ram_start = 0x2000_0000;
        if stack < ram_start || stack > self.default_stack || stack & 7 != 0 {
            return Err(ArmMachineError::BootVector {
                target: self.target,
                vector_base,
                message: format!("initial SP {stack:#010x} is outside or misaligned"),
            });
        }
        let entry_address = entry & !1;
        let flash_end = self
            .flash_base
            .checked_add(u32::try_from(self.flash_size).expect("RP flash fits u32"))
            .expect("RP flash range fits u32");
        if entry & 1 == 0 || !(self.flash_base..flash_end).contains(&entry_address) {
            return Err(ArmMachineError::BootVector {
                target: self.target,
                vector_base,
                message: format!("reset entry {entry:#010x} is not Thumb code in XIP flash"),
            });
        }
        self.cpu.set_vector_base(vector_base);
        self.cpu.set_direct_state(stack, entry)?;
        Ok(())
    }

    /// Enables or disables completed bus-access recording.
    pub fn set_access_recording(&mut self, enabled: bool) {
        self.bus.set_access_recording(enabled);
    }

    /// Returns completed bus accesses retained for diagnostics.
    pub fn access_log(&self) -> &[BusAccessRecord] {
        self.bus.access_log()
    }

    /// Drives or releases one compiler-facade GPIO pin.
    pub fn set_pin(&self, pin: u8, value: Logic) -> Result<(), ArmMachineError> {
        self.gpio.set_input(pin, value, self.now)?;
        if usize::from(pin) < self.chip_gpio.pin_count() {
            self.chip_gpio.set_input(pin, value, self.now)?;
        }
        Ok(())
    }

    /// Queues bytes for delivery through the enumerated USB bulk-OUT endpoint.
    pub fn queue_usb_input(&mut self, bytes: &[u8]) {
        if let Some(host) = &mut self.usb_host {
            host.queue_input(bytes);
        }
    }

    /// Runs until a limit, exit, breakpoint, or fault.
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
        let mut timer_was_pending = false;
        let reason = loop {
            self.sio.select_core(0);
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
            let chip_timer_pending =
                self.chip_timers
                    .iter()
                    .enumerate()
                    .fold(0_u16, |pending, (timer, handle)| {
                        pending | (u16::from(handle.pending(self.now)) << (timer * 4))
                    });
            self.cpu
                .set_interrupt(0, timer_pending || chip_timer_pending & 1 != 0)?;
            for line in 1..self.chip_timers.len() * 4 {
                self.cpu.set_interrupt(
                    u16::try_from(line).expect("RP timer IRQ line fits u16"),
                    chip_timer_pending & (1 << line) != 0,
                )?;
            }
            if let Some(usb) = &self.usb {
                if let (Some(host), Some(dpram)) = (&mut self.usb_host, &self.usb_dpram) {
                    stats.events = stats.events.saturating_add(host.poll(self.now, usb, dpram));
                }
                let usb_irq: u8 = if self.target == TargetId::Rp2040 {
                    5
                } else {
                    14
                };
                self.cpu.set_interrupt(
                    u16::from(usb_irq),
                    usb.interrupt_pending() && self.ppb.interrupt_enabled(usb_irq),
                )?;
            }
            let vector_base = self.ppb.vector_base();
            if vector_base != 0 {
                self.cpu.set_vector_base(vector_base);
            }
            match self.service_functional_bootrom() {
                Ok(true) => {
                    stats.instructions = stats.instructions.saturating_add(1);
                    self.now = self
                        .now
                        .checked_add(renvo_core::SimDuration::TICK)
                        .map_err(|_| ArmMachineError::TimeOverflow)?;
                    stats.time = self.now;
                    continue;
                }
                Ok(false) => {}
                Err(message) => break StopReason::Fault(message),
            }
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

            if let Some(launch) = self.sio.take_core1_launch() {
                self.cpu1.set_vector_base(launch.vector_table);
                if let Err(error) = self
                    .cpu1
                    .set_direct_state(launch.stack_pointer, launch.entry)
                {
                    break StopReason::Fault(format!("core 1 launch: {error}"));
                }
                if let Err(error) = self.cpu1.set_link_register(0x81) {
                    break StopReason::Fault(format!("core 1 launch: {error}"));
                }
                self.cpu1_active = true;
                stats.events = stats.events.saturating_add(1);
            }
            if self.cpu1_active {
                self.sio.select_core(1);
                // ROM services are shared between both processors. Temporarily
                // place core 1 in the primary slot so the same architectural
                // service implementation can complete its host call.
                std::mem::swap(&mut self.cpu, &mut self.cpu1);
                let core1_rom = self.service_functional_bootrom();
                std::mem::swap(&mut self.cpu, &mut self.cpu1);
                match core1_rom {
                    Ok(true) => {
                        stats.instructions = stats.instructions.saturating_add(1);
                        self.now = self
                            .now
                            .checked_add(renvo_core::SimDuration::TICK)
                            .map_err(|_| ArmMachineError::TimeOverflow)?;
                        stats.time = self.now;
                    }
                    Ok(false) => {
                        let core1_outcome = match self.cpu1.step(&mut self.bus, self.now) {
                            Ok(outcome) => outcome,
                            Err(error) => {
                                self.sio.select_core(0);
                                break StopReason::Fault(format!("core 1: {error}"));
                            }
                        };
                        stats.instructions = stats.instructions.saturating_add(1);
                        self.now = self
                            .now
                            .checked_add(core1_outcome.elapsed)
                            .map_err(|_| ArmMachineError::TimeOverflow)?;
                        stats.time = self.now;
                        match core1_outcome.reason {
                            StepReason::Advanced | StepReason::WaitForInterrupt => {}
                            StepReason::Halted => {
                                self.cpu1_active = false;
                            }
                            StepReason::Breakpoint => {
                                self.sio.select_core(0);
                                break StopReason::Breakpoint;
                            }
                        }
                    }
                    Err(message) => {
                        self.sio.select_core(0);
                        break StopReason::Fault(format!("core 1 ROM: {message}"));
                    }
                }
                self.sio.select_core(0);
                for change in self.signals.drain_changes() {
                    digest.change(&change);
                    if let Some(sink) = trace.as_deref_mut() {
                        sink.change(&change)?;
                    }
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
                bytes.extend(self.chip_uart.bytes());
                bytes
            },
            usb: self
                .usb_host
                .as_ref()
                .map_or_else(Vec::new, |host| host.output.clone()),
            trace_digest: digest.finish(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_raspberry_pi_arm_profiles_construct() {
        ArmMachine::new(TargetId::Rp2040).unwrap();
        ArmMachine::new(TargetId::Rp2350).unwrap();
    }
}
