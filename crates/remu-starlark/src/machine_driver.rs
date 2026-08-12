//! Bounded, agent-facing control of a live ESP32-C6 or ESP32-S3 machine.
#![allow(clippy::too_many_arguments)] // Starlark keyword arguments form the public script API.

use allocative::Allocative;
use anyhow::{Context, Result, bail};
use remu_bus::{BusAccessObserver, BusAccessRecord, SharedBusAccessObserver};
use remu_core::{AccessKind, RunLimits, SimTime};
use remu_machines::{RiscVMachine, RunResult, TargetId, XtensaMachine};
use remu_radio::{MediumProfile, RadioProtocol, ReplayArtifact, Spectrum};
use remu_signals::Logic;
use serde_json::{Value as JsonValue, json};
use starlark::environment::{GlobalsBuilder, LibraryExtension, Methods, MethodsBuilder, Module};
use starlark::eval::{Evaluator, ReturnFileLoader};
use starlark::starlark_module;
use starlark::syntax::{AstModule, Dialect};
use starlark::values::list::UnpackList;
use starlark::values::{
    NoSerialize, ProvidesStaticType, StarlarkValue, Value, ValueLike, none::NoneType,
    starlark_value,
};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fmt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::assertion_globals;
use crate::script_loader::{LoadBudget, compile_loads};

const MAX_STARLARK_HEAP_BYTES: usize = 64 << 20;
const MAX_STARLARK_TICKS: u64 = 10_000_000;
const MAX_AGENT_SCRIPT_SOURCE_BYTES: usize = 8 << 20;
const MAX_MACHINE_RUN_INSTRUCTIONS: u64 = 100_000_000;
const MAX_DEBUG_TRANSFER_BYTES: usize = 1 << 20;
const MAX_RADIO_EVENTS_PER_READ: usize = 4096;
const MAX_BUS_EVENTS_PER_READ: usize = 4096;
const MAX_BUS_CAPTURE_RECORDS: usize = 65_536;
const BUS_CAPTURE_READ: u8 = 1 << 0;
const BUS_CAPTURE_WRITE: u8 = 1 << 1;
const BUS_CAPTURE_EXECUTE: u8 = 1 << 2;

/// One live machine supported by the agent-driver surface.
pub enum AgentMachine {
    /// ESP32-C6 RISC-V machine.
    Esp32c6(Box<RiscVMachine>),
    /// ESP32-S3 Xtensa machine.
    Esp32s3(Box<XtensaMachine>),
}

impl AgentMachine {
    fn target(&self) -> TargetId {
        match self {
            Self::Esp32c6(machine) => machine.target(),
            Self::Esp32s3(_) => TargetId::Esp32s3,
        }
    }

    fn run(&mut self, limits: RunLimits) -> Result<RunResult> {
        match self {
            Self::Esp32c6(machine) => Ok(machine.run(limits, None)?),
            Self::Esp32s3(machine) => Ok(machine.run(limits, None)?),
        }
    }

    fn snapshot(&self) -> remu_core::CpuSnapshot {
        match self {
            Self::Esp32c6(machine) => machine.debug_snapshot(),
            Self::Esp32s3(machine) => machine.debug_snapshot(),
        }
    }

    fn read_memory(&mut self, address: u64, length: usize) -> Result<Vec<u8>> {
        match self {
            Self::Esp32c6(machine) => machine
                .debug_read_memory(address, length)
                .map_err(anyhow::Error::msg),
            Self::Esp32s3(machine) => machine
                .debug_read_memory(address, length)
                .map_err(anyhow::Error::msg),
        }
    }

    fn write_memory(&mut self, address: u64, bytes: &[u8]) -> Result<()> {
        match self {
            Self::Esp32c6(machine) => machine
                .debug_write_memory(address, bytes)
                .map_err(anyhow::Error::msg),
            Self::Esp32s3(machine) => machine
                .debug_write_memory(address, bytes)
                .map_err(anyhow::Error::msg),
        }
    }

    fn add_breakpoint(&mut self, address: u64) {
        match self {
            Self::Esp32c6(machine) => machine.add_breakpoint(address),
            Self::Esp32s3(machine) => machine.add_breakpoint(address),
        }
    }

    fn add_watchpoint(&mut self, address: u64) {
        match self {
            Self::Esp32c6(machine) => machine.add_watchpoint(address),
            Self::Esp32s3(machine) => machine.add_watchpoint(address),
        }
    }

    fn clear_debug_stops(&mut self) {
        match self {
            Self::Esp32c6(machine) => machine.clear_debug_stops(),
            Self::Esp32s3(machine) => machine.clear_debug_stops(),
        }
    }

    fn set_pin(&self, pin: u8, value: Logic) -> Result<()> {
        match self {
            Self::Esp32c6(machine) => Ok(machine.set_pin(pin, value)?),
            Self::Esp32s3(machine) => Ok(machine.set_pin(pin, value)?),
        }
    }

    fn queue_usb_input(&mut self, bytes: &[u8]) {
        match self {
            Self::Esp32c6(machine) => machine.queue_usb_input(bytes),
            Self::Esp32s3(machine) => machine.queue_usb_input(bytes),
        }
    }

