#!/bin/sh
set -eu

gate_started_ns=$(date +%s%N)
repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
remu=${REMU_BIN:-"$repo_root/target/release/remu"}
firmware_root=${REMU_FIRMWARE_CACHE:-"$repo_root/.remu/firmware/micropython-v1.28.0"}
manifest=${REMU_FIRMWARE_MANIFEST:-"$repo_root/firmware/micropython-v1.28.0.toml"}
artifact_root=${REMU_ACCEPTANCE_ROOT:-"$repo_root/.remu/qualification/acceptance"}
workload="$repo_root/qualification/micropython-comprehensive.py"
soft_reset_workload="$repo_root/qualification/micropython-soft-reset.py"
thread_workload="$repo_root/qualification/micropython-thread-probe.py"
gpio_input_workload="$repo_root/qualification/micropython-gpio-input.py"
timer_workload="$repo_root/qualification/micropython-timer.py"
persistence_write_workload="$repo_root/qualification/micropython-persistence-write.py"
persistence_read_workload="$repo_root/qualification/micropython-persistence-read.py"

mkdir -p "$artifact_root"
run_root=$(mktemp -d "$artifact_root/run-XXXXXX")
records="$run_root/records.tsv"
scenario_records="$run_root/system-records.tsv"
: > "$records"
: > "$scenario_records"

jobs=${REMU_ACCEPTANCE_JOBS:-$(getconf _NPROCESSORS_ONLN 2>/dev/null || printf '1\n')}
clean_repeats=${REMU_CLEAN_REPEATS:-1}
system_repeats=${REMU_SYSTEM_REPEATS:-1}
case "$jobs" in
    ''|*[!0-9]*|0)
        echo "REMU_ACCEPTANCE_JOBS must be a positive integer" >&2
        exit 1
        ;;
esac
if [ "$jobs" -gt 64 ]
then
    jobs=64
fi
case "$clean_repeats" in
    1|2) ;;
    *)
        echo "REMU_CLEAN_REPEATS must be 1 or 2" >&2
        exit 1
        ;;
esac
case "$system_repeats" in
    1|2) ;;
    *)
        echo "REMU_SYSTEM_REPEATS must be 1 or 2" >&2
        exit 1
        ;;
esac

job_fifo="$run_root/job-tokens.fifo"
mkfifo "$job_fifo"
exec 9<> "$job_fifo"
rm "$job_fifo"
job_index=0
while [ "$job_index" -lt "$jobs" ]
do
    printf 'token\n' >&9
    job_index=$((job_index + 1))
done
job_pids=

spawn_job()
{
    # Acquire the slot in the dispatcher.  Having each child compete for a token
    # allows the kernel to wake waiters out of order, which can leave an early,
    # expensive job until the end of the gate and create a long idle tail.
    IFS= read -r _token <&9
    (
        status=0
        "$@" || status=$?
        printf 'token\n' >&9
        exit "$status"
    ) &
    job_pids="$job_pids $!"
}

wait_for_jobs()
{
    failed=0
    for pid in $job_pids
    do
        if ! wait "$pid"
        then
            failed=1
        fi
    done
    exec 9>&-
    if [ "$failed" -ne 0 ]
    then
        echo "One or more acceptance jobs failed" >&2
        exit 1
    fi
}

if [ ! -x "$remu" ]
then
    "$HOME/.cargo/bin/cargo" build --release --package remu-cli
fi

"$remu" firmware verify \
    --manifest "$manifest" \
    --cache "$firmware_root" \
    --artifact "$run_root/firmware.json"

