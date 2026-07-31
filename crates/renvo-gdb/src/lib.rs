//! Small, deterministic GDB Remote Serial Protocol server.

use renvo_core::CpuSnapshot;
use serde::Serialize;
use std::fmt::Write as _;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use thiserror::Error;

const MAX_MEMORY_PACKET: usize = 0x4000;

/// Architecture name advertised through `target.xml`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum DebugArchitecture {
    /// 32-bit RISC-V.
    RiscV32,
    /// 32-bit Arm M-profile.
    Arm,
    /// 32-bit Xtensa.
    Xtensa,
}

impl DebugArchitecture {
    const fn gdb_name(self) -> &'static str {
        match self {
            Self::RiscV32 => "riscv:rv32",
            Self::Arm => "arm",
            Self::Xtensa => "xtensa",
        }
    }
}

/// Result of a continue or single-step request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DebugStop {
    /// Execution stopped for debugger inspection.
    Signal(u8),
    /// Firmware exited with this low-byte status.
    Exited(u8),
}

/// Stable machine operations consumed by the RSP adapter.
pub trait DebugTarget {
    /// Target architecture.
    fn architecture(&self) -> DebugArchitecture;
    /// Current architectural state.
    fn snapshot(&self) -> CpuSnapshot;
    /// Reads guest-visible bytes.
    fn read_memory(&mut self, address: u64, length: usize) -> Result<Vec<u8>, String>;
    /// Writes guest-visible bytes.
    fn write_memory(&mut self, address: u64, bytes: &[u8]) -> Result<(), String>;
    /// Installs an execution breakpoint.
    fn add_breakpoint(&mut self, address: u64);
    /// Removes an execution breakpoint.
    fn remove_breakpoint(&mut self, address: u64);
    /// Executes one architectural action.
    fn step(&mut self) -> Result<DebugStop, String>;
    /// Executes until a terminal condition or the supplied safety bound.
    fn continue_run(&mut self, max_instructions: u64) -> Result<DebugStop, String>;
}

/// One completed debugger session report.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct SessionReport {
    /// Architecture selected by the ELF.
    pub architecture: Option<DebugArchitecture>,
    /// Valid RSP request packets processed.
    pub packets: u64,
    /// Full or individual register reads.
    pub register_reads: u64,
    /// Memory-read packets.
    pub memory_reads: u64,
    /// Memory-write packets.
    pub memory_writes: u64,
    /// Breakpoint insertions and removals.
    pub breakpoint_operations: u64,
    /// Single-step requests.
    pub steps: u64,
    /// Continue requests.
    pub continues: u64,
    /// Whether the client detached cleanly.
    pub detached: bool,
}

/// RSP server configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServerConfig {
    /// Safety bound for one continue packet.
    pub max_continue_instructions: u64,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            max_continue_instructions: 10_000_000,
        }
    }
}

/// GDB server failure.
#[derive(Debug, Error)]
pub enum GdbError {
    /// Network transport failure.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Malformed hexadecimal request.
    #[error("malformed RSP hexadecimal value {0:?}")]
    Hex(String),
}

/// Accepts one debugger and serves it until detach or disconnect.
pub fn serve_once<T: DebugTarget>(
    listener: &TcpListener,
    target: &mut T,
    config: ServerConfig,
) -> Result<SessionReport, GdbError> {
    let (mut stream, _) = listener.accept()?;
    stream.set_nodelay(true)?;
    serve_stream(&mut stream, target, config)
}

fn serve_stream<T: DebugTarget>(
    stream: &mut TcpStream,
    target: &mut T,
    config: ServerConfig,
) -> Result<SessionReport, GdbError> {
    let mut report = SessionReport {
        architecture: Some(target.architecture()),
        ..SessionReport::default()
    };
    let mut no_ack = false;
    while let Some(packet) = read_packet(stream, no_ack)? {
        if packet.is_empty() {
            continue;
        }
        report.packets = report.packets.saturating_add(1);
        let (response, detach, enable_no_ack) = handle_packet(&packet, target, config, &mut report);
        write_packet(stream, &response)?;
        if enable_no_ack {
            no_ack = true;
        }
        if detach {
            report.detached = true;
            break;
        }
    }
    Ok(report)
}

