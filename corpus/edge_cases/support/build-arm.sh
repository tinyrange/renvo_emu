#!/bin/sh
set -eu

cpu=${1:-cortex-m0plus}
optimization=${2:--O2}
selection=${3:-case_*.c}

for source in cases/$selection
do
    name=${source##*/}
    name=${name%.c}
    arm-none-eabi-gcc \
        "-mcpu=$cpu" -mthumb \
        -ffreestanding -fno-builtin -nostdlib \
        "$optimization" -g0 \
        -Wl,--build-id=none -Wl,-T,support/arm.ld \
        support/arm-start.S support/runtime.c "$source" -lgcc \
        -o "/workspace/out/$name.elf"
done
