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

libgcc=$(riscv64-unknown-elf-gcc "-march=$march" "-mabi=$mabi" -print-libgcc-file-name)
libgcc_dir=${libgcc%/*}

for source in cases/$selection
do
    name=${source##*/}
    name=${name%.c}
    clang-18 \
        --target=riscv32-unknown-elf "-march=$march" "-mabi=$mabi" \
        -ffreestanding -fno-builtin -nostdlib \
        "$optimization" -g0 -fuse-ld=lld \
        -Wno-unused-command-line-argument -Wno-tautological-compare \
        -Wl,--build-id=none -Wl,-T,"$linker" \
        support/riscv-start.S support/runtime.c "$source" \
        "-L$libgcc_dir" -lgcc \
        -o "/workspace/out/$name.elf"
done
