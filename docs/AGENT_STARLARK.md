# Agent-driven Starlark sessions

Renvo can keep one ESP32-C6 or ESP32-S3 machine alive while a Starlark script
drives a sequence of bounded experiments. This is the control-plane pattern
used for iterative debugging: the script contains repeatable setup and
assertions, while `repl()` can pause inside the same scope for exploration.

The Rust machine remains the only owner of CPU execution, virtual time, MMIO,
DMA, interrupts, radio scheduling, coexistence, and legal-state validation.
The driver does not expose symbol hooks and cannot replace LLE behavior with a
Starlark callback. ESP radio/native sessions still require the matching real
mask-ROM image.

## Running a driver

Every driver defines `main()` and must call `machine.run()` at least once:

```starlark
def main():
    print(machine.target(), machine.cpu())
    first = machine.run(instructions = 100000)
    if first["reason"] == "InstructionLimit":
        second = machine.run(instructions = 100000)
        assert_true(second["stats"]["time"] >= first["stats"]["time"])
    return machine.radio_events(cursor = 0, limit = 128)
```

Run it through the normal firmware-loading boundary:

```sh
remu run \
  --target esp32c6 \
  --elf firmware.elf \
  --boot-rom esp32c6_rev0_rom.elf \
  --agent-script drive.star \
  --agent-artifact agent-session.json \
  --radio-replay radio.json \
  --result result.json
```

Add `--agent-repl` and call `repl()` in `main()` to open Starlark's scoped
terminal console. The `machine` value and the function's local variables remain
available. Leaving the REPL resumes the script.

Agent scripts may load reusable `.star` modules below the invocation workspace.
Workspace labels use `//package:file.star`; parent-directory components,
non-`.star` files, and symlink escapes are rejected. The checked helper module
at `qualification/starlark/agent_automation.star` provides bounded
`run_until()`, `wait_for_radio()`, and paginated `drain_radio()` workflows.
It also provides `wait_for_bus()` and `drain_bus()` for a configured bounded
bus capture. They advance only through `machine.run()` and observe native
machine evidence.

Radio investigations can layer checked, peripheral-specific interpreters over
that generic capture. `qualification/starlark/wifi_crypto_evidence.star`
selects the pinned C6/S3 native key-table windows, summarizes valid-bit,
control, and key-payload writes, and rejects control classes that neither
pinned vendor HAL can emit. It observes firmware bus traffic only; key
installation and frame protection remain native emulated hardware behavior.

For an iterative MMIO experiment, narrow the capture before running firmware:

```starlark
machine.capture_bus(
    start = 0x600a4000,
    end = 0x600a7000,
    regions = ["esp32c6.wifi-mac-registers"],
    kinds = ["read", "write"],
    capacity = 8192,
)
result = machine.run(instructions = 100000)
page = machine.bus_events(cursor = 0, limit = 256)
repl()
```

The capture is a ring with an explicit capacity rather than an unbounded log.
Every page reports retained and dropped counts plus cursor loss. The checked
`drain_bus()` helper fails on loss so a qualification script cannot silently
accept incomplete hardware evidence. `stop_bus_capture()` freezes the retained
window while it is inspected. Existing CLI `--bus-log` streaming continues in
parallel when both facilities are enabled.

`--agent-artifact` writes schema `remu.agent-session.v1`: the JSON-compatible
value returned by `main()` plus a compact final-run summary. The full run and
RF event stream remain separate `--result` and `--radio-replay` artifacts, so
long UART/RF histories are not duplicated into the agent artifact.

## Machine methods

| Method | Contract |
| --- | --- |
| `target()` | Returns `esp32c6` or `esp32s3`. |
| `run(instructions=1000000, deadline=0)` | Resumes execution. At least one bound must be nonzero; one call is capped at 100 million instructions. `deadline` is an absolute simulation tick. |
| `cpu()` | Returns the architecture-neutral CPU snapshot. |
| `read(address, length)` | Reads up to 1 MiB through the debugger bus boundary. |
| `write(address, bytes)` | Writes up to 1 MiB through the debugger bus boundary. |
| `breakpoint(address)` | Stops before executing the address. |
| `watchpoint(address)` | Stops after a data access overlaps the address. |
| `clear_stops()` | Removes agent-installed breakpoint and watchpoint stops. |
| `pin(pin, value)` | Immediately drives `0`, `1`, `z`, or `x` at the current tick. |
| `usb_input(bytes)` | Queues up to 1 MiB for the emulated USB serial host. |
| `inject_radio(...)` | Schedules an explicit Wi-Fi, BLE, or 802.15.4 frame in the isolated deterministic medium. |
| `radio_events(cursor=0, limit=256)` | Returns at most 4096 append-only RF events and a continuation cursor. |
| `coexistence_events(cursor=0, limit=256)` | Returns at most 4096 coexistence grant, denial, preemption, and release events. |
| `capture_bus(start=0, end=None, regions=None, kinds=None, capacity=4096)` | Clears and starts a bounded filtered bus ring. Kinds default to reads and writes; the hard capacity ceiling is 65,536 records. |
| `stop_bus_capture()` | Stops adding records while retaining the current ring. |
| `bus_events(cursor=0, limit=256)` | Pages at most 4096 retained accesses with stable cursors and explicit drop/loss accounting. |

The Starlark evaluator is limited to a 64 MiB heap, ten million stable
evaluation ticks, and a 256-frame call stack. Native machine work is separately
bounded by each `run()` call. Paginated radio reads prevent a long vendor run
from copying its complete event history into the script heap. The supplied
automation helper additionally caps each RF drain at 4,096 events by default
and fails instead of silently accumulating an unbounded history. The main
source is capped at 8 MiB; its transitive load graph is capped at 256 modules
and 16 MiB of source, with the same heap, tick, and call-stack limits applied
while each loaded module initializes.

## Capability separation

There are intentionally two radio-facing script modes:

- An agent driver can inspect and operate its explicitly supplied machine. It
  is the right place for boot sequencing, bounded runs, breakpoints, register
  experiments, and evidence assertions.
- A radio peer's `on_event(event, state)` callback only receives immutable RF
  events and can return future peer frames. It cannot access the driver,
  machine, memory, registers, symbols, files, clocks, or host networks.
  Checked `wifi_ccmp_protect()`/`wifi_ccmp_unprotect()` helpers let that peer
  exchange preformatted native CCMP frames with firmware; they do not select
  guest key slots or replace the emulated MAC crypto datapath.

That separation lets an agent control the experiment without accidentally
turning a virtual peer into a hidden high-level implementation of the guest's
radio peripheral.
