typedef unsigned int u32;

#define REG32(address) (*(volatile u32 *)(address))

__attribute__((section(".drom")))
static volatile const u32 mapped_data = 0x2468ace0u;

__attribute__((noinline, section(".irom")))
static u32 mapped_code(u32 value)
{
    return value ^ mapped_data;
}

__attribute__((noreturn, section(".text.start"))) void _start(void)
{
    void (*const configure_instruction_cache)(u32, u32, u32) =
        (void (*)(u32, u32, u32))0x40001a1cu;
    configure_instruction_cache(0x4000u, 8u, 32u);
    u32 mapped = mapped_code(0x13579bdfu);

    REG32(0x60004024u) = 1u << 2;
    REG32(0x60004008u) = 1u << 2;
    REG32(0xfffffff0u) = mapped == 0x373f373fu ? 0u : 32u;
    __asm__ volatile("break 0, 0");
    for (;;) {
    }
}
