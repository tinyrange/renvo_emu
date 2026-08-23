#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"

artifact_root=${REMU_RADIO_POWER_TRANSITION_ROOT:-.remu/qualification/radio-power-transitions}
mkdir -p "$artifact_root"

cargo test -p remu-devices \
    stress_replays_sleep_wake_and_reset_cancellation_without_stale_edges --locked
cargo test -p remu-devices \
    c6_ble_sleep_timer_stress_cancels_or_completes_each_wake_exactly_once --locked
cargo test -p remu-machines \
    esp32c6_active_wifi_power_and_reset_stress_has_exact_replay_accounting --locked
cargo test -p remu-machines \
    esp32s3_active_wifi_clock_and_reset_stress_has_exact_replay_accounting --locked
cargo test -p remu-machines \
    esp32c6_ieee802154_stop_clock_gate_wake_and_rearm_replays_exactly --locked
cargo test -p remu-machines \
    esp32c6_ieee802154_clock_gate_during_receive_is_a_hard_machine_error --locked

REMU_RADIO_ROM_ROOT="$artifact_root/rom" scripts/qualify-esp-radio-rom.sh esp32c6
REMU_RADIO_ROM_ROOT="$artifact_root/rom" scripts/qualify-esp-radio-rom.sh esp32s3
REMU_RADIO_BLE_VENDOR_ROOT="$artifact_root/ble" scripts/qualify-esp-radio-ble-vendor.sh esp32c6
REMU_RADIO_BLE_VENDOR_ROOT="$artifact_root/ble" scripts/qualify-esp-radio-ble-vendor.sh esp32s3
REMU_RADIO_COEX_ROOT="$artifact_root/coexistence" scripts/qualify-esp-radio-coexistence-vendor.sh esp32c6
REMU_RADIO_COEX_ROOT="$artifact_root/coexistence" scripts/qualify-esp-radio-coexistence-vendor.sh esp32s3
REMU_RADIO_OPENTHREAD_VENDOR_ROOT="$artifact_root/openthread" scripts/qualify-esp-radio-openthread-vendor.sh esp32c6

scripts/check-radio-power-transition-contract.py \
    --artifacts "$artifact_root" \
    --output "$artifact_root/summary.json"

echo "software-only ESP radio power-transition qualification passed: $artifact_root/summary.json"
