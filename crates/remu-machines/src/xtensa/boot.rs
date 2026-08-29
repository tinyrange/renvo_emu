use super::*;
use remu_image::{EspExecutableImage, EspImageSegment};
use serde::Serialize;

const ESP32S3_CHIP_ID: u16 = 9;
const CACHE_PAGE_SIZE: u32 = 64 * 1024;
const DRAM_START: u32 = 0x3fc8_8000;
const DRAM_END: u32 = 0x3fd0_0000;
const IRAM_START: u32 = 0x4037_0000;
const IRAM_END: u32 = 0x403f_0000;
const DROM_START: u32 = 0x3c00_0000;
const DROM_END: u32 = 0x3d00_0000;
const IROM_START: u32 = 0x4200_0000;
const IROM_END: u32 = 0x4300_0000;
const RTC_MEMORY_START: u32 = 0x5000_0000;
const RTC_MEMORY_END: u32 = 0x5000_2000;
const RTC_FAST_MEMORY_START: u32 = 0x600f_e000;
const RTC_FAST_MEMORY_END: u32 = 0x6010_0000;

/// How one validated ESP32-S3 image segment reaches the CPU address space.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Esp32S3BootSegmentKind {
    /// The ROM copies second-stage bootloader bytes into internal data RAM.
    BootloaderDram,
    /// The ROM copies second-stage bootloader bytes into internal instruction RAM.
    BootloaderIram,
    /// The second-stage bootloader copies application bytes into internal data RAM.
    ApplicationDram,
    /// The second-stage bootloader copies application bytes into internal instruction RAM.
    ApplicationIram,
    /// The second-stage bootloader maps application data directly from SPI flash.
    ApplicationDrom,
    /// The second-stage bootloader maps application instructions directly from SPI flash.
    ApplicationIrom,
    /// The second-stage bootloader copies retained application bytes into RTC memory.
    ApplicationRtcMemory,
    /// Zero-filled esptool segment used only to align a following mapped segment.
    ApplicationPadding,
}

/// One segment in the validated functional ESP32-S3 flash-boot plan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Esp32S3BootSegment {
    /// ROM or application stage responsible for the segment.
    pub kind: Esp32S3BootSegmentKind,
    /// CPU-visible load or mapping address.
    pub address: u32,
    /// First byte of the segment payload in SPI flash.
    pub flash_offset: u32,
    /// Segment payload length.
    pub size: usize,
}

/// One cache-MMU mapping reconstructed by the functional second-stage bootloader.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Esp32S3BootMapping {
    /// Absolute DROM or IROM address of the first mapped virtual page.
    pub virtual_page_address: u32,
    /// Byte offset in SPI flash of the first mapped physical page.
    pub flash_page_offset: u32,
    /// First shared cache-MMU table entry used by this mapping.
    pub table_index: usize,
    /// Number of consecutive 64-KiB pages in the mapping.
    pub page_count: usize,
}

/// Observable stages and addresses produced by ESP32-S3 functional flash boot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Esp32S3BootReport {
    /// Schema identifier for stable qualification consumers.
    pub schema: &'static str,
    /// Validated second-stage bootloader entry point.
    pub bootloader_entry: u32,
    /// Selected application entry point.
    pub application_entry: u32,
    /// Selected application partition label.
    pub application_partition: String,
    /// Selected application partition byte offset in SPI flash.
    pub application_partition_offset: u32,
    /// Ordered bootloader and application segment operations.
    pub segments: Vec<Esp32S3BootSegment>,
    /// Cache-MMU mappings reconstructed for DROM and IROM segments.
    pub mappings: Vec<Esp32S3BootMapping>,
    /// Ordered functional stages completed before application handoff.
    pub stages: [&'static str; 5],
}

pub(super) fn checked_flash_bytes<'a>(
    segment: &EspImageSegment,
    flash: &'a [u8],
) -> Result<&'a [u8], XtensaMachineError> {
    let start = usize::try_from(segment.flash_offset).map_err(|_| XtensaMachineError::Load {
        address: u64::from(segment.address),
        message: "ESP segment flash offset does not fit the host address space".to_owned(),
    })?;
    let end = start
        .checked_add(segment.data.len())
        .ok_or_else(|| XtensaMachineError::Load {
            address: u64::from(segment.address),
            message: "ESP segment flash range overflows the host address space".to_owned(),
        })?;
    let bytes = flash
        .get(start..end)
        .ok_or_else(|| XtensaMachineError::Load {
            address: u64::from(segment.address),
            message: format!(
                "ESP segment flash range {start:#x}..{end:#x} exceeds simulated flash size {:#x}",
                flash.len()
            ),
        })?;
    if bytes != segment.data {
        return Err(XtensaMachineError::Load {
            address: u64::from(segment.address),
            message: format!(
                "parsed ESP segment at flash offset {start:#x} differs from persistent flash state"
            ),
        });
    }
    Ok(bytes)
}