validate_run()
{
    profile=$1
    repeat=$2
    expected_digest=$3
    image=$4
    profile_root=$5
    result="$profile_root/result.json"
    transcript="$profile_root/transcript.txt"
    vcd="$profile_root/pins.vcd"

    jq -er '.usb | implode' "$result" > "$transcript"
    jq -e '.reason == "HostInputComplete" or .reason == "InstructionLimit" or .reason == "Halted"' "$result" >/dev/null
    grep -aF 'MicroPython v1.28.0 on 2026-04-06;' "$transcript" >/dev/null
    case_count=$(grep -ao 'REMU_CASE ' "$transcript" | wc -l)
    test "$case_count" -eq 15
    grep -aF "REMU_QUAL_DIGEST $expected_digest" "$transcript" >/dev/null
    grep -aF 'REMU_QUAL_OK 15' "$transcript" >/dev/null
    if grep -aE 'Traceback|MemoryError|AssertionError' "$transcript" >/dev/null
    then
        echo "$profile repeat $repeat contains a Python failure" >&2
        exit 1
    fi
    grep -F '$timescale 1ns $end' "$vcd" >/dev/null
    grep -F '$enddefinitions $end' "$vcd" >/dev/null
    grep -F '#0' "$vcd" >/dev/null
    var_count=$(grep -c '^\$var wire 1 ' "$vcd")
    test "$var_count" -ge 16
    change_count=$(grep -c '^#[1-9][0-9]*$' "$vcd")
    test "$change_count" -ge 3

    firmware_sha=$(sha256sum "$image" | cut -d ' ' -f 1)
    transcript_sha=$(sha256sum "$transcript" | cut -d ' ' -f 1)
    result_sha=$(sha256sum "$result" | cut -d ' ' -f 1)
    vcd_sha=$(sha256sum "$vcd" | cut -d ' ' -f 1)
    trace_digest=$(jq -r '.trace_digest' "$result")
    instructions=$(jq -r '.stats.instructions' "$result")
    events=$(jq -r '.stats.events' "$result")
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
        "$profile" "$repeat" "$firmware_sha" "$expected_digest" \
        "$transcript_sha" "$trace_digest" "$result_sha" "$vcd_sha" \
        "$instructions" "$events" > "$profile_root/record.tsv"
}

run_profile_repeat()
{
    profile=$1
    target=$2
    cpu=$3
    filename=$4
    limit=$5
    expected_digest=$6
    repeat=$7
    image="$firmware_root/$filename"
    profile_root="$run_root/$profile/repeat-$repeat"
    mkdir -p "$profile_root"
    echo "MicroPython qualification: $profile repeat $repeat/$clean_repeats"

    set -- "$remu" firmware boot \
        --target "$target" \
        --image "$image" \
        --usb-script "$workload" \
        --max-instructions "$limit" \
        --vcd "$profile_root/pins.vcd" \
        --result "$profile_root/result.json"
    if [ -n "$cpu" ]
    then
        set -- "$@" --cpu "$cpu"
    fi
    if [ "$profile" = atoms3-xtensa ]
    then
        set -- "$@" \
            --esp-base-image \
            "$firmware_root/M5STACK_ATOMS3_LITE-20260406-v1.28.0.bin"
    fi
    case "$target" in
        esp32c6|esp32s3)
            set -- "$@" --flash-state "$profile_root/flash.bin"
            ;;
    esac
    "$@"

    validate_run \
        "$profile" "$repeat" "$expected_digest" "$image" "$profile_root"

}

