use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use remu_signals::{SignalId, SignalValue};
use std::sync::{Arc, Mutex};

const CTRLA_ENABLE: u32 = 1 << 1;
const CTRLA_PRESCALER_SHIFT: u32 = 8;
const CTRLA_SUPPORTED_MASK: u32 = 0x0000_7f03;
const INT_OVF: u8 = 1;
const INT_MC0: u8 = 1 << 4;

const DRVCTRL_OFFSET: u64 = 0x18;
const INTENCLR_OFFSET: u64 = 0x24;
const INTENSET_OFFSET: u64 = 0x28;
const INTFLAG_OFFSET: u64 = 0x2c;
const COUNT_OFFSET: u64 = 0x34;
const WAVE_OFFSET: u64 = 0x3c;
const PERIOD_OFFSET: u64 = 0x40;
const COMPARE_OFFSET: u64 = 0x44;

fn prescaler_divider(value: u8) -> u64 {
    match value & 7 {
        0 => 1,
        1 => 2,
        2 => 4,
        3 => 8,
        4 => 16,
        5 => 64,
        6 => 256,
        _ => 1024,
    }
}

struct TccState {
    ctrla: u32,
    drvctrl: u32,
    wave: u32,
    period: u32,
    count: u32,
    compare: [u32; 4],
    interrupt_enable: u8,
    interrupt_flags: u8,
    started: u64,
    outputs: [bool; 4],
    channels: usize,
    counter_mask: u32,
    hub: super::SignalHub,
    output_signals: [Option<SignalId>; 4],
    irq_signal: SignalId,
    count_signal: SignalId,
}

impl TccState {
    fn set_signal(&self, signal: SignalId, value: u64, width: u16, at: SimTime) {
        self.hub
            .set(
                signal,
                SignalValue::from_u64(value, width).expect("fixed TCC signal width is valid"),
                at,
            )
            .expect("TCC signal identity is fixed at construction");
    }

    fn reset(&mut self, at: SimTime) {
        self.ctrla = 0;
        self.drvctrl = 0;
        self.wave = 0;
        self.period = self.counter_mask;
        self.count = 0;
        self.compare = [0; 4];
        self.interrupt_enable = 0;
        self.interrupt_flags = 0;
        self.started = at.ticks();
        self.outputs = [false; 4];
        for signal in self.output_signals.into_iter().flatten() {
            self.set_signal(signal, 0, 1, at);
        }
        self.set_signal(self.irq_signal, 0, 1, at);
        self.set_signal(self.count_signal, 0, 32, at);
    }

    fn interrupt_pending(&self) -> bool {
        self.interrupt_flags & self.interrupt_enable != 0
    }

    fn refresh_outputs(&mut self, at: SimTime) {
        let pwm = self.wave & 3 == 2;
        for channel in 0..self.channels {
            let mut output =
                self.ctrla & CTRLA_ENABLE != 0 && pwm && self.count < self.compare[channel];
            if self.drvctrl & (1 << channel) != 0 {
                output = !output;
            }
            if self.outputs[channel] != output {
                self.outputs[channel] = output;
                if let Some(signal) = self.output_signals[channel] {
                    self.set_signal(signal, u64::from(output), 1, at);
                }
            }
        }
        self.set_signal(self.irq_signal, u64::from(self.interrupt_pending()), 1, at);
        self.set_signal(self.count_signal, u64::from(self.count), 32, at);
    }

    fn poll(&mut self, now: SimTime) -> bool {
        if self.ctrla & CTRLA_ENABLE != 0 {
            let elapsed = now.ticks().saturating_sub(self.started);
            let ticks = elapsed / prescaler_divider((self.ctrla >> CTRLA_PRESCALER_SHIFT) as u8);
            if ticks != 0 {
                let top = u64::from(self.period & self.counter_mask).saturating_add(1);
                let total = u64::from(self.count).saturating_add(ticks);
                if total >= top {
                    self.interrupt_flags |= INT_OVF;
                }
                let previous = self.count;
                self.count = (total % top) as u32;
                self.started = now.ticks();
                for channel in 0..self.channels {
                    if self.compare[channel] != 0
                        && ((previous < self.compare[channel]
                            && self.count >= self.compare[channel])
                            || total >= u64::from(self.compare[channel]))
                    {
                        self.interrupt_flags |= INT_MC0 << channel;
                    }
                }
            }
        }
        self.refresh_outputs(now);
        self.interrupt_pending()
    }
}