fn address_range_contains(start: u32, end: u32, address: u32, size: usize) -> bool {
    u32::try_from(size)
        .ok()
        .and_then(|size| address.checked_add(size))
        .is_some_and(|segment_end| address >= start && segment_end <= end)
}

pub(super) fn classify_segment(
    segment: &EspImageSegment,
    bootloader: bool,
) -> Result<Esp32S3BootSegmentKind, XtensaMachineError> {
    let range =
        |start, end| address_range_contains(start, end, segment.address, segment.data.len());
    let kind = if !bootloader
        && segment.address == 0
        && !segment.data.is_empty()
        && segment.data.iter().all(|byte| *byte == 0)
    {
        Esp32S3BootSegmentKind::ApplicationPadding
    } else if range(DRAM_START, DRAM_END) {
        if bootloader {
            Esp32S3BootSegmentKind::BootloaderDram
        } else {
            Esp32S3BootSegmentKind::ApplicationDram
        }
    } else if range(IRAM_START, IRAM_END) {
        if bootloader {
            Esp32S3BootSegmentKind::BootloaderIram
        } else {
            Esp32S3BootSegmentKind::ApplicationIram
        }
    } else if !bootloader && range(DROM_START, DROM_END) {
        Esp32S3BootSegmentKind::ApplicationDrom
    } else if !bootloader && range(IROM_START, IROM_END) {
        Esp32S3BootSegmentKind::ApplicationIrom
    } else if !bootloader
        && (range(RTC_MEMORY_START, RTC_MEMORY_END)
            || range(RTC_FAST_MEMORY_START, RTC_FAST_MEMORY_END))
    {
        Esp32S3BootSegmentKind::ApplicationRtcMemory
    } else {
        return Err(XtensaMachineError::Load {
            address: u64::from(segment.address),
            message: if bootloader {
                "ESP32-S3 second-stage bootloader segment is outside internal DRAM/IRAM".to_owned()
            } else {
                "ESP32-S3 application segment is outside internal RAM and DROM/IROM windows"
                    .to_owned()
            },
        });
    };
    Ok(kind)
}

fn entry_is_executable(image: &EspExecutableImage, entry: u32, bootloader: bool) -> bool {
    image.segments.iter().any(|segment| {
        let Ok(kind) = classify_segment(segment, bootloader) else {
            return false;
        };
        matches!(
            kind,
            Esp32S3BootSegmentKind::BootloaderIram
                | Esp32S3BootSegmentKind::ApplicationIram
                | Esp32S3BootSegmentKind::ApplicationIrom
                | Esp32S3BootSegmentKind::ApplicationRtcMemory
        ) && u32::try_from(segment.data.len())
            .ok()
            .and_then(|size| segment.address.checked_add(size))
            .is_some_and(|end| (segment.address..end).contains(&entry))
    })
}

pub(super) fn segment_mapping(
    segment: &EspImageSegment,
    kind: Esp32S3BootSegmentKind,
) -> Result<Option<Esp32S3BootMapping>, XtensaMachineError> {
    let virtual_base = match kind {
        Esp32S3BootSegmentKind::ApplicationDrom => DROM_START,
        Esp32S3BootSegmentKind::ApplicationIrom => IROM_START,
        _ => return Ok(None),
    };
    let address_offset = segment.address % CACHE_PAGE_SIZE;
    let flash_page =
        segment
            .flash_offset
            .checked_sub(address_offset)
            .ok_or(XtensaMachineError::Load {
                address: u64::from(segment.address),
                message: "ESP mapped segment flash/virtual offsets disagree".to_owned(),
            })?;
    if flash_page % CACHE_PAGE_SIZE != 0 {
        return Err(XtensaMachineError::Load {
            address: u64::from(segment.address),
            message: "ESP mapped segment is not cache-page congruent".to_owned(),
        });
    }
    let span = address_offset
        .checked_add(
            u32::try_from(segment.data.len()).map_err(|_| XtensaMachineError::Load {
                address: u64::from(segment.address),
                message: "ESP mapped segment length exceeds u32".to_owned(),
            })?,
        )
        .ok_or(XtensaMachineError::Load {
            address: u64::from(segment.address),
            message: "ESP mapped segment span overflow".to_owned(),
        })?;
    Ok(Some(Esp32S3BootMapping {
        virtual_page_address: segment.address - address_offset,
        flash_page_offset: flash_page,
        table_index: usize::try_from(
            (segment.address - virtual_base - address_offset) / CACHE_PAGE_SIZE,
        )
        .expect("ESP32-S3 MMU index fits usize"),
        page_count: usize::try_from(span.div_ceil(CACHE_PAGE_SIZE))
            .expect("ESP32-S3 page count fits usize"),
    }))
}

