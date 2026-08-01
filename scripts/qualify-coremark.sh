#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
. "$repo_root/scripts/lib/toolchain-images.sh"
commit=1f483d5b8316753a742cbf5590caf5bd0a4e4777
upstream=https://github.com/eembc/coremark.git
source_root=${COREMARK_SOURCE:-"$repo_root/.renvo/reference/coremark-$commit"}
artifact_root=${COREMARK_ARTIFACT_ROOT:-"$repo_root/.renvo/qualification/coremark"}
renvo=${RENVO_BIN:-"$repo_root/target/release/renvo"}
iterations=${COREMARK_ITERATIONS:-250}
cross_image=$(resolve_toolchain_image sha256:8f78d0ea26f75e5b44c2ad88175202f66f1e7054c6ec695c72d26948a48ba736 renvo/cross-gcc:local)
xtensa_image=$(resolve_toolchain_image sha256:e0c54aeaae63f842234ec88f7b5a61b69bfa4d9005ba7490df47328e0dc9892f renvo/xtensa-esp-gcc:local)

mkdir -p "$artifact_root"

if [ ! -d "$source_root/.git" ]
then
    if [ "${COREMARK_OFFLINE:-0}" = 1 ]
    then
        echo "Pinned CoreMark source is absent from the offline cache: $source_root" >&2
        exit 1
    fi
    git clone --filter=blob:none "$upstream" "$source_root"
    git -C "$source_root" checkout --detach "$commit"
fi

actual_commit=$(git -C "$source_root" rev-parse HEAD)
if [ "$actual_commit" != "$commit" ]
then
    echo "CoreMark checkout is $actual_commit, expected $commit" >&2
    exit 1
fi

# Prove that every benchmark source is the exact blob recorded by the pinned
# upstream tree. Only core_portme.*, startup, printf and linker files are ours.
for source in \
    core_list_join.c core_main.c core_matrix.c core_state.c core_util.c coremark.h
do
    expected_blob=$(git -C "$source_root" rev-parse "$commit:$source")
    actual_blob=$(git -C "$source_root" hash-object "$source_root/$source")
    if [ "$actual_blob" != "$expected_blob" ]
    then
        echo "CoreMark source differs from pinned upstream: $source" >&2
        exit 1
    fi
done

for image in "$cross_image" "$xtensa_image"
do
    if ! docker image inspect "$image" >/dev/null 2>&1
    then
        echo "Pinned compiler image is not available locally: $image" >&2
        exit 1
    fi
done

if [ ! -x "$renvo" ]
then
    cargo build --release --package renvo-cli
fi

run_root=$(mktemp -d "$artifact_root/run-XXXXXX")
records="$run_root/records.tsv"
: > "$records"

stage_sources()
{
    stage=$1
    start=$2
    linker=$3
    mkdir -p "$stage"
    cp \
        "$source_root/core_list_join.c" \
        "$source_root/core_main.c" \
        "$source_root/core_matrix.c" \
        "$source_root/core_state.c" \
        "$source_root/core_util.c" \
        "$source_root/coremark.h" \
        "$repo_root/qualification/coremark/core_portme.c" \
        "$repo_root/qualification/coremark/core_portme.h" \
        "$repo_root/qualification/coremark/ee_printf.c" \
        "$repo_root/qualification/coremark/$start" \
        "$repo_root/qualification/coremark/$linker" \
        "$stage/"
}

