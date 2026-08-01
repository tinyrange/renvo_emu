#!/bin/sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"
. scripts/lib/toolchain-images.sh

remu=${REMU_BIN:-target/debug/remu}
toolchain=toolchains/rust-baremetal.toml
recorded_image=$(sed -n 's/^image = "\(.*\)"/\1/p' "$toolchain")
local_image=$(sed -n 's/^local_image = "\(.*\)"/\1/p' "$toolchain")
image=$(resolve_toolchain_image "$recorded_image" "$local_image")

if [ ! -x "$remu" ]; then
    cargo build -q -p remu-cli
fi

root=.remu/rust-abi-qualification-v2
proofs=$root/proofs.jsonl
mkdir -p "$root" qualification
: > "$proofs"

run_one()
{
    id=$1
    target=$2
    cpu=$3
    optimization=$4
    triple=$5
    elf=$6
    build_artifact=$7
    result=$root/$id-o$optimization-run.json
    repeat=$root/$id-o$optimization-repeat.json

    "$remu" run \
        --target "$target" \
        --elf "$elf" \
        --max-instructions 100000 \
        --result "$result"
    "$remu" run \
        --target "$target" \
        --elf "$elf" \
        --max-instructions 100000 \
        --result "$repeat"

    jq -e '.reason == "Halted" and .exit_code == 0' "$result" >/dev/null
    cmp "$result" "$repeat"

    elf_sha=$(sha256sum "$elf" | cut -d ' ' -f 1)
    build_sha=$(jq -S -c 'del(.stdout, .stderr)' "$build_artifact" \
        | sha256sum | cut -d ' ' -f 1)
    result_sha=$(sha256sum "$result" | cut -d ' ' -f 1)
    jq -c \
        --arg id "$id-o$optimization" \
        --arg target "$target" \
        --arg cpu "$cpu" \
        --arg optimization "$optimization" \
        --arg triple "$triple" \
        --arg elf "$elf" \
        --arg elf_sha256 "$elf_sha" \
        --arg build_artifact "$build_artifact" \
        --arg build_provenance_sha256 "$build_sha" \
        --arg result_artifact "$result" \
        --arg result_sha256 "$result_sha" \
        '{
          id: $id,
          target: $target,
          cpu: $cpu,
          optimization: $optimization,
          rust_target: $triple,
          elf: $elf,
          elf_sha256: $elf_sha256,
          build_artifact: $build_artifact,
          build_provenance_sha256: $build_provenance_sha256,
          result_artifact: $result_artifact,
          result_sha256: $result_sha256,
          reason,
          exit_code,
          stats,
          trace_digest,
          deterministic_repeat: true,
          result: "pass"
        }' "$result" >> "$proofs"
}

build_one()
{
    id=$1
    triple=$2
    profile=$3
    optimization=$4
    output=$root/build-$id-o$optimization
    artifact=$root/build-$id-o$optimization.json
    mkdir -p "$output"
    "$remu" corpus build \
        --toolchain "$toolchain" \
        --source corpus/rust_abi \
        --output "$output" \
        --target "$triple" \
        --artifact "$artifact" \
        -- "$triple" "$profile" "$optimization" case.elf
}

for optimization in 0 2 s
do
    build_one wch riscv32e-unknown-none-elf wch "$optimization"
    wch_elf=$root/build-wch-o$optimization/case.elf
    wch_build=$root/build-wch-o$optimization.json
    run_one ch32v003 ch32v003 qingke-v2a "$optimization" \
        riscv32e-unknown-none-elf "$wch_elf" "$wch_build"
    run_one ch32v006 ch32v006 qingke-v2c "$optimization" \
        riscv32e-unknown-none-elf "$wch_elf" "$wch_build"

    build_one esp32c6 riscv32imac-unknown-none-elf esp32c6 "$optimization"
    run_one esp32c6 esp32c6 esp-rv32imac-hp "$optimization" \
        riscv32imac-unknown-none-elf \
        "$root/build-esp32c6-o$optimization/case.elf" \
        "$root/build-esp32c6-o$optimization.json"

    build_one rp2040 thumbv6m-none-eabi rp2040 "$optimization"
    run_one rp2040 rp2040 cortex-m0plus "$optimization" \
        thumbv6m-none-eabi \
        "$root/build-rp2040-o$optimization/case.elf" \
        "$root/build-rp2040-o$optimization.json"

    build_one rp2350-arm thumbv8m.main-none-eabi rp2350_arm "$optimization"
    run_one rp2350-arm rp2350 cortex-m33 "$optimization" \
        thumbv8m.main-none-eabi \
        "$root/build-rp2350-arm-o$optimization/case.elf" \
        "$root/build-rp2350-arm-o$optimization.json"

    build_one rp2350-riscv riscv32imac-unknown-none-elf rp2350_riscv "$optimization"
    run_one rp2350-riscv rp2350 hazard3-rv32imac "$optimization" \
        riscv32imac-unknown-none-elf \
        "$root/build-rp2350-riscv-o$optimization/case.elf" \
        "$root/build-rp2350-riscv-o$optimization.json"
done

source_sha=$(sha256sum \
    corpus/rust_abi/Cargo.toml \
    corpus/rust_abi/Cargo.lock \
    corpus/rust_abi/build.sh \
    corpus/rust_abi/src/main.rs \
    corpus/rust_abi/start-wch.S \
    corpus/rust_abi/start-esp32c6.S \
    corpus/rust_abi/start-arm.S \
    corpus/rust_abi/start-rp2350-riscv.S \
    corpus/rust_abi/link-wch.ld \
    corpus/rust_abi/link-esp32c6.ld \
    corpus/rust_abi/link-arm.ld \
    corpus/rust_abi/link-rp2350-riscv.ld \
    toolchains/rust-baremetal.toml \
    toolchains/rust-baremetal/Dockerfile \
    toolchains/rust-baremetal/preload/Cargo.toml \
    toolchains/rust-baremetal/preload/Cargo.lock \
    toolchains/rust-baremetal/preload/src/main.rs \
    scripts/qualify-rust-abi.sh | sha256sum | cut -d ' ' -f 1)
image_id=$(docker image inspect "$image" --format '{{.Id}}')

jq -n \
    --arg schema remu.rust-abi.v1 \
    --arg source_sha256 "$source_sha" \
    --arg image "$image" \
    --arg image_id "$image_id" \
    --slurpfile proofs "$proofs" \
    '{
      schema: $schema,
      source_sha256: $source_sha256,
      toolchain: {
        rustc: "1.97.1",
        requested_image: $image,
        image_id: $image_id,
        network: "none"
      },
      optimizations: ["0", "2", "s"],
      proofs: $proofs,
      result: "pass"
    }' > qualification/rust-abi.json

echo "Rust ABI matrix passed 18 deterministic runs; artifact: qualification/rust-abi.json"
