#!/bin/sh
set -eu

cpu=${1:-cortex-m0plus}
optimization=${2:--O2}
selection=${3:-case_*.c}
libgcc=$(arm-none-eabi-gcc "-mcpu=$cpu" -mthumb -print-libgcc-file-name)
libgcc_dir=${libgcc%/*}

for source in cases/$selection
do
    name=${source##*/}
    name=${name%.c}
    clang-18 \
        --target=arm-none-eabi "-mcpu=$cpu" -mthumb \
        -ffreestanding -fno-builtin -nostdlib \
        "$optimization" -g0 -fuse-ld=lld \
        -Wno-unused-command-line-argument -Wno-tautological-compare \
        -Wl,--build-id=none -Wl,-T,support/arm.ld \
        support/arm-start.S support/runtime.c "$source" \
        "-L$libgcc_dir" -lgcc \
        -o "/workspace/out/$name.elf"
done
