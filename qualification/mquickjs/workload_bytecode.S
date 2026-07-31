.section .data.renvo_mquickjs_bytecode, "aw"
.balign 16
.global renvo_mquickjs_bytecode_start
.global renvo_mquickjs_bytecode_end
renvo_mquickjs_bytecode_start:
.incbin "/bytecode/workload.bin"
renvo_mquickjs_bytecode_end:
