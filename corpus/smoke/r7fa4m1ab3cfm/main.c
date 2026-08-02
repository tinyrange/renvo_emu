typedef unsigned char u8;
typedef unsigned int u32;

#define REG8(address) (*(volatile u8 *)(address))
#define REG16(address) (*(volatile unsigned short *)(address))
#define REG32(address) (*(volatile u32 *)(address))

#define SYSTEM_SCKSCR REG8(0x4001e026u)
#define SYSTEM_OSCSF REG8(0x4001e03cu)
#define MSTPCRD REG32(0x40047008u)
#define PORT1_PCNTR2 REG32(0x40040024u)
#define PORT1_PCNTR3 REG32(0x40040028u)
#define P111PFS REG32(0x4004086cu)
#define ICU_IELSR7 REG32(0x4000631cu)
#define ICU_IELSR8 REG32(0x40006320u)
#define GPT0_GTCR REG32(0x4007802cu)
#define GPT0_GTINTAD REG32(0x40078038u)
#define GPT0_GTST REG32(0x4007803cu)
#define GPT0_GTPR REG32(0x40078064u)
#define SCI9_SCR REG8(0x40070122u)
#define SCI9_TDR REG8(0x40070123u)
#define SCI9_SSR REG8(0x40070124u)
#define NVIC_ISER0 REG32(0xe000e100u)
#define ADC_ADCSR REG16(0x4005c000u)
#define ADC_ADREF REG8(0x4005c002u)
#define ADC_ADANSA0 REG16(0x4005c004u)
#define ADC_ADDR0 REG16(0x4005c020u)

static volatile u32 timer_interrupts;
static volatile u32 adc_interrupts;
static volatile u32 dividend = 100000u;
static volatile u32 switch_input = 7u;

struct abi_record {
    u8 tag;
    unsigned short value;
    u32 wide;
};

enum abi_state {
    ABI_ZERO,
    ABI_LARGE = 300
};

static u32 recursive_sum(u32 value)
{
    return value == 0u ? 0u : value + recursive_sum(value - 1u);
}

static u32 switch_value(u32 value)
{
    switch (value) {
    case 0: return 11u;
    case 2: return 29u;
    case 7: return 47u;
    case 19: return 83u;
    default: return 101u;
    }
}

void gpt0_handler(void)
{
    GPT0_GTST = 0u;
    GPT0_GTCR = 0u;
    ++timer_interrupts;
}

void adc_handler(void)
{
    ADC_ADREF = 0u;
    ++adc_interrupts;
}

static void uart_write(const char *text)
{
    while (*text != '\0') {
        while ((SCI9_SSR & 0x80u) == 0u) {
        }
        SCI9_TDR = (u8)*text++;
    }
}

int main(void)
{
    u32 failures = 0u;
    struct abi_record record = { 9u, 0x1234u, 0x89abcdefu };
    enum abi_state state = ABI_LARGE;
    volatile float left = 1.75f;
    volatile float right = 4.0f;
    if ((int)(left * right) != 7) {
        failures |= 1u << 0;
    }
    failures |= (u32)((sizeof(char) != 1u) << 1);
    failures |= (u32)((sizeof(short) != 2u) << 2);
    failures |= (u32)((sizeof(int) != 4u) << 3);
    failures |= (u32)((sizeof(long) != 4u) << 4);
    failures |= (u32)((sizeof(void *) != 4u) << 5);
    failures |= (u32)((recursive_sum(12u) != 78u) << 6);
    failures |= (u32)(((dividend / 37u) != 2702u) << 7);
    failures |= (u32)(((dividend % 37u) != 26u) << 8);
    failures |= (u32)((switch_value(switch_input) != 47u) << 9);
    failures |= (u32)((record.tag != 9u || record.value != 0x1234u ||
                       record.wide != 0x89abcdefu) << 10);
    failures |= (u32)(((int)state != 300) << 11);

    if ((SYSTEM_OSCSF & 1u) == 0u) {
        failures |= 1u << 12;
    }
    SYSTEM_SCKSCR = 0;
    MSTPCRD &= ~(1u << 5);

    P111PFS = (1u << 2) | 1u;
    PORT1_PCNTR3 = 1u << 11;

    SCI9_SCR = 1u << 5;
    uart_write("RA4M1\n");

    ICU_IELSR7 = 0x05du;
    GPT0_GTPR = 7u;
    GPT0_GTINTAD = 1u << 6;
    NVIC_ISER0 = 1u << 7;
    GPT0_GTCR = 1u;
    while (timer_interrupts == 0u) {
    }

    ICU_IELSR8 = 0x29u;
    NVIC_ISER0 = 1u << 8;
    ADC_ADANSA0 = 1u;
    ADC_ADCSR = (1u << 15) | (1u << 12);
    while (adc_interrupts == 0u) {
    }
    failures |= (u32)((ADC_ADDR0 != 0u) << 13);

    while ((PORT1_PCNTR2 & (1u << 3)) == 0u) {
    }
    PORT1_PCNTR3 = 1u << (11u + 16u);
    return (int)failures;
}
