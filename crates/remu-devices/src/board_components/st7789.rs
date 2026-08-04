use serde::Serialize;
use thiserror::Error;

/// ST7789 panel geometry and controller wiring parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct St7789Config {
    /// Visible horizontal pixels.
    pub width: u16,
    /// Visible vertical pixels.
    pub height: u16,
    /// Controller column offset used by the panel wiring.
    pub x_offset: u16,
    /// Controller row offset used by the panel wiring.
    pub y_offset: u16,
    /// Whether the panel is configured for inverted colors.
    pub inverted: bool,
}

impl St7789Config {
    /// M5StickS3's 135x240 ST7789 panel configuration.
    pub const fn m5stick_s3() -> Self {
        Self {
            width: 135,
            height: 240,
            x_offset: 52,
            y_offset: 40,
            inverted: true,
        }
    }
}

/// ST7789 command-level model error.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum St7789Error {
    /// A command did not receive its required parameter bytes.
    #[error("ST7789 command {command:#04x} expected {expected} bytes, received {actual}")]
    ParameterLength {
        /// Command byte.
        command: u8,
        /// Required parameter count.
        expected: usize,
        /// Supplied parameter count.
        actual: usize,
    },
    /// A CASET or RASET window was outside the visible panel.
    #[error("ST7789 {axis} range {start}..={end} is outside the panel")]
    Coordinate {
        /// Coordinate axis.
        axis: &'static str,
        /// Inclusive start.
        start: u16,
        /// Inclusive end.
        end: u16,
    },
    /// Pixel payloads must contain complete RGB565 words.
    #[error("ST7789 RAMWR payload has an odd byte count {actual}")]
    OddPixelBytes {
        /// Supplied payload size.
        actual: usize,
    },
}

/// Stable panel state suitable for board-level result artifacts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct St7789Snapshot {
    /// Panel configuration.
    pub config: St7789Config,
    /// Whether the display has received DISPON.
    pub display_on: bool,
    /// Current MADCTL-independent inversion state.
    pub inverted: bool,
    /// Number of accepted commands.
    pub commands: u64,
    /// Deterministic FNV-1a hash of the RGB565 framebuffer.
    pub frame_hash: u64,
}

/// Functional ST7789 command sink with a deterministic RGB565 framebuffer.
#[derive(Clone, Debug)]
pub struct St7789 {
    config: St7789Config,
    framebuffer: Vec<u16>,
    column_start: u16,
    column_end: u16,
    row_start: u16,
    row_end: u16,
    cursor_x: u16,
    cursor_y: u16,
    display_on: bool,
    inverted: bool,
    commands: u64,
}

impl St7789 {
    /// Creates a reset panel with a full-screen write window.
    pub fn new(config: St7789Config) -> Self {
        let width = usize::from(config.width);
        let height = usize::from(config.height);
        Self {
            config,
            framebuffer: vec![0; width.saturating_mul(height)],
            column_start: 0,
            column_end: config.width.saturating_sub(1),
            row_start: 0,
            row_end: config.height.saturating_sub(1),
            cursor_x: 0,
            cursor_y: 0,
            display_on: false,
            inverted: config.inverted,
            commands: 0,
        }
    }

    /// Applies one command and its data-phase bytes.
    pub fn command(&mut self, command: u8, data: &[u8]) -> Result<(), St7789Error> {
        self.commands = self.commands.saturating_add(1);
        match command {
            0x11 | 0x20 | 0x21 | 0x29 => {
                if !data.is_empty() {
                    return Err(St7789Error::ParameterLength {
                        command,
                        expected: 0,
                        actual: data.len(),
                    });
                }
                match command {
                    0x20 => self.inverted = false,
                    0x21 => self.inverted = true,
                    0x29 => self.display_on = true,
                    _ => {}
                }
            }
            0x2a => {
                let (start, end) = range(data, command)?;
                let (start, end) = normalize_range(
                    start,
                    end,
                    self.config.x_offset,
                    self.config.width,
                    "column",
                )?;
                self.column_start = start;
                self.column_end = end;
                self.cursor_x = start;
                self.cursor_y = self.row_start;
            }
            0x2b => {
                let (start, end) = range(data, command)?;
                let (start, end) =
                    normalize_range(start, end, self.config.y_offset, self.config.height, "row")?;
                self.row_start = start;
                self.row_end = end;
                self.cursor_x = self.column_start;
                self.cursor_y = start;
            }
            0x2c => self.write_pixels(data)?,
            _ => {}
        }
        Ok(())
    }

