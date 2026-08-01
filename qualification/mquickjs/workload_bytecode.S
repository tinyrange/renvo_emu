.section .data.remu_mquickjs_bytecode, "aw"
.balign 16
.global remu_mquickjs_bytecode_start
.global remu_mquickjs_bytecode_end
remu_mquickjs_bytecode_start:
.incbin "/bytecode/workload.bin"
remu_mquickjs_bytecode_end:
