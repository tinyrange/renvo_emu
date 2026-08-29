use super::*;
use remu_image::FirmwareSegment;

#[test]
fn samd21_firmware_drives_porta_and_produces_a_trace() {
    let mut code = vec![
        0x02, 0x48, 0x80, 0x21, 0x81, 0x60, 0x81, 0x61, 0x30, 0xbf, 0, 0,
    ];
    code.extend_from_slice(&0x4100_4400_u32.to_le_bytes());
    let image = FirmwareImage {
        architecture: FirmwareArchitecture::Arm,
        entry: 1,
        segments: vec![FirmwareSegment {
            address: 0,
            load_address: None,
            initialized_size: code.len(),
            data: code,
            executable: true,
            writable: false,
            alignment: 4,
        }],
        symbols: Vec::new(),
    };
    let mut machine = ArmMcuMachine::new(TargetId::Atsamd21e18).unwrap();
    machine.load_firmware(&image).unwrap();
    let result = machine
        .run_with_stimuli(
            RunLimits {
                instructions: Some(4),
                deadline: None,
            },
            &[],
            None,
        )
        .unwrap();
    assert_eq!(machine.gpio_output(), 1 << 7);
    assert_eq!(result.reason, StopReason::InstructionLimit);
    assert_ne!(result.trace_digest, "");
}