    fn inject_radio_frame(
        &mut self,
        at: SimTime,
        protocol: RadioProtocol,
        spectrum: Spectrum,
        phy: String,
        bytes: Vec<u8>,
        power_dbm: i16,
    ) -> Result<()> {
        match self {
            Self::Esp32c6(machine) => {
                machine.inject_radio_frame_at(at, protocol, spectrum, phy, bytes, power_dbm)?;
            }
            Self::Esp32s3(machine) => {
                machine.inject_radio_frame_at(at, protocol, spectrum, phy, bytes, power_dbm)?;
            }
        }
        Ok(())
    }

    fn radio_replay(&self) -> ReplayArtifact {
        match self {
            Self::Esp32c6(machine) => machine
                .radio_replay_artifact()
                .unwrap_or_else(|| ReplayArtifact::new(MediumProfile::default(), Vec::new())),
            Self::Esp32s3(machine) => machine.radio_replay_artifact(),
        }
    }

    fn add_access_observer(&mut self, observer: SharedBusAccessObserver) {
        match self {
            Self::Esp32c6(machine) => machine.add_access_observer(observer),
            Self::Esp32s3(machine) => machine.add_access_observer(observer),
        }
    }
}

#[derive(Debug)]
struct AgentBusCapture {
    enabled: bool,
    start: u64,
    end: u64,
    kinds: u8,
    regions: BTreeSet<String>,
    capacity: usize,
    next_cursor: u64,
    dropped: u64,
    records: VecDeque<(u64, BusAccessRecord)>,
}

impl Default for AgentBusCapture {
    fn default() -> Self {
        Self {
            enabled: false,
            start: 0,
            end: u64::MAX,
            kinds: BUS_CAPTURE_READ | BUS_CAPTURE_WRITE,
            regions: BTreeSet::new(),
            capacity: 4096,
            next_cursor: 0,
            dropped: 0,
            records: VecDeque::new(),
        }
    }
}

impl AgentBusCapture {
    fn accepts(&self, record: &BusAccessRecord) -> bool {
        let kind = match record.kind {
            AccessKind::Read => BUS_CAPTURE_READ,
            AccessKind::Write => BUS_CAPTURE_WRITE,
            AccessKind::Execute => BUS_CAPTURE_EXECUTE,
        };
        let access_end = record
            .address
            .saturating_add(u64::from(record.width.bytes()));
        self.enabled
            && self.kinds & kind != 0
            && record.address < self.end
            && access_end > self.start
            && (self.regions.is_empty() || self.regions.contains(&record.region))
    }

    fn configure(
        &mut self,
        start: u64,
        end: u64,
        regions: BTreeSet<String>,
        kinds: &[String],
        capacity: usize,
    ) -> Result<()> {
        if start >= end {
            bail!("bus capture start must be below end");
        }
        if capacity == 0 || capacity > MAX_BUS_CAPTURE_RECORDS {
            bail!("bus capture capacity must be in 1..={MAX_BUS_CAPTURE_RECORDS}");
        }
        let mut kind_mask = 0;
        for kind in kinds {
            match kind.as_str() {
                "read" => kind_mask |= BUS_CAPTURE_READ,
                "write" => kind_mask |= BUS_CAPTURE_WRITE,
                "execute" => kind_mask |= BUS_CAPTURE_EXECUTE,
                _ => bail!("unknown bus access kind {kind:?}"),
            }
        }
        if kind_mask == 0 {
            bail!("bus capture requires at least one access kind");
        }
        self.enabled = true;
        self.start = start;
        self.end = end;
        self.kinds = kind_mask;
        self.regions = regions;
        self.capacity = capacity;
        self.next_cursor = 0;
        self.dropped = 0;
        self.records.clear();
        Ok(())
    }

    fn record(&mut self, record: &BusAccessRecord) {
        if !self.accepts(record) {
            return;
        }
        let cursor = self.next_cursor;
        self.next_cursor = self.next_cursor.saturating_add(1);
        if self.records.len() == self.capacity {
            self.records.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        self.records.push_back((cursor, record.clone()));
    }
}

struct AgentBusObserver {
    capture: Rc<RefCell<AgentBusCapture>>,
}

impl BusAccessObserver for AgentBusObserver {
    fn observe(&mut self, record: &BusAccessRecord) {
        self.capture.borrow_mut().record(record);
    }
}

struct AgentSession {
    machine: Option<AgentMachine>,
    last_result: Option<RunResult>,
    bus_capture: Rc<RefCell<AgentBusCapture>>,
}

thread_local! {
    static AGENT_SESSIONS: RefCell<BTreeMap<u64, AgentSession>> = const { RefCell::new(BTreeMap::new()) };
}

static NEXT_AGENT_SESSION: AtomicU64 = AtomicU64::new(1);

struct AgentSessionGuard(u64);

impl AgentSessionGuard {
    fn insert(machine: AgentMachine, bus_capture: Rc<RefCell<AgentBusCapture>>) -> Self {
        let id = NEXT_AGENT_SESSION.fetch_add(1, Ordering::Relaxed);
        AGENT_SESSIONS.with(|sessions| {
            let previous = sessions.borrow_mut().insert(
                id,
                AgentSession {
                    machine: Some(machine),
                    last_result: None,
                    bus_capture,
                },
            );
            debug_assert!(previous.is_none());
        });
        Self(id)
    }

