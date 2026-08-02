use super::*;

#[test]
fn ch32v006_touch_key_maps_the_adc_register_sequence() {
    let mut machine = RiscVMachine::new(TargetId::Ch32v006).unwrap();
    machine.set_touch_key(2, 0x0bcd).unwrap();
    let base = 0x4001_2400;
    machine
        .bus
        .write(base + 0x08, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(
            base + 0x04,
            AccessWidth::Word,
            (1 << 24) | (1 << 5),
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(base + 0x34, AccessWidth::Word, 2, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(base + 0x3c, AccessWidth::Word, 4, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(base + 0x4c, AccessWidth::Word, 5, SimTime::ZERO)
        .unwrap();

    assert_eq!(
        machine
            .bus
            .read(base, AccessWidth::Word, AccessKind::Read, SimTime::ZERO)
            .unwrap()
            & 0x1f,
        1 << 4
    );
    assert_eq!(
        machine
            .bus
            .read(
                base + 0x4c,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::from_ticks(22)
            )
            .unwrap(),
        0x0bcd
    );
    assert_eq!(
        machine
            .bus
            .read(
                base,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::from_ticks(22)
            )
            .unwrap()
            & 0x1f,
        0
    );
}

#[test]
fn power_is_mapped_for_both_qingke_targets() {
    const PWR: u64 = 0x4000_7000;
    const PVDE: u64 = 1 << 4;
    const PDDS: u64 = 1 << 1;
    for target in [TargetId::Ch32v003, TargetId::Ch32v006] {
        let mut machine = RiscVMachine::new(target).unwrap();
        let power = machine.wch_power().expect("WCH target exposes PWR");
        machine
            .bus
            .write(PWR, AccessWidth::Word, PVDE, SimTime::ZERO)
            .unwrap();
        power.set_supply_low(true);
        assert_eq!(
            machine
                .bus
                .read(
                    PWR + 0x04,
                    AccessWidth::Word,
                    AccessKind::Read,
                    SimTime::ZERO
                )
                .unwrap(),
            1 << 2,
            "{target} PVD status"
        );
        machine
            .bus
            .write(PWR, AccessWidth::Word, PVDE | PDDS, SimTime::ZERO)
            .unwrap();
        assert!(power.standby_requested(), "{target} standby request");
    }
}

#[test]
fn i2c1_is_mapped_for_both_qingke_targets() {
    const I2C1: u64 = 0x4000_5400;
    const PE: u64 = 1;
    const START: u64 = 1 << 8;
    const STOP: u64 = 1 << 9;
    const ADDR: u64 = 1 << 1;
    const RXNE: u64 = 1 << 6;
    for target in [TargetId::Ch32v003, TargetId::Ch32v006] {
        let mut machine = RiscVMachine::new(target).unwrap();
        let i2c = machine.wch_i2c().expect("WCH target exposes I2C1");
        i2c.queue_read(0x50, &[0xde, 0xad]);
        machine
            .bus
            .write(I2C1, AccessWidth::HalfWord, PE | START, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            machine
                .bus
                .read(
                    I2C1 + 0x14,
                    AccessWidth::HalfWord,
                    AccessKind::Read,
                    SimTime::ZERO
                )
                .unwrap()
                & 1,
            1,
            "{target} start flag"
        );
        machine
            .bus
            .write(I2C1 + 0x10, AccessWidth::HalfWord, 0xa1, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            machine
                .bus
                .read(
                    I2C1 + 0x14,
                    AccessWidth::HalfWord,
                    AccessKind::Read,
                    SimTime::ZERO
                )
                .unwrap()
                & ADDR,
            ADDR,
            "{target} address acknowledge"
        );
        let _ = machine
            .bus
            .read(
                I2C1 + 0x14,
                AccessWidth::HalfWord,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap();
        let _ = machine
            .bus
            .read(
                I2C1 + 0x18,
                AccessWidth::HalfWord,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap();
        assert_ne!(
            machine
                .bus
                .read(
                    I2C1 + 0x14,
                    AccessWidth::HalfWord,
                    AccessKind::Read,
                    SimTime::ZERO
                )
                .unwrap()
                & RXNE,
            0,
            "{target} receive data ready"
        );
        assert_eq!(
            machine
                .bus
                .read(
                    I2C1 + 0x10,
                    AccessWidth::HalfWord,
                    AccessKind::Read,
                    SimTime::ZERO
                )
                .unwrap(),
            0xde,
            "{target} first received byte"
        );
        machine
            .bus
            .write(I2C1, AccessWidth::HalfWord, PE | STOP, SimTime::ZERO)
            .unwrap();
    }
}

#[test]
fn dma_moves_a_memory_word_and_latches_channel_completion() {
    let mut machine = RiscVMachine::new(TargetId::Ch32v003).unwrap();
    machine
        .bus
        .write(0x2000_0000, AccessWidth::Word, 0x1234_5678, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x4002_0010, AccessWidth::Word, 0x2000_0004, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x4002_0014, AccessWidth::Word, 0x2000_0000, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x4002_000c, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(
            0x4002_0008,
            AccessWidth::Word,
            u64::from(1_u32 | (1 << 1) | (1 << 4) | (1 << 6) | (2 << 8) | (2 << 10)),
            SimTime::ZERO,
        )
        .unwrap();
    let dma = machine
        .wch
        .as_ref()
        .expect("WCH target has DMA")
        .dma
        .clone();
    assert_eq!(dma.service(&mut machine.bus, SimTime::ZERO).unwrap(), 1);
    assert_eq!(
        machine
            .bus
            .read(
                0x2000_0004,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO
            )
            .unwrap(),
        0x1234_5678
    );
    assert!(dma.channel_pending(0));
}
