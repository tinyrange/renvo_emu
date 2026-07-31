#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
renvo=${RENVO_BIN:-"$repo_root/target/release/renvo"}
firmware_root=${RENVO_FIRMWARE_CACHE:-"$repo_root/.renvo/firmware/micropython-v1.28.0"}
manifest=${RENVO_FIRMWARE_MANIFEST:-"$repo_root/firmware/micropython-v1.28.0.toml"}
artifact_root=${RENVO_ACCEPTANCE_ROOT:-"$repo_root/.renvo/qualification/acceptance"}
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

if [ ! -x "$renvo" ]
then
    "$HOME/.cargo/bin/cargo" build --release --package renvo-cli
fi

"$renvo" firmware verify \
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
    case_count=$(grep -ao 'RENVO_CASE ' "$transcript" | wc -l)
    test "$case_count" -eq 15
    grep -aF "RENVO_QUAL_DIGEST $expected_digest" "$transcript" >/dev/null
    grep -aF 'RENVO_QUAL_OK 15' "$transcript" >/dev/null
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
        "$instructions" "$events" >> "$records"
}

run_profile()
{
    profile=$1
    target=$2
    cpu=$3
    filename=$4
    limit=$5
    expected_digest=$6
    image="$firmware_root/$filename"

    repeat=1
    while [ "$repeat" -le 2 ]
    do
        profile_root="$run_root/$profile/repeat-$repeat"
        mkdir -p "$profile_root"
        echo "MicroPython qualification: $profile repeat $repeat/2"

        set -- "$renvo" firmware boot \
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
        repeat=$((repeat + 1))
    done
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
    image="$firmware_root/$filename"
    phase_root="$run_root/$profile/system/$scenario/repeat-$repeat/$phase"
    result="$phase_root/result.json"
    transcript="$phase_root/transcript.txt"
    vcd="$phase_root/pins.vcd"
    mkdir -p "$phase_root"

    echo "MicroPython system qualification: $profile $scenario $phase repeat $repeat/2"
    set -- "$renvo" firmware boot \
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
        "$instructions:$events" >> "$scenario_records"
}

run_system_profile()
{
    profile=$1
    target=$2
    cpu=$3
    filename=$4
    limit=$5

    repeat=1
    while [ "$repeat" -le 2 ]
    do
        soft_root="$run_root/$profile/system/soft-reset/repeat-$repeat"
        thread_root="$run_root/$profile/system/thread/repeat-$repeat"
        timer_root="$run_root/$profile/system/timer/repeat-$repeat"
        persistence_root="$run_root/$profile/system/persistence/repeat-$repeat"
        mkdir -p "$soft_root" "$thread_root" "$timer_root" "$persistence_root"

        run_system_phase \
            "$profile" "$target" "$cpu" "$filename" "$limit" \
            soft-reset "$repeat" run "$soft_reset_workload" \
            "RENVO_SOFT_RESET_OK 84" "$soft_root/flash.bin"
        run_system_phase \
            "$profile" "$target" "$cpu" "$filename" "$limit" \
            thread "$repeat" run "$thread_workload" \
            "RENVO_THREAD_OK 0xd062b2b8 True" "$thread_root/flash.bin"
        run_system_phase \
            "$profile" "$target" "$cpu" "$filename" "$limit" \
            timer "$repeat" run "$timer_workload" \
            "RENVO_TIMER_OK" "$timer_root/flash.bin"
        gpio_root="$run_root/$profile/system/gpio-input/repeat-$repeat"
        mkdir -p "$gpio_root"
        run_system_phase \
            "$profile" "$target" "$cpu" "$filename" "$limit" \
            gpio-input "$repeat" run "$gpio_input_workload" \
            "RENVO_GPIO_INPUT_OK 1 0 010" "$gpio_root/flash.bin" gpio-input
        run_system_phase \
            "$profile" "$target" "$cpu" "$filename" "$limit" \
            persistence "$repeat" write "$persistence_write_workload" \
            "RENVO_PERSIST_WRITE_OK 1024 0x1fe00" "$persistence_root/flash.bin"
        run_system_phase \
            "$profile" "$target" "$cpu" "$filename" "$limit" \
            persistence "$repeat" read "$persistence_read_workload" \
            "RENVO_PERSIST_READ_OK 1024 0x1fe00" "$persistence_root/flash.bin"

        repeat=$((repeat + 1))
    done
}

run_profile \
    nanoc6-riscv esp32c6 "" \
    M5STACK_NANOC6-20260406-v1.28.0.bin 25000000 \
    14b3676418863a42de4e917e8f9c68b95e416c25ac9a80c7aee72c34ad022ecf
