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
    counts: [u16; UNITS],
}

impl PcntState {
    const fn reset() -> Self {
        Self {
            units: [PcntUnit::reset(); UNITS],
            int_raw: 0,
            int_ena: 0,
            control: 0,
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
        match mode {
            1 => unit_state.count = unit_state.count.saturating_add(1),
            2 => unit_state.count = unit_state.count.saturating_sub(1),
            _ => {}
        }
        let event = unit_state.update_status();
        if event {
            self.int_raw |= 1 << unit;
        }
        self.sync_counts();
        mode != 0
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
        if let Some((unit, register)) = unit_register(offset) {
            let unit_state = state.units[unit];
            return Ok(match register {
                0 => unit_state.conf0,
                4 => unit_state.conf1,
                8 => unit_state.conf2,
                _ => unreachable!(),
            });
        }
        if let Some(unit) = count_register(offset) {
            return Ok(u32::from(state.counts[unit]));
        }
        if let Some(unit) = status_register(offset) {
            return Ok(state.units[unit].status);
        }
        match offset {
            INT_RAW => Ok(state.int_raw),
            INT_ST => Ok(state.int_raw & state.int_ena),
            INT_ENA => Ok(state.int_ena),
            CONTROL => Ok(state.control),
            DATE => Ok(0x1907_2601),
            _ => Err(DeviceError::new(format!(
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

fn unit_register(offset: u64) -> Option<(usize, u64)> {
    if offset >= COUNT_BASE {
        return None;
    }
    let unit = usize::try_from(offset / UNIT_STRIDE).ok()?;
    let register = offset % UNIT_STRIDE;
    (unit < UNITS && register % 4 == 0).then_some((unit, register))
}

fn count_register(offset: u64) -> Option<usize> {
    (COUNT_BASE..STATUS_BASE)
        .contains(&offset)
        .then(|| usize::try_from((offset - COUNT_BASE) / 4).ok())
        .flatten()
        .filter(|unit| *unit < UNITS)
}

fn status_register(offset: u64) -> Option<usize> {
    (STATUS_BASE..CONTROL)
        .contains(&offset)
        .then(|| usize::try_from((offset - STATUS_BASE) / 4).ok())
        .flatten()
        .filter(|unit| *unit < UNITS)
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
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("PCNT value fits u32");
        let mut state = self.state.borrow_mut();
        if let Some((unit, register)) = unit_register(offset) {
            let unit_state = &mut state.units[unit];
            match register {
                0 => unit_state.conf0 = value,
                4 => unit_state.conf1 = value,
                8 => unit_state.conf2 = value,
                _ => unreachable!(),
            }
        } else if let Some(unit) = count_register(offset) {
            return Err(DeviceError::new(format!(
                "ESP32-S3 PCNT U{unit} count is read-only"
            )));
        } else if let Some(unit) = status_register(offset) {
            return Err(DeviceError::new(format!(
                "ESP32-S3 PCNT U{unit} status is read-only"
            )));
        } else {
            match offset {
                INT_ENA => state.int_ena = value & 0xf,
                INT_CLR => state.int_raw &= !value,
                CONTROL => {
                    state.control = value & 0x0001_00ff;
                    for unit in 0..UNITS {
                        if value & (1 << (2 * unit)) != 0 {
                            state.units[unit].count = 0;
                            state.units[unit].status = 0;
                        }
                    }
                    state.sync_counts();
                }
                DATE => return Err(DeviceError::new("PCNT DATE is read-only")),
                INT_RAW | INT_ST => {
                    return Err(DeviceError::new("PCNT interrupt status is read-only"));
                }
                _ => {
                    return Err(DeviceError::new(format!(
                        "unmodeled ESP32-S3 PCNT write at offset {offset:#x}"
                    )));
                }
            }
        }
        drop(state);
        self.publish_counts(at)
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.borrow_mut() = PcntState::reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
