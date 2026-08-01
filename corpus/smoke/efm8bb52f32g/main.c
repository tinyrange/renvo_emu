#include <stdint.h>

__sfr __at (0x80) P0;
__sfr __at (0x88) TCON;
__sfr __at (0x89) TMOD;
__sfr __at (0x8a) TL0;
__sfr __at (0x8c) TH0;
__sfr __at (0x97) WDTCN;
__sfr __at (0x98) SCON0;
__sfr __at (0x99) SBUF0;
__sfr __at (0xa4) P0MDOUT;
__sfr __at (0xa7) SFRPAGE;
__sfr __at (0xa8) IE;
__sfr __at (0xe1) XBR0;
__sfr __at (0xe3) XBR2;

static volatile uint8_t interrupt_count;
static volatile uint16_t failure_mask;
static volatile uint8_t cells[8];

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
    SBUF0 = value;
    while ((SCON0 & 0x02u) == 0u) {
    }
    SCON0 &= (uint8_t)~0x02u;
}

static void uart_text(const char *text)
{
    while (*text != '\0') {
        uart_write((uint8_t)*text++);
    }
}

void timer0_isr(void) __interrupt (1)
{
    TCON &= (uint8_t)~0x20u;
    P0 ^= 0x01u;
    interrupt_count++;
}

void main(void)
{
    uint8_t index;
    uint8_t byte_value = 250u;
    uint16_t word_value = 0x1234u;
    uint32_t wide_value = 0x12345678ul;
    record_t record = { 0xa5u, 0x5aa5u, 0x3cu };
    enum native_mode mode = MODE_LARGE;
    volatile uint8_t *pointer = &cells[0];

    WDTCN = 0xdeu;
    WDTCN = 0xadu;

    expect((uint8_t)(byte_value + 17u) == 11u, 0u);
    expect((uint16_t)(word_value * 3u) == 0x369cu, 1u);
    expect((uint16_t)(0x369cu / 7u) == 0x07cdu, 2u);
    expect((uint16_t)(0x369cu % 7u) == 1u, 3u);
    expect(rotate_left_32(wide_value, 7u) == 0x1a2b3c09ul, 4u);
    expect((wide_value / 37ul) == 0x007df47ful, 5u);
    expect((wide_value % 37ul) == 29ul, 6u);
    expect(recursive_count(8u) == 8u, 7u);
    expect(switch_lane(6u) == 0x6767u, 8u);

    expect(sizeof(char) == 1u, 9u);
    expect(sizeof(int) == 2u, 10u);
    expect(sizeof(long) == 4u, 11u);
    expect(sizeof(void *) == 3u, 12u);
    expect(record.tag == 0xa5u && record.value == 0x5aa5u && record.tail == 0x3cu, 13u);
    expect((unsigned int)mode == 257u, 14u);
    for (index = 0u; index < 8u; index++) {
        pointer[index] = (uint8_t)(index * 17u + 3u);
    }
    expect(pointer[0] == 3u && pointer[7] == 122u, 15u);

    SFRPAGE = 0u;
    P0 = 0x02u;
    P0MDOUT = 0x01u;
    XBR0 = 0x01u;
    XBR2 = 0x40u;
    SCON0 = 0u;
    if ((P0 & 0x02u) == 0u) {
        failure_mask |= 0x8000u;
    }
    uart_text("EFM8BB52:");
    uart_write(failure_mask == 0u ? (uint8_t)'O' : (uint8_t)'F');
    uart_write(failure_mask == 0u ? (uint8_t)'K' : (uint8_t)failure_mask);
    uart_write((uint8_t)'\n');

    TMOD = 0x02u;
    TH0 = 0xc0u;
    TL0 = 0xc0u;
    TCON = 0x10u;
    IE = 0x82u;
    while (interrupt_count < 4u) {
    }
    TCON = 0u;
    uart_text("IRQ\n");
    for (;;) {
    }
}
