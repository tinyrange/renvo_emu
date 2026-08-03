use super::*;

const UNITS: usize = 4;
const UNIT_STRIDE: u64 = 0x0c;
const COUNT_BASE: u64 = 0x30;
const STATUS_BASE: u64 = 0x50;
const INT_RAW: u64 = 0x40;
const INT_ST: u64 = 0x44;
const INT_ENA: u64 = 0x48;
const INT_CLR: u64 = 0x4c;
const CONTROL: u64 = 0x60;
const DATE: u64 = 0xfc;
const CONF0_MASK: u32 = u32::MAX;
const CONF1_MASK: u32 = u32::MAX;
const CONF2_MASK: u32 = u32::MAX;
const COUNT_MASK: u32 = 0x0000_ffff;
const STATUS_MASK: u32 = 0x0000_007f;
const INTERRUPT_MASK: u32 = 0x0000_000f;
const CONTROL_MASK: u32 = 0x0001_00ff;
const CONTROL_RESET: u32 = 0x0000_0055;
const DATE_RESET: u32 = 0x1907_2601;

/// Named ESP32-S3 PCNT register IDs covered by the functional model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Esp32s3PcntRegister {
    /// Unit configuration register zero.
    UnitConf0(usize),
    /// Unit threshold configuration register.
    UnitConf1(usize),
    /// Unit limit configuration register.
    UnitConf2(usize),
    /// Unit signed pulse-count readback register.
    UnitCount(usize),
    /// Unit threshold-status readback register.
    UnitStatus(usize),
    /// Raw threshold-event interrupt status.
    IntRaw,
    /// Masked threshold-event interrupt status.
    IntSt,
    /// Threshold-event interrupt enables.
    IntEna,
    /// Write-one-to-clear threshold-event interrupts.
    IntClr,
    /// Counter reset, pause and register-clock controls.
    Control,
    /// Version/date register.
    Date,
}

impl Esp32s3PcntRegister {
    /// Returns the native byte offset of this register ID.
    pub const fn offset(self) -> u64 {
        match self {
            Self::UnitConf0(unit) => (unit as u64) * UNIT_STRIDE,
            Self::UnitConf1(unit) => (unit as u64) * UNIT_STRIDE + 0x04,
            Self::UnitConf2(unit) => (unit as u64) * UNIT_STRIDE + 0x08,
            Self::UnitCount(unit) => COUNT_BASE + (unit as u64) * 4,
            Self::UnitStatus(unit) => STATUS_BASE + (unit as u64) * 4,
            Self::IntRaw => INT_RAW,
            Self::IntSt => INT_ST,
            Self::IntEna => INT_ENA,
            Self::IntClr => INT_CLR,
            Self::Control => CONTROL,
            Self::Date => DATE,
        }
    }

    /// Converts a modeled native offset into a named register ID.
    pub const fn from_offset(offset: u64) -> Option<Self> {
        if offset < COUNT_BASE {
            let unit = offset / UNIT_STRIDE;
            let register = offset % UNIT_STRIDE;
            if unit < UNITS as u64 {
                return Some(match register {
                    0x00 => Self::UnitConf0(unit as usize),
                    0x04 => Self::UnitConf1(unit as usize),
                    0x08 => Self::UnitConf2(unit as usize),
                    _ => return None,
                });
            }
        }
        if offset >= COUNT_BASE && offset < STATUS_BASE {
            let unit = (offset - COUNT_BASE) / 4;
            if unit < UNITS as u64 {
                return Some(Self::UnitCount(unit as usize));
            }
        }
        if offset >= STATUS_BASE && offset < CONTROL {
            let unit = (offset - STATUS_BASE) / 4;
            if unit < UNITS as u64 {
                return Some(Self::UnitStatus(unit as usize));
            }
        }
        Some(match offset {
            INT_RAW => Self::IntRaw,
            INT_ST => Self::IntSt,
            INT_ENA => Self::IntEna,
            INT_CLR => Self::IntClr,
            CONTROL => Self::Control,
            DATE => Self::Date,
            _ => return None,
        })
    }
}

