#!/bin/sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"

image=sha256:114a9f8cde8bdc5a7e95809745b492663f7c5afe9c2821a4127ff133754fafd2
loader_image=renvo/nanoc6-esptool:5.3.0
device=${RENVO_NANOC6_PORT:-/dev/ttyACM0}
artifact_dir=.renvo/nanoc6
build_dir=/artifacts/build
ram_build_dir=/artifacts/ram-build

mkdir -p "$artifact_dir"

case ${1:-test} in
    build)
        docker run --rm --network=none \
            --mount "type=bind,src=$project_dir,dst=/project,readonly" \
            --mount "type=bind,src=$project_dir/$artifact_dir,dst=/artifacts" \
            --workdir /project/hardware/nanoc6 \
            --env IDF_GIT_SAFE_DIR=/project \
            "$image" \
            idf.py -B "$build_dir" \
                -D SDKCONFIG=/artifacts/sdkconfig \
                -D SDKCONFIG_DEFAULTS=/project/hardware/nanoc6/sdkconfig.defaults \
                -D IDF_TARGET=esp32c6 \
                build
        ;;
    ram-build)
        docker run --rm --network=none \
            --mount "type=bind,src=$project_dir,dst=/project,readonly" \
            --mount "type=bind,src=$project_dir/$artifact_dir,dst=/artifacts" \
            --workdir /project/hardware/nanoc6 \
            --env IDF_GIT_SAFE_DIR=/project \
            "$image" \
            idf.py -B "$ram_build_dir" \
                -D SDKCONFIG=/artifacts/ram-sdkconfig \
                -D "SDKCONFIG_DEFAULTS=/project/hardware/nanoc6/sdkconfig.defaults;/project/hardware/nanoc6/sdkconfig.ram.defaults" \
                -D IDF_TARGET=esp32c6 \
                build
        ;;
    ram-run)
        if ! docker image inspect "$loader_image" >/dev/null 2>&1
        then
            docker build -t "$loader_image" \
                -f hardware/nanoc6/Dockerfile.esptool hardware/nanoc6
        fi
        docker run --rm --network=none --device="$device" \
            --mount "type=bind,src=$project_dir/$artifact_dir,dst=/artifacts" \
            "$loader_image" \
            --chip esp32c6 --port "$device" --no-stub \
                load-ram /artifacts/ram-build/renvo_nanoc6_oracle.bin
        ;;
    jtag-run)
        ram_elf="$artifact_dir/ram-build/renvo_nanoc6_oracle.elf"
        results_addr=$(nm -n "$ram_elf" |
            awk '$3 == "renvo_hw_results" { print "0x" $1 }')
        ready_addr=$(nm -n "$ram_elf" |
            awk '$3 == "renvo_hw_ready" { print "0x" $1 }')

        docker run --rm --privileged --network=none \
            "$image" \
            openocd -f board/esp32c6-builtin.cfg \
                -c "init; reset run; shutdown"

        sleep 1
        "$0" ram-run
        sleep 1

        docker run --rm --privileged --network=none \
            --mount "type=bind,src=$project_dir/$artifact_dir,dst=/artifacts" \
            "$image" \
            openocd -f board/esp32c6-builtin.cfg \
                -c "init; halt; dump_image /artifacts/hw-ready.bin $ready_addr 4; dump_image /artifacts/hw-results.bin $results_addr 160; resume; shutdown"

        ready=$(od -An -tx4 "$artifact_dir/hw-ready.bin" | tr -d ' ')
        if [ "$ready" != 52454e56 ]
        then
            echo "NanoC6 oracle did not publish its completion marker" >&2
            exit 1
        fi

        od -An -tx4 -w4 -v "$artifact_dir/hw-results.bin" |
            awk 'BEGIN {
                split("0000 0025 0050 0075 0100 0125 0150 0175 0200 0225 0250 0275 0300 0325 0350 0375 0400 0425 0450 0475 0500 0525 0550 0575 0600 0625 0650 0675 0700 0725 0750 0775 0800 0825 0850 0875 0900 0925 0950 0975", ids)
            }
            {
                sub(/^[[:space:]]+/, "")
                printf "RENVO_HW case_%s %s\n", ids[NR], $1
            }' | tee "$artifact_dir/capture.txt"
        ;;
    backup)
        if [ -e "$artifact_dir/factory-flash.bin" ]
        then
            echo "factory backup already exists: $artifact_dir/factory-flash.bin"
            exit 0
        fi
        docker run --rm --device="$device" \
            --mount "type=bind,src=$project_dir/$artifact_dir,dst=/artifacts" \
            "$image" \
            python -m esptool --chip esp32c6 --port "$device" \
                read_flash 0 0x400000 /artifacts/factory-flash.bin
        sha256sum "$artifact_dir/factory-flash.bin" \
            > "$artifact_dir/factory-flash.bin.sha256"
        ;;
    flash)
        docker run --rm --network=none --device="$device" \
            --mount "type=bind,src=$project_dir,dst=/project,readonly" \
            --mount "type=bind,src=$project_dir/$artifact_dir,dst=/artifacts" \
            --workdir /project/hardware/nanoc6 \
            --env IDF_GIT_SAFE_DIR=/project \
            "$image" \
            idf.py -B "$build_dir" -p "$device" flash
        ;;
    capture)
        docker run --rm --network=none --device="$device" \
            --mount "type=bind,src=$project_dir,dst=/project,readonly" \
            "$image" \
            python /project/hardware/nanoc6/capture.py "$device" \
            | tee "$artifact_dir/capture.txt"
        ;;
    compare)
        awk -f hardware/nanoc6/compare.awk \
            "$artifact_dir/capture.txt" corpus/edge_cases/manifest.tsv
        ;;
    test)
        "$0" ram-build
        "$0" jtag-run
        "$0" compare
        ;;
    *)
        echo "usage: $0 [build|ram-build|ram-run|jtag-run|backup|flash|capture|compare|test]" >&2
        exit 2
        ;;
esac
