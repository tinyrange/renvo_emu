#!/bin/sh
set -eu

release=20260528
archive=esp-rom-elfs-${release}.tar.gz
archive_sha256=caa463d3cbef2430a5a35847c1d9f2f152403b17a802050927ff60c8da54fe46
url=https://github.com/espressif/esp-rom-elfs/releases/download/${release}/${archive}
output_dir=${REMU_ESP_ROM_DIR:-.remu/qualification/esp-rom-elfs/${release}}
temporary_dir=$(mktemp -d)
trap 'rm -rf "$temporary_dir"' EXIT HUP INT TERM

command -v curl >/dev/null
command -v sha256sum >/dev/null
command -v tar >/dev/null
command -v readelf >/dev/null

curl --fail --location --silent --show-error \
    --output "$temporary_dir/$archive" "$url"
printf '%s  %s\n' "$archive_sha256" "$temporary_dir/$archive" | sha256sum -c -

mkdir -p "$output_dir"
tar -xzf "$temporary_dir/$archive" \
    --directory "$output_dir" \
    --no-same-owner \
    --no-same-permissions \
    esp32c6_rev0_rom.elf \
    esp32s3_rev0_rom.elf

for rom in esp32c6_rev0_rom.elf esp32s3_rev0_rom.elf; do
    readelf -h "$output_dir/$rom" >/dev/null
    sha256sum "$output_dir/$rom"
done
