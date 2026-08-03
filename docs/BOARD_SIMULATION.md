# Starlark board and device simulation

Renvo Emulator can describe a physical board in one Starlark module and load it from
many independent test scenarios. The Rust simulation kernel owns component
behavior, deterministic time, signal resolution, protocol transactions, and
VCD generation; Starlark only assembles topology and queues bounded actions.

The initial reusable models are:

- active-high or active-low momentary buttons with deterministic contact bounce;
- active-high or active-low digital LEDs with accumulated on-time;
- WS2812 RGB chains with GRB waveform decoding and reset-latch timing;
- SGP30 at I2C address `0x58`, including identity, feature, IAQ init/measure,
  baseline, self-test, raw measurement, humidity and Sensirion CRC-8 behavior.

Machine-facing UART connectors can use `BoardUartEndpoint` with a target's
`UartHandle`. The endpoint queues host RX bytes, polls newly transmitted guest
bytes, and exposes byte/strobe signals for VCD and protocol assertions:

```rust,no_run
let mut uart = BoardUartEndpoint::new("teaching", &console, uart_handle, hub)?;
uart.host_write(b"ping\\n", SimTime::from_ticks(10))?;
let tx = uart.poll_tx(SimTime::from_ticks(20))?;
```

This endpoint is deliberately transport-oriented. It does not invent baud
rate timing or a UART electrical model; the target's existing functional UART
device remains responsible for register semantics. Host writes and TX polls
must use nondecreasing simulation timestamps, so the byte/strobe trace cannot
silently move backwards in time. UART connectors with aliased TX/RX pins are
rejected during endpoint construction.

The M5Stack NanoC6 definition is
[`boards/m5stack_nanoc6.star`](../boards/m5stack_nanoc6.star). It contains the
published board topology: button GPIO9, blue LED GPIO7, WS2812 GPIO20 with
GPIO19 enable, and the Grove connector on GPIO2/GPIO1. A test does not repeat
that wiring:

```python
load("//boards:m5stack_nanoc6.star", "m5stack_nanoc6")

sensor = sgp30(name = "air_quality")
board = m5stack_nanoc6()
board.connect("grove", sensor)
board.i2c_write_read("grove", 0x58, [0x20, 0x03])
board.run_for(seconds(15))
board.i2c_write_read("grove", 0x58, [0x20, 0x08], read_len = 6)
board
```

`board.connect()` resolves the connector's MCU pins, protocol, and supply
metadata. It rejects an unknown connector, incompatible protocol, or a second
device on an occupied connector. Low-level direct wiring remains an internal
extension point rather than the normal board API.

Run a scenario and produce JSON plus VCD with:

```sh
cargo run -p remu-cli -- board \
  --file qualification/m5stack-nanoc6-sgp30.star \
  --load-root . \
  --artifact .remu/board/result.json \
  --vcd .remu/board/signals.vcd
```

`load()` labels are confined beneath `--load-root`; absolute paths, parent
traversal and cyclic imports are rejected. The script's final expression must
be a board instance.

The current board runner is a protocol/component qualification layer. It does
not yet route firmware MMIO activity into the assembled board, so its results
prove topology, device protocols, deterministic external behavior and
waveforms—not execution of a firmware driver against the SGP30. The typed UART
endpoint above is the first host transport bridge; machine-specific UART
accessors and complete connector-to-machine assembly can be added at the
machine pin/bus boundary without moving CPU or scheduler state into Starlark.

Run the deterministic qualification, including byte-identical JSON and VCD
replay, with:

```sh
scripts/qualify-board-models.sh
```

Hardware and protocol references:

- [M5Stack NanoC6 pin map](https://docs.m5stack.com/en/core/M5NanoC6)
- [M5Stack NanoC6 RGB control guide](https://docs.m5stack.com/en/arduino/m5nanoc6/program)
- [Sensirion SGP30 datasheet](https://sensirion.com/media/documents/984E0DD5/61644B8B/Sensirion_Gas_Sensors_Datasheet_SGP30.pdf)
