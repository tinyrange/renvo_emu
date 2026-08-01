#include <stdint.h>
#include <stdio.h>

#include "uart.h"

__attribute__((noreturn, noinline)) static void finish(uint8_t code)
{
    __asm__ volatile("mov r24,%0\n\tbreak" : : "r"(code) : "r24");
    __builtin_unreachable();
}

int main(void)
{
    static const char message[] = "OFFICIAL\n";
    const char *cursor;

    uart_init();
    for (cursor = message; *cursor != '\0'; ++cursor) {
        uart_putchar(*cursor, NULL);
    }
    finish(0);
}