/// Edge polarity accepted by the host-facing pulse-counter handle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EspPcntEdge {
    /// A low-to-high input transition.
    Rising,
    /// A high-to-low input transition.
    Falling,
}

#[derive(Clone, Copy)]
struct PcntUnit {
    conf0: u32,
    conf1: u32,
    conf2: u32,
    count: i16,
    status: u32,
}

impl PcntUnit {
    const fn reset() -> Self {
        Self {
            conf0: (16 << 0) | (1 << 10) | (1 << 11) | (1 << 12) | (1 << 13),
            conf1: 0,
            conf2: 0,
            count: 0,
            status: 0,
        }
    }

    fn mode(self, edge: EspPcntEdge) -> u32 {
        let shift = match edge {
            EspPcntEdge::Rising => 18,
            EspPcntEdge::Falling => 16,
        };
        (self.conf0 >> shift) & 3
    }

    fn update_status(&mut self) -> bool {
        let count = i32::from(self.count);
        let mut status = 0_u32;
        if count == 0 && self.conf0 & (1 << 11) != 0 {
            status |= 1 << 6;
        }
        let thres0 = i32::from((self.conf1 & 0xffff) as i16);
        let thres1 = i32::from((self.conf1 >> 16) as i16);
        let high_limit = i32::from((self.conf2 & 0xffff) as i16);
        let low_limit = i32::from((self.conf2 >> 16) as i16);
        if self.conf0 & (1 << 14) != 0 && count == thres0 {
            status |= 1 << 3;
        }
        if self.conf0 & (1 << 15) != 0 && count == thres1 {
            status |= 1 << 2;
        }
        if self.conf0 & (1 << 12) != 0 && count == high_limit {
            status |= 1 << 5;
        }
        if self.conf0 & (1 << 13) != 0 && count == low_limit {
            status |= 1 << 4;
        }
        let event = status != 0;
        self.status = status;
        event
    }
}

struct PcntState {
    units: [PcntUnit; UNITS],
    int_raw: u32,
    int_ena: u32,
    control: u32,
    date: u32,
    counts: [u16; UNITS],
}

impl PcntState {
    const fn reset() -> Self {
        Self {
            units: [PcntUnit::reset(); UNITS],
            int_raw: 0,
            int_ena: 0,
            control: CONTROL_RESET,
            date: DATE_RESET,
            counts: [0; UNITS],
        }
    }

    fn sync_counts(&mut self) {
        for (index, unit) in self.units.iter().enumerate() {
            self.counts[index] = unit.count as u16;
        }
    }

    fn pulse(&mut self, unit: usize, edge: EspPcntEdge) -> bool {
        if self.control & (1 << (2 * unit + 1)) != 0 {
            return false;
        }
        let unit_state = &mut self.units[unit];
        let mode = unit_state.mode(edge);
        let previous = unit_state.count;
        match mode {
            1 => unit_state.count = unit_state.count.saturating_add(1),
            2 => unit_state.count = unit_state.count.saturating_sub(1),
            _ => {}
        }
        let changed = unit_state.count != previous;
        if changed {
            let event = unit_state.update_status();
            if event {
                self.int_raw |= 1 << unit;
            }

            // The native counter wraps through zero when an enabled high or
            // low limit is reached.  A zero register value means that limit
            // is not configured, so it must not make every pulse wrap.
            let count = i32::from(unit_state.count);
            let high_limit = i32::from((unit_state.conf2 & 0xffff) as i16);
            let low_limit = i32::from((unit_state.conf2 >> 16) as i16);
            let reached_high =
                unit_state.conf0 & (1 << 12) != 0 && high_limit > 0 && count >= high_limit;
            let reached_low =
                unit_state.conf0 & (1 << 13) != 0 && low_limit < 0 && count <= low_limit;
            if reached_high || reached_low {
                unit_state.count = 0;
            }
        }
        self.sync_counts();
        changed
    }
}

/// Host-facing ESP32-S3 pulse-counter handle.
#[derive(Clone)]
pub struct Esp32S3PcntHandle {
    state: Rc<RefCell<PcntState>>,
    hub: SignalHub,
    count_signals: Vec<SignalId>,
}

