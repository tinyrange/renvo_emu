__attribute__((noreturn, section(".text.start"))) void _start(void)
{
    __asm__ volatile(".byte 0xff, 0xff, 0xff");
    for (;;) {
    }
}
