use remu_core::SimTime;
use remu_signals::Logic;
use serde::Serialize;

/// One deterministic electrical transition produced by a push button.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ButtonTransition {
    /// Transition time.
    pub at: SimTime,
    /// Electrical level driven by the button contact.
    pub level: Logic,
}

/// Momentary push button with optional deterministic contact bounce.
#[derive(Clone, Debug)]
pub struct PushButton {
    active_low: bool,
    bounce_ticks: u64,
    pressed: bool,
}

impl PushButton {
    /// Creates a button. `bounce_ticks` is the total bounce window per edge.
    pub const fn new(active_low: bool, bounce_ticks: u64) -> Self {
        Self {
            active_low,
            bounce_ticks,
            pressed: false,
        }
    }

    /// Current logical pressed state.
    pub const fn pressed(&self) -> bool {
        self.pressed
    }

    /// Configured deterministic bounce window per edge.
    pub const fn bounce_ticks(&self) -> u64 {
        self.bounce_ticks
    }

    /// Current electrical output level.
    pub const fn level(&self) -> Logic {
        match (self.active_low, self.pressed) {
            (true, true) | (false, false) => Logic::Zero,
            (true, false) | (false, true) => Logic::One,
        }
    }

    /// Changes the button state and returns its deterministic contact waveform.
    pub fn set_pressed(&mut self, pressed: bool, at: SimTime) -> Vec<ButtonTransition> {
        if self.pressed == pressed {
            return Vec::new();
        }
        let old_level = self.level();
        self.pressed = pressed;
        let final_level = self.level();
        if self.bounce_ticks == 0 {
            return vec![ButtonTransition {
                at,
                level: final_level,
            }];
        }
        let offsets = [
            0,
            self.bounce_ticks / 4,
            self.bounce_ticks / 2,
            self.bounce_ticks.saturating_mul(3) / 4,
            self.bounce_ticks,
        ];
        offsets
            .into_iter()
            .zip([final_level, old_level, final_level, old_level, final_level])
            .map(|(offset, level)| ButtonTransition {
                at: SimTime::from_ticks(at.ticks().saturating_add(offset)),
                level,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_low_button_has_repeatable_bounce() {
        let mut button = PushButton::new(true, 40);
        assert_eq!(button.level(), Logic::One);
        let transitions = button.set_pressed(true, SimTime::from_ticks(100));
        assert_eq!(transitions.len(), 5);
        assert_eq!(transitions[0].level, Logic::Zero);
        assert_eq!(transitions[1].level, Logic::One);
        assert_eq!(transitions[4].at, SimTime::from_ticks(140));
        assert!(button.pressed());
    }

    #[test]
    fn short_bounce_stays_inside_configured_window() {
        let mut button = PushButton::new(true, 1);
        let transitions = button.set_pressed(true, SimTime::from_ticks(100));
        assert_eq!(transitions.len(), 5);
        assert!(
            transitions
                .iter()
                .all(|transition| transition.at <= SimTime::from_ticks(101))
        );
    }
}
