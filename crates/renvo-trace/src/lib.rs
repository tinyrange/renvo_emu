//! Structured digital tracing and standards-compatible VCD output.

use renvo_core::SimTime;
use renvo_signals::{SignalChange, SignalDescriptor, SignalId, SignalRegistry, SignalValue};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{self, Write};
use thiserror::Error;

/// VCD timescale declaration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Timescale {
    /// One picosecond per simulation tick.
    Picosecond,
    /// One nanosecond per simulation tick.
    #[default]
    Nanosecond,
    /// One microsecond per simulation tick.
    Microsecond,
    /// One millisecond per simulation tick.
    Millisecond,
}

impl Timescale {
    const fn vcd(self) -> &'static str {
        match self {
            Self::Picosecond => "1ps",
            Self::Nanosecond => "1ns",
            Self::Microsecond => "1us",
            Self::Millisecond => "1ms",
        }
    }
}

/// Trace output failure.
#[derive(Debug, Error)]
pub enum TraceError {
    /// Underlying output failed.
    #[error(transparent)]
    Io(#[from] io::Error),
    /// A change was emitted before the signal was declared.
    #[error("change references undeclared signal {0}")]
    Undeclared(u32),
    /// Header was written more than once.
    #[error("VCD header is already written")]
    HeaderAlreadyWritten,
    /// A change was requested before the header.
    #[error("VCD header has not been written")]
    HeaderNotWritten,
    /// Changes were supplied out of timestamp order.
    #[error("trace time moved backwards from {previous} to {next}")]
    TimeRegression {
        /// Previous emitted timestamp.
        previous: SimTime,
        /// Requested timestamp.
        next: SimTime,
    },
}

/// Consumer of declared signals and changes.
pub trait TraceSink {
    /// Declares initial signals and values.
    fn begin(&mut self, registry: &SignalRegistry) -> Result<(), TraceError>;

    /// Emits a signal transition.
    fn change(&mut self, change: &SignalChange) -> Result<(), TraceError>;

    /// Completes buffered output.
    fn finish(&mut self) -> Result<(), TraceError>;
}

#[derive(Default)]
struct Scope<'a> {
    children: BTreeMap<&'a str, Scope<'a>>,
    signals: Vec<(&'a SignalDescriptor, String)>,
}

/// Streaming Value Change Dump writer.
pub struct VcdWriter<W>
where
    W: Write,
{
    output: W,
    timescale: Timescale,
    identifiers: BTreeMap<SignalId, String>,
    header_written: bool,
    current_time: SimTime,
}

impl<W> VcdWriter<W>
where
    W: Write,
{
    /// Creates a VCD writer.
    pub fn new(output: W, timescale: Timescale) -> Self {
        Self {
            output,
            timescale,
            identifiers: BTreeMap::new(),
            header_written: false,
            current_time: SimTime::ZERO,
        }
    }

    /// Returns the wrapped output.
    pub fn into_inner(self) -> W {
        self.output
    }

    fn write_scope(&mut self, name: &str, scope: &Scope<'_>) -> Result<(), TraceError> {
        writeln!(self.output, "$scope module {} $end", sanitize_name(name))?;
        for (descriptor, identifier) in &scope.signals {
            let leaf = descriptor
                .path
                .rsplit('.')
                .next()
                .expect("validated non-empty signal path");
            writeln!(
                self.output,
                "$var wire {} {} {} $end",
                descriptor.width,
                identifier,
                sanitize_name(leaf)
            )?;
        }
        for (child_name, child) in &scope.children {
            self.write_scope(child_name, child)?;
        }
        writeln!(self.output, "$upscope $end")?;
        Ok(())
    }

    fn write_value(&mut self, identifier: &str, value: &SignalValue) -> Result<(), TraceError> {
        if value.width() == 1 {
            writeln!(
                self.output,
                "{}{}",
                value.bit(0).expect("one-bit value contains bit zero").vcd(),
                identifier
            )?;
        } else {
            writeln!(self.output, "b{} {}", value.to_vcd_binary(), identifier)?;
        }
        Ok(())
    }
}

impl<W> TraceSink for VcdWriter<W>
where
    W: Write,
{
    fn begin(&mut self, registry: &SignalRegistry) -> Result<(), TraceError> {
        if self.header_written {
            return Err(TraceError::HeaderAlreadyWritten);
        }
        writeln!(self.output, "$date deterministic simulation $end")?;
        writeln!(
            self.output,
            "$version Renvo {} $end",
            env!("CARGO_PKG_VERSION")
        )?;
        writeln!(self.output, "$timescale {} $end", self.timescale.vcd())?;

        let mut root = Scope::default();
        for (index, descriptor) in registry.descriptors().enumerate() {
            let identifier = encode_identifier(index);
            self.identifiers.insert(descriptor.id, identifier.clone());
            let components: Vec<_> = descriptor.path.split('.').collect();
            let mut scope = &mut root;
            for component in &components[..components.len().saturating_sub(1)] {
                scope = scope.children.entry(component).or_default();
            }
            scope.signals.push((descriptor, identifier));
        }
        self.write_scope("renvo", &root)?;
        writeln!(self.output, "$enddefinitions $end")?;
        writeln!(self.output, "#0")?;
        writeln!(self.output, "$dumpvars")?;
        for descriptor in registry.descriptors() {
            let identifier = self
                .identifiers
                .get(&descriptor.id)
                .cloned()
                .expect("identifier assigned for every descriptor");
            let value = registry
                .value(descriptor.id)
                .expect("value exists for every descriptor");
            self.write_value(&identifier, value)?;
        }
        writeln!(self.output, "$end")?;
        self.header_written = true;
        Ok(())
    }

    fn change(&mut self, change: &SignalChange) -> Result<(), TraceError> {
        if !self.header_written {
            return Err(TraceError::HeaderNotWritten);
        }
        let identifier = self
            .identifiers
            .get(&change.signal)
            .cloned()
            .ok_or(TraceError::Undeclared(change.signal.get()))?;
        if change.at < self.current_time {
            return Err(TraceError::TimeRegression {
                previous: self.current_time,
                next: change.at,
            });
        }
        if change.at != self.current_time {
            writeln!(self.output, "#{}", change.at.ticks())?;
            self.current_time = change.at;
        }
        self.write_value(&identifier, &change.value)
    }

    fn finish(&mut self) -> Result<(), TraceError> {
        self.output.flush()?;
        Ok(())
    }
}

fn encode_identifier(mut index: usize) -> String {
    // VCD identifiers can use the 94 printable ASCII characters from ! to ~.
    let mut encoded = String::new();
    loop {
        let digit = u8::try_from(index % 94).expect("modulo 94 fits u8");
        encoded.push(char::from(b'!' + digit));
        index /= 94;
        if index == 0 {
            break;
        }
    }
    encoded
}

fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect()
}

