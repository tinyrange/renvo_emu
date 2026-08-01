use renvo_core::SimTime;
use renvo_signals::Logic;
use serde::Serialize;

/// Observable state of a digital LED.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct LedSnapshot {
    /// Whether the LED is currently illuminated.
    pub on: bool,
    /// Accumulated illuminated time in ticks.
    pub on_ticks: u64,
    /// Accumulated observation time in ticks.
    pub observed_ticks: u64,
    /// Average brightness in thousandths over the observation window.
    pub brightness_milli: u16,
}

/// Digital LED observer with deterministic PWM brightness accumulation.
#[derive(Clone, Debug)]
pub struct DigitalLed {
    active_low: bool,
    on: bool,
    last_at: SimTime,
    on_ticks: u64,
}

impl DigitalLed {
    /// Creates an LED at time zero in its electrically inactive state.
    pub const fn new(active_low: bool) -> Self {
        Self {
            active_low,
            on: false,
            last_at: SimTime::ZERO,
            on_ticks: 0,
        }
    }

    /// Observes a new electrical input level.
    pub fn observe(&mut self, level: Logic, at: SimTime) {
        self.accrue(at);
        self.on = match level {
            Logic::Zero => self.active_low,
            Logic::One => !self.active_low,
            Logic::Z | Logic::X => false,
        };
    }

    /// Returns the current state after accounting for time through `at`.
    pub fn snapshot(&mut self, at: SimTime) -> LedSnapshot {
        self.accrue(at);
        let observed_ticks = self.last_at.ticks();
        let brightness_milli = if observed_ticks == 0 {
            u16::from(self.on) * 1000
        } else {
            u16::try_from(self.on_ticks.saturating_mul(1000) / observed_ticks).unwrap_or(1000)
        };
        LedSnapshot {
            on: self.on,
            on_ticks: self.on_ticks,
            observed_ticks,
            brightness_milli,
        }
    }

    fn accrue(&mut self, at: SimTime) {
        let elapsed = at.ticks().saturating_sub(self.last_at.ticks());
        if self.on {
            self.on_ticks = self.on_ticks.saturating_add(elapsed);
        }
        if at > self.last_at {
            self.last_at = at;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn led_accumulates_pwm_brightness() {
        let mut led = DigitalLed::new(false);
        led.observe(Logic::One, SimTime::ZERO);
        led.observe(Logic::Zero, SimTime::from_ticks(25));
        assert_eq!(led.snapshot(SimTime::from_ticks(100)).brightness_milli, 250);
    }
}
