use remu_image::FirmwareArchitecture;
use serde::Serialize;
use std::fmt;
use std::str::FromStr;

/// Stable identifier for one supported or explicitly planned microcontroller.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TargetId {
    /// WCH CH32V003.
    Ch32v003,
    /// WCH CH32V006.
    Ch32v006,
    /// Raspberry Pi RP2040.
    Rp2040,
    /// Raspberry Pi RP2350A.
    Rp2350,
    /// Espressif ESP32-S3.
    Esp32s3,
    /// Espressif ESP32-C6.
    Esp32c6,
    /// Microchip ATSAMD21E18A.
    Atsamd21e18,
    /// STMicroelectronics STM32L432KC.
    Stm32l432kc,
    /// Renesas R7FA4M1AB3CFM (RA4M1).
    R7fa4m1ab3cfm,
    /// Microchip ATmega328PB.
    Atmega328pb,
    /// Texas Instruments MSP430FR2433.
    Msp430fr2433,
    /// Microchip PIC16F15376.
    Pic16f15376,
    /// Silicon Labs EFM8BB52F32G.
    Efm8bb52f32g,
}

impl TargetId {
    /// Canonical CLI spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ch32v003 => "ch32v003",
            Self::Ch32v006 => "ch32v006",
            Self::Rp2040 => "rp2040",
            Self::Rp2350 => "rp2350",
            Self::Esp32s3 => "esp32s3",
            Self::Esp32c6 => "esp32c6",
            Self::Atsamd21e18 => "atsamd21e18",
            Self::Stm32l432kc => "stm32l432kc",
            Self::R7fa4m1ab3cfm => "r7fa4m1ab3cfm",
            Self::Atmega328pb => "atmega328pb",
            Self::Msp430fr2433 => "msp430fr2433",
            Self::Pic16f15376 => "pic16f15376",
            Self::Efm8bb52f32g => "efm8bb52f32g",
        }
    }
}

impl fmt::Display for TargetId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for TargetId {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "ch32v003" => Ok(Self::Ch32v003),
            "ch32v006" => Ok(Self::Ch32v006),
            "rp2040" => Ok(Self::Rp2040),
            "rp2350" | "rp2350a" => Ok(Self::Rp2350),
            "esp32s3" | "esp32-s3" => Ok(Self::Esp32s3),
            "esp32c6" | "esp32-c6" => Ok(Self::Esp32c6),
            "atsamd21e18" | "samd21e18" => Ok(Self::Atsamd21e18),
            "stm32l432kc" => Ok(Self::Stm32l432kc),
            "r7fa4m1ab3cfm" | "ra4m1" | "uno-r4-minima" => Ok(Self::R7fa4m1ab3cfm),
            "atmega328pb" => Ok(Self::Atmega328pb),
            "msp430fr2433" => Ok(Self::Msp430fr2433),
            "pic16f15376" => Ok(Self::Pic16f15376),
            "efm8bb52f32g" | "efm8bb52f32g-qfn32" => Ok(Self::Efm8bb52f32g),
            _ => Err(format!("unknown target {value:?}")),
        }
    }
}

/// Accuracy promise attached to a modeled surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Fidelity {
    /// Implemented according to the named architecture.
    Architectural,
    /// Deterministic functional approximation.
    Functional,
    /// Deliberately synthetic compiler-testing interface.
    CompilerFacade,
    /// Declared portfolio target whose CPU engine is not implemented yet.
    Planned,
}

/// One selectable application CPU configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct CpuOption {
    /// Stable option name.
    pub name: &'static str,
    /// ELF architecture accepted by direct-load mode.
    pub architecture: FirmwareArchitecture,
    /// Current implementation fidelity.
    pub fidelity: Fidelity,
}

/// Memory role.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    /// On-chip or external non-volatile executable storage.
    Flash,
    /// Read-only boot ROM.
    Rom,
    /// Read/write static RAM.
    Ram,
}

/// Sourced address range needed by direct ELF mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct MemoryRegion {
    /// Region label.
    pub name: &'static str,
    /// First byte address.
    pub start: u64,
    /// Byte size.
    pub size: usize,
    /// Memory role.
    pub kind: MemoryKind,
    /// Whether instruction fetches are accepted.
    pub executable: bool,
}

