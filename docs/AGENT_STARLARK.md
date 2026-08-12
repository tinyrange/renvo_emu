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
  --radio-replay radio.json \
  --result result.json
```

Add `--agent-repl` and call `repl()` in `main()` to open Starlark's scoped
terminal console. The `machine` value and the function's local variables remain
available. Leaving the REPL resumes the script.

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

The Starlark evaluator is limited to a 64 MiB heap, ten million stable
evaluation ticks, and a 256-frame call stack. Native machine work is separately
bounded by each `run()` call. Paginated radio reads prevent a long vendor run
from copying its complete event history into the script heap.

## Capability separation

There are intentionally two radio-facing script modes:

- An agent driver can inspect and operate its explicitly supplied machine. It
  is the right place for boot sequencing, bounded runs, breakpoints, register
  experiments, and evidence assertions.
- A radio peer's `on_event(event, state)` callback only receives immutable RF
  events and can return future peer frames. It cannot access the driver,
  machine, memory, registers, symbols, files, clocks, or host networks.

That separation lets an agent control the experiment without accidentally
turning a virtual peer into a hidden high-level implementation of the guest's
radio peripheral.
