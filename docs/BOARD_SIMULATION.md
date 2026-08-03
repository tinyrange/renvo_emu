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
  baseline, self-test, raw measurement, humidity and Sensirion CRC-8 behavior;
- ST7789 displays with command/data phases, address windows, RGB565 framebuffer,
  reset, inversion, sleep/display state and deterministic frame hashes;
- M5PM1 power-management companions with identity, rails, telemetry, GPIO,
  active-low host IRQ, ADC, timer, button, NeoPixel and retained-RAM behavior;
- BMI270 inertial sensors with configuration upload, power modes, status and
  deterministic accelerometer, gyroscope and temperature samples; and
- ES8311 audio codecs with the control and power sequences used by M5Unified.

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

The M5StickS3 topology is available as
[`boards/m5sticks3.star`](../boards/m5sticks3.star). It records the LCD SPI3
signals (MOSI GPIO39, SCLK GPIO40, CS GPIO41, DC GPIO45, reset GPIO21 and
backlight GPIO38); internal I2C1 on SDA GPIO47/SCL GPIO48 with the M5PM1 at
`0x6e`, BMI270 at `0x68`, and ES8311 at `0x18`; audio MCLK GPIO18, DOUT GPIO14,
BCLK GPIO17, LRCK GPIO15 and DIN GPIO16; active-low buttons on GPIO11/GPIO12;
IR TX/RX on GPIO46/GPIO42; Grove SDA/SCL on GPIO9/GPIO10; and the published
Hat2 expansion pins. M5PM1 GPIO1 drives the active-low host interrupt on
ESP32-S3 GPIO13, GPIO2 controls the LCD/audio L3B domain, GPIO3 controls the
speaker amplifier, and GPIO4 receives the BMI270 interrupt.

The standalone scenario in
[`qualification/m5sticks3-components.star`](../qualification/m5sticks3-components.star)
qualifies the component protocols. The live scenario in
[`qualification/m5sticks3-live.star`](../qualification/m5sticks3-live.star)
attaches the same topology to the Xtensa machine. Firmware accesses the native
ESP32-S3 SPI3, I2C1, I2S0/I2S1, RMT and GPIO registers; the resulting JSON
contains the LCD framebuffer, power/IRQ state, IMU sample, codec and audio
frames, IR pulses, buttons, Grove, and Hat2 state. Starlark still describes only
immutable topology and bounded external actions—it never owns CPU or scheduler
state.

Run a scenario and produce JSON plus VCD with:

```sh
cargo run -p remu-cli -- board \
  --file qualification/m5stack-nanoc6-sgp30.star \
  --load-root . \
  --artifact .remu/board/result.json \
  --vcd .remu/board/signals.vcd
```

Run a vendor-toolchain-built M5StickS3 probe against the live board with:

```sh
cargo run -p remu-cli -- board \
  --file qualification/m5sticks3-live.star \
  --load-root . \
  --elf build/m5sticks3/probe.elf \
  --max-instructions 10000 \
  --artifact build/m5sticks3/result.json \
  --vcd build/m5sticks3/signals.vcd
```

`load()` labels are confined beneath `--load-root`; absolute paths, parent
traversal and cyclic imports are rejected. The script's final expression must
be a board instance.

The generic board runner and ESP32-C6 NanoC6 scenario remain a
protocol/component qualification layer; live ESP32-C6 firmware MMIO is not yet
routed into that graph. M5StickS3 is the first live board attachment and does
execute firmware drivers against its assembled non-radio peripherals. The
models are functional and deterministic, not cycle-accurate electrical,
analogue, acoustic or optical simulations.

The Rust `BoardGpioEndpoint` adds a typed machine boundary for GPIO. A scheduler
can attach mounted buttons and LEDs to an existing machine `SignalHub`, turn
button actions into `PinStimulus` values, and poll resolved MCU GPIO levels into
`board.<name>.component.<component>.pin/state` signals. It deliberately rejects
external protocol connections and WS2812 waveform mounts until their typed bus
endpoints are implemented. This GPIO slice does not claim that ESP32-C6
firmware drivers communicate with the NanoC6 SGP30; that requires the separate
I2C endpoint.

The endpoint keeps all CPU, scheduler, peripheral, and electrical state in
Rust. Starlark continues to describe topology and bounded actions only.

Each mounted GPIO component must claim a unique primary MCU pin. Attaching two
components to the same pin fails with an explicit `GpioPinConflict` error rather
than silently creating an electrical contention that this first endpoint slice
cannot resolve.

A direct machine run uses the shared hub and polls the endpoint at the same
deterministic boundaries as execution:

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

The typed `BoardI2cEndpoint` resolves an I2C connector against the same machine
hub, creates `board.<name>.connector.<connector>.data/clock` signals, and routes
host-side transfers through supported external models such as the SGP30. This
makes Starlark's `connect("grove", sensor)` topology reusable at a machine signal
boundary while keeping the implementation in Rust.

The I2C endpoint intentionally remains a host-transfer slice: it emits the bus
waveform and returns the model response, but it does not yet observe a firmware
I2C controller's MMIO transaction or inject device ACK/data bits into that
controller. Data/clock pin aliases are rejected, transfers must start at or
after the previous transfer's stop time, and waveform timestamp overflow is
reported instead of being saturated. Waveform timing is preflighted before
signals or the target model are changed, so a rejected transfer is
transactional. The generic board runner applies the same checked timing
boundary. These interfaces therefore do not claim that an ESP32-C6 firmware
driver talks to the SGP30.

An external protocol transfer produces connector-level VCD changes:

```rust
let mut i2c = BoardI2cEndpoint::new(
    &scenario,
    machine.signal_hub(),
    "board.esp32c6.chip_gpio",
)?;
let transfer = i2c.transfer("grove", 0x58, &[0x20, 0x03], 0, SimTime::ZERO)?;
assert!(transfer.completed_at > transfer.at);
```

Machine models also expose serial transport through the typed
`UartEndpointProvider` API. A runner can discover the `Compiler` capture UART
and the target's `Native` UART, then pass the selected `UartHandle` to a board
transport. Endpoint roles are named rather than address- or index-based, and
the returned handles share the machine's deterministic transmit state. Repeated
discovery returns handles for the same role, while the compiler and native
roles remain independent so a board connector cannot silently capture the
other stream. This first slice covers the RISC-V, Arm RP, and Xtensa machines
whose native UARTs use the common functional handle; vendor-specific peripheral
handles remain target-specific until their receive/transmit adapters are added.

Run the deterministic qualification, including byte-identical JSON and VCD
replay, with:

```sh
scripts/qualify-board-models.sh
```

Hardware and protocol references:

- [M5Stack NanoC6 pin map](https://docs.m5stack.com/en/core/M5NanoC6)
- [M5Stack NanoC6 RGB control guide](https://docs.m5stack.com/en/arduino/m5nanoc6/program)
- [M5Stack StickS3 pin map and specifications](https://docs.m5stack.com/en/core/StickS3)
- [M5Stack M5Unified board drivers](https://github.com/m5stack/M5Unified)
- [M5Stack StickS3 infrared API](https://docs.m5stack.com/en/arduino/m5sticks3/ir_nec)
- [M5Stack StickS3 M5PM1 API](https://docs.m5stack.com/en/arduino/m5sticks3/m5pm1)
- [Sensirion SGP30 datasheet](https://sensirion.com/media/documents/984E0DD5/61644B8B/Sensirion_Gas_Sensors_Datasheet_SGP30.pdf)
