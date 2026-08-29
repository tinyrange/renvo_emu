#include <msp430.h>
#include <stdint.h>

__attribute__((noreturn, noinline)) static void finish(uint16_t code)
{
    __asm__ volatile("mov %0, r12\n\t.word 0" : : "r"(code) : "r12");
    __builtin_unreachable();
}

int main(void)
{
    WDTCTL = WDTPW | WDTHOLD;
    P1OUT = 0;
    P1DIR = BIT0;
    PM5CTL0 &= (uint16_t)~LOCKLPM5;
    P1OUT = BIT0;
    finish(0);
}
