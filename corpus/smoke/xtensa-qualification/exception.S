    .section .level1_vector,"ax",@progbits
    .align 4
    .literal_position
    .global level_one_vector
    .type level_one_vector,@function
level_one_vector:
    movi    a2, 0x3fc
    slli    a2, a2, 20
    // ESP32-S3 DRAM starts at 0x3fc88000; construct the low address without
    // relying on a literal that the Xtensa assembler cannot encode directly.
    movi    a3, 0x440
    slli    a3, a3, 9
    add     a2, a2, a3
    addi    a2, a2, 16
    l32i    a3, a2, 0
    addi    a3, a3, 1
    s32i    a3, a2, 0
    movi    a2, 1
    wsr     a2, intclear
    rsync
    rfe
    .size level_one_vector, .-level_one_vector