/// Cumulative public capability tier backed by named qualification evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct SupportTier {
    /// Stable tier identifier.
    pub name: &'static str,
    /// Evidence manifests required to maintain the tier.
    pub evidence: &'static [&'static str],
}

/// Evidence-backed initial support description for one chip.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct TargetManifest {
    /// Manifest schema version.
    pub schema: u16,
    /// Target identity.
    pub id: TargetId,
    /// Display name.
    pub name: &'static str,
    /// Primary application CPU options.
    pub cpus: &'static [CpuOption],
    /// Direct-load memory windows currently represented.
    pub memory: &'static [MemoryRegion],
    /// Physical GPIO count or package-independent maximum described by the vendor.
    pub gpio_count: u8,
    /// Currently promised support tier.
    pub fidelity: Fidelity,
    /// Cumulative public capability tiers and their evidence manifests.
    pub support_tiers: &'static [SupportTier],
    /// Explicitly implemented or planned baseline surface.
    pub baseline: &'static [&'static str],
    /// Primary vendor evidence used to establish the manifest.
    pub sources: &'static [&'static str],
    /// Important current limitations.
    pub limitations: &'static [&'static str],
}

const RISCV_QINGKE: CpuOption = CpuOption {
    name: "qingke-v2-rv32ec",
    architecture: FirmwareArchitecture::RiscV32,
    fidelity: Fidelity::Architectural,
};
const HAZARD3: CpuOption = CpuOption {
    name: "hazard3-rv32imac",
    architecture: FirmwareArchitecture::RiscV32,
    fidelity: Fidelity::Architectural,
};
const CORTEX_M0P: CpuOption = CpuOption {
    name: "cortex-m0plus-armv6m",
    architecture: FirmwareArchitecture::Arm,
    fidelity: Fidelity::Functional,
};
const CORTEX_M33: CpuOption = CpuOption {
    name: "cortex-m33-armv8m",
    architecture: FirmwareArchitecture::Arm,
    fidelity: Fidelity::Functional,
};
const CORTEX_M4F: CpuOption = CpuOption {
    name: "cortex-m4f-armv7em",
    architecture: FirmwareArchitecture::Arm,
    fidelity: Fidelity::Functional,
};
const AVR8: CpuOption = CpuOption {
    name: "avr8-enhanced",
    architecture: FirmwareArchitecture::Avr8,
    fidelity: Fidelity::Functional,
};
const MSP430X: CpuOption = CpuOption {
    name: "msp430-cpuxv2",
    architecture: FirmwareArchitecture::Msp430X,
    fidelity: Fidelity::Functional,
};
const PIC16_ENHANCED: CpuOption = CpuOption {
    name: "pic16-enhanced-midrange",
    architecture: FirmwareArchitecture::Pic16Enhanced,
    fidelity: Fidelity::Functional,
};
const MCS51: CpuOption = CpuOption {
    name: "mcs51-efm8",
    architecture: FirmwareArchitecture::Mcs51,
    fidelity: Fidelity::Functional,
};
const XTENSA_LX7: CpuOption = CpuOption {
    name: "xtensa-lx7",
    architecture: FirmwareArchitecture::Xtensa,
    fidelity: Fidelity::Functional,
};
const ESP_RISCV: CpuOption = CpuOption {
    name: "esp-rv32imac-hp",
    architecture: FirmwareArchitecture::RiscV32,
    fidelity: Fidelity::Architectural,
};

const COMMON_BASELINE: &[&str] = &[
    "direct ELF loading",
    "deterministic interpreted execution",
    "compiler-test exit convention",
    "functional GPIO, timer, and UART facades",
    "external digital pin stimulus",
    "VCD output",
];

const WCH_BASELINE: &[&str] = &[
    "direct ELF loading",
    "deterministic interpreted execution",
    "compiler-test exit convention",
    "functional GPIO, timer, and UART facades",
    "native SPI1 transfer",
    "external digital pin stimulus",
    "VCD output",
];

const WCH_V006_BASELINE: &[&str] = &[
    WCH_BASELINE[0],
    WCH_BASELINE[1],
    WCH_BASELINE[2],
    WCH_BASELINE[3],
    WCH_BASELINE[4],
    WCH_BASELINE[5],
    WCH_BASELINE[6],
    "native ADC/TKEY single-channel conversion sequence",
];