run_system_phase()
{
    profile=$1
    target=$2
    cpu=$3
    filename=$4
    limit=$5
    scenario=$6
    repeat=$7
    phase=$8
    script=$9
    marker=${10}
    flash_state=${11}
    stimulus_set=${12:-none}
    extra_script_1=${13:-}
    extra_script_2=${14:-}
    image="$firmware_root/$filename"
    phase_root="$run_root/$profile/system/$scenario/repeat-$repeat/$phase"
    result="$phase_root/result.json"
    transcript="$phase_root/transcript.txt"
    vcd="$phase_root/pins.vcd"
    mkdir -p "$phase_root"

    echo "MicroPython system qualification: $profile $scenario $phase repeat $repeat/$system_repeats"
    set -- "$remu" firmware boot \
        --target "$target" \
        --image "$image" \
        --usb-script "$script" \
        --max-instructions "$limit" \
        --flash-state "$flash_state" \
        --vcd "$vcd" \
        --result "$result"
    if [ -n "$cpu" ]
    then
        set -- "$@" --cpu "$cpu"
    fi
    if [ -n "$extra_script_1" ]
    then
        set -- "$@" --usb-script "$extra_script_1"
    fi
    if [ -n "$extra_script_2" ]
    then
        set -- "$@" --usb-script "$extra_script_2"
    fi
    if [ "$profile" = atoms3-xtensa ]
    then
        set -- "$@" \
            --esp-base-image \
            "$firmware_root/M5STACK_ATOMS3_LITE-20260406-v1.28.0.bin"
    fi
    case "$stimulus_set" in
        none) ;;
        gpio-input)
            set -- "$@" --pin 0=1@0 --pin 1=0@0
            ;;
        *)
            echo "Unknown system stimulus set: $stimulus_set" >&2
            exit 1
            ;;
    esac
    "$@"

    jq -r '(.usb | implode), (.uart | implode)' "$result" > "$transcript"
    jq -e '.reason == "HostInputComplete" or .reason == "InstructionLimit" or .reason == "Halted"' "$result" >/dev/null
    grep -aF "$marker" "$transcript" >/dev/null
    if grep -aE 'Traceback|MemoryError|AssertionError|Unhandled exception' "$transcript" >/dev/null
    then
        echo "$profile $scenario $phase repeat $repeat contains a Python failure" >&2
        exit 1
    fi
    grep -F '$timescale 1ns $end' "$vcd" >/dev/null
    grep -F '$enddefinitions $end' "$vcd" >/dev/null
    grep -F '#0' "$vcd" >/dev/null
    var_count=$(grep -c '^\$var wire 1 ' "$vcd")
    test "$var_count" -ge 16

    transcript_sha=$(sha256sum "$transcript" | cut -d ' ' -f 1)
    result_sha=$(sha256sum "$result" | cut -d ' ' -f 1)
    flash_sha=$(sha256sum "$flash_state" | cut -d ' ' -f 1)
    vcd_sha=$(sha256sum "$vcd" | cut -d ' ' -f 1)
    trace_digest=$(jq -r '.trace_digest' "$result")
    instructions=$(jq -r '.stats.instructions' "$result")
    events=$(jq -r '.stats.events' "$result")
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
        "$profile" "$scenario" "$repeat" "$phase" "$marker" \
        "$transcript_sha" "$trace_digest" "$result_sha" "$flash_sha" "$vcd_sha" \
        "$instructions:$events" > "$phase_root/record.tsv"
}

run_io_persistence_pair()
{
    profile=$1
    target=$2
    cpu=$3
    filename=$4
    limit=$5

    repeat=$6
    pair_root="$run_root/$profile/system/io-persistence/repeat-$repeat"
    mkdir -p "$pair_root"
    run_system_phase \
        "$profile" "$target" "$cpu" "$filename" "$limit" \
        io-persistence "$repeat" write "$timer_workload" \
        "REMU_PERSIST_WRITE_OK 1024 0x1fe00" "$pair_root/flash.bin" gpio-input \
        "$gpio_input_workload" "$persistence_write_workload"
    transcript="$pair_root/write/transcript.txt"
    grep -aF 'REMU_TIMER_PERIODIC_OK' "$transcript" >/dev/null
    grep -aF 'REMU_TIMER_ONE_SHOT_OK 1' "$transcript" >/dev/null
    grep -aF 'REMU_TIMER_OK' "$transcript" >/dev/null
    grep -aF 'REMU_GPIO_INPUT_OK 1 0 010' "$transcript" >/dev/null
    run_system_phase \
        "$profile" "$target" "$cpu" "$filename" "$limit" \
        persistence "$repeat" read "$persistence_read_workload" \
        "REMU_PERSIST_READ_OK 1024 0x1fe00" "$pair_root/flash.bin"
}

schedule_profile_jobs()
{
    repeat=1
    while [ "$repeat" -le "$clean_repeats" ]
    do
        spawn_job run_profile_repeat "$1" "$2" "$3" "$4" "$5" "$6" "$repeat"
        repeat=$((repeat + 1))
    done
}

schedule_io_persistence_jobs()
{
    repeat=1
    while [ "$repeat" -le "$system_repeats" ]
    do
        spawn_job run_io_persistence_pair "$1" "$2" "$3" "$4" "$5" "$repeat"
        repeat=$((repeat + 1))
    done
}

schedule_phase_repeats()
{
    profile=$1
    target=$2
    cpu=$3
    filename=$4
    limit=$5
    scenario=$6
    script=$7
    marker=$8
    stimulus_set=${9:-none}
    repeat=1
    while [ "$repeat" -le "$system_repeats" ]
    do
        phase_root="$run_root/$profile/system/$scenario/repeat-$repeat"
        spawn_job run_system_phase \
            "$profile" "$target" "$cpu" "$filename" "$limit" \
            "$scenario" "$repeat" run "$script" "$marker" \
            "$phase_root/flash.bin" "$stimulus_set"
        repeat=$((repeat + 1))
    done
}