/// Host-facing functional TCC state and waveform outputs.
#[derive(Clone)]
pub struct Samd21TccHandle(Arc<Mutex<TccState>>);

impl Samd21TccHandle {
    /// Advances the counter and returns an enabled overflow/compare request.
    pub fn poll(&self, now: SimTime) -> bool {
        self.0.lock().expect("TCC lock poisoned").poll(now)
    }

    /// Returns the current waveform output level for a channel.
    pub fn output(&self, channel: usize) -> bool {
        self.0
            .lock()
            .expect("TCC lock poisoned")
            .outputs
            .get(channel)
            .copied()
            .unwrap_or(false)
    }

    /// Returns the latched interrupt flags.
    pub fn interrupt_flags(&self) -> u8 {
        self.0.lock().expect("TCC lock poisoned").interrupt_flags
    }
}

/// Functional SAM D21 TCC0/TCC1/TCC2 PWM and compare slice.
pub struct Samd21Tcc {
    name: String,
    state: Arc<Mutex<TccState>>,
    registers: [u32; 32],
}

impl Samd21Tcc {
    /// Constructs a TCC instance. TCC0/TCC1 use 24-bit registers; TCC2 uses 16-bit registers.
    pub fn new(
        name: impl Into<String>,
        path: &str,
        channels: usize,
        large_register_map: bool,
        hub: super::SignalHub,
    ) -> Result<(Self, Samd21TccHandle), remu_signals::SignalError> {
        let mut output_signals = [None; 4];
        for (channel, signal) in output_signals.iter_mut().enumerate().take(channels) {
            *signal = Some(hub.declare(
                format!("{path}.wo{channel}"),
                SignalValue::from_u64(0, 1)?,
                Some(format!("TCC waveform output {channel}")),
            )?);
        }
        let irq_signal = hub.declare(
            format!("{path}.irq"),
            SignalValue::from_u64(0, 1)?,
            Some("TCC overflow/compare interrupt request".to_owned()),
        )?;
        let count_signal = hub.declare(
            format!("{path}.count"),
            SignalValue::from_u64(0, 32)?,
            Some("TCC counter value".to_owned()),
        )?;
        let counter_mask = if large_register_map {
            0x00ff_ffff
        } else {
            0x0000_ffff
        };
        let state = Arc::new(Mutex::new(TccState {
            ctrla: 0,
            drvctrl: 0,
            wave: 0,
            period: counter_mask,
            count: 0,
            compare: [0; 4],
            interrupt_enable: 0,
            interrupt_flags: 0,
            started: 0,
            outputs: [false; 4],
            channels,
            counter_mask,
            hub,
            output_signals,
            irq_signal,
            count_signal,
        }));
        state
            .lock()
            .expect("new TCC lock poisoned")
            .reset(SimTime::ZERO);
        Ok((
            Self {
                name: name.into(),
                state: state.clone(),
                registers: [0; 32],
            },
            Samd21TccHandle(state),
        ))
    }
}