const BASELINE_RISCV_COMPILER_TIER: SupportTier = SupportTier {
    name: "compiler-execution",
    evidence: &["riscv-cpu.json", "rust-abi.json"],
};
const BASELINE_ARM_COMPILER_TIER: SupportTier = SupportTier {
    name: "compiler-execution",
    evidence: &["arm-cpu.json", "rust-abi.json"],
};
const BASELINE_DUAL_CORE_COMPILER_TIER: SupportTier = SupportTier {
    name: "compiler-execution",
    evidence: &["riscv-cpu.json", "arm-cpu.json", "rust-abi.json"],
};
const BASELINE_XTENSA_COMPILER_TIER: SupportTier = SupportTier {
    name: "compiler-execution",
    evidence: &["xtensa-cpu.json", "rust-abi.json"],
};
const BASELINE_FIRMWARE_TIER: SupportTier = SupportTier {
    name: "firmware-functional-slice",
    evidence: &[
        "vendor-samples.json",
        "register-coverage/",
        "stop-conditions.json",
    ],
};
const BASELINE_BOARD_TIER: SupportTier = SupportTier {
    name: "board-or-sdk-workflow",
    evidence: &[
        "starlark.json",
        "native-images.json",
        "acceptance-report.html",
    ],
};

const BASELINE_RISCV_SUPPORT_TIERS: &[SupportTier] = &[
    BASELINE_RISCV_COMPILER_TIER,
    BASELINE_FIRMWARE_TIER,
    BASELINE_BOARD_TIER,
];
const BASELINE_ARM_SUPPORT_TIERS: &[SupportTier] = &[
    BASELINE_ARM_COMPILER_TIER,
    BASELINE_FIRMWARE_TIER,
    BASELINE_BOARD_TIER,
];
const BASELINE_DUAL_CORE_SUPPORT_TIERS: &[SupportTier] = &[
    BASELINE_DUAL_CORE_COMPILER_TIER,
    BASELINE_FIRMWARE_TIER,
    BASELINE_BOARD_TIER,
];
const BASELINE_XTENSA_SUPPORT_TIERS: &[SupportTier] = &[
    BASELINE_XTENSA_COMPILER_TIER,
    BASELINE_FIRMWARE_TIER,
    BASELINE_BOARD_TIER,
];

const EXPANSION_SUPPORT_TIERS: &[SupportTier] = &[
    SupportTier {
        name: "compiler-execution",
        evidence: &["expansion/summary.json"],
    },
    SupportTier {
        name: "firmware-functional-slice",
        evidence: &["expansion/summary.json", "register-coverage/"],
    },
];

