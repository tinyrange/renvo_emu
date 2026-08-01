#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
image=remu/mquickjs:203d5bb79789bc47b74855d9207415dab71661a0
artifact_root=${MQUICKJS_ARTIFACT_ROOT:-"$repo_root/.remu/qualification/mquickjs"}
remu=${REMU_BIN:-"$repo_root/target/release/remu"}
workload=${MQUICKJS_WORKLOAD:-"$repo_root/qualification/mquickjs/workload.js"}

mkdir -p "$artifact_root"

if ! docker image inspect "$image" >/dev/null 2>&1
then
    if [ "${MQUICKJS_OFFLINE:-0}" = 1 ]
    then
        echo "Pinned MQuickJS image is not available in the offline cache: $image" >&2
        exit 1
    fi
    docker build --pull=false --tag "$image" "$repo_root/toolchains/mquickjs"
fi

if [ ! -x "$remu" ]
then
    "$HOME/.cargo/bin/cargo" build --release --package remu-cli
fi

image_id=$(docker image inspect --format '{{.Id}}' "$image")
source_hash=$(sha256sum \
    "$repo_root/qualification/mquickjs/harness.c" \
    "$repo_root/qualification/mquickjs/mini_libc.c" \
    "$workload" \
    | sha256sum | cut -d ' ' -f 1)

docker run --rm --network=none \
    --user "$(id -u):$(id -g)" \
    --cap-drop=ALL \
    --security-opt=no-new-privileges \
    --read-only \
    --volume "$repo_root:/workspace:ro" \
    --volume "$artifact_root:/output:rw" \
    "$image" \
    mqjs --no-column -m32 \
    -o /output/workload.bin \
    "/workspace/${workload#"$repo_root/"}"

compile_profile()
{
    profile=$1
    compiler=$2
    target=$3
    start=$4
    linker=$5
    stack_define=$6
    shift 6

    case " ${MQUICKJS_PROFILES:-pico-arm pico2-arm pico2-riscv nanoc6-riscv atoms3-xtensa} " in
        *" $profile "*) ;;
        *) return ;;
    esac

    profile_dir="$artifact_root/$profile"
    mkdir -p "$profile_dir"
    docker run --rm --network=none \
        --user "$(id -u):$(id -g)" \
        --cap-drop=ALL \
        --security-opt=no-new-privileges \
        --read-only \
        --tmpfs /tmp:rw,noexec,nosuid,size=64m \
        --volume "$repo_root:/workspace:ro" \
        --volume "$artifact_root:/bytecode:ro" \
        --volume "$profile_dir:/output:rw" \
        --workdir /workspace \
        "$image" \
        "$compiler" \
        -Os -DNDEBUG -ffreestanding -fno-builtin \
        -ffunction-sections -fdata-sections \
        -fno-math-errno -fno-trapping-math \
        -I/opt/mquickjs -I/workspace/qualification/mquickjs \
        -isystem /usr/include/newlib \
        "$@" \
        "$stack_define" \
        "-DREMU_ERROR_OFFSET=${MQUICKJS_ERROR_OFFSET:-0}" \
        -nostdlib \
        -Wl,--gc-sections \
        -Wl,-T,"/workspace/qualification/mquickjs/$linker" \
        -o /output/mquickjs.elf \
        "/workspace/qualification/mquickjs/$start" \
        /workspace/qualification/mquickjs/workload_bytecode.S \
        /workspace/qualification/mquickjs/harness.c \
        /workspace/qualification/mquickjs/mini_libc.c \
        /opt/mquickjs/mquickjs.c \
        /opt/mquickjs/dtoa.c \
        /opt/mquickjs/libm.c \
        /opt/mquickjs/cutils.c \
        -lgcc

    "$remu" run \
        --target "$target" \
        --elf "$profile_dir/mquickjs.elf" \
        --max-instructions 30000000 \
        --result "$profile_dir/run.json"

    exit_code=$(jq -r '.exit_code // "null"' "$profile_dir/run.json")
    if [ "$exit_code" != 0 ]
    then
        echo "MQuickJS profile $profile failed with exit code $exit_code" >&2
        jq '.reason,.stats,.cpu.pc' "$profile_dir/run.json" >&2
        exit 1
    fi

    elf_hash=$(sha256sum "$profile_dir/mquickjs.elf" | cut -d ' ' -f 1)
    trace_digest=$(jq -r '.trace_digest' "$profile_dir/run.json")
    printf '%s\t%s\t%s\t%s\n' "$profile" "$elf_hash" "$trace_digest" "$exit_code" \
        > "$profile_dir/profile.tsv"
}

: > "$artifact_root/profiles.tsv"
compile_profile pico-arm arm-none-eabi-gcc rp2040 start_arm.S link_rp_arm.ld \
    -DREMU_STACK_TOP=0 \
    -mcpu=cortex-m0plus -mthumb &
pids=$!
compile_profile pico2-arm arm-none-eabi-gcc rp2350 start_arm.S link_rp_arm.ld \
    -DREMU_STACK_TOP=0 \
    -mcpu=cortex-m33 -mthumb &
pids="$pids $!"
compile_profile pico2-riscv riscv64-unknown-elf-gcc rp2350 start_riscv.S link_rp_riscv.ld \
    -DREMU_STACK_TOP=0x20082000 \
    -march=rv32imac_zicsr -mabi=ilp32 &
pids="$pids $!"
compile_profile nanoc6-riscv riscv64-unknown-elf-gcc esp32c6 start_riscv.S link_esp_riscv.ld \
    -DREMU_STACK_TOP=0x40880000 \
    -march=rv32imac_zicsr -mabi=ilp32 &
pids="$pids $!"
compile_profile atoms3-xtensa xtensa-esp32s3-elf-gcc esp32s3 start_xtensa.S link_xtensa.ld \
    -DREMU_STACK_TOP=0 \
    -mlongcalls -mtext-section-literals &
pids="$pids $!"

failed=0
for pid in $pids
do
    if ! wait "$pid"
    then
        failed=1
    fi
done
if [ "$failed" -ne 0 ]
then
    echo "One or more MQuickJS profiles failed" >&2
    exit 1
fi
cat "$artifact_root"/*/profile.tsv | LC_ALL=C sort > "$artifact_root/profiles.tsv"

jq -Rn \
    --arg image "$image" \
    --arg image_id "$image_id" \
    --arg commit "203d5bb79789bc47b74855d9207415dab71661a0" \
    --arg source_sha256 "$source_hash" \
    '[inputs | split("\t") | {
        profile: .[0],
        elf_sha256: .[1],
        trace_digest: .[2],
        exit_code: (.[3] | tonumber)
    }] as $profiles | {
        schema: "remu.mquickjs-qualification.v1",
        upstream: "https://github.com/bellard/mquickjs",
        commit: $commit,
        image: $image,
        image_id: $image_id,
        harness_sha256: $source_sha256,
        profiles: $profiles
    }' < "$artifact_root/profiles.tsv" > "$artifact_root/summary.json"

echo "MQuickJS qualification passed: $artifact_root/summary.json"