run_profile \
    atoms3-xtensa esp32s3 "" \
    M5STACK_ATOMS3_LITE-20260406-v1.28.0.uf2 75000000 \
    bddff80c4a7e1b11362031cfeda5f8b87d26f835d46dc349606a20944f5c4627
run_profile \
    pico-arm rp2040 arm \
    RPI_PICO-20260406-v1.28.0.uf2 55000000 \
    986e7d814fc89da543d49ce7948cae2c461add4739add21e427fc581d3bb67b0
run_profile \
    pico2-arm rp2350 arm \
    RPI_PICO2-20260406-v1.28.0.uf2 55000000 \
    261aaaaef68699f8d96dbccd9e390a4b32d82b3e9a3e39b5131a33b17fb9c56c
run_profile \
    pico2-riscv rp2350 riscv \
    RPI_PICO2-RISCV-20260406-v1.28.0.uf2 105000000 \
    261aaaaef68699f8d96dbccd9e390a4b32d82b3e9a3e39b5131a33b17fb9c56c

run_system_profile \
    nanoc6-riscv esp32c6 "" \
    M5STACK_NANOC6-20260406-v1.28.0.bin 60000000
run_system_profile \
    atoms3-xtensa esp32s3 "" \
    M5STACK_ATOMS3_LITE-20260406-v1.28.0.uf2 80000000
run_system_profile \
    pico-arm rp2040 arm \
    RPI_PICO-20260406-v1.28.0.uf2 60000000
run_system_profile \
    pico2-arm rp2350 arm \
    RPI_PICO2-20260406-v1.28.0.uf2 60000000
run_system_profile \
    pico2-riscv rp2350 riscv \
    RPI_PICO2-RISCV-20260406-v1.28.0.uf2 110000000

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

for profile in nanoc6-riscv atoms3-xtensa pico-arm pico2-arm pico2-riscv
do
    for scenario_phase in soft-reset:run thread:run timer:run gpio-input:run persistence:write persistence:read
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

MQUICKJS_OFFLINE=1 \
MQUICKJS_ARTIFACT_ROOT="$run_root/mquickjs" \
    "$repo_root/scripts/qualify-mquickjs.sh"

renvo_sha=$(sha256sum "$renvo" | cut -d ' ' -f 1)
workload_sha=$(sha256sum "$workload" | cut -d ' ' -f 1)
soft_reset_workload_sha=$(sha256sum "$soft_reset_workload" | cut -d ' ' -f 1)
thread_workload_sha=$(sha256sum "$thread_workload" | cut -d ' ' -f 1)
gpio_input_workload_sha=$(sha256sum "$gpio_input_workload" | cut -d ' ' -f 1)
timer_workload_sha=$(sha256sum "$timer_workload" | cut -d ' ' -f 1)
persistence_write_workload_sha=$(sha256sum "$persistence_write_workload" | cut -d ' ' -f 1)
persistence_read_workload_sha=$(sha256sum "$persistence_read_workload" | cut -d ' ' -f 1)
mquickjs_sha=$(sha256sum "$run_root/mquickjs/summary.json" | cut -d ' ' -f 1)

jq -Rn \
    --slurpfile firmware "$run_root/firmware.json" \
    --slurpfile mquickjs "$run_root/mquickjs/summary.json" \
    --rawfile system_records "$scenario_records" \
    --arg renvo_sha256 "$renvo_sha" \
    --arg workload_sha256 "$workload_sha" \
    --arg soft_reset_workload_sha256 "$soft_reset_workload_sha" \
    --arg thread_workload_sha256 "$thread_workload_sha" \
    --arg gpio_input_workload_sha256 "$gpio_input_workload_sha" \
    --arg timer_workload_sha256 "$timer_workload_sha" \
    --arg persistence_write_workload_sha256 "$persistence_write_workload_sha" \
    --arg persistence_read_workload_sha256 "$persistence_read_workload_sha" \
    --arg mquickjs_summary_sha256 "$mquickjs_sha" \
    --arg run_directory "${run_root#"$repo_root/"}" \
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
        schema: "renvo.micropython-acceptance.v4",
        status: "passed",
        offline: true,
        release: "MicroPython v1.28.0 (2026-04-06)",
        boards: 4,
        execution_profiles: 5,
        clean_repeats: 2,
        firmware_patches: 0,
        renvo_sha256: $renvo_sha256,
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

echo "MicroPython acceptance passed: $artifact_root/report.html"
