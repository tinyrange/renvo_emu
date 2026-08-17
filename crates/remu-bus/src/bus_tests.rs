use super::*;
use std::cell::Cell;

struct CollectingObserver(Rc<RefCell<Vec<BusAccessRecord>>>);

impl BusAccessObserver for CollectingObserver {
    fn observe(&mut self, record: &BusAccessRecord) {
        self.0.borrow_mut().push(record.clone());
    }
}

struct InterruptCollectingObserver(Rc<RefCell<Vec<InterruptTransitionRecord>>>);

impl BusAccessObserver for InterruptCollectingObserver {
    fn observe(&mut self, _record: &BusAccessRecord) {}

    fn observe_interrupt(&mut self, record: &InterruptTransitionRecord) {
        self.0.borrow_mut().push(record.clone());
    }
}

struct TraceableRegister {
    value: u64,
    reads: Rc<Cell<u32>>,
}

impl Device for TraceableRegister {
    fn name(&self) -> &str {
        "traceable-register"
    }

    fn read(
        &mut self,
        _offset: u64,
        _width: AccessWidth,
        _at: SimTime,
    ) -> Result<u64, DeviceError> {
        self.reads.set(self.reads.get() + 1);
        Ok(self.value)
    }

    fn write(
        &mut self,
        _offset: u64,
        _width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        self.value = value;
        Ok(())
    }

    fn trace_value(&self, _offset: u64, _width: AccessWidth, _at: SimTime) -> Option<u64> {
        Some(self.value)
    }
}

#[test]
fn maps_and_accesses_little_endian_memory() {
    let mut bus = AddressSpace::default();
    bus.map_ram("ram", 0x2000_0000, 16, true).unwrap();
    bus.write(0x2000_0000, AccessWidth::Word, 0x4433_2211, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        bus.read(
            0x2000_0001,
            AccessWidth::HalfWord,
            AccessKind::Read,
            SimTime::ZERO
        )
        .unwrap(),
        0x3322
    );
}

#[test]
fn fast_fetch_is_disabled_when_accesses_are_observable() {
    let mut bus = AddressSpace::default();
    bus.map_rom("rom", 0x1000, vec![0x11, 0x22, 0x33, 0x44])
        .unwrap();

    assert_eq!(
        bus.fast_fetch32(0x1000, SimTime::ZERO)
            .expect("unobserved memory fetch uses the fast path")
            .unwrap(),
        0x4433_2211
    );

    bus.set_access_recording(true);
    assert!(bus.fast_fetch32(0x1000, SimTime::ZERO).is_none());
    bus.set_access_recording(false);
    bus.add_watchpoint(0x1000);
    assert!(bus.fast_fetch32(0x1000, SimTime::ZERO).is_none());
}

#[test]
fn fast_fetch_falls_back_at_a_region_boundary() {
    let mut bus = AddressSpace::default();
    bus.map_rom("rom", 0x1000, vec![0x11, 0x22, 0x33, 0x44])
        .unwrap();

    assert!(bus.fast_fetch32(0x1002, SimTime::ZERO).is_none());
    assert_eq!(
        bus.read(
            0x1002,
            AccessWidth::HalfWord,
            AccessKind::Execute,
            SimTime::ZERO
        )
        .unwrap(),
        0x4433
    );
}

#[test]
fn fast_data_paths_preserve_width_endianness_and_observation() {
    let mut little = AddressSpace::default();
    little.map_ram("ram", 0x1000, 16, false).unwrap();
    assert!(little.fast_write(0x1001, AccessWidth::Word, 0x8877_6655));
    assert_eq!(
        little.fast_read(0x1001, AccessWidth::Word),
        Some(0x8877_6655)
    );
    assert_eq!(
        little.fast_read(0x1002, AccessWidth::HalfWord),
        Some(0x7766)
    );
    assert!(little.fast_read(0x100e, AccessWidth::Word).is_none());

    little.set_access_recording(true);
    assert!(little.fast_read(0x1001, AccessWidth::Word).is_none());
    assert!(!little.fast_write(0x1001, AccessWidth::Word, 0));

    let mut big = AddressSpace::new(Endianness::Big);
    big.map_ram("ram", 0x2000, 8, false).unwrap();
    assert!(big.fast_write(0x2000, AccessWidth::Word, 0x1122_3344));
    assert_eq!(big.fast_read(0x2001, AccessWidth::HalfWord), Some(0x2233));
}

