#include <stdint.h>

volatile uint32_t bss_probe;

__attribute__((noreturn, section(".text.start"))) void _start(void)
{
    volatile uint32_t *const exit_code = (volatile uint32_t *)0xfffffff0u;
#ifdef CLEAR_BSS
    volatile uint32_t *cursor = &bss_probe;
    *cursor = 0u;
#endif

    *exit_code = bss_probe == 0u ? 0u : 70u;
    __asm__ volatile("break 0, 0");
    for (;;) {
    }
}
