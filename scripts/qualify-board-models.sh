#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"
artifact_root=${REMU_BOARD_ARTIFACT_ROOT:-.remu/qualification/board-models}
mkdir -p "$artifact_root"

cargo build -p remu-cli --quiet
target/debug/remu board \
    --file qualification/m5stack-nanoc6-sgp30.star \
    --load-root . \
    --artifact "$artifact_root/result.json" \
    --vcd "$artifact_root/signals.vcd"

jq -e '
    .schema == "remu.board-simulation.v1" and
    .board == "m5stack_nanoc6" and
    .target == "esp32c6" and
    .result == "pass" and
    (.connectors == [{
        "name": "grove",
        "protocol": "i2c",
        "data_pin": 2,
        "clock_pin": 1,
        "voltage_mv": 5000
    }]) and
    (any(.events[]; .kind == "i2c" and .write == [54,130] and (.read | length) == 9)) and
    (any(.events[]; .kind == "i2c" and .write == [32,47] and .read == [0,34,101])) and
    (any(.events[]; .kind == "i2c" and .write == [32,8] and .read == [4,176,189,0,180,250])) and
    (any(.events[]; .kind == "button" and .component == "button")) and
    (any(.events[]; .kind == "led" and .component == "blue_led" and .on)) and
    (any(.events[]; .kind == "ws2812" and .colors == [{"red":255,"green":0,"blue":0}])) and
    (any(.components[]; .kind == "sgp30" and .name == "air_quality" and
        .state.initialized and .state.eco2 == 1200 and .state.tvoc == 180)) and
    (any(.components[]; .kind == "push-button" and .name == "button" and (.pressed | not))) and
    (any(.components[]; .kind == "led" and .name == "blue_led" and .state.on)) and
    (any(.components[]; .kind == "ws2812" and .name == "rgb" and .frames == 1))
' "$artifact_root/result.json" >/dev/null

grep -q '\$scope module m5stack_nanoc6' "$artifact_root/signals.vcd"
grep -q '\$scope module grove' "$artifact_root/signals.vcd"
grep -q '\$scope module air_quality' "$artifact_root/signals.vcd"

target/debug/remu board \
    --file qualification/m5stack-nanoc6-sgp30.star \
    --load-root . \
    --artifact "$artifact_root/replay.json" \
    --vcd "$artifact_root/replay.vcd" >/dev/null
cmp "$artifact_root/result.json" "$artifact_root/replay.json"
cmp "$artifact_root/signals.vcd" "$artifact_root/replay.vcd"

target/debug/remu board \
    --file qualification/m5sticks3-topology.star \
    --load-root . \
    --artifact "$artifact_root/m5sticks3.json" \
    --vcd "$artifact_root/m5sticks3.vcd"

jq -e '
    .schema == "remu.board-simulation.v1" and
    .board == "m5sticks3" and
    .target == "esp32s3" and
    .result == "pass" and
    (any(.connectors[]; .name == "lcd_spi3" and .data_pin == 39 and .clock_pin == 40)) and
    (any(.connectors[]; .name == "imu_i2c1" and .data_pin == 47 and .clock_pin == 48)) and
    (any(.connectors[]; .name == "audio_control_i2c1")) and
    (any(.connectors[]; .name == "infrared" and .data_pin == 46 and .clock_pin == 42)) and
    (any(.connectors[]; .name == "grove_port_a" and .data_pin == 9 and .clock_pin == 10)) and
    (any(.connectors[]; .name == "hat2_primary")) and
    (.mounts | map({name:.component.name, pin:.pin}) | sort_by(.name)) == [
        {"name":"button_a","pin":11},
        {"name":"button_b","pin":12},
        {"name":"lcd_backlight","pin":38}
    ]
' "$artifact_root/m5sticks3.json" >/dev/null

grep -q '\$scope module m5sticks3' "$artifact_root/m5sticks3.vcd"

target/debug/remu board \
    --file qualification/m5sticks3-components.star \
    --load-root . \
    --artifact "$artifact_root/m5sticks3-components.json" \
    --vcd "$artifact_root/m5sticks3-components.vcd"