    fn take(mut self) -> Result<AgentSession> {
        let session = AGENT_SESSIONS.with(|sessions| sessions.borrow_mut().remove(&self.0));
        self.0 = 0;
        session.context("agent session disappeared during evaluation")
    }
}

impl Drop for AgentSessionGuard {
    fn drop(&mut self) {
        if self.0 != 0 {
            AGENT_SESSIONS.with(|sessions| {
                sessions.borrow_mut().remove(&self.0);
            });
        }
    }
}

#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
struct AgentMachineValue {
    session: u64,
}

impl fmt::Display for AgentMachineValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let target = AGENT_SESSIONS.with(|sessions| {
            sessions
                .borrow()
                .get(&self.session)
                .and_then(|session| session.machine.as_ref().map(AgentMachine::target))
        });
        match target {
            Some(target) => write!(formatter, "machine({target})"),
            None => formatter.write_str("machine(unavailable)"),
        }
    }
}

starlark::starlark_simple_value!(AgentMachineValue);

#[starlark_value(type = "machine")]
#[allow(clippy::elidable_lifetime_names)]
impl<'v> StarlarkValue<'v> for AgentMachineValue {
    fn get_methods() -> Option<&'static Methods> {
        Some(AGENT_MACHINE_METHODS.methods())
    }
}

fn agent_machine(value: Value<'_>) -> Result<&AgentMachineValue> {
    value
        .downcast_ref::<AgentMachineValue>()
        .context("method receiver is not an agent machine")
}

fn with_machine<T>(
    value: Value<'_>,
    operation: impl FnOnce(&mut AgentMachine) -> Result<T>,
) -> Result<T> {
    let driver = agent_machine(value)?;
    AGENT_SESSIONS.with(|sessions| {
        let mut sessions = sessions.borrow_mut();
        let session = sessions
            .get_mut(&driver.session)
            .context("agent machine session is no longer available")?;
        let machine = session
            .machine
            .as_mut()
            .context("agent machine is no longer available")?;
        operation(machine)
    })
}