impl Esp32S3PcntHandle {
    /// Applies one deterministic rising or falling edge to a unit.
    pub fn pulse(&self, unit: usize, edge: EspPcntEdge, at: SimTime) -> Result<bool, SignalError> {
        let mut state = self.state.borrow_mut();
        let changed = state.pulse(unit, edge);
        if changed {
            self.hub.set(
                self.count_signals[unit],
                SignalValue::from_u64(u64::from(state.counts[unit]), 16)?,
                at,
            )?;
        }
        Ok(changed)
    }

    /// Returns the signed count for a unit.
    pub fn count(&self, unit: usize) -> i16 {
        self.state.borrow().units[unit].count
    }

    /// Returns the pending raw interrupt mask.
    pub fn raw_interrupts(&self) -> u32 {
        self.state.borrow().int_raw
    }
}

/// Functional ESP32-S3 PCNT pulse-counter block.
///
/// The model covers the native four-unit configuration, signed 16-bit counts,
/// positive/negative edge actions, pause/reset controls, threshold status and
/// interrupt latches, and deterministic count VCD signals. GPIO-matrix input
/// routing, glitch-filter timing, quadrature coupling, and exact interrupt
/// matrix delivery remain outside this functional slice.
pub struct Esp32S3Pcnt {
    name: String,
    state: Rc<RefCell<PcntState>>,
    hub: SignalHub,
    count_signals: Vec<SignalId>,
}

impl Esp32S3Pcnt {
    /// Creates the four-unit PCNT block and host pulse handle.
    pub fn new(
        name: impl Into<String>,
        signal_path: &str,
        hub: SignalHub,
    ) -> Result<(Self, Esp32S3PcntHandle), SignalError> {
        let mut count_signals = Vec::with_capacity(UNITS);
        for unit in 0..UNITS {
            count_signals.push(hub.declare(
                format!("{signal_path}.u{unit}"),
                SignalValue::from_u64(0, 16)?,
                Some("Functional ESP32-S3 PCNT signed count".to_owned()),
            )?);
        }
        let state = Rc::new(RefCell::new(PcntState::reset()));
        let device = Self {
            name: name.into(),
            state: state.clone(),
            hub: hub.clone(),
            count_signals: count_signals.clone(),
        };
        let handle = Esp32S3PcntHandle {
            state,
            hub,
            count_signals,
        };
        Ok((device, handle))
    }

    fn read_register(&self, offset: u64) -> Result<u32, DeviceError> {
        let state = self.state.borrow();
        match Esp32s3PcntRegister::from_offset(offset) {
            Some(Esp32s3PcntRegister::UnitConf0(unit)) => Ok(state.units[unit].conf0),
            Some(Esp32s3PcntRegister::UnitConf1(unit)) => Ok(state.units[unit].conf1),
            Some(Esp32s3PcntRegister::UnitConf2(unit)) => Ok(state.units[unit].conf2),
            Some(Esp32s3PcntRegister::UnitCount(unit)) => {
                Ok(u32::from(state.counts[unit]) & COUNT_MASK)
            }
            Some(Esp32s3PcntRegister::UnitStatus(unit)) => {
                Ok(state.units[unit].status & STATUS_MASK)
            }
            Some(Esp32s3PcntRegister::IntRaw) => Ok(state.int_raw & INTERRUPT_MASK),
            Some(Esp32s3PcntRegister::IntSt) => {
                Ok((state.int_raw & state.int_ena) & INTERRUPT_MASK)
            }
            Some(Esp32s3PcntRegister::IntEna) => Ok(state.int_ena & INTERRUPT_MASK),
            Some(Esp32s3PcntRegister::IntClr) => {
                Err(DeviceError::new("ESP32-S3 PCNT INT_CLR is write-only"))
            }
            Some(Esp32s3PcntRegister::Control) => Ok(state.control & CONTROL_MASK),
            Some(Esp32s3PcntRegister::Date) => Ok(state.date),
            None => Err(DeviceError::new(format!(
                "unmodeled ESP32-S3 PCNT read at offset {offset:#x}"
            ))),
        }
    }