jq -e '
    .schema == "remu.board-simulation.v1" and
    .board == "m5sticks3" and
    .target == "esp32s3" and
    .result == "pass" and
    (any(.events[]; .kind == "i2c" and .address == 110 and .read == [1,1,1,65])) and
    (any(.events[]; .kind == "spi" and .data == [248,0,7,224] and .data_phase)) and
    (any(.events[]; .kind == "button" and .component == "button_a")) and
    (any(.events[]; .kind == "button" and .component == "button_b")) and
    (any(.components[]; .kind == "st7789" and .name == "lcd" and
        .state.display_on and .state.frame_hash == 14056900767137360904)) and
    (any(.components[]; .kind == "m5-pm1" and .name == "m5pm1" and
        .state.transactions == 4 and .state.power_config == 15 and
        .state.l3b_powered and .state.speaker_amp_enabled and
        .state.battery_mv == 3950 and .state.output_5v_mv == 5000)) and
    (any(.components[]; .kind == "bmi270" and .name == "bmi270" and
        .state.initialized and .state.accelerometer_enabled and .state.gyroscope_enabled and
        .state.accel == [0,0,16384])) and
    (any(.components[]; .kind == "es8311" and .name == "es8311" and
        .state.powered and .state.adc_enabled and .state.dac_enabled and
        .state.adc_volume == 255 and .state.dac_volume == 191)) and
    (any(.components[]; .kind == "led" and .name == "lcd_backlight" and .state.on))
' "$artifact_root/m5sticks3-components.json" >/dev/null

mkdir -p "$artifact_root/m5sticks3-live-out"
target/debug/remu corpus build \
    --toolchain toolchains/xtensa-esp-gcc-esp32s3.toml \
    --source corpus/smoke/xtensa-m5sticks3 \
    --output "$artifact_root/m5sticks3-live-out" \
    --target esp32s3 \
    --artifact "$artifact_root/m5sticks3-live-build.json" \
    -- -O2 main.c -Wl,-T,link.ld -o /workspace/out/probe.elf

target/debug/remu board \
    --file qualification/m5sticks3-live.star \
    --load-root . \
    --elf "$artifact_root/m5sticks3-live-out/probe.elf" \
    --max-instructions 10000 \
    --artifact "$artifact_root/m5sticks3-live.json" \
    --vcd "$artifact_root/m5sticks3-live.vcd"

jq -e '
    .schema == "remu.m5sticks3-firmware-board.v1" and
    .board == "m5sticks3" and
    .result == "pass" and
    .run.reason == "Halted" and
    .run.exit_code == 0 and
    .components.display.panel.display_on and
    .components.display_active and
    .components.display.panel.inverted and
    .components.display.panel.frame_hash == 14056900767137360904 and
    .components.display.backlight_on and
    (.components.display.reset_asserted | not) and
    .components.display.transfers == 8 and
    (.components.power.transactions >= 4) and
    .components.power.power_config == 15 and
    .components.power.l3b_powered and
    .components.power.speaker_amp_enabled and
    .components.power.battery_mv == 3950 and
    .components.imu.initialized and
    .components.imu.accelerometer_enabled and
    .components.imu.gyroscope_enabled and
    .components.imu.accel == [100,-200,16000] and
    .components.audio.codec.powered and
    .components.audio.codec.adc_enabled and
    .components.audio.codec.dac_enabled and
    .components.audio.speaker_frames == 1 and
    .components.audio.speaker_last_sample == 2774143540 and
    .components.audio.microphone_frames == 1 and
    .components.audio.microphone_last_sample == 305419896 and
    .components.infrared.transmitter.frames == 1 and
    (.components.infrared.transmitter.last_items | length) == 2 and
    .components.infrared.receiver.frames == 1 and
    (.components.infrared.receiver.last_items | length) == 2 and
    .components.infrared.receiver_high and
    .components.expansion.grove_powered and
    .components.button_a_pressed and
    (.components.button_b_pressed | not)
' "$artifact_root/m5sticks3-live.json" >/dev/null

grep -q '\$scope module lcd' "$artifact_root/m5sticks3-live.vcd"
grep -q '\$scope module m5pm1' "$artifact_root/m5sticks3-live.vcd"
grep -q '\$scope module bmi270' "$artifact_root/m5sticks3-live.vcd"
grep -q '\$scope module es8311' "$artifact_root/m5sticks3-live.vcd"
grep -q '\$scope module spi3' "$artifact_root/m5sticks3-live.vcd"
grep -q '\$scope module i2c1' "$artifact_root/m5sticks3-live.vcd"

target/debug/remu board \
    --file qualification/m5sticks3-live.star \
    --load-root . \
    --elf "$artifact_root/m5sticks3-live-out/probe.elf" \
    --max-instructions 10000 \
    --artifact "$artifact_root/m5sticks3-live-replay.json" \
    --vcd "$artifact_root/m5sticks3-live-replay.vcd" >/dev/null
cmp "$artifact_root/m5sticks3-live.json" "$artifact_root/m5sticks3-live-replay.json"
cmp "$artifact_root/m5sticks3-live.vcd" "$artifact_root/m5sticks3-live-replay.vcd"

echo "board model qualification passed; artifact: $artifact_root/result.json"
