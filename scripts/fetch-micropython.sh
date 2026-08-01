#!/bin/sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
manifest=${RENVO_FIRMWARE_MANIFEST:-"$project_dir/firmware/micropython-v1.28.0.toml"}
cache=${RENVO_FIRMWARE_CACHE:-"$project_dir/.renvo/firmware/micropython-v1.28.0"}
report=${RENVO_FIRMWARE_REPORT:-"$cache/verified.json"}
image=renvo/firmware-fetch:local

mkdir -p "$cache"

if ! docker image inspect "$image" >/dev/null 2>&1
then
    docker build --pull=false -t "$image" \
        "$project_dir/toolchains/firmware-fetch"
fi

awk -F '"' '
    /^url = / { url = $2 }
    /^filename = / { print url "\t" $2 }
' "$manifest" |
while IFS='	' read -r url filename
do
    case "$filename" in
        ""|.*|*/*|*..*)
            echo "unsafe firmware filename: $filename" >&2
            exit 1
            ;;
    esac
    part="$filename.part"
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
    mv "$cache/$part" "$cache/$filename"
done

cargo=${CARGO:-cargo}
"$cargo" build --quiet -p renvo-cli
"$project_dir/target/debug/renvo" firmware verify \
    --manifest "$manifest" \
    --cache "$cache" \
    --artifact "$report"