run_variant()
{
    id=$1
    mcu=$2
    target=$3
    architecture=$4
    cpu_mode=$5
    toolchain=$6
    start=$7
    linker=$8
    stack=$9
    flags=${10}
    variant=${11}
    data_size=${12}
    reportable=${13}
    note=${14}

    profile_root="$run_root/$id/$variant"
    stage="$run_root/stage-$id-$variant"
    mkdir -p "$profile_root"
    stage_sources "$stage" "$start" "$linker"

    case "$variant" in
        performance)
            run_define=-DPERFORMANCE_RUN=1
            expected_label='2K performance run parameters for coremark.'
            expected_crc='0xe714 0x1fd7 0x8e3a'
            ;;
        validation)
            run_define=-DVALIDATION_RUN=1
            expected_label='2K validation run parameters for coremark.'
            expected_crc='0xe3c1 0x0747 0x8d84'
            ;;
        profile)
            run_define=-DPROFILE_RUN=1
            expected_label='Profile generation run parameters for coremark.'
            expected_crc='0x6a79 0x5608 0xe5a4'
            ;;
        *)
            echo "Unknown CoreMark variant: $variant" >&2
            exit 1
            ;;
    esac

    set -- \
        -O2 -ffunction-sections -fdata-sections \
        "-DITERATIONS=$iterations" \
        "$run_define" \
        "-DTOTAL_DATA_SIZE=$data_size"
    if [ -n "$stack" ]
    then
        set -- "$@" "-DRENVO_STACK_TOP=$stack"
    fi
    set -- "$@" \
        "-DRENVO_COREMARK_FLAGS=\"$flags\"" \
        "$start" \
        core_list_join.c core_main.c core_matrix.c core_state.c core_util.c \
        core_portme.c ee_printf.c \
        -Wl,--gc-sections "-Wl,-T,$linker" -lgcc \
        -o /workspace/out/coremark.elf

    echo "CoreMark: $mcu $cpu_mode $variant"
    "$renvo" corpus build \
        --toolchain "$repo_root/toolchains/$toolchain" \
        --source "$stage" \
        --output "$profile_root" \
        --target "$target" \
        --artifact "$profile_root/build.json" \
        -- "$@"

    started_ns=$(date +%s%N)
    "$renvo" run \
        --target "$target" \
        --elf "$profile_root/coremark.elf" \
        --max-instructions 1000000000 \
        --result "$profile_root/run.json"
    finished_ns=$(date +%s%N)
    elapsed_ns=$((finished_ns - started_ns))

    jq -er '.uart | implode' "$profile_root/run.json" > "$profile_root/transcript.txt"
    jq -e \
        '(.reason == "Halted") and (.exit_code == 0)' \
        "$profile_root/run.json" >/dev/null
    grep -F "$expected_label" "$profile_root/transcript.txt" >/dev/null
    grep -F 'Correct operation validated.' "$profile_root/transcript.txt" >/dev/null
    if grep -F 'ERROR!' "$profile_root/transcript.txt" >/dev/null
    then
        echo "$mcu $cpu_mode $variant emitted a CoreMark error" >&2
        exit 1
    fi

    observed_iterations=$(awk -F: \
        '/^Iterations       :/{gsub(/ /, "", $2); print $2}' \
        "$profile_root/transcript.txt")
    test "$observed_iterations" -eq "$iterations"

    ticks=$(awk -F: \
        '/^Total ticks      :/{gsub(/ /, "", $2); print $2}' \
        "$profile_root/transcript.txt")
    instructions=$(jq -r '.stats.instructions' "$profile_root/run.json")
    seed_crc=$(awk -F: '/^seedcrc/{gsub(/ /, "", $2); print $2}' \
        "$profile_root/transcript.txt")
    list_crc=$(awk -F: '/crclist/{gsub(/ /, "", $2); print $2}' \
        "$profile_root/transcript.txt")
    matrix_crc=$(awk -F: '/crcmatrix/{gsub(/ /, "", $2); print $2}' \
        "$profile_root/transcript.txt")
    state_crc=$(awk -F: '/crcstate/{gsub(/ /, "", $2); print $2}' \
        "$profile_root/transcript.txt")
    final_crc=$(awk -F: '/crcfinal/{gsub(/ /, "", $2); print $2}' \
        "$profile_root/transcript.txt")
    observed_crc="$list_crc $matrix_crc $state_crc"
    if [ "$observed_crc" != "$expected_crc" ]
    then
        echo "$mcu $cpu_mode $variant CRC mismatch: $observed_crc" >&2
        exit 1
    fi
    action_score=$(awk -v count="$iterations" -v count_ticks="$ticks" \
        'BEGIN { printf "%.6f", count * 1000000 / count_ticks }')
    host_seconds=$(awk -v ns="$elapsed_ns" \
        'BEGIN { printf "%.6f", ns / 1000000000 }')
    host_score=$(awk -v count="$iterations" -v ns="$elapsed_ns" \
        'BEGIN { printf "%.6f", count * 1000000000 / ns }')
    elf_sha=$(sha256sum "$profile_root/coremark.elf" | cut -d ' ' -f 1)
    run_sha=$(sha256sum "$profile_root/run.json" | cut -d ' ' -f 1)
    compiler=$(awk -F: '/^Compiler version/{sub(/^ /, "", $2); print $2}' \
        "$profile_root/transcript.txt")

    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
        "$id" "$mcu" "$target" "$architecture" "$cpu_mode" "$variant" \
        "$reportable" "$data_size" "$iterations" "$ticks" "$instructions" \
        "$action_score" "$host_seconds" "$host_score" "$seed_crc" \
        "$list_crc" "$matrix_crc" "$state_crc" "$final_crc" "$compiler" \
        "$flags" "$elf_sha" "$run_sha" >> "$records"
}