    /// Returns a pixel in panel-local coordinates as an RGB565 word.
    pub fn pixel(&self, x: u16, y: u16) -> Option<u16> {
        if x >= self.config.width || y >= self.config.height {
            return None;
        }
        self.framebuffer
            .get(usize::from(y) * usize::from(self.config.width) + usize::from(x))
            .copied()
    }

    /// Returns the current deterministic panel snapshot.
    pub fn snapshot(&self) -> St7789Snapshot {
        St7789Snapshot {
            config: self.config,
            display_on: self.display_on,
            inverted: self.inverted,
            commands: self.commands,
            frame_hash: self.frame_hash(),
        }
    }

    /// Returns a deterministic FNV-1a hash of framebuffer words in row order.
    pub fn frame_hash(&self) -> u64 {
        self.framebuffer
            .iter()
            .fold(0xcbf2_9ce4_8422_2325, |hash, pixel| {
                let mut hash = hash ^ u64::from(*pixel & 0xff);
                hash = hash.wrapping_mul(0x1000_0000_01b3);
                hash ^= u64::from(*pixel >> 8);
                hash.wrapping_mul(0x1000_0000_01b3)
            })
    }

    fn write_pixels(&mut self, data: &[u8]) -> Result<(), St7789Error> {
        if !data.len().is_multiple_of(2) {
            return Err(St7789Error::OddPixelBytes { actual: data.len() });
        }
        for bytes in data.chunks_exact(2) {
            let pixel = u16::from_be_bytes([bytes[0], bytes[1]]);
            let index = usize::from(self.cursor_y) * usize::from(self.config.width)
                + usize::from(self.cursor_x);
            if let Some(destination) = self.framebuffer.get_mut(index) {
                *destination = pixel;
            }
            if self.cursor_x == self.column_end {
                self.cursor_x = self.column_start;
                self.cursor_y = if self.cursor_y == self.row_end {
                    self.row_start
                } else {
                    self.cursor_y + 1
                };
            } else {
                self.cursor_x += 1;
            }
        }
        Ok(())
    }
}

fn range(data: &[u8], command: u8) -> Result<(u16, u16), St7789Error> {
    if data.len() != 4 {
        return Err(St7789Error::ParameterLength {
            command,
            expected: 4,
            actual: data.len(),
        });
    }
    Ok((
        u16::from_be_bytes([data[0], data[1]]),
        u16::from_be_bytes([data[2], data[3]]),
    ))
}

fn normalize_range(
    start: u16,
    end: u16,
    offset: u16,
    length: u16,
    axis: &'static str,
) -> Result<(u16, u16), St7789Error> {
    let offset_end = offset.saturating_add(length.saturating_sub(1));
    let (start, end) = if end >= offset_end {
        (start.saturating_sub(offset), end.saturating_sub(offset))
    } else {
        (start, end)
    };
    if start > end || end >= length {
        return Err(St7789Error::Coordinate { axis, start, end });
    }
    Ok((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn m5sticks3_window_offsets_and_frame_hash_are_deterministic() {
        let config = St7789Config::m5stick_s3();
        let mut panel = St7789::new(config);
        panel.command(0x21, &[]).unwrap();
        panel.command(0x29, &[]).unwrap();
        panel.command(0x2a, &[0, 52, 0, 186]).unwrap();
        panel.command(0x2b, &[0, 40, 1, 23]).unwrap();
        panel.command(0x2c, &[0xf8, 0x00, 0x07, 0xe0]).unwrap();

        assert_eq!(panel.pixel(0, 0), Some(0xf800));
        assert_eq!(panel.pixel(1, 0), Some(0x07e0));
        assert_eq!(panel.snapshot().config, config);
        assert!(panel.snapshot().display_on);
        assert!(panel.snapshot().inverted);
        assert_ne!(panel.frame_hash(), St7789::new(config).frame_hash());
    }

    #[test]
    fn rejects_bad_windows_and_odd_pixel_payloads() {
        let mut panel = St7789::new(St7789Config::m5stick_s3());
        assert!(matches!(
            panel.command(0x2a, &[0, 0, 0]),
            Err(St7789Error::ParameterLength { command: 0x2a, .. })
        ));
        assert!(matches!(
            panel.command(0x2a, &[0, 0, 1, 0]),
            Err(St7789Error::Coordinate { axis: "column", .. })
        ));
        assert!(matches!(
            panel.command(0x2c, &[0]),
            Err(St7789Error::OddPixelBytes { actual: 1 })
        ));
    }
}
