#!/bin/sh
set -eu

layout=${1:?layout must be wch, rp2350, or esp32c6}
optimization=${2:--O2}
selection=${3:-case_*.c}

case "$layout" in
    wch)
        march=rv32ec_zicsr
        mabi=ilp32e
        linker=support/riscv-wch.ld
        ;;
    rp2350)
        march=rv32imac_zicsr
        mabi=ilp32
        linker=support/riscv-rp2350.ld
        ;;
    esp32c6)
        march=rv32imac_zicsr
        mabi=ilp32
        linker=support/riscv-esp32c6.ld
        ;;
    *)
        echo "unknown RISC-V layout: $layout" >&2
        exit 2
        ;;
esac

for source in cases/$selection
do
    name=${source##*/}
    name=${name%.c}
    riscv64-unknown-elf-gcc \
        "-march=$march" "-mabi=$mabi" \
        -ffreestanding -fno-builtin -nostdlib \
        "$optimization" -g0 \
        -Wl,--build-id=none,--discard-all -Wl,-T,"$linker" \
        support/riscv-start.S support/runtime.c "$source" -lgcc \
        -o "/workspace/out/$name.elf"
done
