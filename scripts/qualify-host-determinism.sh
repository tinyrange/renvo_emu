#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"

image=rust@sha256:5c6f46a6e4472ab1ca7ba7d494e6677f2f219ebc02f32025d3986f057635ec9c
toolchain=1.97.1
expected=6fdc01d7e6734b3a6674686e707a8ebdcffe0a2880c422af0fc4594a555784df
artifact=qualification/host-determinism.json
if test -n "${RENVO_CARGO_REGISTRY:-}"
then
    registry=$RENVO_CARGO_REGISTRY
else
    user_dir=$(getent passwd "$(id -u)" | cut -d: -f6)
    registry=$user_dir/.cargo/registry
fi

command -v docker >/dev/null
command -v jq >/dev/null
test -d "$registry"
mkdir -p .renvo/host-determinism/amd64-target .renvo/host-determinism/arm64-target

run_platform()
{
    platform=$1
    architecture=$2
    target_dir=$3
    echo "checking $platform" >&2
    output=$(docker run --rm \
        --platform "$platform" \
        --user "$(id -u):$(id -g)" \
        --network none \
        --read-only \
        --cap-drop ALL \
        --security-opt no-new-privileges \
        --tmpfs /tmp:rw,noexec,nosuid,size=256m \
        -v "$repo_root:/workspace:ro" \
        -v "$registry:/usr/local/cargo/registry:ro" \
        -v "$target_dir:/workspace-target:rw" \
        -w /workspace \
        -e CARGO_TARGET_DIR=/workspace-target \
        -e RUSTUP_TOOLCHAIN="$toolchain" \
        "$image" \
        cargo test -p renvo-trace \
            fake_multicore_timer_digest_survives_repeats_and_insertion_stress \
            --locked -- --nocapture 2>&1) || {
        printf '%s\n' "$output" >&2
        if test "$platform" = linux/arm64
        then
            echo "linux/arm64 execution failed; run on a native host or install aarch64 binfmt support" >&2
        fi
        return 1
    }
    digest=$(printf '%s\n' "$output" | sed -n 's/.*RENVO_HOST_DIGEST //p' | tail -n 1)
    test "$digest" = "$expected"
    rust_host=$(docker run --rm \
        --platform "$platform" \
        --network none \
        --read-only \
        --cap-drop ALL \
        --security-opt no-new-privileges \
        -e RUSTUP_TOOLCHAIN="$toolchain" \
        "$image" rustc -Vv | sed -n 's/^host: //p')
    jq -n \
        --arg platform "$platform" \
        --arg architecture "$architecture" \
        --arg rust_host "$rust_host" \
        --arg digest "$digest" \
        '{platform: $platform, architecture: $architecture, rust_host: $rust_host, digest: $digest, result: "pass"}'
}

amd64=$(run_platform linux/amd64 amd64 "$repo_root/.renvo/host-determinism/amd64-target")
arm64=$(run_platform linux/arm64 arm64 "$repo_root/.renvo/host-determinism/arm64-target")
test "$(printf '%s\n%s\n' "$amd64" "$arm64" | jq -s 'map(.digest) | unique | length')" -eq 1

source_sha=$(sha256sum \
    Cargo.lock \
    crates/renvo-core/src/event.rs \
    crates/renvo-trace/src/lib.rs | sha256sum | cut -d ' ' -f 1)
mkdir -p "$(dirname -- "$artifact")"
jq -n \
    --arg schema "renvo.host-determinism.v1" \
    --arg image "$image" \
    --arg toolchain "$toolchain" \
    --arg digest "$expected" \
    --arg source_sha256 "$source_sha" \
    --argjson amd64 "$amd64" \
    --argjson arm64 "$arm64" \
    '{
      schema: $schema,
      container_image: $image,
      rust_toolchain: $toolchain,
      source_sha256: $source_sha256,
      fake_multicore_timer_digest: $digest,
      repeats_per_platform: 64,
      insertion_stress_variants_per_platform: 64,
      supported_hosts: [$amd64, $arm64]
    }' > "$artifact"

echo "host determinism passed on linux/amd64 and linux/arm64; artifact: $artifact"
