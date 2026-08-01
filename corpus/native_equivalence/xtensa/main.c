typedef unsigned int u32;

#define REG32(address) (*(volatile u32 *)(address))

__attribute__((noreturn, section(".text.start"))) void _start(void)
{
    REG32(0x60004024u) = 1u << 2;
    REG32(0x60004008u) = 1u << 2;
    REG32(0xfffffff0u) = 0u;
    __asm__ volatile("break 0, 0");
    for (;;) {
    }
}
