#!/bin/sh
set -eu

dist=${1:?usage: verify-release-quickstart.sh DIST RELEASE_REF}
release_ref=${2:?usage: verify-release-quickstart.sh DIST RELEASE_REF}
release_version=${release_ref#v}
quickstart=$dist/quickstart
archive=$dist/remu-quickstart-$release_ref.tar.gz
asset_checksums=$dist/remu-$release_ref-sha256sums.txt

test -x "$dist/amd64/remu"
test -x "$dist/arm64/remu"
test "$("$dist/amd64/remu" --version)" = "remu $release_version"

test -s "$quickstart/build/quickstart.elf"
test -s "$quickstart/build.json"
test -s "$quickstart/run.json"
test -s "$quickstart/run.vcd"
test -s "$quickstart/SHA256SUMS"

jq -e '
    .schema == "remu.build-artifact.v1" and
    .target == "ch32v003" and
    .exit_code == 0 and
    .timed_out == false and
    (.outputs | any(.path == "quickstart.elf"))
' "$quickstart/build.json" >/dev/null
jq -e '
    .target == "ch32v003" and
    .reason == "Halted" and
    .exit_code == 0
' "$quickstart/run.json" >/dev/null
grep -Fq '$scope module gpio $end' "$quickstart/run.vcd"

(cd "$quickstart" && sha256sum --check SHA256SUMS)

test -s "$archive"
archive_listing=$(tar -tzf "$archive")
for member in \
    quickstart/README.md \
    quickstart/link.ld \
    quickstart/main.c \
    quickstart/start.S \
    quickstart/build/quickstart.elf \
    quickstart/build.json \
    quickstart/run.json \
    quickstart/run.vcd \
    quickstart/SHA256SUMS
do
    printf '%s\n' "$archive_listing" | grep -Fqx "$member"
done

test "$(wc -l < "$asset_checksums")" -eq 3
seen_amd64=0
seen_arm64=0
seen_archive=0
while read -r expected name extra
do
    test -n "$expected"
    test -n "$name"
    test -z "$extra"
    case "$name" in
        remu-$release_ref-linux-amd64)
            test "$seen_amd64" -eq 0
            seen_amd64=1
            path=$dist/amd64/remu
            ;;
        remu-$release_ref-linux-arm64)
            test "$seen_arm64" -eq 0
            seen_arm64=1
            path=$dist/arm64/remu
            ;;
        remu-quickstart-$release_ref.tar.gz)
            test "$seen_archive" -eq 0
            seen_archive=1
            path=$archive
            ;;
        *)
            echo "unexpected release checksum entry: $name" >&2
            exit 1
            ;;
    esac
    test "$expected" = "$(sha256sum "$path" | awk '{print $1}')"
done < "$asset_checksums"
test "$seen_amd64" -eq 1
test "$seen_arm64" -eq 1
test "$seen_archive" -eq 1

echo "release quick-start artifacts verified for $release_ref"
