typedef unsigned int u32;

extern u32 remu_case(void);

__attribute__((noreturn, section(".text.start"))) void _start(void)
{
    volatile u32 *const exit_code = (volatile u32 *)0xfffffff0u;
    *exit_code = remu_case();
    __asm__ volatile("break 0, 0");
    for (;;) {
    }
}