impl Device for Samd21Tcc {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, _width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let state = self.state.lock().expect("TCC lock poisoned");
        let value = match offset {
            0x00 => state.ctrla,
            DRVCTRL_OFFSET => state.drvctrl,
            INTENCLR_OFFSET | INTENSET_OFFSET => u32::from(state.interrupt_enable),
            INTFLAG_OFFSET => u32::from(state.interrupt_flags),
            0x30 => 0,
            COUNT_OFFSET => state.count,
            WAVE_OFFSET => state.wave,
            PERIOD_OFFSET => state.period,
            offset
                if (COMPARE_OFFSET..COMPARE_OFFSET + 0x10)
                    .step_by(4)
                    .any(|candidate| candidate == offset) =>
            {
                let channel = usize::try_from((offset - COMPARE_OFFSET) / 4)
                    .expect("TCC channel offset fits");
                state.compare.get(channel).copied().unwrap_or(0)
            }
            _ => self
                .registers
                .get(usize::try_from(offset / 4).unwrap_or(0).min(31))
                .copied()
                .unwrap_or(0),
        };
        Ok(u64::from(value))
    }

    fn write(
        &mut self,
        offset: u64,
        _width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        let value = value as u32;
        let mut state = self.state.lock().expect("TCC lock poisoned");
        match offset {
            0x00 => {
                if value & 1 != 0 {
                    state.reset(at);
                } else {
                    let was_enabled = state.ctrla & CTRLA_ENABLE != 0;
                    state.ctrla = value & CTRLA_SUPPORTED_MASK;
                    if !was_enabled && state.ctrla & CTRLA_ENABLE != 0 {
                        state.started = at.ticks();
                    }
                }
            }
            DRVCTRL_OFFSET => state.drvctrl = value,
            INTENSET_OFFSET => state.interrupt_enable |= value as u8,
            INTENCLR_OFFSET => state.interrupt_enable &= !(value as u8),
            INTFLAG_OFFSET => state.interrupt_flags &= !(value as u8),
            COUNT_OFFSET => {
                state.count = value & state.counter_mask;
                state.started = at.ticks();
            }
            WAVE_OFFSET => state.wave = value & 3,
            PERIOD_OFFSET => state.period = value & state.counter_mask,
            offset
                if offset >= COMPARE_OFFSET
                    && offset < COMPARE_OFFSET + 4 * u64::try_from(state.channels).unwrap()
                    && (offset - COMPARE_OFFSET) % 4 == 0 =>
            {
                let channel = usize::try_from((offset - COMPARE_OFFSET) / 4)
                    .expect("TCC channel offset fits");
                state.compare[channel] = value & state.counter_mask;
            }
            _ => {
                let index = usize::try_from(offset / 4).unwrap_or(0).min(31);
                self.registers[index] = value;
            }
        }
        state.refresh_outputs(at);
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state
            .lock()
            .expect("TCC lock poisoned")
            .reset(SimTime::ZERO);
        self.registers = [0; 32];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tcc_pwm_wrap_and_compare_are_deterministic() {
        let hub = super::super::SignalHub::new();
        let (mut tcc, handle) =
            Samd21Tcc::new("tcc0", "board.atsamd21e18.tcc0", 4, true, hub).unwrap();
        tcc.write(PERIOD_OFFSET, AccessWidth::Word, 7, SimTime::ZERO)
            .unwrap();
        tcc.write(COMPARE_OFFSET, AccessWidth::Word, 4, SimTime::ZERO)
            .unwrap();
        tcc.write(WAVE_OFFSET, AccessWidth::Word, 2, SimTime::ZERO)
            .unwrap();
        tcc.write(
            INTENSET_OFFSET,
            AccessWidth::Byte,
            u64::from(INT_OVF | INT_MC0),
            SimTime::ZERO,
        )
        .unwrap();
        tcc.write(0x00, AccessWidth::Word, CTRLA_ENABLE.into(), SimTime::ZERO)
            .unwrap();
        assert!(handle.output(0));
        assert!(handle.poll(SimTime::from_ticks(4)));
        assert!(!handle.output(0));
        tcc.write(
            INTFLAG_OFFSET,
            AccessWidth::Byte,
            u64::from(INT_MC0),
            SimTime::from_ticks(4),
        )
        .unwrap();
        assert!(handle.poll(SimTime::from_ticks(8)));
        assert_ne!(handle.interrupt_flags() & INT_OVF, 0);
    }

    #[test]
    fn tcc2_uses_the_native_16_bit_register_stride() {
        let hub = super::super::SignalHub::new();
        let (mut tcc, handle) =
            Samd21Tcc::new("tcc2", "board.atsamd21e18.tcc2", 2, false, hub).unwrap();
        tcc.write(PERIOD_OFFSET, AccessWidth::Word, 3, SimTime::ZERO)
            .unwrap();
        tcc.write(COMPARE_OFFSET + 4, AccessWidth::Word, 2, SimTime::ZERO)
            .unwrap();
        tcc.write(WAVE_OFFSET, AccessWidth::Word, 2, SimTime::ZERO)
            .unwrap();
        tcc.write(
            INTENSET_OFFSET,
            AccessWidth::Byte,
            u64::from(INT_OVF | (INT_MC0 << 1)),
            SimTime::ZERO,
        )
        .unwrap();
        tcc.write(0x00, AccessWidth::Word, CTRLA_ENABLE.into(), SimTime::ZERO)
            .unwrap();
        assert!(handle.output(1));
        assert!(handle.poll(SimTime::from_ticks(2)));
        assert!(!handle.output(1));
        tcc.write(
            INTFLAG_OFFSET,
            AccessWidth::Byte,
            u64::from(INT_MC0 << 1),
            SimTime::from_ticks(2),
        )
        .unwrap();
        assert!(handle.poll(SimTime::from_ticks(4)));
    }
}
