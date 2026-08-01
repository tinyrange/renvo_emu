__attribute__((noreturn, section(".text.start"))) void _start(void)
{
    static const unsigned char message[] = "REMU-ESP\n";
    volatile unsigned int *const uart_fifo =
        (volatile unsigned int *)0x60000000u;
    volatile unsigned int *const exit_code =
        (volatile unsigned int *)0xfffffff0u;
    for (unsigned int index = 0; index < sizeof(message) - 1u; ++index) {
        *uart_fifo = message[index];
    }
    *exit_code = 0;
    __asm__ volatile("break 0, 0");
    for (;;) {
    }
}
