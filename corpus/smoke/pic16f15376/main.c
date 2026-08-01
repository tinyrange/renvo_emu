#include <xc.h>
#include <stdint.h>

typedef struct {
    uint8_t tag;
    uint16_t value;
    uint8_t tail;
} record_t;

enum native_mode {
    MODE_ZERO,
    MODE_SEVEN = 7,
    MODE_LARGE = 257
};

volatile uint8_t interrupt_count;
volatile uint16_t failure_mask;
volatile uint8_t recursion_result;
volatile uint8_t volatile_cells[8];

static uint8_t recursive_count(uint8_t value)
{
    if (value == 0u) {
        return 0u;
    }
    return (uint8_t)(recursive_count((uint8_t)(value - 1u)) + 1u);
}

static uint16_t switch_lane(uint8_t selector)
{
    switch (selector) {
    case 0: return 0x0101u;
    case 1: return 0x1212u;
    case 2: return 0x2323u;
    case 3: return 0x3434u;
    case 4: return 0x4545u;
    case 5: return 0x5656u;
    case 6: return 0x6767u;
    case 7: return 0x7878u;
    default: return 0xaaaau;
    }
}

static uint32_t rotate_left_32(uint32_t value, uint8_t count)
{
    count &= 31u;
    return (value << count) | (value >> ((32u - count) & 31u));
}

static void expect(uint8_t condition, uint8_t bit)
{
    if (condition == 0u) {
        failure_mask |= (uint16_t)1u << bit;
    }
}

static void uart_write(uint8_t value)
{
    while (PIR3bits.TX1IF == 0u) {
    }
    TX1REG = value;
}

static void uart_text(const char *text)
{
    while (*text != '\0') {
        uart_write((uint8_t)*text++);
    }
}

void __interrupt() default_isr(void)
{
    if (PIR0bits.TMR0IF != 0u) {
        PIR0bits.TMR0IF = 0u;
        LATA ^= 1u;
        interrupt_count++;
    }
}

void main(void)
{
    uint8_t index;
    uint8_t byte_value = 250u;
    uint16_t word_value = 0x1234u;
    uint32_t wide_value = 0x12345678ul;
    record_t record = { 0xa5u, 0x5aa5u, 0x3cu };
    enum native_mode mode = MODE_LARGE;
    volatile uint8_t *pointer = &volatile_cells[0];

    /* Portable fixed-width and generated-library lane. */
    expect((uint8_t)(byte_value + 17u) == 11u, 0u);
    expect((uint16_t)(word_value * 3u) == 0x369cu, 1u);
    expect((uint16_t)(0x369cu / 7u) == 0x07cdu, 2u);
    expect((uint16_t)(0x369cu % 7u) == 1u, 3u);
    expect(rotate_left_32(wide_value, 7u) == 0x1a2b3c09ul, 4u);
    expect((wide_value / 37ul) == 0x007df47ful, 5u);
    expect((wide_value % 37ul) == 29ul, 6u);
    recursion_result = recursive_count(8u);
    expect(recursion_result == 8u, 7u);
    expect(switch_lane(6u) == 0x6767u, 8u);

    /* Native XC8 ABI, aggregate, pointer, and volatile ordering lane. */
    expect(sizeof(char) == 1u, 9u);
    expect(sizeof(int) == 2u, 10u);
    expect(sizeof(long) == 4u, 11u);
    expect(record.tag == 0xa5u && record.value == 0x5aa5u && record.tail == 0x3cu, 12u);
    expect((unsigned int)mode == 257u, 13u);
    for (index = 0u; index < 8u; index++) {
        pointer[index] = (uint8_t)(index * 17u + 3u);
    }
    expect(pointer[0] == 3u && pointer[7] == 122u, 14u);

    /* Selected PIC16F15376 peripheral and interrupt lane. */
    ANSELA = 0u;
    LATA = 0u;
    TRISA = 0xfeu;
    if ((PORTA & 0x02u) == 0u) {
        failure_mask |= 0x8000u;
    }
    RC1STA = 0x80u;
    TX1STA = 0x24u;
    uart_text("PIC16F15376:");
    uart_write(failure_mask == 0u ? (uint8_t)'O' : (uint8_t)'F');
    uart_write(failure_mask == 0u ? (uint8_t)'K' : (uint8_t)failure_mask);
    if (failure_mask != 0u) {
        uart_write((uint8_t)(failure_mask >> 8));
        uart_write(recursion_result);
    }
    uart_write((uint8_t)'\n');

    TMR0H = 127u;
    TMR0L = 0u;
    PIR0bits.TMR0IF = 0u;
    PIE0bits.TMR0IE = 1u;
    INTCONbits.PEIE = 1u;
    INTCONbits.GIE = 1u;
    T0CON0 = 0x80u;

    while (interrupt_count < 4u) {
    }
    T0CON0 = 0u;
    uart_text("IRQ\n");
    for (;;) {
        NOP();
    }
}