#[test]
fn samd21_native_sercom0_accepts_spi_and_i2c_master_registers() {
    let mut machine = ArmMcuMachine::new(TargetId::Atsamd21e18).unwrap();
    let sercom0 = 0x4200_0800;

    machine
        .bus
        .write(sercom0, AccessWidth::Word, 3_u64 << 2, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(sercom0 + 0x04, AccessWidth::Word, 1 << 17, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(sercom0, AccessWidth::Word, (3_u64 << 2) | 2, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(sercom0 + 0x28, AccessWidth::Byte, 0x5a, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        machine
            .bus
            .read(
                sercom0 + 0x28,
                AccessWidth::Byte,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        0x5a
    );

    machine
        .bus
        .write(sercom0, AccessWidth::Word, 5_u64 << 2, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(sercom0, AccessWidth::Word, (5_u64 << 2) | 2, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(sercom0 + 0x24, AccessWidth::Byte, 0xa0, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        machine
            .bus
            .read(
                sercom0 + 0x18,
                AccessWidth::Byte,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap()
            & 1,
        1
    );
}

#[test]
fn samd21_adc_latches_a_host_sample_through_native_registers() {
    let mut machine = ArmMcuMachine::new(TargetId::Atsamd21e18).unwrap();
    machine.set_adc_sample(3, 0x0abc).unwrap();
    machine
        .bus
        .write(0x4200_4000, AccessWidth::Byte, 2, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x4200_4010, AccessWidth::Word, 3, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x4200_400c, AccessWidth::Byte, 2, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        machine
            .bus
            .read(
                0x4200_401a,
                AccessWidth::HalfWord,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        0x0abc
    );
}

#[test]
fn samd21_ac_latches_a_host_comparison_through_native_registers() {
    let mut machine = ArmMcuMachine::new(TargetId::Atsamd21e18).unwrap();
    machine.set_ac_input(0, 0x0900).unwrap();
    machine
        .bus
        .write(0x4200_4400, AccessWidth::Byte, 2, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(
            0x4200_4410,
            AccessWidth::Word,
            (1 << 5) | (1 << 1) | (4 << 8) | 1,
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(0x4200_4401, AccessWidth::Byte, 1, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        machine
            .bus
            .read(
                0x4200_4408,
                AccessWidth::Byte,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        1
    );
}

#[test]
fn samd21_maps_the_native_dac_register_window() {
    let mut machine = ArmMcuMachine::new(TargetId::Atsamd21e18).unwrap();
    machine
        .bus
        .write(0x4200_4800, AccessWidth::Byte, 1, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x4200_4808, AccessWidth::HalfWord, 0x02a5, SimTime::ZERO)
        .unwrap();
    assert_eq!(machine.dac.as_ref().expect("SAM D21 DAC").data(), 0x02a5);
    assert_eq!(
        machine
            .bus
            .read(
                0x4200_4808,
                AccessWidth::HalfWord,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        0
    );
}

#[test]
fn stm32l432_uses_the_distinct_m4f_profile_and_gpioa_bsrr() {
    let mut machine = ArmMcuMachine::new(TargetId::Stm32l432kc).unwrap();
    assert_eq!(machine.cpu.profile(), ArmProfile::CortexM4F);
    machine
        .bus
        .write(0x4800_0000, AccessWidth::Word, 1 << 10, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x4800_0018, AccessWidth::Word, 1 << 5, SimTime::ZERO)
        .unwrap();
    assert_eq!(machine.gpio_output(), 1 << 5);
}

#[test]
fn stm32l432_maps_usart1_and_lpuart1_native_windows() {
    let mut machine = ArmMcuMachine::new(TargetId::Stm32l432kc).unwrap();
    machine
        .bus
        .write(
            0x4001_3828,
            AccessWidth::Word,
            u64::from(b'1'),
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(
            0x4000_8028,
            AccessWidth::Word,
            u64::from(b'L'),
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(machine.uart.bytes(), b"1L");
    assert_eq!(
        machine
            .bus
            .read(
                0x4001_381c,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap()
            & (1 << 7),
        1 << 7
    );
}

#[test]
fn stm32f411re_maps_flash_alias_gpio_tim2_and_usart2() {
    let mut machine = ArmMcuMachine::new(TargetId::Stm32f411re).unwrap();
    assert_eq!(machine.cpu.profile(), ArmProfile::CortexM4F);
    let image = FirmwareImage {
        architecture: FirmwareArchitecture::Arm,
        entry: 0x0800_0001,
        segments: vec![FirmwareSegment {
            address: 0x0800_0000,
            load_address: None,
            initialized_size: 4,
            data: vec![0x00, 0xbe, 0x00, 0xbf],
            executable: true,
            writable: false,
            alignment: 4,
        }],
        symbols: Vec::new(),
    };
    machine.load_firmware(&image).unwrap();
    assert_eq!(machine.debug_read_memory(0, 2).unwrap(), [0x00, 0xbe]);

    machine
        .bus
        .write(0x4002_0000, AccessWidth::Word, 1 << 10, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x4002_0018, AccessWidth::Word, 1 << 5, SimTime::ZERO)
        .unwrap();
    assert_eq!(machine.gpio_output(), 1 << 5);

    machine
        .bus
        .write(0x4000_4404, AccessWidth::Word, b'F'.into(), SimTime::ZERO)
        .unwrap();
    assert_eq!(machine.uart.bytes(), b"F");
    assert_eq!(
        machine
            .bus
            .read(
                0x4000_4400,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO
            )
            .unwrap()
            & (1 << 7),
        1 << 7
    );

    machine
        .bus
        .write(0x4000_002c, AccessWidth::Word, 3, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x4000_000c, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x4000_0000, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    assert_eq!(machine.timer.poll(SimTime::from_ticks(4)), (Some(28), true));
}

#[test]
fn nrf52840_maps_gpio_uart_tasks_and_timer_compare() {
    let mut machine = ArmMcuMachine::new(TargetId::Nrf52840).unwrap();
    assert_eq!(machine.cpu.profile(), ArmProfile::CortexM4F);
    machine
        .bus
        .write(0x5000_0518, AccessWidth::Word, 1 << 13, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x5000_0508, AccessWidth::Word, 1 << 13, SimTime::ZERO)
        .unwrap();
    assert_eq!(machine.gpio_output(), 1 << 13);

    machine
        .bus
        .write(0x4000_2500, AccessWidth::Word, 4, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x4000_2008, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x4000_251c, AccessWidth::Word, b'N'.into(), SimTime::ZERO)
        .unwrap();
    assert_eq!(machine.uart.bytes(), b"N");

    machine
        .bus
        .write(0x4000_8540, AccessWidth::Word, 3, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x4000_8304, AccessWidth::Word, 1 << 16, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x4000_8000, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    assert_eq!(machine.timer.poll(SimTime::from_ticks(3)), (Some(8), true));
    assert_eq!(
        machine
            .bus
            .read(
                0x4000_8140,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::from_ticks(3)
            )
            .unwrap(),
        1
    );
}

#[test]
fn stm32l432_maps_both_i2c_event_controllers() {
    let mut machine = ArmMcuMachine::new(TargetId::Stm32l432kc).unwrap();
    machine
        .bus
        .write(0x4000_5400, AccessWidth::Word, 1 << 1, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(
            0x4000_5404,
            AccessWidth::Word,
            (2 << 16) | (1 << 13) | (1 << 25),
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(0x4000_5428, AccessWidth::Word, 0xa5, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x4000_5428, AccessWidth::Word, 0x5a, SimTime::ZERO)
        .unwrap();
    assert_ne!(
        machine
            .bus
            .read(
                0x4000_5418,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap()
            & (1 << 5),
        0
    );
    machine
        .bus
        .write(
            0x4000_5c04,
            AccessWidth::Word,
            (1 << 13) | (1 << 25),
            SimTime::ZERO,
        )
        .unwrap();
    assert_ne!(
        machine
            .bus
            .read(
                0x4000_5c18,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap()
            & (1 << 15),
        0
    );
}

#[test]
fn stm32l432_spi1_and_spi3_transfer_and_route_interrupt_status() {
    let mut machine = ArmMcuMachine::new(TargetId::Stm32l432kc).unwrap();
    for (index, base) in [(0, 0x4001_3000_u64), (1, 0x4000_3c00_u64)] {
        machine.stm32_spi[index].1.inject_rx(0xa5 + index as u8);
        machine
            .bus
            .write(
                base,
                AccessWidth::Word,
                u64::from((1_u32 << 6) | (1_u32 << 2)),
                SimTime::ZERO,
            )
            .unwrap();
        machine
            .bus
            .write(base + 4, AccessWidth::Word, 1 << 6, SimTime::ZERO)
            .unwrap();
        machine
            .bus
            .write(base + 0x0c, AccessWidth::Word, 0x3c, SimTime::ZERO)
            .unwrap();
        assert_eq!(machine.stm32_spi[index].1.tx_bytes(), [0x3c]);
        assert!(machine.stm32_spi[index].1.interrupt_pending());
        assert_eq!(
            machine
                .bus
                .read(
                    base + 0x0c,
                    AccessWidth::Word,
                    AccessKind::Read,
                    SimTime::ZERO
                )
                .unwrap(),
            u64::from(0xa5 + index as u8)
        );
    }
}

#[test]
fn stm32l432_maps_adc1_and_converts_scripted_input() {
    let mut machine = ArmMcuMachine::new(TargetId::Stm32l432kc).unwrap();
    machine.adc().unwrap().set_input(4, 0x0abc);
    machine
        .bus
        .write(0x5004_0030, AccessWidth::Word, 4 << 6, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x5004_0008, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x5004_0008, AccessWidth::Word, 1 << 2, SimTime::ZERO)
        .unwrap();
    assert_eq!(machine.adc().unwrap().value(), 0x0abc);
    assert_eq!(
        machine
            .bus
            .read(
                0x5004_0040,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        0x0abc
    );
}

#[test]
fn stm32l432_maps_crc_and_accepts_word_data() {
    let mut machine = ArmMcuMachine::new(TargetId::Stm32l432kc).unwrap();
    machine
        .bus
        .write(0x4002_3008, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x4002_3000, AccessWidth::Word, 0x1234_5678, SimTime::ZERO)
        .unwrap();
    assert_ne!(machine.crc().unwrap().value(), u32::MAX);
    assert_eq!(
        machine
            .bus
            .read(
                0x4002_3000,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap() as u32,
        machine.crc().unwrap().value()
    );
}

#[test]
fn stm32l432_maps_rtc_calendar_and_alarm_registers() {
    let mut machine = ArmMcuMachine::new(TargetId::Stm32l432kc).unwrap();
    machine.rtc().unwrap().set_seconds(0);
    machine
        .bus
        .write(0x4000_2824, AccessWidth::Word, 0xca, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x4000_2824, AccessWidth::Word, 0x53, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(
            0x4000_281c,
            AccessWidth::Word,
            (1 << 31) | (1 << 8),
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(0x4000_2818, AccessWidth::Word, 1 << 8, SimTime::ZERO)
        .unwrap();
    let _ = machine.bus.read(
        0x4000_2800,
        AccessWidth::Word,
        AccessKind::Read,
        SimTime::from_ticks(60),
    );
    assert!(machine.rtc().unwrap().alarm_flags().0);
}

#[test]
fn stm32l432_maps_rng_and_replays_seeded_value() {
    let mut machine = ArmMcuMachine::new(TargetId::Stm32l432kc).unwrap();
    machine.rng().unwrap().seed(0x1234_5678);
    machine
        .bus
        .write(0x5006_0800, AccessWidth::Word, 1 << 2, SimTime::ZERO)
        .unwrap();
    let first = machine
        .bus
        .read(
            0x5006_0808,
            AccessWidth::Word,
            AccessKind::Read,
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(
        machine
            .bus
            .read(
                0x5006_0804,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        0
    );
    machine.rng().unwrap().seed(0x1234_5678);
    let replay = machine
        .bus
        .read(
            0x5006_0808,
            AccessWidth::Word,
            AccessKind::Read,
            SimTime::from_ticks(1),
        )
        .unwrap();
    assert_eq!(first, replay);
}

#[test]
fn stm32l432_native_can_window_supports_loopback_mailbox() {
    let mut machine = ArmMcuMachine::new(TargetId::Stm32l432kc).unwrap();
    let base = 0x4000_6400;
    machine
        .bus
        .write(base, AccessWidth::Word, 0, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(base + 0x1c, AccessWidth::Word, 1 << 30, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(base + 0x184, AccessWidth::Word, 4, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(base + 0x188, AccessWidth::Word, 0x4433_2211, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(base + 0x18c, AccessWidth::Word, 0, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(
            base + 0x180,
            AccessWidth::Word,
            (0x123 << 21) | 1,
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(
        machine.bus.read(
            base + 0x0c,
            AccessWidth::Word,
            AccessKind::Read,
            SimTime::ZERO
        ),
        Ok(1)
    );
    assert_eq!(
        machine.bus.read(
            base + 0x1b8,
            AccessWidth::Word,
            AccessKind::Read,
            SimTime::ZERO
        ),
        Ok(0x4433_2211)
    );
}

#[test]
fn stm32l432_native_dac_window_latches_and_triggers_both_channels() {
    let mut machine = ArmMcuMachine::new(TargetId::Stm32l432kc).unwrap();
    let base = 0x4000_7400;
    machine
        .bus
        .write(
            base,
            AccessWidth::Word,
            1 | (1 << 2) | (1 << 16),
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(
            base + 0x08,
            AccessWidth::Word,
            0xabc,
            SimTime::from_ticks(1),
        )
        .unwrap();
    assert_eq!(
        machine.bus.read(
            base + 0x2c,
            AccessWidth::Word,
            AccessKind::Read,
            SimTime::ZERO
        ),
        Ok(0)
    );
    machine
        .bus
        .write(base + 0x04, AccessWidth::Word, 1, SimTime::from_ticks(2))
        .unwrap();
    machine
        .bus
        .write(base + 0x1c, AccessWidth::Word, 0x5a, SimTime::from_ticks(3))
        .unwrap();
    assert_eq!(
        machine.bus.read(
            base + 0x2c,
            AccessWidth::Word,
            AccessKind::Read,
            SimTime::ZERO
        ),
        Ok(0xabc)
    );
    assert_eq!(
        machine.bus.read(
            base + 0x30,
            AccessWidth::Word,
            AccessKind::Read,
            SimTime::ZERO
        ),
        Ok(0x5a0)
    );
}

#[test]
fn stm32l432_native_tim1_window_routes_update_and_pwm() {
    let mut machine = ArmMcuMachine::new(TargetId::Stm32l432kc).unwrap();
    let base = 0x4001_2c00;
    for (offset, value) in [
        (0x2c, 9),
        (0x34, 4),
        (0x18, 6 << 4),
        (0x20, 1 | (1 << 2)),
        (0x44, 1 << 15),
        (0x0c, 1),
        (0x00, 1),
    ] {
        machine
            .bus
            .write(base + offset, AccessWidth::Word, value, SimTime::ZERO)
            .unwrap();
    }
    assert!(
        machine
            .stm32_tim1
            .as_ref()
            .expect("STM32 machine has TIM1")
            .poll(SimTime::from_ticks(10))
    );
    assert_eq!(
        machine.bus.read(
            base + 0x10,
            AccessWidth::Word,
            AccessKind::Read,
            SimTime::ZERO
        ),
        Ok(1)
    );
}

#[test]
fn stm32l432_native_exti_window_latches_gpioa_edges() {
    let mut machine = ArmMcuMachine::new(TargetId::Stm32l432kc).unwrap();
    let base = 0x4001_0400;
    machine
        .bus
        .write(base, AccessWidth::Word, 1 << 3, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(base + 0x08, AccessWidth::Word, 1 << 3, SimTime::ZERO)
        .unwrap();
    let exti = machine.stm32_exti.as_ref().expect("STM32 machine has EXTI");
    assert_eq!(exti.poll(0, SimTime::ZERO), 0);
    machine.set_pin(3, Logic::One).unwrap();
    assert_eq!(exti.poll(1 << 3, SimTime::from_ticks(1)), 1 << 3);
    assert_eq!(
        machine.bus.read(
            base + 0x14,
            AccessWidth::Word,
            AccessKind::Read,
            SimTime::ZERO
        ),
        Ok(1 << 3)
    );
}

#[test]
fn stm32l432_native_wwdg_window_exposes_early_wakeup_and_reset() {
    let mut machine = ArmMcuMachine::new(TargetId::Stm32l432kc).unwrap();
    let base = 0x4000_2c00;
    machine
        .bus
        .write(base + 4, AccessWidth::Word, 1 << 9 | 0x60, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(base, AccessWidth::Word, 1 << 7 | 0x45, SimTime::ZERO)
        .unwrap();
    let wwdg = machine.stm32_wwdg.as_ref().expect("STM32 owns WWDG");
    assert_eq!(wwdg.poll(SimTime::from_ticks(16 * 5)), (true, false));
    assert_eq!(
        machine
            .bus
            .read(base + 8, AccessWidth::Word, AccessKind::Read, SimTime::ZERO),
        Ok(1)
    );
    assert_eq!(wwdg.poll(SimTime::from_ticks(16 * 6)), (true, true));
}

#[test]
fn stm32l432_native_tim6_maps_basic_update_and_interrupt_registers() {
    let mut machine = ArmMcuMachine::new(TargetId::Stm32l432kc).unwrap();
    let base = 0x4000_1000;
    machine
        .bus
        .write(base + 0x28, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(base + 0x2c, AccessWidth::Word, 3, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(base + 0x0c, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(base, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    let tim6 = machine.stm32_tim6.as_ref().expect("STM32 owns TIM6");
    assert!(!tim6.poll(SimTime::from_ticks(7)));
    assert!(tim6.poll(SimTime::from_ticks(8)));
    assert_eq!(
        machine.bus.read(
            base + 0x10,
            AccessWidth::Word,
            AccessKind::Read,
            SimTime::ZERO
        ),
        Ok(1)
    );
}

#[test]
fn stm32l432_native_tim7_maps_basic_update_and_interrupt_registers() {
    let mut machine = ArmMcuMachine::new(TargetId::Stm32l432kc).unwrap();
    let base = 0x4000_1400;
    machine
        .bus
        .write(base + 0x28, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(base + 0x2c, AccessWidth::Word, 3, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(base + 0x0c, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(base, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    let tim7 = machine.stm32_tim7.as_ref().expect("STM32 owns TIM7");
    assert!(!tim7.poll(SimTime::from_ticks(7)));
    assert!(tim7.poll(SimTime::from_ticks(8)));
    assert_eq!(
        machine.bus.read(
            base + 0x10,
            AccessWidth::Word,
            AccessKind::Read,
            SimTime::ZERO
        ),
        Ok(1)
    );
}

#[test]
fn stm32l432_native_tim15_maps_counter_compare_and_pwm_registers() {
    let mut machine = ArmMcuMachine::new(TargetId::Stm32l432kc).unwrap();
    let base = 0x4001_4000;
    for (offset, value) in [
        (0x28, 1),
        (0x2c, 3),
        (0x34, 2),
        (0x18, 6 << 4),
        (0x20, 1),
        (0x0c, 3),
        (0x00, 1),
    ] {
        machine
            .bus
            .write(base + offset, AccessWidth::Word, value, SimTime::ZERO)
            .unwrap();
    }
    let tim15 = machine.stm32_tim15.as_ref().expect("STM32 owns TIM15");
    assert!(tim15.poll(SimTime::from_ticks(4)));
    assert!(!tim15.channel_one_output());
    assert_eq!(
        machine.bus.read(
            base + 0x10,
            AccessWidth::Word,
            AccessKind::Read,
            SimTime::ZERO
        ),
        Ok(2)
    );
}

#[test]
fn stm32l432_native_tim16_maps_counter_compare_and_pwm_registers() {
    let mut machine = ArmMcuMachine::new(TargetId::Stm32l432kc).unwrap();
    let base = 0x4001_4400;
    for (offset, value) in [
        (0x28, 1),
        (0x2c, 3),
        (0x34, 2),
        (0x18, 6 << 4),
        (0x20, 1),
        (0x0c, 3),
        (0x00, 1),
    ] {
        machine
            .bus
            .write(base + offset, AccessWidth::Word, value, SimTime::ZERO)
            .unwrap();
    }
    let tim16 = machine.stm32_tim16.as_ref().expect("STM32 owns TIM16");
    assert!(tim16.poll(SimTime::from_ticks(4)));
    assert!(!tim16.channel_one_output());
    assert_eq!(
        machine.bus.read(
            base + 0x10,
            AccessWidth::Word,
            AccessKind::Read,
            SimTime::ZERO
        ),
        Ok(2)
    );
}

fn configure_stm32_lptim(machine: &mut ArmMcuMachine, base: u64) {
    for (offset, value) in [(0x18, 3), (0x14, 2), (0x0c, 1 << 7), (0x08, 3), (0x10, 5)] {
        machine
            .bus
            .write(base + offset, AccessWidth::Word, value, SimTime::ZERO)
            .unwrap();
    }
}

#[test]
fn stm32l432_native_lptim1_maps_counter_compare_and_interrupt_registers() {
    let mut machine = ArmMcuMachine::new(TargetId::Stm32l432kc).unwrap();
    let base = 0x4000_7c00;
    configure_stm32_lptim(&mut machine, base);
    assert!(
        machine
            .stm32_lptim1
            .as_ref()
            .expect("STM32 LPTIM1 handle")
            .poll(SimTime::from_ticks(4))
    );
    assert_eq!(
        machine.bus.read(
            base + 0x1c,
            AccessWidth::Word,
            AccessKind::Read,
            SimTime::ZERO
        ),
        Ok(2)
    );
    assert!(
        machine
            .stm32_lptim1
            .as_ref()
            .expect("STM32 LPTIM1 handle")
            .poll(SimTime::from_ticks(8))
    );
    assert_eq!(
        machine
            .bus
            .read(base, AccessWidth::Word, AccessKind::Read, SimTime::ZERO),
        Ok(3)
    );
    machine
        .bus
        .write(base + 0x04, AccessWidth::Word, 3, SimTime::from_ticks(8))
        .unwrap();
    assert_eq!(
        machine
            .bus
            .read(base, AccessWidth::Word, AccessKind::Read, SimTime::ZERO),
        Ok(0)
    );
}

#[test]
fn stm32l432_native_lptim2_maps_counter_compare_and_interrupt_registers() {
    let mut machine = ArmMcuMachine::new(TargetId::Stm32l432kc).unwrap();
    let base = 0x4000_9400;
    configure_stm32_lptim(&mut machine, base);
    assert!(
        machine
            .stm32_lptim2
            .as_ref()
            .expect("STM32 LPTIM2 handle")
            .poll(SimTime::from_ticks(4))
    );
    assert_eq!(
        machine.bus.read(
            base + 0x1c,
            AccessWidth::Word,
            AccessKind::Read,
            SimTime::ZERO
        ),
        Ok(2)
    );
    assert!(
        machine
            .stm32_lptim2
            .as_ref()
            .expect("STM32 LPTIM2 handle")
            .poll(SimTime::from_ticks(8))
    );
    assert_eq!(
        machine
            .bus
            .read(base, AccessWidth::Word, AccessKind::Read, SimTime::ZERO),
        Ok(3)
    );
    machine
        .bus
        .write(base + 0x04, AccessWidth::Word, 3, SimTime::from_ticks(8))
        .unwrap();
    assert_eq!(
        machine
            .bus
            .read(base, AccessWidth::Word, AccessKind::Read, SimTime::ZERO),
        Ok(0)
    );
}

#[test]
fn ra4m1_uses_m4f_and_its_own_ioport_and_icu_map() {
    let mut machine = ArmMcuMachine::new(TargetId::R7fa4m1ab3cfm).unwrap();
    assert_eq!(machine.cpu.profile(), ArmProfile::CortexM4F);
    machine
        .bus
        .write(0x4004_0020, AccessWidth::Word, 1 << 11, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x4004_0028, AccessWidth::Word, 1 << 11, SimTime::ZERO)
        .unwrap();
    assert_eq!(machine.gpio_output(), 1 << 11);
    machine
        .bus
        .write(
            0x4000_6300,
            AccessWidth::Word,
            u64::from(RA4M1_EVENT_GPT0_OVERFLOW),
            SimTime::ZERO,
        )
        .unwrap();
}