run_standard_profile()
{
    id=$1
    mcu=$2
    target=$3
    architecture=$4
    cpu_mode=$5
    toolchain=$6
    start=$7
    linker=$8
    stack=$9
    flags=${10}

    run_variant \
        "$id" "$mcu" "$target" "$architecture" "$cpu_mode" "$toolchain" \
        "$start" "$linker" "$stack" "$flags" \
        performance 2000 true standard
    run_variant \
        "$id" "$mcu" "$target" "$architecture" "$cpu_mode" "$toolchain" \
        "$start" "$linker" "$stack" "$flags" \
        validation 2000 true standard
}

run_standard_profile \
    rp2040-arm RP2040 rp2040 Armv6-M Cortex-M0+ \
    arm-gcc-cortex-m0plus.toml start_arm.S link_rp_arm.ld "" \
    "-O2 -mcpu=cortex-m0plus -mthumb"
run_standard_profile \
    rp2350-arm RP2350 rp2350 Armv8-M Cortex-M33 \
    arm-gcc-cortex-m33.toml start_arm.S link_rp_arm.ld "" \
    "-O2 -mcpu=cortex-m33 -mthumb"
run_standard_profile \
    rp2350-riscv RP2350 rp2350 RV32IMAC Hazard3 \
    riscv-gcc-rv32imac.toml start_riscv.S link_rp_riscv.ld 0x20082000 \
    "-O2 -march=rv32imac_zicsr -mabi=ilp32"
run_standard_profile \
    esp32c6 ESP32-C6 esp32c6 RV32IMAC HP-core \
    riscv-gcc-rv32imac.toml start_riscv.S link_esp32c6.ld 0x40880000 \
    "-O2 -march=rv32imac_zicsr -mabi=ilp32"
run_standard_profile \
    esp32s3 ESP32-S3 esp32s3 Xtensa-LX7 LX7 \
    xtensa-esp-gcc-esp32s3.toml start_xtensa.S link_xtensa.ld "" \
    "-O2 -mlongcalls -mtext-section-literals"
run_standard_profile \
    ch32v006 CH32V006 ch32v006 RV32EC QingKe-V2C \
    riscv-gcc-rv32ec.toml start_riscv.S link_wch006.ld 0x20002000 \
    "-O2 -march=rv32ec_zicsr -mabi=ilp32e"

# CH32V003 has only 2 KiB SRAM. The standard 2,000-byte dataset leaves 16
# bytes after static data and therefore cannot retain even a small safe stack.
# The linker guard makes that limitation executable evidence.
guard_root="$run_root/ch32v003-standard-guard"
guard_stage="$run_root/stage-ch32v003-standard-guard"
mkdir -p "$guard_root"
stage_sources "$guard_stage" start_riscv.S link_wch003.ld
set +e
"$renvo" corpus build \
    --toolchain "$repo_root/toolchains/riscv-gcc-rv32ec.toml" \
    --source "$guard_stage" \
    --output "$guard_root" \
    --target ch32v003 \
    --artifact "$guard_root/build.json" \
    -- \
    -Os -ffunction-sections -fdata-sections \
    "-DITERATIONS=$iterations" -DPERFORMANCE_RUN=1 -DTOTAL_DATA_SIZE=2000 \
    -DRENVO_STACK_TOP=0x20000800 \
    '-DRENVO_COREMARK_FLAGS="-Os -march=rv32ec_zicsr -mabi=ilp32e"' \
    start_riscv.S core_list_join.c core_main.c core_matrix.c core_state.c \
    core_util.c core_portme.c ee_printf.c \
    -Wl,--gc-sections -Wl,-T,link_wch003.ld -lgcc \
    -o /workspace/out/coremark.elf
