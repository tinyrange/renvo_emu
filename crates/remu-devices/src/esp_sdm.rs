//! ESP32-S3 eight-channel sigma-delta modulator.

use super::*;

const CHANNELS: usize = 8;
const CHANNEL_MASK: u32 = 0x0000_ffff;
const CHANNEL_RESET: u32 = 0x0000_ff00;
const CG_MASK: u32 = 1 << 31;
const MISC_MASK: u32 = 3 << 30;
const FUNCTION_CLK_EN: u32 = 1 << 30;
const DATE_MASK: u32 = 0x0fff_ffff;
const DATE_RESET: u32 = 0x0180_2260;

#[derive(Clone)]
struct SdmState {
    channels: [u32; CHANNELS],
    cg: u32,
    misc: u32,
    date: u32,
    accumulators: [u16; CHANNELS],
    outputs: [bool; CHANNELS],
    last_tick: u64,
}

impl SdmState {
    fn reset() -> Self {
        Self {
            channels: [CHANNEL_RESET; CHANNELS],
            cg: 0,
            misc: 0,
            date: DATE_RESET,
            accumulators: [0; CHANNELS],
            outputs: [false; CHANNELS],
            last_tick: 0,
        }
    }
}

/// Scheduler and waveform-inspection handle for the sigma-delta block.
#[derive(Clone)]
pub struct Esp32S3SdmHandle {
    state: Rc<RefCell<SdmState>>,
    hub: SignalHub,
    signals: Vec<SignalId>,
}

impl Esp32S3SdmHandle {
    /// Advances all enabled modulators to `now` in deterministic APB ticks.
    pub fn poll(&self, now: SimTime) -> Result<bool, SignalError> {
        let mut state = self.state.borrow_mut();
        let elapsed = now.ticks().saturating_sub(state.last_tick);
        state.last_tick = now.ticks();
        if state.misc & FUNCTION_CLK_EN == 0 || elapsed == 0 {
            return Ok(false);
        }
        let channels = state.channels;
        let mut changed = false;
        for channel in 0..CHANNELS {
            let divider = u64::from((channels[channel] >> 8) & 0xff) + 1;
            let cycles = elapsed / divider;
            if cycles == 0 {
                continue;
            }
            let density = u16::from((channels[channel] & 0xff) as u8);
            let initial = u64::from(state.accumulators[channel]);
            let total = initial.saturating_add(cycles.saturating_mul(u64::from(density)));
            state.accumulators[channel] = (total & 0xff) as u16;
            let before_last =
                initial.saturating_add(cycles.saturating_sub(1).saturating_mul(u64::from(density)));
            let level = (before_last & 0xff) + u64::from(density) >= 256;
            if state.outputs[channel] != level {
                state.outputs[channel] = level;
                self.hub.set(
                    self.signals[channel],
                    SignalValue::from_u64(u64::from(level), 1)?,
                    now,
                )?;
                changed = true;
            }
        }
        Ok(changed)
    }

    /// Returns the most recently emitted level for one channel.
    pub fn channel_level(&self, channel: usize) -> Option<bool> {
        self.state.borrow().outputs.get(channel).copied()
    }
}

/// Functional ESP32-S3 GPIO_SD sigma-delta register block.
pub struct Esp32S3Sdm {
    name: String,
    state: Rc<RefCell<SdmState>>,
    handle: Esp32S3SdmHandle,
}

impl Esp32S3Sdm {
    /// Creates the eight native output channels and their scheduler handle.
    pub fn new(
        name: impl Into<String>,
        signal_path: &str,
        hub: SignalHub,
    ) -> Result<(Self, Esp32S3SdmHandle), SignalError> {
        let mut signals = Vec::with_capacity(CHANNELS);
        for channel in 0..CHANNELS {
            signals.push(hub.declare(
                format!("{signal_path}.ch{channel}"),
                SignalValue::from_u64(0, 1)?,
                Some("ESP32-S3 sigma-delta output".to_owned()),
            )?);
        }
        let state = Rc::new(RefCell::new(SdmState::reset()));
        let handle = Esp32S3SdmHandle {
            state: state.clone(),
            hub,
            signals,
        };
        Ok((
            Self {
                name: name.into(),
                state,
                handle: handle.clone(),
            },
            handle,
        ))
    }