/// Validates and describes the functional ESP32-S3 ROM/second-stage boot pipeline.
pub fn plan_esp32s3_boot(
    image: &EspFlashImage,
    flash: &[u8],
) -> Result<Esp32S3BootReport, XtensaMachineError> {
    if image.bootloader.header.chip_id != ESP32S3_CHIP_ID
        || image.application.header.chip_id != ESP32S3_CHIP_ID
    {
        return Err(XtensaMachineError::Load {
            address: 0,
            message: "ESP flash bootloader and application must both target ESP32-S3 chip ID 9"
                .to_owned(),
        });
    }
    if !entry_is_executable(&image.bootloader, image.bootloader.header.entry, true) {
        return Err(XtensaMachineError::Load {
            address: u64::from(image.bootloader.header.entry),
            message: "ESP32-S3 bootloader entry is outside an executable IRAM segment".to_owned(),
        });
    }
    if !entry_is_executable(&image.application, image.application.header.entry, false) {
        return Err(XtensaMachineError::Load {
            address: u64::from(image.application.header.entry),
            message: "ESP32-S3 application entry is outside an executable IRAM/IROM segment"
                .to_owned(),
        });
    }

    let mut segments = Vec::new();
    let mut mappings = Vec::new();
    for (executable, bootloader) in [(&image.bootloader, true), (&image.application, false)] {
        for segment in &executable.segments {
            checked_flash_bytes(segment, flash)?;
            let kind = classify_segment(segment, bootloader)?;
            if let Some(mapping) = segment_mapping(segment, kind)? {
                if mapping
                    .table_index
                    .checked_add(mapping.page_count)
                    .is_none_or(|end| end > 256)
                {
                    return Err(XtensaMachineError::Load {
                        address: u64::from(segment.address),
                        message: "ESP mapped segment exceeds the 256-entry cache-MMU table"
                            .to_owned(),
                    });
                }
                mappings.push(mapping);
            }
            segments.push(Esp32S3BootSegment {
                kind,
                address: segment.address,
                flash_offset: segment.flash_offset,
                size: segment.data.len(),
            });
        }
    }

    Ok(Esp32S3BootReport {
        schema: "remu.esp32s3-boot.v1",
        bootloader_entry: image.bootloader.header.entry,
        application_entry: image.application.header.entry,
        application_partition: image.application_partition.label.clone(),
        application_partition_offset: image.application_partition.offset,
        segments,
        mappings,
        stages: [
            "rom-image-validation",
            "second-stage-load",
            "partition-selection",
            "application-load-and-map",
            "windowed-abi-handoff",
        ],
    })
}

impl XtensaMachine {
    /// Executes the functional ROM/second-stage pipeline for a merged ESP32-S3 flash image.
    pub fn boot_esp_flash_image(
        &mut self,
        image: &EspFlashImage,
    ) -> Result<(), XtensaMachineError> {
        let report = plan_esp32s3_boot(image, &self.flash)?;
        for segment in &image.bootloader.segments {
            classify_segment(segment, true)?;
            let bytes = checked_flash_bytes(segment, &self.flash)?;
            self.bus
                .load(u64::from(segment.address), bytes)
                .map_err(|error| XtensaMachineError::Load {
                    address: u64::from(segment.address),
                    message: format!("cannot copy ESP bootloader segment: {error}"),
                })?;
        }
        // This performs the application copy/map stage once, after the
        // bootloader bytes are resident, and establishes the ABI handoff.
        self.load_esp_application(image)?;
        self.apply_pending_mmu_mappings()?;
        self.boot_report = Some(report);
        Ok(())
    }

    /// Returns the most recent strict native flash-boot report, if any.
    pub fn esp32s3_boot_report(&self) -> Option<&Esp32S3BootReport> {
        self.boot_report.as_ref()
    }
}
