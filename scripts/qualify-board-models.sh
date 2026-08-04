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
    (.connectors == [
        {"name":"lcd_spi3","protocol":"spi","data_pin":39,"clock_pin":40,"voltage_mv":3300},
        {"name":"lcd_control","protocol":"digital","data_pin":45,"clock_pin":41,"voltage_mv":3300},
        {"name":"lcd_reset","protocol":"digital","data_pin":21,"clock_pin":21,"voltage_mv":3300},
        {"name":"m5pm1_i2c1","protocol":"i2c","data_pin":47,"clock_pin":48,"voltage_mv":3300}
    ]) and
    (.mounts | map({name:.component.name, pin:.pin}) | sort_by(.name)) == [
        {"name":"button_a","pin":11},
        {"name":"button_b","pin":12},
        {"name":"lcd_backlight","pin":38}
    ]
' "$artifact_root/m5sticks3.json" >/dev/null

grep -q '\$scope module m5sticks3' "$artifact_root/m5sticks3.vcd"

echo "board model qualification passed; artifact: $artifact_root/result.json"