#[allow(clippy::too_many_lines)]
fn handle_packet<T: DebugTarget>(
    packet: &str,
    target: &mut T,
    config: ServerConfig,
    report: &mut SessionReport,
) -> (String, bool, bool) {
    if packet == "?" {
        return ("S05".to_owned(), false, false);
    }
    if packet.starts_with("qSupported") {
        return (
            "PacketSize=4000;qXfer:features:read+;swbreak+".to_owned(),
            false,
            false,
        );
    }
    if packet == "QStartNoAckMode" {
        return ("OK".to_owned(), false, true);
    }
    if packet == "qAttached" {
        return ("1".to_owned(), false, false);
    }
    if packet == "qC" {
        return ("QC1".to_owned(), false, false);
    }
    if packet == "qfThreadInfo" {
        return ("m1".to_owned(), false, false);
    }
    if packet == "qsThreadInfo" {
        return ("l".to_owned(), false, false);
    }
    if packet.starts_with('H') || packet == "qSymbol::" {
        return ("OK".to_owned(), false, false);
    }
    if packet == "vCont?" {
        return ("vCont;c;s".to_owned(), false, false);
    }
    if let Some(request) = packet.strip_prefix("qXfer:features:read:target.xml:") {
        let Some((offset, length)) = parse_pair(request) else {
            return ("E01".to_owned(), false, false);
        };
        let xml = target_xml(target);
        let offset = usize::try_from(offset).unwrap_or(usize::MAX);
        let length = usize::try_from(length).unwrap_or_default();
        if offset >= xml.len() {
            return ("l".to_owned(), false, false);
        }
        let end = offset.saturating_add(length).min(xml.len());
        let prefix = if end == xml.len() { 'l' } else { 'm' };
        return (format!("{prefix}{}", &xml[offset..end]), false, false);
    }
    if packet == "g" {
        report.register_reads = report.register_reads.saturating_add(1);
        return (encode_registers(&target.snapshot()), false, false);
    }
    if let Some(index) = packet.strip_prefix('p') {
        report.register_reads = report.register_reads.saturating_add(1);
        let Ok(index) = usize::from_str_radix(index, 16) else {
            return ("E01".to_owned(), false, false);
        };
        return (
            encode_register(&target.snapshot(), index).unwrap_or_else(|| "E01".to_owned()),
            false,
            false,
        );
    }
    if let Some(request) = packet.strip_prefix('m') {
        report.memory_reads = report.memory_reads.saturating_add(1);
        let Some((address, length)) = parse_pair(request) else {
            return ("E01".to_owned(), false, false);
        };
        let Some(length) = usize::try_from(length)
            .ok()
            .filter(|length| *length <= MAX_MEMORY_PACKET)
        else {
            return ("E01".to_owned(), false, false);
        };
        return (
            target
                .read_memory(address, length)
                .map_or_else(|_| "E01".to_owned(), hex::encode),
            false,
            false,
        );
    }
    if let Some(request) = packet.strip_prefix('M') {
        report.memory_writes = report.memory_writes.saturating_add(1);
        let Some((range, data)) = request.split_once(':') else {
            return ("E01".to_owned(), false, false);
        };
        let Some((address, length)) = parse_pair(range) else {
            return ("E01".to_owned(), false, false);
        };
        if length > MAX_MEMORY_PACKET as u64 {
            return ("E01".to_owned(), false, false);
        }
        let Ok(bytes) = hex::decode(data) else {
            return ("E01".to_owned(), false, false);
        };
        if bytes.len() != usize::try_from(length).unwrap_or(usize::MAX) {
            return ("E01".to_owned(), false, false);
        }
        return (
            target
                .write_memory(address, &bytes)
                .map_or_else(|_| "E01".to_owned(), |()| "OK".to_owned()),
            false,
            false,
        );
    }
    if let Some(request) = packet
        .strip_prefix("Z0,")
        .or_else(|| packet.strip_prefix("z0,"))
    {
        report.breakpoint_operations = report.breakpoint_operations.saturating_add(1);
        let Some((address, _)) = parse_pair(request) else {
            return ("E01".to_owned(), false, false);
        };
        if packet.starts_with('Z') {
            target.add_breakpoint(address);
        } else {
            target.remove_breakpoint(address);
        }
        return ("OK".to_owned(), false, false);
    }
    if packet == "s" || packet == "vCont;s" {
        report.steps = report.steps.saturating_add(1);
        return (stop_reply(&target.step()), false, false);
    }
    if packet == "c" || packet == "vCont;c" {
        report.continues = report.continues.saturating_add(1);
        return (
            stop_reply(&target.continue_run(config.max_continue_instructions)),
            false,
            false,
        );
    }
    if packet == "D" {
        return ("OK".to_owned(), true, false);
    }
    if packet == "k" {
        return ("OK".to_owned(), true, false);
    }
    (String::new(), false, false)
}