    fn publish_counts(&self, at: SimTime) -> Result<(), DeviceError> {
        let state = self.state.borrow();
        for unit in 0..UNITS {
            self.hub
                .set(
                    self.count_signals[unit],
                    SignalValue::from_u64(u64::from(state.counts[unit]), 16)
                        .map_err(|error| DeviceError::new(error.to_string()))?,
                    at,
                )
                .map_err(|error| DeviceError::new(error.to_string()))?;
        }
        Ok(())
    }
}

impl Device for Esp32S3Pcnt {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP32-S3 PCNT requires aligned word access",
            ));
        }
        Ok(u64::from(self.read_register(offset)?))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP32-S3 PCNT requires aligned word access",
            ));
        }
        let value = u32::try_from(value)
            .map_err(|_| DeviceError::new("ESP32-S3 PCNT value exceeds u32"))?;
        let mut state = self.state.borrow_mut();
        match Esp32s3PcntRegister::from_offset(offset) {
            Some(Esp32s3PcntRegister::UnitConf0(unit)) => {
                state.units[unit].conf0 = value & CONF0_MASK;
            }
            Some(Esp32s3PcntRegister::UnitConf1(unit)) => {
                state.units[unit].conf1 = value & CONF1_MASK;
            }
            Some(Esp32s3PcntRegister::UnitConf2(unit)) => {
                state.units[unit].conf2 = value & CONF2_MASK;
            }
            Some(Esp32s3PcntRegister::UnitCount(unit)) => {
                return Err(DeviceError::new(format!(
                    "ESP32-S3 PCNT U{unit} count is read-only"
                )));
            }
            Some(Esp32s3PcntRegister::UnitStatus(unit)) => {
                return Err(DeviceError::new(format!(
                    "ESP32-S3 PCNT U{unit} status is read-only"
                )));
            }
            Some(Esp32s3PcntRegister::IntEna) => state.int_ena = value & INTERRUPT_MASK,
            Some(Esp32s3PcntRegister::IntClr) => state.int_raw &= !(value & INTERRUPT_MASK),
            Some(Esp32s3PcntRegister::Control) => {
                state.control = value & CONTROL_MASK;
                for unit in 0..UNITS {
                    if value & (1 << (2 * unit)) != 0 {
                        state.units[unit].count = 0;
                        state.units[unit].status = 0;
                    }
                }
                state.sync_counts();
            }
            Some(Esp32s3PcntRegister::Date) => state.date = value,
            Some(Esp32s3PcntRegister::IntRaw | Esp32s3PcntRegister::IntSt) => {
                return Err(DeviceError::new("PCNT interrupt status is read-only"));
            }
            None => {
                return Err(DeviceError::new(format!(
                    "unmodeled ESP32-S3 PCNT write at offset {offset:#x}"
                )));
            }
        }
        drop(state);
        self.publish_counts(at)
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.borrow_mut() = PcntState::reset();
        for signal in &self.count_signals {
            self.hub
                .set(
                    *signal,
                    SignalValue::from_u64(0, 16).expect("PCNT count signal is 16 bits"),
                    SimTime::ZERO,
                )
                .expect("PCNT count signals remain declared");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_ids_round_trip_native_offsets_and_reset_values() {
        let expected = [
            (Esp32s3PcntRegister::UnitConf0(0), 0x00),
            (Esp32s3PcntRegister::UnitConf1(3), 0x28),
            (Esp32s3PcntRegister::UnitConf2(3), 0x2c),
            (Esp32s3PcntRegister::UnitCount(0), 0x30),
            (Esp32s3PcntRegister::UnitCount(3), 0x3c),
            (Esp32s3PcntRegister::UnitStatus(0), 0x50),
            (Esp32s3PcntRegister::UnitStatus(3), 0x5c),
            (Esp32s3PcntRegister::IntRaw, 0x40),
            (Esp32s3PcntRegister::IntSt, 0x44),
            (Esp32s3PcntRegister::IntEna, 0x48),
            (Esp32s3PcntRegister::IntClr, 0x4c),
            (Esp32s3PcntRegister::Control, 0x60),
            (Esp32s3PcntRegister::Date, 0xfc),
        ];
        for (register, offset) in expected {
            assert_eq!(register.offset(), offset);
            assert_eq!(Esp32s3PcntRegister::from_offset(offset), Some(register));
        }
        assert_eq!(Esp32s3PcntRegister::from_offset(0x0a), None);
        assert_eq!(Esp32s3PcntRegister::from_offset(0x64), None);

        let hub = SignalHub::new();
        let (mut pcnt, _) = Esp32S3Pcnt::new("pcnt", "board.esp32s3.pcnt", hub).unwrap();
        assert_eq!(
            pcnt.read(0x00, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0x0000_3c10
        );
        assert_eq!(
            pcnt.read(0x60, AccessWidth::Word, SimTime::ZERO).unwrap(),
            u64::from(CONTROL_RESET)
        );
        assert_eq!(
            pcnt.read(0xfc, AccessWidth::Word, SimTime::ZERO).unwrap(),
            u64::from(DATE_RESET)
        );
    }

    #[test]
    fn register_masks_access_modes_and_date_write_follow_vendor_layout() {
        let hub = SignalHub::new();
        let (mut pcnt, handle) = Esp32S3Pcnt::new("pcnt", "board.esp32s3.pcnt", hub).unwrap();
        pcnt.write(0x00, AccessWidth::Word, u64::from(u32::MAX), SimTime::ZERO)
            .unwrap();
        pcnt.write(0x04, AccessWidth::Word, u64::from(u32::MAX), SimTime::ZERO)
            .unwrap();
        pcnt.write(0x08, AccessWidth::Word, u64::from(u32::MAX), SimTime::ZERO)
            .unwrap();
        assert_eq!(
            pcnt.read(0x00, AccessWidth::Word, SimTime::ZERO).unwrap(),
            u64::from(u32::MAX)
        );
        assert_eq!(
            pcnt.read(0x04, AccessWidth::Word, SimTime::ZERO).unwrap(),
            u64::from(u32::MAX)
        );
        assert_eq!(
            pcnt.read(0x08, AccessWidth::Word, SimTime::ZERO).unwrap(),
            u64::from(u32::MAX)
        );
        pcnt.write(0x48, AccessWidth::Word, u64::from(u32::MAX), SimTime::ZERO)
            .unwrap();
        assert_eq!(
            pcnt.read(0x48, AccessWidth::Word, SimTime::ZERO).unwrap(),
            u64::from(INTERRUPT_MASK)
        );
        pcnt.write(0x60, AccessWidth::Word, u64::from(u32::MAX), SimTime::ZERO)
            .unwrap();
        assert_eq!(
            pcnt.read(0x60, AccessWidth::Word, SimTime::ZERO).unwrap(),
            u64::from(CONTROL_MASK)
        );
        pcnt.write(0xfc, AccessWidth::Word, 0x1234_5678, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            pcnt.read(0xfc, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0x1234_5678
        );
        assert!(
            pcnt.write(0x30, AccessWidth::Word, 0, SimTime::ZERO)
                .is_err()
        );
        assert!(
            pcnt.write(0x50, AccessWidth::Word, 0, SimTime::ZERO)
                .is_err()
        );
        assert!(pcnt.read(0x4c, AccessWidth::Word, SimTime::ZERO).is_err());
        assert!(
            pcnt.write(
                0x00,
                AccessWidth::Word,
                u64::from(u32::MAX) + 1,
                SimTime::ZERO
            )
            .is_err()
        );
        assert!(
            !handle
                .pulse(0, EspPcntEdge::Rising, SimTime::from_ticks(1))
                .unwrap()
        );
    }

    #[test]
    fn configured_edges_update_signed_count_and_vcd() {
        let hub = SignalHub::new();
        let (mut pcnt, handle) =
            Esp32S3Pcnt::new("pcnt", "board.esp32s3.pcnt", hub.clone()).unwrap();
        pcnt.write(0x00, AccessWidth::Word, 1 << 18, SimTime::ZERO)
            .unwrap();
        assert!(
            handle
                .pulse(0, EspPcntEdge::Rising, SimTime::from_ticks(1))
                .unwrap()
        );
        assert!(
            handle
                .pulse(0, EspPcntEdge::Rising, SimTime::from_ticks(2))
                .unwrap()
        );
        assert_eq!(handle.count(0), 2);
        pcnt.write(0x00, AccessWidth::Word, 2 << 16, SimTime::from_ticks(2))
            .unwrap();
        handle
            .pulse(0, EspPcntEdge::Falling, SimTime::from_ticks(3))
            .unwrap();
        assert_eq!(handle.count(0), 1);
        let changes = hub.drain_changes();
        assert!(
            changes
                .iter()
                .any(|change| change.value.bit(1) == Some(Logic::One))
        );
        assert!(
            changes
                .iter()
                .any(|change| change.value.bit(0) == Some(Logic::One)
                    && change.value.bit(1) == Some(Logic::Zero))
        );
    }

    #[test]
    fn thresholds_latch_raw_interrupt_and_control_resets_count() {
        let hub = SignalHub::new();
        let (mut pcnt, handle) = Esp32S3Pcnt::new("pcnt", "board.esp32s3.pcnt", hub).unwrap();
        pcnt.write(
            0x00,
            AccessWidth::Word,
            (1 << 18) | (1 << 14),
            SimTime::ZERO,
        )
        .unwrap();
        pcnt.write(0x04, AccessWidth::Word, 2, SimTime::ZERO)
            .unwrap();
        pcnt.write(0x48, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        handle
            .pulse(0, EspPcntEdge::Rising, SimTime::from_ticks(1))
            .unwrap();
        handle
            .pulse(0, EspPcntEdge::Rising, SimTime::from_ticks(2))
            .unwrap();
        assert_eq!(handle.raw_interrupts(), 1);
        assert_eq!(
            pcnt.read(0x44, AccessWidth::Word, SimTime::ZERO).unwrap(),
            1
        );
        pcnt.write(0x4c, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        pcnt.write(0x60, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.count(0), 0);
    }

    #[test]
    fn configured_limits_wrap_through_zero_and_noop_edges_report_false() {
        let hub = SignalHub::new();
        let (mut pcnt, handle) = Esp32S3Pcnt::new("pcnt", "board.esp32s3.pcnt", hub).unwrap();

        // A positive edge reaches the enabled high limit and wraps to zero.
        pcnt.write(
            0x00,
            AccessWidth::Word,
            (1 << 18) | (1 << 12),
            SimTime::ZERO,
        )
        .unwrap();
        pcnt.write(0x08, AccessWidth::Word, 2, SimTime::ZERO)
            .unwrap();
        assert!(
            handle
                .pulse(0, EspPcntEdge::Rising, SimTime::from_ticks(1))
                .unwrap()
        );
        assert_eq!(handle.count(0), 1);
        assert!(
            handle
                .pulse(0, EspPcntEdge::Rising, SimTime::from_ticks(2))
                .unwrap()
        );
        assert_eq!(handle.count(0), 0);

        // A negative edge reaches the enabled low limit and also wraps.
        pcnt.write(
            0x00,
            AccessWidth::Word,
            (2 << 16) | (1 << 13),
            SimTime::from_ticks(2),
        )
        .unwrap();
        pcnt.write(0x08, AccessWidth::Word, 0xfffe_0000, SimTime::from_ticks(2))
            .unwrap();
        assert!(
            handle
                .pulse(0, EspPcntEdge::Falling, SimTime::from_ticks(3))
                .unwrap()
        );
        assert_eq!(handle.count(0), -1);
        assert!(
            handle
                .pulse(0, EspPcntEdge::Falling, SimTime::from_ticks(4))
                .unwrap()
        );
        assert_eq!(handle.count(0), 0);

        // Mode 3 is the native no-effect encoding, not a count change.
        pcnt.write(0x00, AccessWidth::Word, 3 << 18, SimTime::from_ticks(4))
            .unwrap();
        assert!(
            !handle
                .pulse(0, EspPcntEdge::Rising, SimTime::from_ticks(5))
                .unwrap()
        );
        assert_eq!(handle.count(0), 0);
    }
}