guard_status=$?
set -e
if [ "$guard_status" -eq 0 ] ||
    ! jq -er '.stderr | contains("leaves less than 512 bytes for stack")' \
        "$guard_root/build.json" >/dev/null
then
    echo "CH32V003 standard-size guard did not reject the unsafe image" >&2
    exit 1
fi

run_variant \
    ch32v003-profile CH32V003 ch32v003 RV32EC QingKe-V2A \
    riscv-gcc-rv32ec.toml start_riscv.S link_wch003.ld 0x20000800 \
    "-O2 -march=rv32ec_zicsr -mabi=ilp32e; 1200-byte profile" \
    profile 1200 false "non-reportable reduced-memory profile"

host_model=$(lscpu | awk -F: '/^Model name:/{sub(/^[ \t]+/, "", $2); print $2}')
host_kernel=$(uname -srmo)
renvo_sha=$(sha256sum "$renvo" | cut -d ' ' -f 1)
cross_image_id=$(docker image inspect --format '{{.Id}}' "$cross_image")
xtensa_image_id=$(docker image inspect --format '{{.Id}}' "$xtensa_image")
generated_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)

jq -Rn \
    --arg upstream "$upstream" \
    --arg commit "$commit" \
    --arg generated_at "$generated_at" \
    --arg host_model "$host_model" \
    --arg host_kernel "$host_kernel" \
    --arg renvo_sha256 "$renvo_sha" \
    --arg cross_image "$cross_image" \
    --arg cross_image_id "$cross_image_id" \
    --arg xtensa_image "$xtensa_image" \
    --arg xtensa_image_id "$xtensa_image_id" \
    --arg run_directory "${run_root#"$repo_root/"}" \
    '[
        inputs | split("\t") | {
            id: .[0],
            mcu: .[1],
            target: .[2],
            architecture: .[3],
            cpu_mode: .[4],
            variant: .[5],
            standard_reportable: (.[6] == "true"),
            data_size: (.[7] | tonumber),
            iterations: (.[8] | tonumber),
            abstract_ticks: (.[9] | tonumber),
            machine_instructions: (.[10] | tonumber),
            iterations_per_million_actions: (.[11] | tonumber),
            host_elapsed_seconds: (.[12] | tonumber),
            host_iterations_per_second: (.[13] | tonumber),
            crc: {
                seed: .[14],
                list: .[15],
                matrix: .[16],
                state: .[17],
                final: .[18]
            },
            compiler: .[19],
            flags: .[20],
            elf_sha256: .[21],
            run_sha256: .[22],
            status: "passed"
        }
    ] as $runs | {
        schema: "renvo.coremark-qualification.v1",
        generated_at: $generated_at,
        source: {
            upstream: $upstream,
            commit: $commit,
            integrity: "all six benchmark files match pinned Git blobs"
        },
        methodology: {
            compiler_execution: "pinned network-isolated Docker containers",
            execution: "Renvo release interpreter on host",
            score: "iterations divided by measured host wall-clock seconds",
            timing_disclaimer:
                "host score measures emulator throughput, not MCU silicon performance",
            repetitions: 1
        },
        host: {
            model: $host_model,
            kernel: $host_kernel
        },
        renvo_sha256: $renvo_sha256,
        containers: [
            {reference: $cross_image, image_id: $cross_image_id},
            {reference: $xtensa_image, image_id: $xtensa_image_id}
        ],
        run_directory: $run_directory,
        runs: $runs,
        limitations: [{
            target: "CH32V003",
            standard_dataset_bytes: 2000,
            ram_bytes: 2048,
            static_bytes: 2032,
            remaining_stack_bytes: 16,
            required_stack_reserve_bytes: 512,
            disposition:
                "standard run rejected; 1200-byte upstream profile run recorded separately"
        }]
    }' < "$records" > "$run_root/results.json"

cp "$run_root/results.json" "$artifact_root/results.json"
echo "CoreMark qualification passed: $artifact_root/results.json"