#[starlark_module]
fn agent_machine_methods(builder: &mut MethodsBuilder) {
    /// Returns the stable target identifier.
    fn target<'v>(#[starlark(this)] this: Value<'v>) -> anyhow::Result<String> {
        with_machine(this, |machine| Ok(machine.target().to_string()))
    }

    /// Runs a resumable machine slice with explicit deterministic bounds.
    fn run<'v>(
        #[starlark(this)] this: Value<'v>,
        #[starlark(default = 1_000_000)] instructions: u64,
        #[starlark(default = 0)] deadline: u64,
    ) -> anyhow::Result<JsonValue> {
        if instructions == 0 && deadline == 0 {
            bail!("machine.run requires instructions or deadline");
        }
        if instructions > MAX_MACHINE_RUN_INSTRUCTIONS {
            bail!(
                "machine.run instruction budget {instructions} exceeds {MAX_MACHINE_RUN_INSTRUCTIONS}"
            );
        }
        let driver = agent_machine(this)?;
        AGENT_SESSIONS.with(|sessions| {
            let mut sessions = sessions.borrow_mut();
            let session = sessions
                .get_mut(&driver.session)
                .context("agent machine session is no longer available")?;
            let machine = session
                .machine
                .as_mut()
                .context("agent machine is no longer available")?;
            let result = machine.run(RunLimits {
                instructions: (instructions != 0).then_some(instructions),
                deadline: (deadline != 0).then(|| SimTime::from_ticks(deadline)),
            })?;
            let json = serde_json::to_value(&result)?;
            session.last_result = Some(result);
            Ok(json)
        })
    }

    /// Returns the current architecture-neutral CPU snapshot.
    fn cpu<'v>(#[starlark(this)] this: Value<'v>) -> anyhow::Result<JsonValue> {
        with_machine(this, |machine| {
            Ok(serde_json::to_value(machine.snapshot())?)
        })
    }

    /// Reads a bounded byte range through the debugger bus boundary.
    fn read<'v>(
        #[starlark(this)] this: Value<'v>,
        address: u64,
        length: u32,
    ) -> anyhow::Result<JsonValue> {
        let length = usize::try_from(length).context("read length does not fit usize")?;
        if length > MAX_DEBUG_TRANSFER_BYTES {
            bail!("machine.read length exceeds {MAX_DEBUG_TRANSFER_BYTES} bytes");
        }
        with_machine(this, |machine| {
            Ok(json!(machine.read_memory(address, length)?))
        })
    }

    /// Writes a bounded byte list through the debugger bus boundary.
    fn write<'v>(
        #[starlark(this)] this: Value<'v>,
        address: u64,
        bytes: UnpackList<u32>,
    ) -> anyhow::Result<NoneType> {
        if bytes.items.len() > MAX_DEBUG_TRANSFER_BYTES {
            bail!("machine.write length exceeds {MAX_DEBUG_TRANSFER_BYTES} bytes");
        }
        let bytes = bytes
            .items
            .into_iter()
            .map(|byte| u8::try_from(byte).context("machine.write byte must fit in 8 bits"))
            .collect::<Result<Vec<_>>>()?;
        with_machine(this, |machine| machine.write_memory(address, &bytes))?;
        Ok(NoneType)
    }

    /// Stops before the next instruction at an address.
    fn breakpoint<'v>(#[starlark(this)] this: Value<'v>, address: u64) -> anyhow::Result<NoneType> {
        with_machine(this, |machine| {
            machine.add_breakpoint(address);
            Ok(())
        })?;
        Ok(NoneType)
    }

    /// Stops after the next data access overlapping an address.
    fn watchpoint<'v>(#[starlark(this)] this: Value<'v>, address: u64) -> anyhow::Result<NoneType> {
        with_machine(this, |machine| {
            machine.add_watchpoint(address);
            Ok(())
        })?;
        Ok(NoneType)
    }

    /// Removes all agent-installed breakpoint and watchpoint stops.
    fn clear_stops<'v>(#[starlark(this)] this: Value<'v>) -> anyhow::Result<NoneType> {
        with_machine(this, |machine| {
            machine.clear_debug_stops();
            Ok(())
        })?;
        Ok(NoneType)
    }

    /// Drives one exposed GPIO pin immediately at the current simulation time.
    fn pin<'v>(
        #[starlark(this)] this: Value<'v>,
        pin: u32,
        value: &str,
    ) -> anyhow::Result<NoneType> {
        let pin = u8::try_from(pin).context("pin must fit in 8 bits")?;
        let value = match value {
            "0" => Logic::Zero,
            "1" => Logic::One,
            "z" | "Z" => Logic::Z,
            "x" | "X" => Logic::X,
            _ => bail!("pin value must be 0, 1, z, or x"),
        };
        with_machine(this, |machine| machine.set_pin(pin, value))?;
        Ok(NoneType)
    }

    /// Queues bounded bytes for the emulated USB serial host.
    fn usb_input<'v>(
        #[starlark(this)] this: Value<'v>,
        bytes: UnpackList<u32>,
    ) -> anyhow::Result<NoneType> {
        if bytes.items.len() > MAX_DEBUG_TRANSFER_BYTES {
            bail!("machine.usb_input length exceeds {MAX_DEBUG_TRANSFER_BYTES} bytes");
        }
        let bytes = bytes
            .items
            .into_iter()
            .map(|byte| u8::try_from(byte).context("USB input byte must fit in 8 bits"))
            .collect::<Result<Vec<_>>>()?;
        with_machine(this, |machine| {
            machine.queue_usb_input(&bytes);
            Ok(())
        })?;
        Ok(NoneType)
    }

    /// Injects one deterministic isolated-medium frame at an absolute tick.
    fn inject_radio<'v>(
        #[starlark(this)] this: Value<'v>,
        protocol: &str,
        at: u64,
        center_khz: u32,
        bandwidth_khz: u32,
        phy: &str,
        bytes: UnpackList<u32>,
        #[starlark(default = -40)] power_dbm: i32,
    ) -> anyhow::Result<NoneType> {
        if bytes.items.len() > MAX_DEBUG_TRANSFER_BYTES {
            bail!("machine.inject_radio frame exceeds {MAX_DEBUG_TRANSFER_BYTES} bytes");
        }
        let protocol = match protocol {
            "wifi" => RadioProtocol::Wifi,
            "bluetooth-le" | "ble" => RadioProtocol::BluetoothLe,
            "ieee802154" | "802.15.4" => RadioProtocol::Ieee802154,
            _ => bail!("unknown radio protocol {protocol:?}"),
        };
        let power_dbm = i16::try_from(power_dbm).context("power_dbm must fit in 16 bits")?;
        let bytes = bytes
            .items
            .into_iter()
            .map(|byte| u8::try_from(byte).context("radio byte must fit in 8 bits"))
            .collect::<Result<Vec<_>>>()?;
        with_machine(this, |machine| {
            machine.inject_radio_frame(
                SimTime::from_ticks(at),
                protocol,
                Spectrum::new(center_khz, bandwidth_khz),
                phy.to_owned(),
                bytes,
                power_dbm,
            )
        })?;
        Ok(NoneType)
    }

    /// Returns a bounded page from the append-only RF-medium evidence.
    fn radio_events<'v>(
        #[starlark(this)] this: Value<'v>,
        #[starlark(default = 0)] cursor: u32,
        #[starlark(default = 256)] limit: u32,
    ) -> anyhow::Result<JsonValue> {
        let cursor = usize::try_from(cursor).context("radio cursor does not fit usize")?;
        let limit = usize::try_from(limit).context("radio limit does not fit usize")?;
        if limit == 0 || limit > MAX_RADIO_EVENTS_PER_READ {
            bail!("radio event limit must be in 1..={MAX_RADIO_EVENTS_PER_READ}");
        }
        with_machine(this, |machine| {
            let artifact = machine.radio_replay();
            let end = cursor.saturating_add(limit).min(artifact.events.len());
            let events = artifact.events.get(cursor..end).unwrap_or_default();
            Ok(json!({
                "schema": "remu.agent-radio-events.v1",
                "cursor": cursor,
                "next_cursor": end,
                "complete": end == artifact.events.len(),
                "total": artifact.events.len(),
                "events": events,
                "coexistence_total": artifact.coexistence_events.len(),
            }))
        })
    }

    /// Returns a bounded page from append-only coexistence arbitration evidence.
    fn coexistence_events<'v>(
        #[starlark(this)] this: Value<'v>,
        #[starlark(default = 0)] cursor: u32,
        #[starlark(default = 256)] limit: u32,
    ) -> anyhow::Result<JsonValue> {
        let cursor = usize::try_from(cursor).context("coexistence cursor does not fit usize")?;
        let limit = usize::try_from(limit).context("coexistence limit does not fit usize")?;
        if limit == 0 || limit > MAX_RADIO_EVENTS_PER_READ {
            bail!("coexistence event limit must be in 1..={MAX_RADIO_EVENTS_PER_READ}");
        }
        with_machine(this, |machine| {
            let artifact = machine.radio_replay();
            let end = cursor
                .saturating_add(limit)
                .min(artifact.coexistence_events.len());
            let events = artifact
                .coexistence_events
                .get(cursor..end)
                .unwrap_or_default();
            Ok(json!({
                "schema": "remu.agent-coexistence-events.v1",
                "cursor": cursor,
                "next_cursor": end,
                "complete": end == artifact.coexistence_events.len(),
                "total": artifact.coexistence_events.len(),
                "events": events,
            }))
        })
    }

    /// Starts a fresh bounded bus-access capture for a native address/region window.
    fn capture_bus<'v>(
        #[starlark(this)] this: Value<'v>,
        #[starlark(default = 0)] start: u64,
        end: Option<u64>,
        regions: Option<UnpackList<String>>,
        kinds: Option<UnpackList<String>>,
        #[starlark(default = 4096)] capacity: u32,
    ) -> anyhow::Result<NoneType> {
        let driver = agent_machine(this)?;
        let end = end.unwrap_or(u64::MAX);
        let regions = regions
            .unwrap_or_default()
            .items
            .into_iter()
            .collect::<BTreeSet<_>>();
        let kinds = kinds.map_or_else(
            || vec!["read".to_owned(), "write".to_owned()],
            |kinds| kinds.items,
        );
        let capacity = usize::try_from(capacity).context("bus capacity does not fit usize")?;
        AGENT_SESSIONS.with(|sessions| {
            let sessions = sessions.borrow();
            let session = sessions
                .get(&driver.session)
                .context("agent machine session is no longer available")?;
            session
                .bus_capture
                .borrow_mut()
                .configure(start, end, regions, &kinds, capacity)
        })?;
        Ok(NoneType)
    }

    /// Stops recording new bus accesses while retaining the bounded evidence window.
    fn stop_bus_capture<'v>(#[starlark(this)] this: Value<'v>) -> anyhow::Result<NoneType> {
        let driver = agent_machine(this)?;
        AGENT_SESSIONS.with(|sessions| {
            let sessions = sessions.borrow();
            let session = sessions
                .get(&driver.session)
                .context("agent machine session is no longer available")?;
            session.bus_capture.borrow_mut().enabled = false;
            Ok::<(), anyhow::Error>(())
        })?;
        Ok(NoneType)
    }

    /// Returns a bounded page from the current filtered bus-access capture.
    fn bus_events<'v>(
        #[starlark(this)] this: Value<'v>,
        #[starlark(default = 0)] cursor: u64,
        #[starlark(default = 256)] limit: u32,
    ) -> anyhow::Result<JsonValue> {
        let driver = agent_machine(this)?;
        let limit = usize::try_from(limit).context("bus event limit does not fit usize")?;
        if limit == 0 || limit > MAX_BUS_EVENTS_PER_READ {
            bail!("bus event limit must be in 1..={MAX_BUS_EVENTS_PER_READ}");
        }
        AGENT_SESSIONS.with(|sessions| {
            let sessions = sessions.borrow();
            let session = sessions
                .get(&driver.session)
                .context("agent machine session is no longer available")?;
            let capture = session.bus_capture.borrow();
            let earliest = capture
                .records
                .front()
                .map_or(capture.next_cursor, |(cursor, _)| *cursor);
            let start = cursor.max(earliest);
            let records = capture
                .records
                .iter()
                .filter(|(record_cursor, _)| *record_cursor >= start)
                .take(limit)
                .map(|(record_cursor, record)| json!({"cursor": record_cursor, "access": record}))
                .collect::<Vec<_>>();
            let next_cursor = records
                .last()
                .and_then(|record| record.get("cursor"))
                .and_then(JsonValue::as_u64)
                .map_or(start, |cursor| cursor.saturating_add(1));
            Ok(json!({
                "schema": "remu.agent-bus-events.v1",
                "cursor": cursor,
                "earliest_cursor": earliest,
                "next_cursor": next_cursor,
                "complete": next_cursor >= capture.next_cursor,
                "enabled": capture.enabled,
                "captured": capture.next_cursor,
                "retained": capture.records.len(),
                "dropped": capture.dropped,
                "missed_before_cursor": earliest.saturating_sub(cursor),
                "events": records,
            }))
        })
    }
}