const MANIFESTS: &[TargetManifest] = &[
    TargetManifest {
        schema: 1,
        id: TargetId::Ch32v003,
        name: "WCH CH32V003",
        cpus: &[RISCV_QINGKE],
        memory: &[
            MemoryRegion {
                name: "flash",
                start: 0,
                size: 16 * 1024,
                kind: MemoryKind::Flash,
                executable: true,
            },
            MemoryRegion {
                name: "sram",
                start: 0x2000_0000,
                size: 2 * 1024,
                kind: MemoryKind::Ram,
                executable: true,
            },
        ],
        gpio_count: 18,
        fidelity: Fidelity::Functional,
        support_tiers: BASELINE_RISCV_SUPPORT_TIERS,
        baseline: WCH_BASELINE,
        sources: &[
            "https://www.wch-ic.com/downloads/CH32V003DS0_PDF.html",
            "https://www.wch-ic.com/downloads/CH32V003RM_PDF.html",
            "https://www.wch-ic.com/downloads/QingKeV2_Processor_Manual_PDF.html",
        ],
        limitations: &[
            "exact PFIC nesting and HPE/VTF edge behavior remain approximate",
            "CH32V003 has no native TKEY block; ADC-aided capacitive touch is outside the baseline",
            "analog and debug-wire behavior is otherwise outside the baseline",
        ],
    },
    TargetManifest {
        schema: 1,
        id: TargetId::Ch32v006,
        name: "WCH CH32V006",
        cpus: &[RISCV_QINGKE],
        memory: &[
            MemoryRegion {
                name: "flash",
                start: 0,
                size: 64 * 1024,
                kind: MemoryKind::Flash,
                executable: true,
            },
            MemoryRegion {
                name: "sram",
                start: 0x2000_0000,
                size: 8 * 1024,
                kind: MemoryKind::Ram,
                executable: true,
            },
        ],
        gpio_count: 24,
        fidelity: Fidelity::Functional,
        support_tiers: BASELINE_RISCV_SUPPORT_TIERS,
        baseline: WCH_V006_BASELINE,
        sources: &[
            "https://www.wch-ic.com/downloads/CH32V006DS0_PDF.html",
            "https://www.wch-ic.com/downloads/CH32V00XRM_PDF.html",
            "https://www.wch-ic.com/downloads/QingKeV2_Processor_Manual_PDF.html",
        ],
        limitations: &[
            "exact PFIC nesting and HPE/VTF edge behavior remain approximate",
            "USART1 and USART2 are transmit-focused slices without physical baud timing or a complete receive path",
            "TKEY uses deterministic host-provided samples rather than analogue capacitance physics",
            "ADC DMA, scan/injection groups, watchdogs and exact HBCLK timing remain outside the baseline",
            "TIM3 streamlined mode covers deterministic internal-clock counting and compare/DMA event observation; timer cascade, center-alignment, preload transfers, waveform pins, and exact clock timing are deferred",
        ],
    },
    TargetManifest {
        schema: 1,
        id: TargetId::Rp2040,
        name: "Raspberry Pi RP2040",
        cpus: &[CORTEX_M0P],
        memory: &[
            MemoryRegion {
                name: "xip",
                start: 0x1000_0000,
                size: 16 * 1024 * 1024,
                kind: MemoryKind::Flash,
                executable: true,
            },
            MemoryRegion {
                name: "sram",
                start: 0x2000_0000,
                size: 264 * 1024,
                kind: MemoryKind::Ram,
                executable: true,
            },
        ],
        gpio_count: 30,
        fidelity: Fidelity::Functional,
        support_tiers: BASELINE_ARM_SUPPORT_TIERS,
        baseline: COMMON_BASELINE,
        sources: &["https://datasheets.raspberrypi.com/rp2040/rp2040-datasheet.pdf"],
        limitations: &[
            "the initial Thumb interpreter covers the compiler baseline, not the complete ISA",
            "NVIC priority/preemption, USB PHY/protocol completeness, and exact XIP timing are deferred",
        ],
    },
    TargetManifest {
        schema: 1,
        id: TargetId::Rp2350,
        name: "Raspberry Pi RP2350A",
        cpus: &[CORTEX_M33, HAZARD3],
        memory: &[
            MemoryRegion {
                name: "xip",
                start: 0x1000_0000,
                size: 16 * 1024 * 1024,
                kind: MemoryKind::Flash,
                executable: true,
            },
            MemoryRegion {
                name: "sram",
                start: 0x2000_0000,
                size: 520 * 1024,
                kind: MemoryKind::Ram,
                executable: true,
            },
        ],
        gpio_count: 48,
        fidelity: Fidelity::Functional,
        support_tiers: BASELINE_DUAL_CORE_SUPPORT_TIERS,
        baseline: COMMON_BASELINE,
        sources: &["https://datasheets.raspberrypi.com/rp2350/rp2350-datasheet.pdf"],
        limitations: &[
            "Cortex-M33 and Hazard3 direct-ELF modes implement compiler-facing ISA subsets",
            "RP2350 IO_BANK0 provides functional STATUS/CTRL overrides and PROC0/PROC1 interrupt registers for the active first 32 GPIOs; pad electrical muxing, secure routing, and GPIO32-47 connections are deferred",
            "TrustZone, HSTX, USB, NVIC priority/preemption, and exact QMI timing are deferred",
            "SPI0/SPI1 use deterministic PrimeCell FIFO transfers without serial-clock, DMA, or exact slave timing",
        ],
    },
    TargetManifest {
        schema: 1,
        id: TargetId::Esp32s3,
        name: "Espressif ESP32-S3",
        cpus: &[XTENSA_LX7],
        memory: &[
            MemoryRegion {
                name: "rom",
                start: 0x4000_0000,
                size: 384 * 1024,
                kind: MemoryKind::Rom,
                executable: true,
            },
            MemoryRegion {
                name: "dram",
                start: 0x3fc8_8000,
                size: 480 * 1024,
                kind: MemoryKind::Ram,
                executable: false,
            },
            MemoryRegion {
                name: "iram",
                start: 0x4037_0000,
                size: 512 * 1024,
                kind: MemoryKind::Ram,
                executable: true,
            },
            MemoryRegion {
                name: "irom",
                start: 0x4200_0000,
                size: 16 * 1024 * 1024,
                kind: MemoryKind::Flash,
                executable: true,
            },
            MemoryRegion {
                name: "psram-octal",
                start: 0x3d00_0000,
                size: 8 * 1024 * 1024,
                kind: MemoryKind::Ram,
                executable: true,
            },
        ],
        gpio_count: 49,
        fidelity: Fidelity::Functional,
        support_tiers: BASELINE_XTENSA_SUPPORT_TIERS,
        baseline: COMMON_BASELINE,
        sources: &[
            "https://www.espressif.com/sites/default/files/documentation/esp32-s3_datasheet_en.pdf",
            "https://www.espressif.com/sites/default/files/documentation/esp32-s3_technical_reference_manual_en.pdf",
        ],
        limitations: &[
            "the LX7 interpreter is a compiler-baseline subset, not a complete Xtensa ISA model",
            "windowed ABI, level-one exceptions, S32C1I atomics, and FPv5 compiler cases are tested without claiming cycle accuracy",
            "interrupt priority/nesting, exact SMP timing, wireless, full USB, and PSRAM initialization timing are deferred",
        ],
    },
    TargetManifest {
        schema: 1,
        id: TargetId::Esp32c6,
        name: "Espressif ESP32-C6",
        cpus: &[ESP_RISCV],
        memory: &[
            MemoryRegion {
                name: "rom",
                start: 0x4000_0000,
                size: 320 * 1024,
                kind: MemoryKind::Rom,
                executable: true,
            },
            MemoryRegion {
                name: "hp-sram",
                start: 0x4080_0000,
                size: 512 * 1024,
                kind: MemoryKind::Ram,
                executable: true,
            },
            MemoryRegion {
                name: "irom",
                start: 0x4200_0000,
                size: 16 * 1024 * 1024,
                kind: MemoryKind::Flash,
                executable: true,
            },
            MemoryRegion {
                name: "lp-sram",
                start: 0x5000_0000,
                size: 16 * 1024,
                kind: MemoryKind::Ram,
                executable: true,
            },
        ],
        gpio_count: 31,
        fidelity: Fidelity::Functional,
        support_tiers: BASELINE_RISCV_SUPPORT_TIERS,
        baseline: COMMON_BASELINE,
        sources: &[
            "https://www.espressif.com/sites/default/files/documentation/esp32-c6_datasheet_en.pdf",
            "https://www.espressif.com/sites/default/files/documentation/esp32-c6_technical_reference_manual_en.pdf",
        ],
        limitations: &[
            "PMU, cache, interrupt and watchdog timing is instruction ordered rather than cycle accurate",
            "Trace capture, analog physics, full monitor/APM policy engines and wireless peripherals are deferred",
        ],
    },
    TargetManifest {
        schema: 1,
        id: TargetId::Atsamd21e18,
        name: "Microchip ATSAMD21E18A",
        cpus: &[CORTEX_M0P],
        memory: &[
            MemoryRegion {
                name: "flash",
                start: 0x0000_0000,
                size: 256 * 1024,
                kind: MemoryKind::Flash,
                executable: true,
            },
            MemoryRegion {
                name: "sram",
                start: 0x2000_0000,
                size: 32 * 1024,
                kind: MemoryKind::Ram,
                executable: true,
            },
        ],
        gpio_count: 26,
        fidelity: Fidelity::Functional,
        support_tiers: EXPANSION_SUPPORT_TIERS,
        baseline: &[
            "PORT/EIC GPIO",
            "TC3 timer interrupt",
            "SERCOM0 USART/SPI/I2C host slice",
            "EVSYS channel/user mux and software event",
            "USB device control and endpoint register slice",
            "DMAC software-triggered descriptor transfer",
            "I2S clock/serializer/data slice",
            "ADC software-triggered conversion",
            "AC comparator pair",
            "VCD",
        ],
        sources: &[
            "https://www.microchip.com/en-us/product/ATSAMD21E18",
            "https://packs.download.microchip.com/Microchip.SAMD21_DFP.3.8.270.atpack",
            "https://ww1.microchip.com/downloads/en/DeviceDoc/SAM_D21_DA1_Family%20Data%20Sheet_DS40001882E.pdf",
            "https://raw.githubusercontent.com/arduino/ArduinoCore-samd/master/bootloaders/mzero/Bootloader_D21/src/ASF/sam0/utils/cmsis/samd21/include/component/sercom.h",
            "https://raw.githubusercontent.com/arduino/ArduinoCore-samd/master/bootloaders/mzero/Bootloader_D21/src/ASF/sam0/utils/cmsis/samd21/include/instance/sercom0.h",
            "https://raw.githubusercontent.com/arduino/ArduinoCore-samd/master/bootloaders/mzero/Bootloader_D21/src/ASF/sam0/utils/cmsis/samd21/include/component/adc.h",
            "https://raw.githubusercontent.com/arduino/ArduinoCore-samd/master/bootloaders/mzero/Bootloader_D21/src/ASF/sam0/utils/cmsis/samd21/include/component/ac.h",
        ],
        limitations: &[
            "ADC and AC inputs are deterministic host codes rather than analog voltages",
            "full USB packet protocol and USB descriptor DMA, DMAC linked descriptors/peripheral/event trigger routing/CRC, I2S serial timing/framing/pin waveforms/DMAC coupling, ADC averaging/event-DMA coupling, AC filtering/window electrical behavior, live peripheral event-generator/user routing, and exact clock timing are outside this tranche",
            "SERCOM transfers are functional and deterministic; pin-level bus timing and full slave behavior remain deferred",
        ],
    },
    TargetManifest {
        schema: 1,
        id: TargetId::Stm32l432kc,
        name: "STMicroelectronics STM32L432KC",
        cpus: &[CORTEX_M4F],
        memory: &[
            MemoryRegion {
                name: "flash",
                start: 0x0800_0000,
                size: 256 * 1024,
                kind: MemoryKind::Flash,
                executable: true,
            },
            MemoryRegion {
                name: "sram",
                start: 0x2000_0000,
                size: 64 * 1024,
                kind: MemoryKind::Ram,
                executable: true,
            },
        ],
        gpio_count: 26,
        fidelity: Fidelity::Functional,
        support_tiers: EXPANSION_SUPPORT_TIERS,
        baseline: &["GPIO/EXTI", "TIM2 interrupt", "USART2", "VCD"],
        sources: &[
            "https://www.st.com/en/microcontrollers-microprocessors/stm32l432kc.html",
            "https://www.st.com/resource/en/reference_manual/rm0394-stm32l41xxx42xxx43xxx44xxx45xxx46xxx-advanced-armbased-32bit-mcus-stmicroelectronics.pdf",
        ],
        limitations: &["analog, USB, DMA, low-power fidelity, and exact clocks are deferred"],
    },
    TargetManifest {
        schema: 1,
        id: TargetId::R7fa4m1ab3cfm,
        name: "Renesas R7FA4M1AB3CFM#AA0",
        cpus: &[CORTEX_M4F],
        memory: &[
            MemoryRegion {
                name: "code-flash",
                start: 0x0000_0000,
                size: 256 * 1024,
                kind: MemoryKind::Flash,
                executable: true,
            },
            MemoryRegion {
                name: "sram",
                start: 0x2000_0000,
                size: 32 * 1024,
                kind: MemoryKind::Ram,
                executable: true,
            },
            MemoryRegion {
                name: "data-flash",
                start: 0x4010_0000,
                size: 8 * 1024,
                kind: MemoryKind::Flash,
                executable: false,
            },
        ],
        gpio_count: 49,
        fidelity: Fidelity::Functional,
        support_tiers: EXPANSION_SUPPORT_TIERS,
        baseline: &["IOPORT/ICU", "GPT0 interrupt", "SCI9 UART", "VCD"],
        sources: &[
            "https://www.renesas.com/en/document/mah/renesas-ra4m1-group-users-manual-hardware",
            "https://github.com/arduino/ArduinoCore-renesas",
        ],
        limitations: &["USB, CAN, analog, LCD, and Wi-Fi-board bridge behavior are deferred"],
    },
    TargetManifest {
        schema: 1,
        id: TargetId::Atmega328pb,
        name: "Microchip ATmega328PB",
        cpus: &[AVR8],
        memory: &[
            MemoryRegion {
                name: "program-flash",
                start: 0,
                size: 32 * 1024,
                kind: MemoryKind::Flash,
                executable: true,
            },
            MemoryRegion {
                name: "data-sram",
                start: 0x0100,
                size: 2 * 1024,
                kind: MemoryKind::Ram,
                executable: false,
            },
        ],
        gpio_count: 27,
        fidelity: Fidelity::Functional,
        support_tiers: EXPANSION_SUPPORT_TIERS,
        baseline: &[
            "PORT/interrupt GPIO",
            "Timer0/1/2/3/4",
            "USART0/1 transmit",
            "SPI0/1",
            "TWI0/1",
            "ADC",
            "EEPROM",
            "analog comparator",
            "SMCR/CLKPR sleep and clock control",
            "VCD",
        ],
        sources: &[
            "https://ww1.microchip.com/downloads/en/DeviceDoc/Microchip-AVR-Microcontroller-ATmega328PB-Data-Sheet-DS40001906.pdf",
            "https://www.microchip.com/content/dam/mchp/documents/MCU08/ProductDocuments/DataSheets/40001906C.pdf",
            "https://packs.download.microchip.com/Microchip.ATmega_DFP.3.6.299.atpack",
        ],
        limitations: &[
            "comparator timing is functional rather than clock-accurate",
            "Timer3/4 PWM and input-capture waveform fidelity are deferred",
            "peripheral clock domains, exact low-power wake behavior, and oscillator timing are deferred",
            "PTC analog acquisition and SPM/boot-lock/fuse persistence are explicitly unsupported",
        ],
    },
    TargetManifest {
        schema: 1,
        id: TargetId::Msp430fr2433,
        name: "Texas Instruments MSP430FR2433",
        cpus: &[MSP430X],
        memory: &[
            MemoryRegion {
                name: "fram",
                start: 0x0000_c000,
                size: 16 * 1024,
                kind: MemoryKind::Flash,
                executable: true,
            },
            MemoryRegion {
                name: "sram",
                start: 0x0000_2000,
                size: 4 * 1024,
                kind: MemoryKind::Ram,
                executable: true,
            },
        ],
        gpio_count: 19,
        fidelity: Fidelity::Functional,
        support_tiers: EXPANSION_SUPPORT_TIERS,
        baseline: &[
            "GPIO/low-power wake",
            "Timer_A",
            "eUSCI_A0 UART",
            "FRAM",
            "VCD",
        ],
        sources: &[
            "https://www.ti.com/lit/ds/symlink/msp430fr2433.pdf",
            "https://www.ti.com/lit/ug/slau445/slau445.pdf",
        ],
        limitations: &["ADC, capacitive touch, BSL, and clock accuracy are deferred"],
    },
    TargetManifest {
        schema: 1,
        id: TargetId::Pic16f15376,
        name: "Microchip PIC16F15376",
        cpus: &[PIC16_ENHANCED],
        memory: &[
            MemoryRegion {
                name: "program-words-14bit",
                start: 0,
                size: 16 * 1024,
                kind: MemoryKind::Flash,
                executable: true,
            },
            MemoryRegion {
                name: "data-ram",
                start: 0,
                size: 2 * 1024,
                kind: MemoryKind::Ram,
                executable: false,
            },
        ],
        gpio_count: 36,
        fidelity: Fidelity::Functional,
        support_tiers: EXPANSION_SUPPORT_TIERS,
        baseline: &[
            "PORT/PPS GPIO",
            "Timer0/1",
            "EUSART",
            "interrupt routing",
            "VCD",
        ],
        sources: &[
            "https://www.microchip.com/en-us/product/PIC16F15376",
            "https://packs.download.microchip.com/Microchip.PIC16F1xxxx_DFP.1.31.465.atpack",
        ],
        limitations: &["analog, CLC, NCO, PWM, and exact oscillator timing are deferred"],
    },
    TargetManifest {
        schema: 1,
        id: TargetId::Efm8bb52f32g,
        name: "Silicon Labs EFM8BB52F32G-QFN32",
        cpus: &[MCS51],
        memory: &[
            MemoryRegion {
                name: "code-flash",
                start: 0,
                size: 32 * 1024,
                kind: MemoryKind::Flash,
                executable: true,
            },
            MemoryRegion {
                name: "xram",
                start: 0,
                size: 2304,
                kind: MemoryKind::Ram,
                executable: false,
            },
        ],
        gpio_count: 29,
        fidelity: Fidelity::Functional,
        support_tiers: EXPANSION_SUPPORT_TIERS,
        baseline: &[
            "crossbar GPIO",
            "crossbar priority/skip assignment",
            "Timer0/1/2/3/4/5",
            "UART0/1",
            "SPI0 functional transfers",
            "SMBus0 functional transfers",
            "ADC0 deterministic conversion",
            "DAC0 digital-code output",
            "CMP0/1 deterministic comparison",
            "CLU0-3 LUT and edge logic",
            "P0/1/2 port-match interrupt",
            "keyed 32 KiB flash program/erase",
            "PCA0 PWM/capture/compare",
            "clock source/divider and CPU power controls",
            "interrupt priority",
            "VCD",
        ],
        sources: &[
            "https://www.silabs.com/documents/public/data-sheets/efm8bb52-datasheet.pdf",
            "https://www.silabs.com/documents/public/reference-manuals/efm8bb52-rm.pdf",
        ],
        limitations: &[
            "UART1 FIFO thresholds, LIN, flow control, analog electrical physics, additional serial modes, and historical 8051 machine timing are deferred",
            "oscillator settling, missing-clock detection, external-clock waveforms, peripheral clock domains, and exact low-power timing are functional approximations",
        ],
    },
];

