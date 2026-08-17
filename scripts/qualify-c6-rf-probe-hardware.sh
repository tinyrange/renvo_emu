#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"
. scripts/lib/toolchain-images.sh

port=${1:-}
if [ -z "$port" ] || [ ! -c "$port" ]; then
    echo "usage: $0 /dev/ttyACM0" >&2
    echo "a real ESP32-C6 serial device is required" >&2
    exit 2
fi
artifact_root=${REMU_C6_RF_PROBE_ROOT:-.remu/qualification/c6-rf-probe}
image=$artifact_root/c6-rf-probe-flash.bin
if [ ! -s "$image" ]; then scripts/qualify-c6-rf-probe.sh >/dev/null; fi
package_image=$(resolve_toolchain_image sha256:5aa633e02afc7f2657a5ddc76bdd1ba0f720545e8d6b8690eeca045c29496e09 remu/nanoc6-esptool:5.3.0)
image_root=$(CDPATH= cd -- "$artifact_root" && pwd)
docker run --rm --network=none --pull=never --device "$port:$port" \
    --entrypoint /opt/esptool-5.3/bin/esptool --volume "$image_root:/work:ro" \
    "$package_image" --chip esp32c6 --port "$port" flash-id
docker run --rm --network=none --pull=never --device "$port:$port" \
    --entrypoint /opt/esptool-5.3/bin/esptool --volume "$image_root:/work:ro" \
    "$package_image" --chip esp32c6 --port "$port" write-flash 0 /work/c6-rf-probe-flash.bin
python3 scripts/capture-c6-rf-probe-hardware.py --port "$port" --image "$image" \
    --output "$artifact_root/hardware.json"
jq . "$artifact_root/hardware.json"
