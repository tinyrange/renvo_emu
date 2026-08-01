use super::*;
use remu_image::{EspExecutableImage, EspFlashImage, EspImageHeader, EspImageSegment};

fn handoff_image(entry: u32, code: &[u8]) -> EspFlashImage {
    let header = || EspImageHeader {
        segment_count: 1,
        flash_mode: 2,
        flash_size_frequency: 0x20,
        entry,
        write_protect_pin: 0xee,
        drive_settings: [0; 3],
        chip_id: 9,
        minimum_revision_legacy: 0,
        minimum_revision: 0,
        maximum_revision: 0x63,
        hash_appended: false,
    };
    EspFlashImage {
        bootloader: EspExecutableImage {
            flash_offset: 0,
            header: header(),
            segments: Vec::new(),
            checksum: 0xef,
            appended_sha256: None,
            end_offset: 0,
        },
        partition_table: remu_image::EspPartitionTable {
            partitions: Vec::new(),
            has_md5: false,
        },
        application: EspExecutableImage {
            flash_offset: 0,
            header: header(),
            segments: vec![EspImageSegment {
                address: entry,
                flash_offset: 0x1000,
                data: code.to_vec(),
            }],
            checksum: 0xef,
            appended_sha256: None,
            end_offset: 0x1000 + code.len() as u32,
        },
        application_partition: remu_image::EspPartition {
            partition_type: 0,
            subtype: 0,
            offset: 0,
            size: 0,
            label: "factory".to_owned(),
            flags: 0,
        },
    }
}

#[test]
fn verified_handoff_requires_entry_and_rotates_callx8_window() {
    let mut missing_entry = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    missing_entry
        .load_esp_application(&handoff_image(0x4037_0040, &[0x3d, 0xf0]))
        .unwrap();
    let fault = missing_entry
        .run(
            RunLimits {
                instructions: Some(1),
                deadline: None,
            },
            None,
        )
        .unwrap();
    assert!(matches!(
        fault.reason,
        StopReason::Fault(message) if message.contains("requires ENTRY")
    ));

    let mut valid = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    valid
        .load_esp_application(&handoff_image(0x4037_0040, &[0x36, 0x41, 0x00, 0x3d, 0xf0]))
        .unwrap();
    let result = valid
        .run(
            RunLimits {
                instructions: Some(1),
                deadline: None,
            },
            None,
        )
        .unwrap();
    assert_eq!(result.reason, StopReason::InstructionLimit);
    let ps = result
        .cpu
        .registers
        .iter()
        .find(|register| register.name == "ps")
        .expect("Xtensa snapshot includes PS")
        .value;
    assert_eq!(ps & (3 << 16), 0);
}

#[test]
fn direct_load_starts_with_appcpu_reset_and_parked() {
    let machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    assert!(machine.appcpu_boot_address.is_none());
    assert_eq!(machine.cpu1.snapshot().pc, 0);
    assert!(!machine.cpu1.snapshot().waiting);
    assert!(!machine.cpu1.snapshot().halted);
}

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
    host.output
        .extend_from_slice(b"__REMU_HOST_SCRIPT_COMPLETE__\r\n\x04\x04>");
    assert!(host.input_complete());
}
