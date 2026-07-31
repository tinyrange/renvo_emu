#!/bin/sh
set -eu

# This is the RP2350 software-facing ISA string documented by Raspberry Pi.
# GCC 13 accepts the complete string in its compiler front end, but its bundled
# binutils predates Zcb/Zcmp. Keep that acceptance proof, then assemble the
# executable subset and supply the two newer compressed instructions as raw,
# specification-defined halfwords in extensions.S.
full_march=rv32imac_zicsr_zifencei_zba_zbb_zbs_zbkb_zcb_zcmp
assembler_march=rv32imac_zicsr_zifencei_zba_zbb_zbs_zbkb

riscv64-unknown-elf-gcc \
    -march="$full_march" -mabi=ilp32 -ffreestanding -nostdlib -O2 \
    -S compiler-cases.c -o /workspace/out/full-march.s
printf '%s\n' "$full_march" > /workspace/out/full-march.txt

# The generated instructions are all supported by this binutils release. Its
# attribute parser alone is older than Zcb/Zcmp, so remove only that metadata
# line and assemble the compiler output with the executable subset.
sed '/^[[:space:]]*\.attribute arch,/d' /workspace/out/full-march.s \
    > /tmp/compiler-cases.s
riscv64-unknown-elf-gcc \
    -march="$assembler_march" -mabi=ilp32 -ffreestanding -nostdlib \
    -c /tmp/compiler-cases.s -o /tmp/compiler-cases.o

riscv64-unknown-elf-gcc \
    -march="$assembler_march" -mabi=ilp32 -ffreestanding -nostdlib \
    start.S extensions.S /tmp/compiler-cases.o \
    -Wl,-T,link.ld -o /workspace/out/smoke.elf

riscv64-unknown-elf-readelf -A /workspace/out/smoke.elf \
    > /workspace/out/elf-attributes.txt
