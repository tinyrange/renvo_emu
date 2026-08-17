use remu_bus::{BusAccessObserver, BusAccessRecord, SharedBusAccessObserver};
use remu_core::AccessKind;
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::rc::Rc;

/// Bounded execution-access information retained after a direct run.
#[derive(Debug, Default)]
pub(super) struct AccessSummary {
    pub(super) fetch_accesses: u64,
    pub(super) execute_addresses: BTreeSet<u64>,
}

/// Owns the CLI's streaming bus log and bounded coverage accumulator.
pub(super) struct DirectAccessOutput {
    state: Rc<RefCell<DirectAccessState>>,
    enabled: bool,
}

impl DirectAccessOutput {
    pub(super) fn new(
        bus_log: Option<&Path>,
        bus_log_regions: &[String],
        coverage: bool,
    ) -> io::Result<Self> {
        let bus_log = bus_log.map(create_bus_log).transpose()?;
        let enabled = bus_log.is_some() || coverage;
        Ok(Self {
            state: Rc::new(RefCell::new(DirectAccessState {
                bus_log,
                bus_log_regions: bus_log_regions.iter().cloned().collect(),
                coverage: coverage.then(AccessSummary::default),
            })),
            enabled,
        })
    }

    pub(super) fn observer(&self) -> Option<SharedBusAccessObserver> {
        self.enabled.then(|| {
            Rc::new(RefCell::new(DirectAccessObserver {
                state: self.state.clone(),
            })) as SharedBusAccessObserver
        })
    }

    pub(super) fn finish(self) -> io::Result<AccessSummary> {
        let mut state = self.state.borrow_mut();
        if let Some(writer) = &mut state.bus_log {
            writer.finish()?;
        }
        Ok(state.coverage.take().unwrap_or_default())
    }
}

fn create_bus_log(path: &Path) -> io::Result<StreamingAccessLog<BufWriter<File>>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(StreamingAccessLog::new(BufWriter::new(File::create(path)?)))
}

struct DirectAccessObserver {
    state: Rc<RefCell<DirectAccessState>>,
}

impl BusAccessObserver for DirectAccessObserver {
    fn observe(&mut self, record: &BusAccessRecord) {
        let mut state = self.state.borrow_mut();
        if retain_in_bus_log(&state.bus_log_regions, record) {
            if let Some(writer) = &mut state.bus_log {
                writer.record(record);
            }
        }
        if record.kind == AccessKind::Execute {
            if let Some(coverage) = &mut state.coverage {
                coverage.fetch_accesses = coverage.fetch_accesses.saturating_add(1);
                coverage.execute_addresses.insert(record.address);
            }
        }
    }
}

fn retain_in_bus_log(regions: &BTreeSet<String>, record: &BusAccessRecord) -> bool {
    regions.is_empty() || regions.contains(&record.region)
}

struct DirectAccessState {
    bus_log: Option<StreamingAccessLog<BufWriter<File>>>,
    bus_log_regions: BTreeSet<String>,
    coverage: Option<AccessSummary>,
}

/// Incremental encoder that is byte-identical to `serde_json::to_vec_pretty`
/// on the same ordered slice, without retaining prior records.
struct StreamingAccessLog<W> {
    writer: W,
    records: u64,
    error: Option<String>,
    finished: bool,
}

impl<W: Write> StreamingAccessLog<W> {
    const fn new(writer: W) -> Self {
        Self {
            writer,
            records: 0,
            error: None,
            finished: false,
        }
    }

    fn record(&mut self, record: &BusAccessRecord) {
        if self.error.is_some() || self.finished {
            return;
        }
        let first = self.records == 0;
        match write_pretty_array_element(&mut self.writer, first, record) {
            Ok(()) => self.records = self.records.saturating_add(1),
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn finish(&mut self) -> io::Result<()> {
        if self.finished {
            return Ok(());
        }
        if let Some(error) = self.error.take() {
            return Err(io::Error::other(error));
        }
        if self.records == 0 {
            self.writer.write_all(b"[]")?;
        } else {
            self.writer.write_all(b"\n]")?;
        }
        self.writer.flush()?;
        self.finished = true;
        Ok(())
    }
}

fn write_pretty_array_element(
    writer: &mut dyn Write,
    first: bool,
    record: &BusAccessRecord,
) -> io::Result<()> {
    let encoded = serde_json::to_vec_pretty(record).map_err(io::Error::other)?;
    writer.write_all(if first { b"[\n" } else { b",\n" })?;
    for (index, line) in encoded.split(|byte| *byte == b'\n').enumerate() {
        if index != 0 {
            writer.write_all(b"\n")?;
        }
        writer.write_all(b"  ")?;
        writer.write_all(line)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use remu_core::{AccessWidth, SimTime};

    fn records() -> Vec<BusAccessRecord> {
        vec![
            BusAccessRecord {
                at: SimTime::from_ticks(1),
                pc: Some(0x4200_0000),
                kind: AccessKind::Execute,
                address: 0x4200_0000,
                width: AccessWidth::Word,
                value: 0x1234_5678,
                pre_value: None,
                post_value: None,
                region: "esp32c6.irom".to_owned(),
            },
            BusAccessRecord {
                at: SimTime::from_ticks(2),
                pc: Some(0x4200_0004),
                kind: AccessKind::Write,
                address: 0x6009_1004,
                width: AccessWidth::Word,
                value: 1 << 7,
                pre_value: Some(0),
                post_value: Some(1 << 7),
                region: "esp32c6.gpio".to_owned(),
            },
        ]
    }

    #[test]
    fn streamed_json_matches_the_existing_pretty_array_encoding() {
        let records = records();
        let mut stream = StreamingAccessLog::new(Vec::new());
        for record in &records {
            stream.record(record);
        }
        stream.finish().unwrap();
        assert_eq!(stream.writer, serde_json::to_vec_pretty(&records).unwrap());
    }

    #[test]
    fn bus_log_region_filter_matches_exact_region_names() {
        let records = records();
        assert!(retain_in_bus_log(&BTreeSet::new(), &records[0]));
        let regions = BTreeSet::from(["esp32c6.gpio".to_owned()]);
        assert!(!retain_in_bus_log(&regions, &records[0]));
        assert!(retain_in_bus_log(&regions, &records[1]));
    }

    #[test]
    fn empty_stream_matches_the_existing_encoding() {
        let mut stream = StreamingAccessLog::new(Vec::new());
        stream.finish().unwrap();
        assert_eq!(stream.writer, b"[]");
    }
}
