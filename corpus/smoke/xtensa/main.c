static volatile unsigned int left = 7;
static volatile unsigned int right = 5;

__attribute__((noreturn, section(".text.start"))) void _start(void)
{
    volatile unsigned int *const gpio_enable_set =
        (volatile unsigned int *)0x60004024u;
    volatile unsigned int *const gpio_output_set =
        (volatile unsigned int *)0x60004008u;
    volatile unsigned int *const exit_code =
        (volatile unsigned int *)0xfffffff0u;
    *gpio_enable_set = 1u << 2;
    *gpio_output_set = 1u << 2;
    *exit_code = left + right;
    __asm__ volatile("break 0, 0");
    for (;;) {
    }
}