fn stop_reply(result: &Result<DebugStop, String>) -> String {
    match result {
        Ok(DebugStop::Signal(signal)) => format!("S{signal:02x}"),
        Ok(DebugStop::Exited(code)) => format!("W{code:02x}"),
        Err(_) => "E01".to_owned(),
    }
}

fn target_xml(target: &impl DebugTarget) -> String {
    let snapshot = target.snapshot();
    let mut xml = format!(
        "<?xml version=\"1.0\"?><target><architecture>{}</architecture><feature name=\"org.renvo.core\">",
        target.architecture().gdb_name()
    );
    for (index, register) in snapshot.registers.iter().enumerate() {
        let _ = write!(
            xml,
            "<reg name=\"{}\" bitsize=\"{}\" regnum=\"{}\"/>",
            register.name, register.bits, index
        );
    }
    let _ = write!(
        xml,
        "<reg name=\"pc\" bitsize=\"32\" regnum=\"{}\" type=\"code_ptr\"/></feature></target>",
        snapshot.registers.len()
    );
    xml
}

fn encode_registers(snapshot: &CpuSnapshot) -> String {
    let mut bytes = Vec::new();
    for register in &snapshot.registers {
        let width = usize::from(register.bits.div_ceil(8));
        bytes.extend_from_slice(&register.value.to_le_bytes()[..width.min(8)]);
    }
    bytes.extend_from_slice(
        &u32::try_from(snapshot.pc)
            .expect("32-bit target program counter")
            .to_le_bytes(),
    );
    hex::encode(bytes)
}

fn encode_register(snapshot: &CpuSnapshot, index: usize) -> Option<String> {
    if index == snapshot.registers.len() {
        return Some(hex::encode(
            u32::try_from(snapshot.pc)
                .expect("32-bit target program counter")
                .to_le_bytes(),
        ));
    }
    let register = snapshot.registers.get(index)?;
    let width = usize::from(register.bits.div_ceil(8));
    Some(hex::encode(&register.value.to_le_bytes()[..width.min(8)]))
}

fn parse_pair(value: &str) -> Option<(u64, u64)> {
    let (left, right) = value.split_once(',')?;
    Some((
        u64::from_str_radix(left, 16).ok()?,
        u64::from_str_radix(right, 16).ok()?,
    ))
}

fn read_packet(stream: &mut TcpStream, no_ack: bool) -> Result<Option<String>, GdbError> {
    let mut byte = [0_u8; 1];
    loop {
        match stream.read_exact(&mut byte) {
            Ok(()) if byte[0] == b'$' => break,
            Ok(()) if byte[0] == 3 => return Ok(Some("?".to_owned())),
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(error) => return Err(error.into()),
        }
    }
    let mut payload = Vec::new();
    loop {
        stream.read_exact(&mut byte)?;
        if byte[0] == b'#' {
            break;
        }
        payload.push(byte[0]);
    }
    let mut checksum = [0_u8; 2];
    stream.read_exact(&mut checksum)?;
    let expected = u8::from_str_radix(
        std::str::from_utf8(&checksum).map_err(|_| GdbError::Hex("checksum".to_owned()))?,
        16,
    )
    .map_err(|_| GdbError::Hex("checksum".to_owned()))?;
    let actual = payload
        .iter()
        .fold(0_u8, |sum, byte| sum.wrapping_add(*byte));
    if !no_ack {
        stream.write_all(if actual == expected { b"+" } else { b"-" })?;
    }
    if actual != expected {
        return Ok(Some(String::new()));
    }
    Ok(Some(String::from_utf8_lossy(&payload).into_owned()))
}

fn write_packet(stream: &mut TcpStream, payload: &str) -> Result<(), GdbError> {
    let checksum = payload
        .as_bytes()
        .iter()
        .fold(0_u8, |sum, byte| sum.wrapping_add(*byte));
    write!(stream, "${payload}#{checksum:02x}")?;
    stream.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use renvo_core::RegisterValue;

    #[test]
    fn register_packet_is_little_endian_and_includes_pc() {
        let snapshot = CpuSnapshot {
            architecture: renvo_core::Architecture::RiscV32,
            pc: 0x1234_5678,
            registers: vec![RegisterValue {
                name: "x0".to_owned(),
                value: 0x1122_3344,
                bits: 32,
            }],
            waiting: false,
            halted: false,
        };
        assert_eq!(encode_registers(&snapshot), "4433221178563412");
    }

    #[test]
    fn pair_parser_uses_rsp_hexadecimal() {
        assert_eq!(parse_pair("1000,20"), Some((0x1000, 0x20)));
    }
}