/// Returns all manifests in stable portfolio order.
pub const fn target_manifests() -> &'static [TargetManifest] {
    MANIFESTS
}

/// Returns the manifest for `target`.
pub fn target_manifest(target: TargetId) -> &'static TargetManifest {
    MANIFESTS
        .iter()
        .find(|manifest| manifest.id == target)
        .expect("every TargetId has one manifest")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portfolio_has_thirteen_unique_targets_with_primary_sources() {
        assert_eq!(target_manifests().len(), 13);
        for (index, manifest) in target_manifests().iter().enumerate() {
            assert!(!manifest.sources.is_empty());
            assert!(
                !target_manifests()[..index]
                    .iter()
                    .any(|earlier| earlier.id == manifest.id)
            );
        }
    }

    #[test]
    fn every_target_has_named_support_tier_evidence() {
        for manifest in target_manifests() {
            assert!(
                !manifest.support_tiers.is_empty(),
                "{} has no tiers",
                manifest.id
            );
            for tier in manifest.support_tiers {
                assert!(!tier.name.is_empty(), "{} has an unnamed tier", manifest.id);
                assert!(
                    !tier.evidence.is_empty(),
                    "{} tier {} has no evidence",
                    manifest.id,
                    tier.name
                );
                assert!(
                    tier.evidence.iter().all(|path| !path.is_empty()),
                    "{} tier {} has empty evidence",
                    manifest.id,
                    tier.name
                );
            }
        }
    }

    #[test]
    fn compiler_support_evidence_matches_declared_cpu_architecture() {
        for manifest in target_manifests() {
            let expected: &[&str] = match manifest.id {
                TargetId::Ch32v003 | TargetId::Ch32v006 | TargetId::Esp32c6 => {
                    &["riscv-cpu.json", "rust-abi.json"]
                }
                TargetId::Rp2040 => &["arm-cpu.json", "rust-abi.json"],
                TargetId::Rp2350 => &["riscv-cpu.json", "arm-cpu.json", "rust-abi.json"],
                TargetId::Esp32s3 => &["xtensa-cpu.json", "rust-abi.json"],
                _ => &["expansion/summary.json"],
            };
            let compiler_tier = manifest
                .support_tiers
                .iter()
                .find(|tier| tier.name == "compiler-execution")
                .expect("every target declares a compiler-execution tier");
            assert_eq!(compiler_tier.evidence, expected, "{}", manifest.id);
        }
    }

    #[test]
    fn accepted_aliases_render_canonically() {
        assert_eq!(
            "esp32-c6".parse::<TargetId>().unwrap().to_string(),
            "esp32c6"
        );
        assert_eq!("rp2350a".parse::<TargetId>().unwrap(), TargetId::Rp2350);
        assert_eq!(
            "uno-r4-minima".parse::<TargetId>().unwrap(),
            TargetId::R7fa4m1ab3cfm
        );
    }
}
