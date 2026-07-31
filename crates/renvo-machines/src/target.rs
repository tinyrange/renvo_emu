use renvo_image::FirmwareArchitecture;
use serde::Serialize;
use std::fmt;
use std::str::FromStr;

/// Stable identifier for one of the six initial microcontrollers.
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
        baseline: COMMON_BASELINE,
        sources: &[
            "https://www.wch-ic.com/downloads/CH32V003DS0_PDF.html",
            "https://www.wch-ic.com/downloads/CH32V003RM_PDF.html",
            "https://www.wch-ic.com/downloads/QingKeV2_Processor_Manual_PDF.html",
        ],
        limitations: &[
            "exact PFIC nesting and HPE/VTF edge behavior remain approximate",
            "analog and debug-wire behavior is outside the baseline",
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
        baseline: COMMON_BASELINE,
        sources: &[
            "https://www.wch-ic.com/downloads/CH32V006DS0_PDF.html",
            "https://www.wch-ic.com/downloads/CH32V00XRM_PDF.html",
            "https://www.wch-ic.com/downloads/QingKeV2_Processor_Manual_PDF.html",
        ],
        limitations: &[
            "exact PFIC nesting and HPE/VTF edge behavior remain approximate",
            "analog and touch peripherals are outside the baseline",
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
        baseline: COMMON_BASELINE,
        sources: &["https://datasheets.raspberrypi.com/rp2040/rp2040-datasheet.pdf"],
        limitations: &[
            "the initial Thumb interpreter covers the compiler baseline, not the complete ISA",
            "NVIC priority/preemption, USB device fidelity, and exact XIP timing are deferred",
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
        baseline: COMMON_BASELINE,
        sources: &["https://datasheets.raspberrypi.com/rp2350/rp2350-datasheet.pdf"],
        limitations: &[
            "Cortex-M33 and Hazard3 direct-ELF modes implement compiler-facing ISA subsets",
            "TrustZone, HSTX, USB, NVIC priority/preemption, and exact QMI timing are deferred",
        ],
    },
    TargetManifest {
        schema: 1,
        id: TargetId::Esp32s3,
        name: "Espressif ESP32-S3",
        cpus: &[XTENSA_LX7],
        memory: &[
            MemoryRegion {
                name: "dram",
                start: 0x3fc8_0000,
                size: 512 * 1024,
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
        ],
        gpio_count: 49,
        fidelity: Fidelity::Functional,
        baseline: COMMON_BASELINE,
        sources: &[
            "https://www.espressif.com/sites/default/files/documentation/esp32-s3_datasheet_en.pdf",
            "https://www.espressif.com/sites/default/files/documentation/esp32-s3_technical_reference_manual_en.pdf",
        ],
        limitations: &[
            "the initial LX7 interpreter covers a compiler-emitted integer subset",
            "remaining register-window, exception-level, atomic, and FPU cases are corpus-driven",
            "CPU1, wireless, ULP, and full USB are deferred",
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
        baseline: COMMON_BASELINE,
        sources: &[
            "https://www.espressif.com/sites/default/files/documentation/esp32-c6_datasheet_en.pdf",
            "https://www.espressif.com/sites/default/files/documentation/esp32-c6_technical_reference_manual_en.pdf",
        ],
        limitations: &[
            "PMP permission enforcement, complete interrupt-matrix behavior, and vendor cache behavior are not implemented yet",
            "LP core and wireless peripherals are deferred",
        ],
    },
];

/// Returns all six manifests in stable portfolio order.
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
    fn portfolio_has_six_unique_targets_with_primary_sources() {
        assert_eq!(target_manifests().len(), 6);
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
    fn accepted_aliases_render_canonically() {
        assert_eq!(
            "esp32-c6".parse::<TargetId>().unwrap().to_string(),
            "esp32c6"
        );
        assert_eq!("rp2350a".parse::<TargetId>().unwrap(), TargetId::Rp2350);
    }
}