run_mquickjs()
{
    MQUICKJS_OFFLINE=1 \
    MQUICKJS_ARTIFACT_ROOT="$run_root/mquickjs" \
        "$repo_root/scripts/qualify-mquickjs.sh"
}

echo "MicroPython acceptance: $jobs parallel jobs"
run_mquickjs

# Longest independent jobs enter the bounded queue first.
schedule_profile_jobs \
    atoms3-xtensa esp32s3 "" \
    M5STACK_ATOMS3_LITE-20260406-v1.28.0.uf2 75000000 \
    bddff80c4a7e1b11362031cfeda5f8b87d26f835d46dc349606a20944f5c4627
schedule_profile_jobs \
    pico-arm rp2040 arm \
    RPI_PICO-20260406-v1.28.0.uf2 55000000 \
    986e7d814fc89da543d49ce7948cae2c461add4739add21e427fc581d3bb67b0
schedule_profile_jobs \
    pico2-riscv rp2350 riscv \
    RPI_PICO2-RISCV-20260406-v1.28.0.uf2 105000000 \
    261aaaaef68699f8d96dbccd9e390a4b32d82b3e9a3e39b5131a33b17fb9c56c
schedule_profile_jobs \
    pico2-arm rp2350 arm \
    RPI_PICO2-20260406-v1.28.0.uf2 55000000 \
    261aaaaef68699f8d96dbccd9e390a4b32d82b3e9a3e39b5131a33b17fb9c56c
schedule_profile_jobs \
    nanoc6-riscv esp32c6 "" \
    M5STACK_NANOC6-20260406-v1.28.0.bin 25000000 \
    14b3676418863a42de4e917e8f9c68b95e416c25ac9a80c7aee72c34ad022ecf

schedule_io_persistence_jobs atoms3-xtensa esp32s3 "" \
    M5STACK_ATOMS3_LITE-20260406-v1.28.0.uf2 80000000
schedule_io_persistence_jobs nanoc6-riscv esp32c6 "" \
    M5STACK_NANOC6-20260406-v1.28.0.bin 60000000
schedule_io_persistence_jobs pico-arm rp2040 arm \
    RPI_PICO-20260406-v1.28.0.uf2 60000000
schedule_io_persistence_jobs pico2-riscv rp2350 riscv \
    RPI_PICO2-RISCV-20260406-v1.28.0.uf2 110000000
schedule_io_persistence_jobs pico2-arm rp2350 arm \
    RPI_PICO2-20260406-v1.28.0.uf2 60000000

schedule_phase_repeats atoms3-xtensa esp32s3 "" \
    M5STACK_ATOMS3_LITE-20260406-v1.28.0.uf2 80000000 \
    soft-reset "$soft_reset_workload" "REMU_SOFT_RESET_OK 84"
schedule_phase_repeats atoms3-xtensa esp32s3 "" \
    M5STACK_ATOMS3_LITE-20260406-v1.28.0.uf2 80000000 \
    thread "$thread_workload" "REMU_THREAD_OK 0xd062b2b8 True"

schedule_phase_repeats nanoc6-riscv esp32c6 "" \
    M5STACK_NANOC6-20260406-v1.28.0.bin 60000000 \
    thread "$thread_workload" "REMU_THREAD_OK 0xd062b2b8 True"
schedule_phase_repeats nanoc6-riscv esp32c6 "" \
    M5STACK_NANOC6-20260406-v1.28.0.bin 60000000 \
    soft-reset "$soft_reset_workload" "REMU_SOFT_RESET_OK 84"
schedule_phase_repeats pico-arm rp2040 arm \
    RPI_PICO-20260406-v1.28.0.uf2 60000000 \
    thread "$thread_workload" "REMU_THREAD_OK 0xd062b2b8 True"
schedule_phase_repeats pico2-riscv rp2350 riscv \
    RPI_PICO2-RISCV-20260406-v1.28.0.uf2 110000000 \
    thread "$thread_workload" "REMU_THREAD_OK 0xd062b2b8 True"