#[test]
fn rejects_overlap_and_cross_boundary_accesses() {
    let mut bus = AddressSpace::default();
    bus.map_ram("ram", 0x1000, 4, false).unwrap();
    assert!(matches!(
        bus.map_ram("overlap", 0x1003, 4, false),
        Err(MapError::Overlap { .. })
    ));
    let fault = bus
        .read(0x1002, AccessWidth::Word, AccessKind::Read, SimTime::ZERO)
        .unwrap_err();
    assert_eq!(fault.kind, BusFaultKind::Boundary);
}

#[test]
fn write_ignored_rom_acknowledges_without_mutating() {
    let mut bus = AddressSpace::default();
    bus.map_write_ignored_rom("rom", 0, vec![0x11, 0x22, 0x33, 0x44])
        .unwrap();

    bus.write(0, AccessWidth::Word, 0xaabb_ccdd, SimTime::ZERO)
        .unwrap();

    assert_eq!(
        bus.read(0, AccessWidth::Word, AccessKind::Read, SimTime::ZERO)
            .unwrap(),
        0x4433_2211
    );
}

#[test]
fn aliases_share_memory() {
    let mut bus = AddressSpace::default();
    let ram = bus.map_ram("ram", 0x1000, 8, false).unwrap();
    bus.map_shared("alias", 0x2000, 8, Permissions::RW, ram, 0)
        .unwrap();
    bus.write(0x1000, AccessWidth::Word, 42, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        bus.read(0x2000, AccessWidth::Word, AccessKind::Read, SimTime::ZERO)
            .unwrap(),
        42
    );
}

#[test]
fn loader_can_initialize_rom() {
    let mut bus = AddressSpace::default();
    bus.map_rom("flash", 0, vec![0; 8]).unwrap();
    bus.load(2, &[0xaa, 0xbb]).unwrap();
    assert_eq!(
        bus.read(2, AccessWidth::HalfWord, AccessKind::Read, SimTime::ZERO)
            .unwrap(),
        0xbbaa
    );
    assert_eq!(
        bus.write(2, AccessWidth::Byte, 0, SimTime::ZERO)
            .unwrap_err()
            .kind,
        BusFaultKind::Permission
    );
}

#[test]
fn watchpoints_report_completed_overlapping_data_accesses_only() {
    let mut bus = AddressSpace::default();
    bus.map_ram("ram", 0x1000, 16, true).unwrap();
    bus.add_watchpoint(0x1002);

    bus.read(
        0x1000,
        AccessWidth::Word,
        AccessKind::Execute,
        SimTime::ZERO,
    )
    .unwrap();
    assert!(bus.take_watchpoint_hit().is_none());

    bus.write(
        0x1000,
        AccessWidth::Word,
        0x4433_2211,
        SimTime::from_ticks(1),
    )
    .unwrap();
    let hit = bus.take_watchpoint_hit().unwrap();
    assert_eq!(hit.address, 0x1000);
    assert_eq!(hit.kind, AccessKind::Write);
    assert_eq!(hit.width, AccessWidth::Word);

    bus.clear_watchpoints();
    bus.read(
        0x1002,
        AccessWidth::Byte,
        AccessKind::Read,
        SimTime::from_ticks(2),
    )
    .unwrap();
    assert!(bus.take_watchpoint_hit().is_none());
}

#[test]
fn write_watchpoints_ignore_reads_and_stop_on_overlapping_writes() {
    let mut bus = AddressSpace::default();
    bus.map_ram("ram", 0x1000, 16, true).unwrap();
    bus.add_write_watchpoint(0x1002);
    bus.read(0x1000, AccessWidth::Word, AccessKind::Read, SimTime::ZERO)
        .unwrap();
    assert!(bus.take_watchpoint_hit().is_none());
    bus.write(
        0x1000,
        AccessWidth::Word,
        0x4433_2211,
        SimTime::from_ticks(1),
    )
    .unwrap();
    assert_eq!(bus.take_watchpoint_hit().unwrap().kind, AccessKind::Write);
}

#[test]
fn masked_write_watchpoints_require_the_value_predicate() {
    let mut bus = AddressSpace::default();
    bus.map_ram("ram", 0x1000, 16, true).unwrap();
    bus.add_masked_write_watchpoint(0x1000, 0xc000_0000, 0xc000_0000);
    bus.write(0x1000, AccessWidth::Word, 0x8000_1234, SimTime::ZERO)
        .unwrap();
    assert!(bus.take_watchpoint_hit().is_none());
    bus.write(
        0x1000,
        AccessWidth::Word,
        0xc000_1234,
        SimTime::from_ticks(1),
    )
    .unwrap();
    assert_eq!(bus.take_watchpoint_hit().unwrap().value, 0xc000_1234);
}

