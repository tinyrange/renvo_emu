#include <msp430.h>
#include <stdint.h>

_Static_assert(sizeof(char) == 1, "MSP430 byte width");
_Static_assert(sizeof(int) == 2, "MSP430 native int must be 16 bit");
_Static_assert(sizeof(long) == 4, "MSP430 native long must be 32 bit");
_Static_assert(sizeof(void *) == 2, "FR2433 small-model pointers must be 16 bit");

static volatile uint16_t timer_interrupts;
static volatile uint16_t port_interrupts;
static volatile uint32_t dividend = 100000UL;
static volatile uint16_t switch_input = 7u;

struct abi_record {
    uint8_t tag;
    uint16_t value;
    uint32_t wide;
};

enum abi_state {
    ABI_ZERO,
    ABI_SEVEN = 7,
    ABI_LARGE = 300
};

static uint16_t recursive_sum(uint16_t value)
{
    return value == 0u ? 0u : (uint16_t)(value + recursive_sum((uint16_t)(value - 1u)));
}

static uint16_t switch_value(uint16_t value)
{
    switch (value) {
    case 0:
        return 11;
    case 2:
        return 29;
    case 7:
        return 47;
    case 19:
        return 83;
    default:
        return 101;
    }
}

static void uart_byte(uint8_t value)
{
    while ((UCA0IFG & UCTXIFG) == 0u) {
    }
    UCA0TXBUF = value;
}

static void uart_text(const char *text)
{
    while (*text != '\0') {
        uart_byte((uint8_t)*text++);
    }
}

__attribute__((interrupt(TIMER0_A0_VECTOR)))
static void timer_a0_isr(void)
{
    TA0CTL = MC__STOP;
    P1OUT ^= BIT0;
    ++timer_interrupts;
    __bic_SR_register_on_exit(LPM0_bits);
}

__attribute__((interrupt(PORT1_VECTOR)))
static void port1_isr(void)
{
    P1IFG &= (uint8_t)~BIT1;
    P1OUT ^= BIT0;
    ++port_interrupts;
    __bic_SR_register_on_exit(LPM0_bits);
}

__attribute__((noreturn, noinline)) static void finish(uint16_t code)
{
    __asm__ volatile("mov %0, r12\n\t.word 0" : : "r"(code) : "r12");
    __builtin_unreachable();
}

int main(void)
{
    uint16_t failures = 0;
    struct abi_record record = { 9u, 0x1234u, 0x89abcdefUL };
    enum abi_state state = ABI_LARGE;
    volatile uint16_t *const persistent_word = (volatile uint16_t *)0xc100u;

    WDTCTL = WDTPW | WDTHOLD;
    FRCTL0 = FRCTLPW | NWAITS_0;
    CSCTL1 = DCORSEL_0;

    failures |= (uint16_t)((recursive_sum(12u) != 78u) << 0);
    failures |= (uint16_t)(((dividend / 7UL) != 14285UL) << 1);
    failures |= (uint16_t)(((dividend % 7UL) != 5UL) << 2);
    failures |= (uint16_t)((switch_value(switch_input) != 47u) << 3);
    failures |= (uint16_t)(((uint16_t)(record.tag + record.value) != 0x123du) << 4);
    failures |= (uint16_t)((record.wide != 0x89abcdefUL) << 5);
    failures |= (uint16_t)(((int)state != 300) << 6);
    *persistent_word = 0x5aa5u;
    failures |= (uint16_t)((*persistent_word != 0x5aa5u) << 7);

    P1OUT = 0;
    P1DIR = BIT0;
    P1IES &= (uint8_t)~BIT1;
    P1IFG &= (uint8_t)~BIT1;
    P1IE |= BIT1;
    PM5CTL0 &= (uint16_t)~LOCKLPM5;

    UCA0CTLW0 = UCSWRST;
    UCA0BRW = 8;
    UCA0MCTLW = 0;
    UCA0CTLW0 = UCSSEL__SMCLK;

    TA0CCR0 = 15;
    TA0CCTL0 = CCIE;
    TA0CTL = TASSEL__SMCLK | MC__UP | TACLR;

    while (timer_interrupts == 0u || port_interrupts == 0u) {
        __bis_SR_register(LPM0_bits | GIE);
    }
    __disable_interrupt();
    failures |= (uint16_t)(((P1IN & BIT1) == 0u) << 8);

    uart_text("MSP430X-FR2433\n");
    finish(failures);
}
