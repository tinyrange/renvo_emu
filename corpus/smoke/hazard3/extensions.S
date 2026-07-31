.section .text
.global hazard3_compressed_cases
.type hazard3_compressed_cases, @function
hazard3_compressed_cases:
    # cm.push {ra}, -16 (Zcmp).
    .2byte 0xb842

    li a2, 0x20000100
    li t0, 0xa5
    sb t0, 3(a2)

    # c.lbu s1, 3(a2) (Zcb).
    .2byte 0x8264
    mv a0, s1

    # cm.popret {ra}, 16 (Zcmp).
    .2byte 0xbe42