#[test]
fn observer_streams_without_populating_the_in_memory_log() {
    let records = Rc::new(RefCell::new(Vec::new()));
    let observer: SharedBusAccessObserver =
        Rc::new(RefCell::new(CollectingObserver(records.clone())));
    let mut bus = AddressSpace::default();
    bus.map_ram("ram", 0x1000, 16, true).unwrap();
    bus.set_access_observer(Some(observer));

    bus.write(
        0x1000,
        AccessWidth::Word,
        0x4433_2211,
        SimTime::from_ticks(1),
    )
    .unwrap();
    bus.read(
        0x1000,
        AccessWidth::Word,
        AccessKind::Execute,
        SimTime::from_ticks(2),
    )
    .unwrap();

    assert!(bus.access_log().is_empty());
    let records = records.borrow();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].kind, AccessKind::Write);
    assert_eq!(records[1].kind, AccessKind::Execute);
}

#[test]
fn observation_pc_is_correlated_without_leaking_into_later_activity() {
    let records = Rc::new(RefCell::new(Vec::new()));
    let observer: SharedBusAccessObserver =
        Rc::new(RefCell::new(CollectingObserver(records.clone())));
    let mut bus = AddressSpace::default();
    bus.map_ram("ram", 0x1000, 16, true).unwrap();
    bus.set_access_observer(Some(observer));

    bus.set_observation_pc(Some(0x4200_1234));
    bus.write(0x1000, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    bus.set_observation_pc(None);
    bus.write(0x1004, AccessWidth::Word, 2, SimTime::from_ticks(1))
        .unwrap();

    let records = records.borrow();
    assert_eq!(records[0].pc, Some(0x4200_1234));
    assert_eq!(records[0].pre_value, Some(0));
    assert_eq!(records[0].post_value, Some(1));
    assert_eq!(records[1].pc, None);
    assert_eq!(records[1].pre_value, Some(0));
    assert_eq!(records[1].post_value, Some(2));
}

#[test]
fn device_pre_post_trace_uses_snapshot_hook_without_reading() {
    let records = Rc::new(RefCell::new(Vec::new()));
    let reads = Rc::new(Cell::new(0));
    let mut bus = AddressSpace::default();
    bus.map_device(
        "register",
        0x2000,
        4,
        Box::new(TraceableRegister {
            value: 0x11,
            reads: reads.clone(),
        }),
    )
    .unwrap();
    bus.set_access_observer(Some(Rc::new(RefCell::new(CollectingObserver(
        records.clone(),
    )))));

    bus.write(0x2000, AccessWidth::Word, 0x22, SimTime::ZERO)
        .unwrap();

    assert_eq!(reads.get(), 0);
    assert_eq!(records.borrow()[0].pre_value, Some(0x11));
    assert_eq!(records.borrow()[0].post_value, Some(0x22));
}

#[test]
fn interrupt_transitions_are_observational_and_pc_scoped() {
    let transitions = Rc::new(RefCell::new(Vec::new()));
    let mut bus = AddressSpace::default();
    bus.set_access_observer(Some(Rc::new(RefCell::new(InterruptCollectingObserver(
        transitions.clone(),
    )))));

    bus.set_observation_pc(Some(0x4000_1234));
    bus.observe_interrupt_transition(SimTime::from_ticks(7), "radio", 12, true);
    bus.set_observation_pc(None);
    bus.observe_interrupt_transition(SimTime::from_ticks(8), "radio", 12, false);

    assert_eq!(
        *transitions.borrow(),
        [
            InterruptTransitionRecord {
                at: SimTime::from_ticks(7),
                pc: Some(0x4000_1234),
                source: "radio".to_owned(),
                line: 12,
                asserted: true,
            },
            InterruptTransitionRecord {
                at: SimTime::from_ticks(8),
                pc: None,
                source: "radio".to_owned(),
                line: 12,
                asserted: false,
            },
        ]
    );
}

#[test]
fn added_observer_preserves_the_existing_stream() {
    let first = Rc::new(RefCell::new(Vec::new()));
    let second = Rc::new(RefCell::new(Vec::new()));
    let mut bus = AddressSpace::default();
    bus.map_ram("ram", 0x1000, 16, true).unwrap();
    bus.set_access_observer(Some(Rc::new(RefCell::new(CollectingObserver(
        first.clone(),
    )))));
    bus.add_access_observer(Rc::new(RefCell::new(CollectingObserver(second.clone()))));

    bus.write(
        0x1000,
        AccessWidth::Word,
        0x4433_2211,
        SimTime::from_ticks(1),
    )
    .unwrap();

    assert_eq!(*first.borrow(), *second.borrow());
    assert_eq!(first.borrow().len(), 1);
}
