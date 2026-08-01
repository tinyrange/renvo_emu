#include <avr/interrupt.h>
#include <avr/io.h>
#include <stdint.h>

_Static_assert(sizeof(char) == 1, "AVR byte width");
_Static_assert(sizeof(int) == 2, "AVR native int must be 16 bit");
_Static_assert(sizeof(long) == 4, "AVR native long must be 32 bit");
_Static_assert(sizeof(void *) == 2, "ATmega data pointers must be 16 bit");

static volatile uint8_t timer_interrupts;
static volatile uint8_t pin_interrupts;
static volatile uint16_t dividend = 1000u;
static volatile uint8_t switch_input = 3u;

struct abi_pair {
    uint8_t first;
    uint16_t second;
};

enum abi_state {
    ABI_ZERO,
    ABI_THREE = 3,
    ABI_LARGE = 300
};

static uint16_t recursive_sum(uint8_t value)
{
    return value == 0u ? 0u : (uint16_t)(value + recursive_sum((uint8_t)(value - 1u)));
}

static uint8_t switch_value(uint8_t value)
{
    switch (value) {
    case 0:
        return 11;
    case 2:
        return 29;
    case 3:
        return 47;
    case 7:
        return 83;
    default:
        return 101;
    }
}

static void uart_byte(uint8_t value)
{
    while ((UCSR0A & _BV(UDRE0)) == 0u) {
    }
    UDR0 = value;
}

static void uart_text(const char *text)
{
    while (*text != '\0') {
        uart_byte((uint8_t)*text++);
    }
}

static uint8_t eeprom_round_trip(uint16_t address, uint8_t value)
{
    EEAR = address;
    EEDR = value;
    EECR = _BV(EEMPE);
    EECR = _BV(EEPE);
    EEAR = address;
    EECR = _BV(EERE);
    return EEDR;
}

ISR(TIMER0_OVF_vect)
{
    TCCR0B = 0;
    TIFR0 = _BV(TOV0);
    PORTB ^= _BV(PORTB0);
    ++timer_interrupts;
}

ISR(PCINT0_vect)
{
    PCIFR = _BV(PCIF0);
    PORTB ^= _BV(PORTB0);
    ++pin_interrupts;
}

__attribute__((noreturn, noinline)) static void finish(uint8_t code)
{
    __asm__ volatile("mov r24,%0\n\tbreak" : : "r"(code) : "r24");
    __builtin_unreachable();
}

int main(void)
{
    uint8_t failures = 0;
    struct abi_pair pair = { 9u, 0x1234u };
    enum abi_state state = ABI_LARGE;

    failures |= (uint8_t)((recursive_sum(12u) != 78u) << 0);
    failures |= (uint8_t)(((dividend / 7u) != 142u) << 1);
    failures |= (uint8_t)(((dividend % 7u) != 6u) << 2);
    failures |= (uint8_t)((switch_value(switch_input) != 47u) << 3);
    failures |= (uint8_t)(((uint16_t)(pair.first + pair.second) != 0x123du) << 4);
    failures |= (uint8_t)(((int)state != 300) << 5);
    failures |= (uint8_t)((eeprom_round_trip(17u, 0x5au) != 0x5au) << 6);

    DDRB = _BV(DDB0);
    PORTB = 0;
    UCSR0B = _BV(TXEN0);
    PCMSK0 = _BV(PCINT1);
    PCICR = _BV(PCIE0);
    OCR0A = 15;
    TIMSK0 = _BV(TOIE0);
    TCCR0B = _BV(CS00);
    sei();

    while (timer_interrupts == 0u || pin_interrupts == 0u) {
        __asm__ volatile("sleep");
    }
    cli();
    failures |= (uint8_t)(((PINB & _BV(PINB1)) == 0u) << 7);

    uart_text("AVR8-PB\n");
    finish(failures);
}
