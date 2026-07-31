#!/bin/sh
set -eu

optimization=${1:--O2}
selection=${2:-case_*.c}

for source in cases/$selection
do
    name=${source##*/}
    name=${name%.c}
    xtensa-esp32s3-elf-gcc \
        -mabi=call0 -mlongcalls -mtext-section-literals \
        -ffreestanding -fno-builtin -nostdlib \
        "$optimization" -g0 \
        -Wl,--build-id=none -Wl,-T,support/xtensa.ld \
        support/xtensa-harness.c support/runtime.c "$source" -lgcc \
        -o "/workspace/out/$name.elf"
done
