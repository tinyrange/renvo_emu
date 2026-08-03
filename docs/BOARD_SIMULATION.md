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

The current `remu board` command is a protocol/component qualification layer.
The Rust `BoardGpioEndpoint` is the first machine-boundary slice: a scheduler
can attach mounted buttons and LEDs to an existing machine `SignalHub`, turn
button actions into `PinStimulus` values, and poll resolved MCU GPIO levels into
`board.<name>.component.<component>.pin/state` signals. It deliberately rejects
external protocol connections and WS2812 waveform mounts until their typed bus
endpoints are implemented. The command-line board runner remains standalone,
so these results still do not claim that an ESP32-C6 firmware driver talks to
the SGP30; that is the next I²C endpoint slice.

The endpoint keeps all CPU, scheduler, peripheral, and electrical state in Rust:
Starlark continues to describe topology and bounded actions only.

For a direct machine run, the machine exposes its shared hub and the caller
keeps endpoint polling at the same deterministic boundaries as execution:

```rust
let endpoint = BoardGpioEndpoint::new(
    &scenario,
    machine.signal_hub(),
    "board.esp32c6.chip_gpio",
)?;
let stimuli = endpoint.button_stimuli(&scenario.actions)?;
let result = machine.run_with_stimuli(limits, &stimuli, None)?;
endpoint.poll(result.stats.time)?;
```

Run the deterministic qualification, including byte-identical JSON and VCD
replay, with:

```sh
scripts/qualify-board-models.sh
```

Hardware and protocol references:

- [M5Stack NanoC6 pin map](https://docs.m5stack.com/en/core/M5NanoC6)
- [M5Stack NanoC6 RGB control guide](https://docs.m5stack.com/en/arduino/m5nanoc6/program)
- [Sensirion SGP30 datasheet](https://sensirion.com/media/documents/984E0DD5/61644B8B/Sensirion_Gas_Sensors_Datasheet_SGP30.pdf)
