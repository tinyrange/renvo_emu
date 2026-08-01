use renvo_core::SimTime;
use renvo_signals::Logic;
use serde::Serialize;
use thiserror::Error;

/// One decoded RGB pixel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Rgb {
    /// Red channel.
    pub red: u8,
    /// Green channel.
    pub green: u8,
    /// Blue channel.
    pub blue: u8,
}

/// WS2812 waveform decoding error.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum Ws2812Error {
    /// A high pulse was outside the accepted functional timing window.
    #[error("WS2812 high pulse of {ticks} ticks is outside the accepted timing window")]
    PulseWidth {
        /// Observed high time.
        ticks: u64,
    },
    /// A reset latched a partial frame.
    #[error("WS2812 frame contains {bits} bits; expected a multiple of 24")]
    PartialFrame {
        /// Bits received before reset.
        bits: usize,
    },
}

/// Timing-tolerant WS2812/NeoPixel waveform decoder.
#[derive(Clone, Debug)]
pub struct Ws2812 {
    count: usize,
    last_level: Logic,
    last_edge: SimTime,
    bits: Vec<bool>,
    pixels: Vec<Rgb>,
    frames: u64,
}

impl Ws2812 {
    /// Functional reset-low threshold in nanosecond ticks.
    pub const RESET_TICKS: u64 = 50_000;
    const MIN_HIGH_TICKS: u64 = 150;
    const ONE_THRESHOLD_TICKS: u64 = 550;
    const MAX_HIGH_TICKS: u64 = 1_050;

    /// Creates a GRB-ordered WS2812 chain.
    pub fn new(count: usize) -> Self {
        Self {
            count,
            last_level: Logic::Zero,
            last_edge: SimTime::ZERO,
            bits: Vec::new(),
            pixels: vec![
                Rgb {
                    red: 0,
                    green: 0,
                    blue: 0,
                };
                count
            ],
            frames: 0,
        }
    }

    /// Observes one data-pin transition.
    pub fn observe(&mut self, level: Logic, at: SimTime) -> Result<(), Ws2812Error> {
        if level == self.last_level {
            return Ok(());
        }
        let elapsed = at.ticks().saturating_sub(self.last_edge.ticks());
        if self.last_level == Logic::One && level == Logic::Zero {
            if !(Self::MIN_HIGH_TICKS..=Self::MAX_HIGH_TICKS).contains(&elapsed) {
                return Err(Ws2812Error::PulseWidth { ticks: elapsed });
            }
            self.bits.push(elapsed >= Self::ONE_THRESHOLD_TICKS);
        } else if self.last_level == Logic::Zero
            && level == Logic::One
            && elapsed >= Self::RESET_TICKS
        {
            self.latch()?;
        }
        self.last_level = level;
        self.last_edge = at;
        Ok(())
    }

    /// Advances through a low reset interval and latches the pending frame.
    pub fn finish(&mut self, at: SimTime) -> Result<(), Ws2812Error> {
        if self.last_level == Logic::Zero
            && at.ticks().saturating_sub(self.last_edge.ticks()) >= Self::RESET_TICKS
        {
            self.latch()?;
        }
        Ok(())
    }

    /// Most recently latched RGB pixels.
    pub fn pixels(&self) -> &[Rgb] {
        &self.pixels
    }

    /// Number of complete frames latched.
    pub const fn frames(&self) -> u64 {
        self.frames
    }

    fn latch(&mut self) -> Result<(), Ws2812Error> {
        if self.bits.is_empty() {
            return Ok(());
        }
        if !self.bits.len().is_multiple_of(24) {
            return Err(Ws2812Error::PartialFrame {
                bits: self.bits.len(),
            });
        }
        for (index, chunk) in self.bits.chunks_exact(24).take(self.count).enumerate() {
            let green = byte(&chunk[0..8]);
            let red = byte(&chunk[8..16]);
            let blue = byte(&chunk[16..24]);
            self.pixels[index] = Rgb { red, green, blue };
        }
        self.bits.clear();
        self.frames = self.frames.saturating_add(1);
        Ok(())
    }
}

fn byte(bits: &[bool]) -> u8 {
    bits.iter()
        .fold(0_u8, |value, bit| (value << 1) | u8::from(*bit))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_grb_frame_with_timing_tolerance() {
        let mut ws = Ws2812::new(1);
        let mut now = 60_000;
        for byte in [0x34_u8, 0x12, 0x56] {
            for bit in (0..8).rev() {
                ws.observe(Logic::One, SimTime::from_ticks(now)).unwrap();
                now += if byte & (1 << bit) == 0 { 350 } else { 700 };
                ws.observe(Logic::Zero, SimTime::from_ticks(now)).unwrap();
                now += if byte & (1 << bit) == 0 { 900 } else { 550 };
            }
        }
        ws.finish(SimTime::from_ticks(now + Ws2812::RESET_TICKS))
            .unwrap();
        assert_eq!(
            ws.pixels(),
            &[Rgb {
                red: 0x12,
                green: 0x34,
                blue: 0x56
            }]
        );
    }
}