/// Incremental canonical digest of declarations and signal changes.
pub struct TraceDigest {
    digest: Sha256,
}

impl Default for TraceDigest {
    fn default() -> Self {
        let mut digest = Sha256::new();
        digest.update(b"renvo-trace-digest-v1\0");
        Self { digest }
    }
}

impl TraceDigest {
    /// Creates an empty versioned digest.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds declarations and initial values in stable ID order.
    pub fn begin(&mut self, registry: &SignalRegistry) {
        for descriptor in registry.descriptors() {
            self.digest.update(b"D\0");
            self.digest.update(descriptor.id.get().to_le_bytes());
            self.digest.update(descriptor.width.to_le_bytes());
            self.digest.update(descriptor.path.as_bytes());
            self.digest.update([0]);
            self.update_value(
                registry
                    .value(descriptor.id)
                    .expect("value exists for every descriptor"),
            );
        }
    }

    /// Adds one signal transition.
    pub fn change(&mut self, change: &SignalChange) {
        self.digest.update(b"C\0");
        self.digest.update(change.at.ticks().to_le_bytes());
        self.digest.update(change.signal.get().to_le_bytes());
        self.update_value(&change.value);
    }

    fn update_value(&mut self, value: &SignalValue) {
        self.digest.update(value.width().to_le_bytes());
        for bit in value.bits() {
            self.digest.update([bit.vcd() as u8]);
        }
    }

    /// Returns the lowercase SHA-256 digest.
    pub fn finish(self) -> String {
        hex::encode(self.digest.finalize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use renvo_signals::{Logic, SignalValue};

    #[test]
    fn emits_hierarchical_vcd_and_suppresses_no_values() {
        let mut registry = SignalRegistry::new();
        let led = registry
            .declare(
                "board.mcu.gpio0",
                SignalValue::repeat(Logic::Zero, 1).unwrap(),
                None,
            )
            .unwrap();
        let bus = registry
            .declare(
                "board.mcu.port",
                SignalValue::from_u64(0xa, 4).unwrap(),
                None,
            )
            .unwrap();

        let mut writer = VcdWriter::new(Vec::new(), Timescale::Nanosecond);
        writer.begin(&registry).unwrap();
        let change = registry
            .set(
                led,
                SignalValue::repeat(Logic::One, 1).unwrap(),
                SimTime::from_ticks(5),
            )
            .unwrap()
            .unwrap();
        writer.change(&change).unwrap();
        let bus_change = registry
            .set(
                bus,
                SignalValue::from_u64(3, 4).unwrap(),
                SimTime::from_ticks(5),
            )
            .unwrap()
            .unwrap();
        writer.change(&bus_change).unwrap();
        writer.finish().unwrap();
        let output = String::from_utf8(writer.into_inner()).unwrap();
        assert!(output.contains("$scope module board $end"));
        assert!(output.contains("$scope module mcu $end"));
        assert!(output.contains("$var wire 1"));
        assert!(output.contains("$var wire 4"));
        assert!(output.contains("#5"));
        assert!(output.contains("b0011"));
    }

    #[test]
    fn digest_is_repeatable() {
        let mut registry = SignalRegistry::new();
        registry
            .declare("pin", SignalValue::repeat(Logic::Z, 1).unwrap(), None)
            .unwrap();
        let mut first = TraceDigest::new();
        first.begin(&registry);
        let mut second = TraceDigest::new();
        second.begin(&registry);
        assert_eq!(first.finish(), second.finish());
    }
}
