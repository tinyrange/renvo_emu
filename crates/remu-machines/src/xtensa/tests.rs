use super::*;
use remu_image::FirmwareSegment;

fn image_with_segment(address: u64, data: Vec<u8>) -> FirmwareImage {
    FirmwareImage {
        architecture: FirmwareArchitecture::Xtensa,
        entry: 0x4037_0000,
        segments: vec![FirmwareSegment {
            address,
            load_address: None,
            initialized_size: data.len(),
            data,
            executable: false,
            writable: true,
            alignment: 4,
        }],
        symbols: Vec::new(),
    }
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
fn esp32s3_dram_starts_at_the_documented_soc_boundary() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let invalid = image_with_segment(0x3fc8_0000, vec![0; 4]);
    let error = machine.load_firmware(&invalid).unwrap_err();
    assert!(matches!(
        error,
        XtensaMachineError::Load {
            address: 0x3fc8_0000,
            ..
        }
    ));

    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let valid = image_with_segment(0x3fc8_8000, vec![0xa5; 4]);
    machine.load_firmware(&valid).unwrap();
}

#[test]
fn esp32s3_dram_upper_boundary_is_exclusive() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let last_word = image_with_segment(0x3fcf_fffc, vec![0xa5; 4]);
    machine.load_firmware(&last_word).unwrap();

    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let outside = image_with_segment(0x3fd0_0000, vec![0; 1]);
    assert!(machine.load_firmware(&outside).is_err());
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
