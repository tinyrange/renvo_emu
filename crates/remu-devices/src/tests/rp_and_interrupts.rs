use super::*;

#[test]
fn rp_sio_echoes_bootrom_launch_and_routes_live_fifo_words() {
    let hub = SignalHub::new();
    let (mut sio, _, multicore) =
        RpSioGpio::new_with_multicore("sio", 4, "board.rp.gpio", hub).unwrap();

    // Initial core-1 ROM ready acknowledgement.
    assert_eq!(
        sio.read(0x050, AccessWidth::Word, SimTime::ZERO).unwrap() & 1,
        1
    );
    assert_eq!(
        sio.read(0x058, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0
    );

    let sequence: [u32; 6] = [0, 0, 1, 0x1000_0000, 0x2004_0000, 0x1000_0101];
    for word in sequence {
        sio.write(0x054, AccessWidth::Word, u64::from(word), SimTime::ZERO)
            .unwrap();
        assert_eq!(
            sio.read(0x058, AccessWidth::Word, SimTime::ZERO).unwrap(),
            u64::from(word)
        );
    }
    assert_eq!(
        multicore.take_core1_launch(),
        Some(RpCoreLaunch {
            vector_table: 0x1000_0000,
            stack_pointer: 0x2004_0000,
            entry: 0x1000_0101,
        })
    );

    multicore.select_core(1);
    assert_eq!(sio.read(0, AccessWidth::Word, SimTime::ZERO).unwrap(), 1);
    sio.write(0x054, AccessWidth::Word, 0xfeed_beef, SimTime::ZERO)
        .unwrap();
    multicore.select_core(0);
    assert_eq!(
        sio.read(0x058, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0xfeed_beef
    );
}

#[test]
fn rp2040_resets_supports_set_and_clear_aliases() {
    let mut resets = Rp2040Resets::new("resets");
    resets
        .write(0x2000, AccessWidth::Word, 0x21, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        resets.read(0, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0x21
    );
    assert_eq!(
        resets.read(8, AccessWidth::Word, SimTime::ZERO).unwrap(),
        u64::from(Rp2040Resets::VALID_MASK & !0x21)
    );
    resets
        .write(0x3000, AccessWidth::Word, 0x20, SimTime::ZERO)
        .unwrap();
    assert_eq!(resets.read(0, AccessWidth::Word, SimTime::ZERO).unwrap(), 1);
}

#[test]
fn deterministic_rng_changes_words_and_restarts_from_its_seed() {
    let mut rng = DeterministicRng::new("rng", 0x7c, 0x1234_5678);
    let first = rng.read(0x7c, AccessWidth::Word, SimTime::ZERO).unwrap();
    let second = rng.read(0x7c, AccessWidth::Word, SimTime::ZERO).unwrap();
    assert_ne!(first, second);
    rng.reset(ResetKind::PowerOn);
    assert_eq!(
        rng.read(0x7c, AccessWidth::Word, SimTime::ZERO).unwrap(),
        first
    );
}

#[test]
fn esp32c6_spimem_reports_idle_axi_fifos_across_reset() {
    let mut spimem = EspSpiMem::new("spimem");
    assert_eq!(
        spimem
            .read(0x170, AccessWidth::Word, SimTime::ZERO)
            .unwrap(),
        0xfc00_0000
    );
    spimem.reset(ResetKind::Software);
    assert_eq!(
        spimem
            .read(0x170, AccessWidth::Word, SimTime::ZERO)
            .unwrap(),
        0xfc00_0000
    );
}

#[test]
fn esp32c6_spimem_exports_program_erase_and_read_transactions() {
    let (mut spimem, handle) = EspSpiMem::new_observed("spimem");

    spimem
        .write(0, AccessWidth::Word, 1 << 30, SimTime::ZERO)
        .unwrap();
    spimem
        .write(0x20, AccessWidth::Word, 0x7000_0002, SimTime::ZERO)
        .unwrap();
    spimem
        .write(0x04, AccessWidth::Word, 0x9120, SimTime::ZERO)
        .unwrap();
    spimem
        .write(0x24, AccessWidth::Word, 31, SimTime::ZERO)
        .unwrap();
    spimem
        .write(0x58, AccessWidth::Word, 0x4433_2211, SimTime::ZERO)
        .unwrap();
    spimem
        .write(0, AccessWidth::Word, 1 << 18, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        handle.take_flash_command(),
        Some(EspSpiFlashCommand::Program {
            address: 0x9120,
            data: vec![0x11, 0x22, 0x33, 0x44],
        })
    );

    spimem
        .write(0, AccessWidth::Word, 1 << 30, SimTime::ZERO)
        .unwrap();
    spimem
        .write(0x20, AccessWidth::Word, 0x7000_0020, SimTime::ZERO)
        .unwrap();
    spimem
        .write(0x04, AccessWidth::Word, 0x9000, SimTime::ZERO)
        .unwrap();
    spimem
        .write(0, AccessWidth::Word, 1 << 18, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        handle.take_flash_command(),
        Some(EspSpiFlashCommand::EraseSector { address: 0x9000 })
    );

    spimem
        .write(0x20, AccessWidth::Word, 0x7000_00bb, SimTime::ZERO)
        .unwrap();
    spimem
        .write(0x04, AccessWidth::Word, 0x9000, SimTime::ZERO)
        .unwrap();
    spimem
        .write(0x28, AccessWidth::Word, 31, SimTime::ZERO)
        .unwrap();
    spimem
        .write(0, AccessWidth::Word, 1 << 18, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        handle.take_flash_command(),
        Some(EspSpiFlashCommand::Read {
            address: 0x9000,
            length: 4,
        })
    );
    handle.complete_flash_read(vec![0xaa, 0xbb, 0xcc, 0xdd]);
    assert_eq!(
        spimem.read(0x58, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0xddcc_bbaa
    );
}

#[test]
fn esp32c6_analog_i2c_completes_rfpll_charge_pump_calibration() {
    let mut i2c = EspAnalogI2c::new("analog-i2c");
    let write = (1_u64 << 24) | (0x20_u64 << 16) | (0x0f_u64 << 8) | 0x62;
    i2c.write(0x188, AccessWidth::Word, write, SimTime::ZERO)
        .unwrap();
    i2c.write(
        0x188,
        AccessWidth::Word,
        (0x0e_u64 << 8) | 0x62,
        SimTime::ZERO,
    )
    .unwrap();
    assert_ne!(
        i2c.read(0x188, AccessWidth::Word, SimTime::ZERO).unwrap() & (0x80 << 16),
        0
    );
}

#[test]
fn rp2040_usb_exposes_vbus_and_atomic_aliases() {
    let mut usb = Rp2040UsbController::new("usb");
    assert_eq!(
        usb.read(
            Rp2040UsbRegister::SieStatus.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap()
            & 1,
        1
    );
    usb.write(
        Rp2040UsbRegister::SieCtrl.offset() + 0x2000,
        AccessWidth::Word,
        1 << 16,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        usb.read(
            Rp2040UsbRegister::SieCtrl.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        1 << 16
    );
}

#[test]
fn rp2040_usb_protocol_events_are_write_clear_and_interrupt_mapped() {
    let (mut usb, handle) = Rp2040UsbController::new_with_handle("usb");

    assert_eq!(
        Rp2040UsbRegister::from_offset(0x50),
        Some(Rp2040UsbRegister::SieStatus)
    );
    assert_eq!(Rp2040UsbRegister::Intr.offset(), 0x8c);
    assert_eq!(Rp2040UsbRegister::from_offset(0x88), None);

    handle.inject_bus_reset();
    let raw = usb
        .read(
            Rp2040UsbRegister::Intr.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap();
    assert_ne!(raw & (1 << 12), 0);
    assert_ne!(raw & (1 << 13), 0);
    assert_ne!(raw & (1 << 11), 0);

    usb.write(
        Rp2040UsbRegister::SieStatus.offset(),
        AccessWidth::Word,
        (1 << 19) | (1 << 16),
        SimTime::ZERO,
    )
    .unwrap();
    let raw = usb
        .read(
            Rp2040UsbRegister::Intr.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(raw & ((1 << 12) | (1 << 13)), 0);
    assert_ne!(raw & (1 << 11), 0);

    usb.write(
        Rp2040UsbRegister::Inte.offset(),
        AccessWidth::Word,
        (1 << 4) | (1 << 16),
        SimTime::ZERO,
    )
    .unwrap();
    handle.inject_setup();
    assert_ne!(
        usb.read(
            Rp2040UsbRegister::Ints.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap()
            & (1 << 16),
        0
    );
    usb.write(
        Rp2040UsbRegister::SieStatus.offset() + 0x3000,
        AccessWidth::Word,
        1 << 17,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        usb.read(
            Rp2040UsbRegister::Intr.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap()
            & (1 << 16),
        0
    );

    handle.complete_buffer(2, true);
    assert_ne!(
        usb.read(
            Rp2040UsbRegister::Intr.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap()
            & (1 << 4),
        0
    );
    usb.write(
        Rp2040UsbRegister::BuffStatus.offset(),
        AccessWidth::Word,
        1 << 4,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        usb.read(
            Rp2040UsbRegister::Intr.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap()
            & (1 << 4),
        0
    );
}

#[test]
fn rp2040_usb_register_masks_reset_values_and_narrow_io_match_the_datasheet() {
    let mut usb = Rp2040UsbController::new("usb");

    assert_eq!(
        usb.read(
            Rp2040UsbRegister::NakPoll.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        0x0010_0010
    );
    assert_eq!(
        usb.read(
            Rp2040UsbRegister::UsbPhyTrim.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        0x1f1f
    );

    usb.write(
        Rp2040UsbRegister::MainCtrl.offset(),
        AccessWidth::Word,
        u64::from(u32::MAX),
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        usb.read(
            Rp2040UsbRegister::MainCtrl.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        0x8000_0003
    );

    usb.write(
        Rp2040UsbRegister::SieCtrl.offset(),
        AccessWidth::Word,
        u64::from(u32::MAX),
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        usb.read(
            Rp2040UsbRegister::SieCtrl.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        0xff07_bf5f & !0x3011
    );

    usb.write(
        Rp2040UsbRegister::UsbMuxing.offset() + 1,
        AccessWidth::Byte,
        0xa5,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        usb.read(
            Rp2040UsbRegister::UsbMuxing.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        0x5
    );
    assert_eq!(
        usb.read(
            Rp2040UsbRegister::UsbPhyTrim.offset() + 1,
            AccessWidth::Byte,
            SimTime::ZERO,
        )
        .unwrap(),
        0x1f
    );

    usb.write(
        Rp2040UsbRegister::UsbPhyDirect.offset(),
        AccessWidth::Word,
        u64::from(u32::MAX),
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        usb.read(
            Rp2040UsbRegister::UsbPhyDirect.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        0xff77
    );

    usb.write(
        Rp2040UsbRegister::Intr.offset(),
        AccessWidth::Word,
        u64::from(u32::MAX),
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        usb.read(
            Rp2040UsbRegister::Intr.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        1 << 11
    );
    assert!(usb.read(0x88, AccessWidth::Word, SimTime::ZERO).is_err());
}

#[test]
fn rp2040_usb_packet_codec_rejects_pid_and_crc_corruption() {
    for packet in [
        RpUsbPacket::Token {
            pid: RpUsbPid::Setup,
            address: 37,
            endpoint: 4,
        },
        RpUsbPacket::Sof(0x5a5),
        RpUsbPacket::Data {
            pid: RpUsbPid::Data1,
            payload: vec![0x00, 0xff, 0x55, 0xaa],
        },
        RpUsbPacket::Handshake(RpUsbPid::Ack),
    ] {
        let encoded = packet.encode().unwrap();
        assert_eq!(RpUsbPacket::decode(&encoded).unwrap(), packet);
    }

    let mut bad_pid = RpUsbPacket::Handshake(RpUsbPid::Ack).encode().unwrap();
    bad_pid[0] ^= 0x10;
    assert_eq!(
        RpUsbPacket::decode(&bad_pid),
        Err(RpUsbPacketError::PidComplement)
    );
    let mut bad_crc = RpUsbPacket::Data {
        pid: RpUsbPid::Data0,
        payload: vec![1, 2, 3],
    }
    .encode()
    .unwrap();
    *bad_crc.last_mut().unwrap() ^= 1;
    assert_eq!(RpUsbPacket::decode(&bad_crc), Err(RpUsbPacketError::Crc16));
}

#[test]
fn rp2040_usb_link_tracks_lines_frames_toggles_and_endpoint_outcomes() {
    let (mut usb, handle) = Rp2040UsbController::new_with_handle("usb");
    assert_eq!(handle.line_state(), RpUsbLineState::Se0);
    handle.inject_bus_reset();
    assert_eq!(handle.line_state(), RpUsbLineState::J);
    handle.inject_sof();
    assert_eq!(
        usb.read(
            Rp2040UsbRegister::SofRd.offset(),
            AccessWidth::Word,
            SimTime::ZERO
        )
        .unwrap(),
        1
    );

    handle
        .transact(0, false, true, &[0; 8], RpUsbPid::Ack)
        .unwrap();
    assert_eq!(handle.data_toggle(0, false), Some(true));
    assert_eq!(handle.data_toggle(0, true), Some(true));
    handle
        .transact(2, true, false, b"abc", RpUsbPid::Ack)
        .unwrap();
    assert_eq!(handle.data_toggle(2, true), Some(true));
    assert_eq!(handle.packet_trace().len(), 7);

    handle
        .transact(3, false, false, b"busy", RpUsbPid::Nak)
        .unwrap();
    assert_ne!(
        usb.read(
            Rp2040UsbRegister::SieStatus.offset(),
            AccessWidth::Word,
            SimTime::ZERO
        )
        .unwrap()
            & (1 << 28),
        0
    );
    assert_eq!(handle.data_toggle(3, false), Some(false));

    handle.inject_suspend();
    assert!(handle.interrupt_pending() == false);
    handle.inject_resume();
    assert_eq!(handle.line_state(), RpUsbLineState::K);
    assert_ne!(
        usb.read(
            Rp2040UsbRegister::UsbPhyDirect.offset(),
            AccessWidth::Word,
            SimTime::ZERO
        )
        .unwrap()
            & (1 << 18),
        0
    );

    usb.write(
        Rp2040UsbRegister::EpAbort.offset(),
        AccessWidth::Word,
        1 << 5,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        usb.read(
            Rp2040UsbRegister::EpAbort.offset(),
            AccessWidth::Word,
            SimTime::ZERO
        )
        .unwrap(),
        0
    );
    assert_ne!(
        usb.read(
            Rp2040UsbRegister::EpAbortDone.offset(),
            AccessWidth::Word,
            SimTime::ZERO
        )
        .unwrap()
            & (1 << 5),
        0
    );
}

#[test]
fn rp_sio_spinlocks_claim_on_read_and_release_on_write() {
    let hub = SignalHub::new();
    let (mut sio, _) = RpSioGpio::new("sio", 4, "board.rp.gpio", hub).unwrap();
    assert_eq!(
        sio.read(0x12c, AccessWidth::Word, SimTime::ZERO).unwrap(),
        1 << 11
    );
    assert_eq!(
        sio.read(0x12c, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0
    );
    sio.write(0x12c, AccessWidth::Word, 0, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        sio.read(0x12c, AccessWidth::Word, SimTime::ZERO).unwrap(),
        1 << 11
    );
}

#[test]
fn arm_ppb_models_all_nvic_banks_and_software_trigger() {
    let (mut ppb, handle) = ArmPrivatePeripheralBus::new("ppb", 0x410f_d210);
    ppb.write(0x11c, AccessWidth::Word, 1 << 15, SimTime::ZERO)
        .unwrap();
    ppb.write(0xf00, AccessWidth::Word, 239, SimTime::ZERO)
        .unwrap();

    assert!(handle.interrupt_enabled(239));
    assert_eq!(handle.take_pending_interrupts(), vec![239]);
    assert!(handle.take_pending_interrupts().is_empty());
}

#[test]
fn arm_ppb_systick_latches_a_deterministic_exception() {
    let (mut ppb, handle) = ArmPrivatePeripheralBus::new("ppb", 0x410c_c601);
    ppb.write(0x014, AccessWidth::Word, 2, SimTime::ZERO)
        .unwrap();
    ppb.write(0x018, AccessWidth::Word, 0, SimTime::ZERO)
        .unwrap();
    ppb.write(0x010, AccessWidth::Word, 7, SimTime::ZERO)
        .unwrap();

    assert!(!handle.take_systick_pending(SimTime::from_ticks(2)));
    assert!(handle.take_systick_pending(SimTime::from_ticks(3)));
    assert!(!handle.take_systick_pending(SimTime::from_ticks(3)));
    assert_ne!(
        ppb.read(0x010, AccessWidth::Word, SimTime::from_ticks(3))
            .unwrap()
            & (1 << 16),
        0
    );
}

fn watchdog_write(
    device: &mut Rp2040Watchdog,
    register: Rp2040WatchdogRegister,
    value: u32,
    at: u64,
) {
    device
        .write(
            register as u64,
            AccessWidth::Word,
            u64::from(value),
            SimTime::from_ticks(at),
        )
        .unwrap();
}

fn watchdog_read(device: &mut Rp2040Watchdog, register: Rp2040WatchdogRegister, at: u64) -> u32 {
    u32::try_from(
        device
            .read(register as u64, AccessWidth::Word, SimTime::from_ticks(at))
            .unwrap(),
    )
    .unwrap()
}

#[test]
fn rp2040_watchdog_counts_down_with_e1_divide_by_two() {
    let (mut device, handle) = Rp2040Watchdog::new_with_handle("watchdog");
    watchdog_write(&mut device, Rp2040WatchdogRegister::Load, 5, 0);
    watchdog_write(&mut device, Rp2040WatchdogRegister::Ctrl, 1 << 30, 0);
    assert_eq!(
        watchdog_read(&mut device, Rp2040WatchdogRegister::Ctrl, 0) & 0x00ff_ffff,
        3
    );
    assert!(!handle.take_reset(SimTime::from_ticks(1)));
    assert!(!handle.take_reset(SimTime::from_ticks(2)));
    assert!(handle.take_reset(SimTime::from_ticks(3)));
    assert_eq!(handle.reason(SimTime::from_ticks(3)), 1);
}

#[test]
fn rp2040_watchdog_force_trigger_and_scratch_reset_semantics_are_deterministic() {
    let (mut device, handle) = Rp2040Watchdog::new_with_handle("watchdog");
    watchdog_write(&mut device, Rp2040WatchdogRegister::Ctrl, 1 << 31, 0);
    assert!(handle.take_reset(SimTime::ZERO));
    assert!(!handle.take_reset(SimTime::ZERO));
    assert_eq!(
        watchdog_read(&mut device, Rp2040WatchdogRegister::Reason, 0),
        2
    );
    watchdog_write(
        &mut device,
        Rp2040WatchdogRegister::Scratch0,
        0xfeed_cafe,
        0,
    );
    Device::reset(&mut device, ResetKind::Software);
    assert_eq!(
        watchdog_read(&mut device, Rp2040WatchdogRegister::Scratch0, 0),
        0xfeed_cafe
    );
    Device::reset(&mut device, ResetKind::PowerOn);
    assert_eq!(
        watchdog_read(&mut device, Rp2040WatchdogRegister::Scratch0, 0),
        0
    );
}
#[test]
fn pwm_advances_counter_and_reports_compare_outputs() {
    let (mut pwm, handle) = FunctionalPwm::new("pwm", 2);
    pwm.write(0x0c, AccessWidth::Word, 4, SimTime::ZERO)
        .unwrap();
    pwm.write(0x10, AccessWidth::Word, 9, SimTime::ZERO)
        .unwrap();
    pwm.write(0x00, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    pwm.write(
        RpPwmRegister::En.global_offset(2).unwrap(),
        AccessWidth::Word,
        1,
        SimTime::ZERO,
    )
    .unwrap();
    pwm.write(
        RpPwmRegister::Inte0.global_offset(2).unwrap(),
        AccessWidth::Word,
        1,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(handle.counter(0), Some(0));
    assert_eq!(handle.outputs(0), Some([true, false]));
    assert_eq!(
        pwm.read(0x08, AccessWidth::Word, SimTime::from_ticks(5))
            .unwrap(),
        5
    );
    assert_eq!(handle.outputs(0), Some([false, false]));
    assert_eq!(handle.pending_interrupts(), 0);
    assert_eq!(
        pwm.read(0x08, AccessWidth::Word, SimTime::from_ticks(12))
            .unwrap(),
        2
    );
    assert_ne!(handle.pending_interrupts(), 0);
}

#[test]
fn pwm_phase_commands_adjust_counter_and_self_clear() {
    let (mut pwm, handle) = FunctionalPwm::new("pwm", 1);
    pwm.write(0x10, AccessWidth::Word, 3, SimTime::ZERO)
        .unwrap();
    pwm.write(0x08, AccessWidth::Word, 3, SimTime::ZERO)
        .unwrap();

    pwm.write(0x00, AccessWidth::Word, 1 << 7, SimTime::ZERO)
        .unwrap();
    assert_eq!(handle.counter(0), Some(0));
    assert_eq!(pwm.read(0x00, AccessWidth::Word, SimTime::ZERO).unwrap(), 0);

    pwm.write(0x00, AccessWidth::Word, 1 << 6, SimTime::ZERO)
        .unwrap();
    assert_eq!(handle.counter(0), Some(3));
    assert_eq!(pwm.read(0x00, AccessWidth::Word, SimTime::ZERO).unwrap(), 0);
}

#[test]
fn i2c_executes_addressed_writes_and_queued_reads() {
    let (mut i2c, handle) = FunctionalI2c::new("i2c");
    handle.queue_read(0x58, &[0x12]);
    i2c.write(0x04, AccessWidth::Word, 0x58, SimTime::ZERO)
        .unwrap();
    i2c.write(0x6c, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    i2c.write(0x10, AccessWidth::Word, 0xa0, SimTime::ZERO)
        .unwrap();
    i2c.write(0x10, AccessWidth::Word, 1 << 8, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        i2c.read(0x10, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0x12
    );
    assert_eq!(
        handle.events(),
        [
            I2cEvent::Write {
                address: 0x58,
                value: 0xa0
            },
            I2cEvent::Read {
                address: 0x58,
                value: 0x12
            },
        ]
    );
}

#[test]
fn i2c_reports_fifo_and_stop_interrupt_state() {
    let (mut i2c, _handle) = FunctionalI2c::new("i2c");
    i2c.write(0x30, AccessWidth::Word, (1 << 4) | (1 << 9), SimTime::ZERO)
        .unwrap();
    i2c.write(0x04, AccessWidth::Word, 0x20, SimTime::ZERO)
        .unwrap();
    i2c.write(0x6c, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    i2c.write(0x10, AccessWidth::Word, (1 << 9) | 0x5a, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        i2c.read(0x2c, AccessWidth::Word, SimTime::ZERO).unwrap(),
        (1 << 4) | (1 << 9)
    );
    assert_eq!(
        i2c.read(0x70, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0x06
    );
}