    fn check(offset: u64, width: AccessWidth) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || !offset.is_multiple_of(4) {
            return Err(DeviceError::new(
                "ESP32-S3 GPIO_SD requires aligned word access",
            ));
        }
        if !matches!(offset, 0x00..=0x28) {
            return Err(DeviceError::new(format!(
                "reserved ESP32-S3 GPIO_SD offset {offset:#x}"
            )));
        }
        Ok(())
    }

    fn refresh(&self, at: SimTime) -> Result<(), DeviceError> {
        self.handle
            .poll(at)
            .map(|_| ())
            .map_err(|error| DeviceError::new(error.to_string()))
    }
}

impl Device for Esp32S3Sdm {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        Self::check(offset, width)?;
        self.refresh(at)?;
        let state = self.state.borrow();
        let value = match offset {
            0x00..=0x1c => state.channels[offset as usize / 4],
            0x20 => state.cg,
            0x24 => state.misc,
            0x28 => state.date,
            _ => unreachable!(),
        };
        Ok(u64::from(value))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        Self::check(offset, width)?;
        self.refresh(at)?;
        let value = u32::try_from(value)
            .map_err(|_| DeviceError::new("ESP32-S3 GPIO_SD word exceeds 32 bits"))?;
        let mut state = self.state.borrow_mut();
        match offset {
            0x00..=0x1c => state.channels[offset as usize / 4] = value & CHANNEL_MASK,
            0x20 => state.cg = value & CG_MASK,
            0x24 => state.misc = value & MISC_MASK,
            0x28 => state.date = value & DATE_MASK,
            _ => unreachable!(),
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.borrow_mut() = SdmState::reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vendor_register_contract_and_reserved_accesses_are_exact() {
        let hub = SignalHub::new();
        let (mut sdm, _) = Esp32S3Sdm::new("sdm", "test.sdm", hub).unwrap();
        for channel in 0..8_u64 {
            assert_eq!(
                sdm.read(channel * 4, AccessWidth::Word, SimTime::ZERO),
                Ok(CHANNEL_RESET.into())
            );
        }
        assert_eq!(
            sdm.read(0x28, AccessWidth::Word, SimTime::ZERO),
            Ok(DATE_RESET.into())
        );
        sdm.write(0, AccessWidth::Word, u32::MAX.into(), SimTime::ZERO)
            .unwrap();
        assert_eq!(
            sdm.read(0, AccessWidth::Word, SimTime::ZERO),
            Ok(CHANNEL_MASK.into())
        );
        assert!(sdm.read(0x2c, AccessWidth::Word, SimTime::ZERO).is_err());
    }

    #[test]
    fn clocked_density_produces_a_deterministic_waveform() {
        let hub = SignalHub::new();
        let (mut sdm, handle) = Esp32S3Sdm::new("sdm", "test.sdm", hub).unwrap();
        sdm.write(0, AccessWidth::Word, 0x0080, SimTime::ZERO)
            .unwrap();
        sdm.write(
            0x24,
            AccessWidth::Word,
            FUNCTION_CLK_EN.into(),
            SimTime::ZERO,
        )
        .unwrap();
        handle.poll(SimTime::from_ticks(1)).unwrap();
        assert!(!handle.channel_level(0).unwrap());
        handle.poll(SimTime::from_ticks(2)).unwrap();
        assert!(handle.channel_level(0).unwrap());
        handle.poll(SimTime::from_ticks(3)).unwrap();
        assert!(!handle.channel_level(0).unwrap());
    }
}
