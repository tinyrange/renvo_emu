#!/bin/sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
manifest=${REMU_FIRMWARE_MANIFEST:-"$project_dir/firmware/micropython-v1.28.0.toml"}
cache=${REMU_FIRMWARE_CACHE:-"$project_dir/.remu/firmware/micropython-v1.28.0"}
report=${REMU_FIRMWARE_REPORT:-"$cache/verified.json"}
image=remu/firmware-fetch:local
remu=${REMU_BIN:-"$project_dir/target/debug/remu"}

mkdir -p "$cache"

if [ ! -x "$remu" ]
then
    cargo=${CARGO:-cargo}
    "$cargo" build --quiet -p remu-cli
fi

# A restored Actions cache should not need Docker or network access. Verify the
# complete cache first, then only construct the downloader image when a file is
# absent or has the wrong digest.
if "$remu" firmware verify \
    --manifest "$manifest" \
    --cache "$cache" \
    --artifact "$report" >/dev/null 2>&1
then
    echo "Using verified MicroPython firmware cache: $cache"
    exit 0
fi

if ! docker image inspect "$image" >/dev/null 2>&1
then
    docker build --pull=false -t "$image" \
        "$project_dir/toolchains/firmware-fetch"
fi

awk -F '"' '
    /^url = / { url = $2 }
    /^filename = / { filename = $2 }
    /^sha256 = / { print url "\t" filename "\t" $2 }
' "$manifest" |
while IFS='	' read -r url filename expected_sha
do
    case "$filename" in
        ""|.*|*/*|*..*)
            echo "unsafe firmware filename: $filename" >&2
            exit 1
            ;;
    esac
    if [ -f "$cache/$filename" ] &&
        [ "$(sha256sum "$cache/$filename" | cut -d ' ' -f 1)" = "$expected_sha" ]
    then
        echo "Using cached firmware: $filename"
        continue
    fi

    part="$filename.part"
    rm -f "$cache/$part"
    docker run --rm \
        --network=bridge \
        --read-only \
        --cap-drop=ALL \
        --security-opt=no-new-privileges \
        --memory=256m \
        --pids-limit=64 \
        --cpus=1 \
        --user="$(id -u):$(id -g)" \
        --tmpfs=/tmp:rw,noexec,nosuid,size=16m \
        --mount "type=bind,src=$cache,dst=/cache" \
        "$image" \
        --fail --location --retry 3 --silent --show-error \
        --output "/cache/$part" "$url"
    actual_sha=$(sha256sum "$cache/$part" | cut -d ' ' -f 1)
    if [ "$actual_sha" != "$expected_sha" ]
    then
        echo "firmware digest mismatch for $filename: $actual_sha (expected $expected_sha)" >&2
        rm -f "$cache/$part"
        exit 1
    fi
    mv "$cache/$part" "$cache/$filename"
done

"$remu" firmware verify \
    --manifest "$manifest" \
    --cache "$cache" \
    --artifact "$report"