starlark::methods_static!(AGENT_MACHINE_METHODS = agent_machine_methods);

/// Result of one complete agent-driver script.
pub struct AgentScriptOutcome {
    /// Live machine after the script has finished, suitable for artifact output.
    pub machine: AgentMachine,
    /// Result of the script's final `machine.run` call.
    pub result: RunResult,
    /// JSON-compatible return value from `main()`.
    pub value: JsonValue,
}

/// Executes a bounded script whose `main()` function drives one live machine.
///
/// `repl()` is an alias for Starlark's scoped `breakpoint()` console. It is
/// enabled only when `interactive` is true. The driver exposes no symbol hook
/// API: all execution and peripheral behavior remains on the native LLE path.
pub fn evaluate_agent_script(
    filename: &str,
    source: &str,
    mut machine: AgentMachine,
    interactive: bool,
) -> Result<AgentScriptOutcome> {
    if source.len() > MAX_AGENT_SCRIPT_SOURCE_BYTES {
        bail!("agent script exceeds {MAX_AGENT_SCRIPT_SOURCE_BYTES} source bytes");
    }
    let source = format!("repl = breakpoint\n{source}");
    let dialect = Dialect::Extended;
    let ast = AstModule::parse(filename, source, &dialect)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let mut globals = GlobalsBuilder::extended_by(&[
        LibraryExtension::Breakpoint,
        LibraryExtension::Json,
        LibraryExtension::Print,
        LibraryExtension::Pprint,
        LibraryExtension::StructType,
    ]);
    assertion_globals(&mut globals);
    let globals = globals.build();
    let load_root = agent_load_root(filename);
    let compiled_modules = compile_loads(
        &ast,
        &load_root,
        &globals,
        &dialect,
        &mut BTreeSet::new(),
        &mut LoadBudget::default(),
    )?;
    let references: HashMap<&str, &starlark::environment::FrozenModule> = compiled_modules
        .iter()
        .map(|(name, module)| (name.as_str(), module))
        .collect();
    let file_loader = ReturnFileLoader {
        modules: &references,
    };
    let bus_capture = Rc::new(RefCell::new(AgentBusCapture::default()));
    machine.add_access_observer(Rc::new(RefCell::new(AgentBusObserver {
        capture: bus_capture.clone(),
    })));
    let session = AgentSessionGuard::insert(machine, bus_capture);
    let value = Module::with_temp_heap(|module| -> Result<JsonValue> {
        let machine = module
            .heap()
            .alloc(AgentMachineValue { session: session.0 });
        module.set("machine", machine);
        let mut evaluator = Evaluator::new(&module);
        evaluator.set_loader(&file_loader);
        evaluator.set_max_callstack_size(256)?;
        evaluator.set_max_heap_size(MAX_STARLARK_HEAP_BYTES)?;
        evaluator.set_max_tick_count(MAX_STARLARK_TICKS)?;
        if interactive {
            evaluator.enable_terminal_breakpoint_console();
        }
        evaluator
            .eval_module(ast, &globals)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        let main = module
            .get("main")
            .context("agent script must define main()")?;
        evaluator
            .eval_function(main, &[], &[])
            .map_err(|error| anyhow::anyhow!(error.to_string()))?
            .to_json_value()
            .context("agent script main() result must be JSON-compatible")
    })?;
    let mut session = session.take()?;
    let machine = session
        .machine
        .take()
        .context("agent script lost ownership of its machine")?;
    let result = session
        .last_result
        .take()
        .context("agent script main() must call machine.run at least once")?;
    Ok(AgentScriptOutcome {
        machine,
        result,
        value,
    })
}