schedule_phase_repeats pico2-arm rp2350 arm \
    RPI_PICO2-20260406-v1.28.0.uf2 60000000 \
    thread "$thread_workload" "REMU_THREAD_OK 0xd062b2b8 True"
schedule_phase_repeats pico-arm rp2040 arm \
    RPI_PICO-20260406-v1.28.0.uf2 60000000 \
    soft-reset "$soft_reset_workload" "REMU_SOFT_RESET_OK 84"
schedule_phase_repeats pico2-riscv rp2350 riscv \
    RPI_PICO2-RISCV-20260406-v1.28.0.uf2 110000000 \
    soft-reset "$soft_reset_workload" "REMU_SOFT_RESET_OK 84"
schedule_phase_repeats pico2-arm rp2350 arm \
    RPI_PICO2-20260406-v1.28.0.uf2 60000000 \
    soft-reset "$soft_reset_workload" "REMU_SOFT_RESET_OK 84"
wait_for_jobs

tab=$(printf '\t')
cat "$run_root"/*/repeat-*/record.tsv \
    | LC_ALL=C sort -t "$tab" -k1,1 -k2,2n > "$records"
cat "$run_root"/*/system/*/repeat-*/*/record.tsv \
    | LC_ALL=C sort -t "$tab" -k1,1 -k2,2 -k3,3n -k4,4 > "$scenario_records"

if [ "$clean_repeats" -ge 2 ]
then
    for profile in nanoc6-riscv atoms3-xtensa pico-arm pico2-arm pico2-riscv
    do
        first=$(awk -F '\t' -v profile="$profile" '$1 == profile && $2 == 1 { print $5 ":" $6 }' "$records")
        second=$(awk -F '\t' -v profile="$profile" '$1 == profile && $2 == 2 { print $5 ":" $6 }' "$records")
        if [ "$first" != "$second" ]
        then
            echo "$profile is nondeterministic across clean repeats" >&2
            exit 1
        fi
    done
fi

if [ "$system_repeats" -ge 2 ]
then
    for profile in nanoc6-riscv atoms3-xtensa pico-arm pico2-arm pico2-riscv
    do
        for scenario_phase in soft-reset:run thread:run io-persistence:write persistence:read
        do
            scenario=${scenario_phase%:*}
            phase=${scenario_phase#*:}
            first=$(awk -F '\t' -v profile="$profile" -v scenario="$scenario" -v phase="$phase" \
                '$1 == profile && $2 == scenario && $3 == 1 && $4 == phase { print $6 ":" $7 ":" $9 ":" $10 }' \
                "$scenario_records")
            second=$(awk -F '\t' -v profile="$profile" -v scenario="$scenario" -v phase="$phase" \
                '$1 == profile && $2 == scenario && $3 == 2 && $4 == phase { print $6 ":" $7 ":" $9 ":" $10 }' \
                "$scenario_records")
            if [ "$first" != "$second" ]
            then
                echo "$profile $scenario $phase is nondeterministic across clean repeats" >&2
                exit 1
            fi
        done
    done
fi

remu_sha=$(sha256sum "$remu" | cut -d ' ' -f 1)
workload_sha=$(sha256sum "$workload" | cut -d ' ' -f 1)
soft_reset_workload_sha=$(sha256sum "$soft_reset_workload" | cut -d ' ' -f 1)
thread_workload_sha=$(sha256sum "$thread_workload" | cut -d ' ' -f 1)
gpio_input_workload_sha=$(sha256sum "$gpio_input_workload" | cut -d ' ' -f 1)
timer_workload_sha=$(sha256sum "$timer_workload" | cut -d ' ' -f 1)
persistence_write_workload_sha=$(sha256sum "$persistence_write_workload" | cut -d ' ' -f 1)
persistence_read_workload_sha=$(sha256sum "$persistence_read_workload" | cut -d ' ' -f 1)
mquickjs_sha=$(sha256sum "$run_root/mquickjs/summary.json" | cut -d ' ' -f 1)
gate_elapsed_ms=$((($(date +%s%N) - gate_started_ns) / 1000000))

jq -Rn \
    --slurpfile firmware "$run_root/firmware.json" \
    --slurpfile mquickjs "$run_root/mquickjs/summary.json" \
    --rawfile system_records "$scenario_records" \
    --arg remu_sha256 "$remu_sha" \
    --arg workload_sha256 "$workload_sha" \
    --arg soft_reset_workload_sha256 "$soft_reset_workload_sha" \
    --arg thread_workload_sha256 "$thread_workload_sha" \
    --arg gpio_input_workload_sha256 "$gpio_input_workload_sha" \
    --arg timer_workload_sha256 "$timer_workload_sha" \
    --arg persistence_write_workload_sha256 "$persistence_write_workload_sha" \
    --arg persistence_read_workload_sha256 "$persistence_read_workload_sha" \
    --arg mquickjs_summary_sha256 "$mquickjs_sha" \
    --arg run_directory "${run_root#"$repo_root/"}" \
    --argjson parallel_jobs "$jobs" \
    --argjson clean_repeats "$clean_repeats" \
    --argjson system_repeats "$system_repeats" \
    --argjson gate_elapsed_ms "$gate_elapsed_ms" \
    '[inputs | split("\t") | {
        profile: .[0],
        repeat: (.[1] | tonumber),
        firmware_sha256: .[2],
        evidence_digest: .[3],
        transcript_sha256: .[4],
        trace_digest: .[5],
        result_sha256: .[6],
        vcd_sha256: .[7],
        instructions: (.[8] | tonumber),
        events: (.[9] | tonumber)
    }] as $runs |
    ($system_records | split("\n") | map(select(length > 0) | split("\t") | {
        profile: .[0],
        scenario: .[1],
        repeat: (.[2] | tonumber),
        phase: .[3],
        expected_marker: .[4],
        transcript_sha256: .[5],
        trace_digest: .[6],
        result_sha256: .[7],
        flash_sha256: .[8],
        vcd_sha256: .[9],
        instructions: (.[10] | split(":")[0] | tonumber),
        events: (.[10] | split(":")[1] | tonumber)
    })) as $system_runs | {
        schema: "remu.micropython-acceptance.v4",
        status: "passed",
        offline: true,
        release: "MicroPython v1.28.0 (2026-04-06)",
        boards: 4,
        execution_profiles: 5,
        clean_repeats: $clean_repeats,
        system_repeats: $system_repeats,
        gate_runtime: {
            parallel_jobs: $parallel_jobs,
            wall_time_ms: $gate_elapsed_ms,
            target_ms: 60000
        },
        firmware_patches: 0,
        remu_sha256: $remu_sha256,
        workloads: {
            comprehensive_sha256: $workload_sha256,
            soft_reset_sha256: $soft_reset_workload_sha256,
            thread_sha256: $thread_workload_sha256,
            gpio_input_sha256: $gpio_input_workload_sha256,
            timer_sha256: $timer_workload_sha256,
            persistence_write_sha256: $persistence_write_workload_sha256,
            persistence_read_sha256: $persistence_read_workload_sha256
        },
        mquickjs_summary_sha256: $mquickjs_summary_sha256,
        run_directory: $run_directory,
        firmware_verification: $firmware[0],
        runs: $runs,
        system_scenarios: $system_runs,
        mquickjs: $mquickjs[0]
    }' < "$records" > "$run_root/summary.json"

cp "$run_root/summary.json" "$artifact_root/summary.json"
cp "$repo_root/qualification/acceptance-report.html" "$run_root/report.html"
cp "$repo_root/qualification/acceptance-report.html" "$artifact_root/report.html"

if [ -n "${REMU_ACCEPTANCE_MAX_SECONDS:-}" ]
then
    case "$REMU_ACCEPTANCE_MAX_SECONDS" in
        ''|*[!0-9]*|0)
            echo "REMU_ACCEPTANCE_MAX_SECONDS must be a positive integer" >&2
            exit 1
            ;;
    esac
    if [ "$gate_elapsed_ms" -ge "$((REMU_ACCEPTANCE_MAX_SECONDS * 1000))" ]
    then
        echo "MicroPython acceptance exceeded ${REMU_ACCEPTANCE_MAX_SECONDS}s: ${gate_elapsed_ms}ms" >&2
        exit 1
    fi
fi

echo "MicroPython acceptance passed: $artifact_root/report.html"
echo "MicroPython acceptance runtime: ${gate_elapsed_ms}ms"