fn agent_load_root(filename: &str) -> PathBuf {
    let path = Path::new(filename);
    if path.is_absolute() {
        if let Ok(workspace) = std::env::current_dir()
            && path.starts_with(&workspace)
        {
            return workspace;
        }
        path.parent()
            .unwrap_or_else(|| Path::new("/"))
            .to_path_buf()
    } else {
        PathBuf::from(".")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn c6_agent_script_resumes_machine_and_pages_radio_evidence() {
        let machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
        let outcome = evaluate_agent_script(
            "agent.star",
            r#"
def main():
    assert_eq(machine.target(), "esp32c6")
    machine.write(0x40800000, [0x13, 0, 0, 0])
    assert_eq(machine.read(0x40800000, 4), [0x13, 0, 0, 0])
    machine.inject_radio(
        protocol = "wifi",
        at = 10,
        center_khz = 2412000,
        bandwidth_khz = 20000,
        phy = "wifi-ht20",
        bytes = [0x40, 0],
    )
    page = machine.radio_events(limit = 1)
    assert_eq(page["total"], 1)
    assert_eq(machine.coexistence_events(limit = 1)["total"], 0)
    result = machine.run(instructions = 1)
    return {"page": page, "pc": result["cpu"]["pc"]}
"#,
            AgentMachine::Esp32c6(Box::new(machine)),
            false,
        )
        .unwrap();

        assert_eq!(outcome.value["page"]["total"], json!(1));
        assert!(matches!(outcome.machine, AgentMachine::Esp32c6(_)));
    }

    #[test]
    fn s3_agent_script_exposes_the_same_portable_surface() {
        let machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
        let outcome = evaluate_agent_script(
            "agent.star",
            r#"
def main():
    assert_eq(machine.target(), "esp32s3")
    before = machine.cpu()
    result = machine.run(instructions = 1)
    return {"before": before["architecture"], "after": result["cpu"]["architecture"]}
"#,
            AgentMachine::Esp32s3(Box::new(machine)),
            false,
        )
        .unwrap();

        assert_eq!(outcome.value["before"], outcome.value["after"]);
        assert!(matches!(outcome.machine, AgentMachine::Esp32s3(_)));
    }

    #[test]
    fn agent_bus_capture_filters_pages_and_reports_ring_loss() {
        let machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
        let outcome = evaluate_agent_script(
            "agent.star",
            r#"
def main():
    machine.capture_bus(
        start = 0x40800000,
        end = 0x40800010,
        regions = ["hp-sram"],
        kinds = ["write"],
        capacity = 2,
    )
    machine.write(0x40800000, [0x11, 0x22, 0x33])
    machine.read(0x40800000, 3)
    machine.stop_bus_capture()
    first = machine.bus_events(cursor = 0, limit = 1)
    second = machine.bus_events(cursor = first["next_cursor"], limit = 1)
    result = machine.run(instructions = 1)
    return {"first": first, "second": second, "result": result["reason"]}
"#,
            AgentMachine::Esp32c6(Box::new(machine)),
            false,
        )
        .unwrap();

        assert_eq!(outcome.value["first"]["captured"], json!(3));
        assert_eq!(outcome.value["first"]["retained"], json!(2));
        assert_eq!(outcome.value["first"]["dropped"], json!(1));
        assert_eq!(outcome.value["first"]["missed_before_cursor"], json!(1));
        assert_eq!(outcome.value["first"]["events"][0]["cursor"], json!(1));
        assert_eq!(
            outcome.value["first"]["events"][0]["access"]["kind"],
            json!("Write")
        );
        assert_eq!(outcome.value["second"]["events"][0]["cursor"], json!(2));
        assert_eq!(outcome.value["second"]["complete"], json!(true));
    }

    #[test]
    fn failed_agent_script_releases_its_scoped_session() {
        let machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
        let error = evaluate_agent_script(
            "agent.star",
            "
def main():
    machine.read(0x40800000, 1048577)
",
            AgentMachine::Esp32c6(Box::new(machine)),
            false,
        )
        .err()
        .expect("oversized read should fail");
        assert!(error.to_string().contains("exceeds 1048576 bytes"));
        AGENT_SESSIONS.with(|sessions| assert!(sessions.borrow().is_empty()));
    }

    #[test]
    fn agent_script_requires_a_bounded_machine_run() {
        let machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
        let error = evaluate_agent_script(
            "agent.star",
            "def main():\n    return machine.cpu()\n",
            AgentMachine::Esp32c6(Box::new(machine)),
            false,
        )
        .err()
        .expect("missing machine.run should fail");
        assert!(
            error
                .to_string()
                .contains("must call machine.run at least once")
        );
    }

    #[test]
    fn agent_script_loads_workspace_confined_workflow_modules() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("workflow")).unwrap();
        fs::write(
            directory.path().join("workflow/run.star"),
            include_str!("../../../qualification/starlark/agent_automation.star"),
        )
        .unwrap();
        let filename = directory.path().join("driver.star");
        let outcome = evaluate_agent_script(
            filename.to_str().unwrap(),
            r#"
load("//workflow:run.star", "drain_bus", "drain_radio", "run_until", "select_bus")
def accept(machine, result):
    return result
def select_write(access):
    if access["kind"] == "Write":
        return {"address": access["address"], "value": access["value"]}
    return None
def main():
    empty = drain_radio(machine, maximum = 1)
    machine.capture_bus(start = 0x40800000, end = 0x40800004, kinds = ["write"])
    machine.write(0x40800000, [1])
    bus = drain_bus(machine, maximum = 1)
    selected = select_bus(machine, select_write, maximum = 1)
    outcome = run_until(machine, accept, instructions = 1, max_slices = 1)
    return {"bus": bus, "empty": empty, "outcome": outcome, "selected": selected}
"#,
            AgentMachine::Esp32c6(Box::new(RiscVMachine::new(TargetId::Esp32c6).unwrap())),
            false,
        )
        .unwrap();
        assert_eq!(outcome.value["empty"]["complete"], json!(true));
        assert_eq!(outcome.value["bus"]["events"].as_array().unwrap().len(), 1);
        assert_eq!(outcome.value["selected"]["matches"][0]["value"], json!(1));
        assert_eq!(outcome.value["outcome"]["slices"], json!(1));
        assert_eq!(
            outcome.value["outcome"]["result"]["target"],
            json!("esp32c6")
        );
    }

    #[test]
    fn checked_in_radio_lab_loads_all_workflow_modules() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let filename = workspace.join("qualification/starlark/radio_lab.star");
        let source = fs::read_to_string(&filename).unwrap();
        let source = format!("repl = breakpoint\n{source}");
        let dialect = Dialect::Extended;
        let ast = AstModule::parse(filename.to_str().unwrap(), source, &dialect).unwrap();
        let mut globals = GlobalsBuilder::extended_by(&[
            LibraryExtension::Breakpoint,
            LibraryExtension::Json,
            LibraryExtension::Print,
            LibraryExtension::Pprint,
            LibraryExtension::StructType,
        ]);
        assertion_globals(&mut globals);
        let modules = compile_loads(
            &ast,
            &workspace,
            &globals.build(),
            &dialect,
            &mut BTreeSet::new(),
            &mut LoadBudget::default(),
        )
        .unwrap();

        assert_eq!(modules.len(), 3);
        let module_names = modules
            .iter()
            .map(|(name, _module)| name.as_str())
            .collect::<BTreeSet<_>>();
        assert!(module_names.contains("//qualification/starlark:agent_automation.star"));
        assert!(module_names.contains("//qualification/starlark:c6_twt_evidence.star"));
        assert!(module_names.contains("//qualification/starlark:wifi_crypto_evidence.star"));
    }

    #[test]
    fn wifi_crypto_workflow_checks_native_table_evidence() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("workflow")).unwrap();
        fs::write(
            directory.path().join("workflow/wifi_crypto.star"),
            include_str!("../../../qualification/starlark/wifi_crypto_evidence.star"),
        )
        .unwrap();
        let filename = directory.path().join("driver.star");
        let outcome = evaluate_agent_script(
            filename.to_str().unwrap(),
            r#"
load("//workflow:wifi_crypto.star", "require_espnow_ccmp_programming", "require_wifi_crypto_programming")
def access(address, value):
    return {"cursor": 0, "access": {"at": 9, "kind": "Write", "address": address, "width": "Word", "value": value, "region": "esp32c6.wifi-mac-registers"}}
def main():
    evidence = [
        access(0x600a5804, (6 << 21) | 0x55),
        access(0x600a5808, 0x03020100),
        access(0x600a5bc0, 0xccbbaa02),
        access(0x600a5bc4, 0xc16c01dd),
        access(0x600a5bc8, 0xfefa53b8),
        access(0x600a5bcc, 0xed440253),
        access(0x600a5bd0, 0x86a9d3bd),
        access(0x600a5bd4, 0xcfed0b48),
        access(0x600a4814, (1 << 24) | 1),
    ]
    summary = require_wifi_crypto_programming(machine.target(), evidence)
    espnow = require_espnow_ccmp_programming(machine.target(), evidence)
    result = machine.run(instructions = 1)
    return {"summary": summary, "espnow": espnow, "reason": result["reason"]}
"#,
            AgentMachine::Esp32c6(Box::new(RiscVMachine::new(TargetId::Esp32c6).unwrap())),
            false,
        )
        .unwrap();

        assert_eq!(outcome.value["summary"]["table_writes"], json!(8));
        assert_eq!(outcome.value["summary"]["key_writes"], json!(5));
        assert_eq!(
            outcome.value["summary"]["control_writes"][0]["class"],
            json!(6)
        );
        assert_eq!(outcome.value["espnow"]["slot"], json!(24));
        assert_eq!(outcome.value["espnow"]["cipher"], json!("ccmp"));
        assert_eq!(
            outcome.value["espnow"]["key_payload_words_verified"],
            json!(4)
        );
    }

    #[test]
    fn c6_twt_workflow_checks_native_timer_programming_evidence() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("workflow")).unwrap();
        fs::write(
            directory.path().join("workflow/c6_twt.star"),
            include_str!("../../../qualification/starlark/c6_twt_evidence.star"),
        )
        .unwrap();
        let filename = directory.path().join("driver.star");
        let outcome = evaluate_agent_script(
            filename.to_str().unwrap(),
            r#"
load("//workflow:c6_twt.star", "require_c6_twt_timer_programming")
def access(address, value):
    return {"cursor": 0, "access": {"at": 11, "kind": "Write", "address": address, "width": "Word", "value": value, "region": "esp32c6.phy-registers"}}
def main():
    evidence = [
        access(0x600ad078, 0x12345678),
        access(0x600ad0b4, 1 << 7),
        access(0x600ad0a8, 1 << 7),
        access(0x600ad074, (1 << 31) | (1 << 30) | 3),
    ]
    summary = require_c6_twt_timer_programming(evidence)
    result = machine.run(instructions = 1)
    return {"summary": summary, "reason": result["reason"]}
"#,
            AgentMachine::Esp32c6(Box::new(RiscVMachine::new(TargetId::Esp32c6).unwrap())),
            false,
        )
        .unwrap();

        assert_eq!(outcome.value["summary"]["programmed_timers"], json!([0]));
        assert_eq!(
            outcome.value["summary"]["timer_controls"][0]["mode"],
            json!(3)
        );
    }

    #[test]
    fn agent_script_load_cannot_escape_its_workspace() {
        let directory = tempfile::tempdir().unwrap();
        let filename = directory.path().join("driver.star");
        let error = evaluate_agent_script(
            filename.to_str().unwrap(),
            "load(\"../outside.star\", \"value\")\ndef main():\n    return value\n",
            AgentMachine::Esp32c6(Box::new(RiscVMachine::new(TargetId::Esp32c6).unwrap())),
            false,
        )
        .err()
        .expect("escaping load must fail");
        assert!(error.to_string().contains("escapes the Starlark root"));
    }
}
