typedef unsigned char u8;
typedef unsigned short u16;
typedef unsigned int u32;

#define REG8(address) (*(volatile u8 *)(address))
#define REG16(address) (*(volatile u16 *)(address))
#define REG32(address) (*(volatile u32 *)(address))

#define PM_APBCMASK REG32(0x40000420u)
#define PM_APBAMASK REG32(0x40000418u)
#define GCLK_CLKCTRL REG16(0x40000c02u)
#define EVSYS_CTRL REG8(0x42000400u)
#define EVSYS_CHANNEL REG32(0x42000404u)
#define EVSYS_CHANNEL_HALF REG16(0x42000404u)
#define EVSYS_USER REG16(0x42000408u)
#define EVSYS_INTENSET REG32(0x42000414u)
#define EVSYS_INTFLAG REG32(0x42000418u)
#define USB_CTRLA REG8(0x41005000u)
#define USB_CTRLB REG16(0x41005008u)
#define USB_DADD REG8(0x4100500au)
#define USB_STATUS REG8(0x4100500cu)
#define USB_INTENSET REG16(0x41005018u)
#define USB_DESCADD REG32(0x41005024u)
#define USB_EPCFG0 REG8(0x41005100u)
#define USB_EPSTATUSCLR0 REG8(0x41005104u)
#define USB_EPSTATUSSET0 REG8(0x41005105u)
#define USB_EPSTATUS0 REG8(0x41005106u)
#define USB_EPINTENSET0 REG8(0x41005109u)
#define PORT_DIRSET REG32(0x41004408u)
#define PORT_OUTSET REG32(0x41004418u)
#define PORT_OUTCLR REG32(0x41004414u)
#define PORT_IN REG32(0x41004420u)
#define TC3_CTRLA REG16(0x42002c00u)
#define TC3_INTENSET REG8(0x42002c0du)
#define TC3_INTFLAG REG8(0x42002c0eu)
#define TC3_COUNT REG16(0x42002c10u)
#define TC3_CC0 REG16(0x42002c18u)
#define EIC_CTRLA REG8(0x40001800u)
#define EIC_INTENSET REG32(0x4000180cu)
#define EIC_INTFLAG REG32(0x40001810u)
#define EIC_CONFIG0 REG32(0x40001818u)
#define SERCOM0_CTRLA REG32(0x42000800u)
#define SERCOM0_CTRLB REG32(0x42000804u)
#define SERCOM0_STATUS REG16(0x4200081au)
#define SERCOM0_INTFLAG REG8(0x42000818u)
#define SERCOM0_ADDR REG32(0x42000824u)
#define SERCOM0_DATA REG16(0x42000828u)
#define SERCOM1_CTRLA REG32(0x42000c00u)
#define SERCOM1_CTRLB REG32(0x42000c04u)
#define SERCOM1_DATA REG16(0x42000c28u)
#define SERCOM2_CTRLA REG32(0x42001000u)
#define SERCOM2_CTRLB REG32(0x42001004u)
#define SERCOM2_DATA REG16(0x42001028u)
#define SERCOM3_CTRLA REG32(0x42001400u)
#define SERCOM3_CTRLB REG32(0x42001404u)
#define SERCOM3_DATA REG16(0x42001428u)
#define TC4_CTRLA REG16(0x42003000u)
#define TC4_INTENSET REG8(0x4200300du)
#define TC4_CC0 REG16(0x42003018u)
#define TC5_CTRLA REG16(0x42003400u)
#define TC5_INTENSET REG8(0x4200340du)
#define TC5_CC0 REG16(0x42003418u)
#define TCC0_CTRLA REG32(0x42002000u)
#define TCC0_INTENSET REG32(0x42002028u)
#define TCC0_PER REG32(0x42002040u)
#define TCC0_CC0 REG32(0x42002044u)
#define RTC_CTRL REG16(0x40001400u)
#define RTC_INTENSET REG8(0x40001407u)
#define RTC_COUNT REG32(0x40001410u)
#define RTC_COMP0 REG32(0x40001418u)
#define DMAC_CTRL REG16(0x41004800u)
#define DMAC_SWTRIGCTRL REG32(0x41004810u)
#define DMAC_BASEADDR REG32(0x41004834u)
#define DMAC_WRBADDR REG32(0x41004838u)
#define DMAC_CHID REG8(0x4100483fu)
#define DMAC_CHCTRLA REG8(0x41004840u)
#define DMAC_CHINTENSET REG8(0x4100484du)
#define DMAC_CHINTFLAG REG8(0x4100484eu)
#define DMAC_CHSTATUS REG8(0x4100484fu)
#define I2S_CTRLA REG8(0x42005000u)
#define I2S_CLKCTRL0 REG32(0x42005004u)
#define I2S_INTENSET REG16(0x42005010u)
#define I2S_INTFLAG REG16(0x42005014u)
#define I2S_SYNCBUSY REG16(0x42005018u)
#define I2S_SERCTRL0 REG32(0x42005020u)
#define I2S_DATA0 REG32(0x42005030u)
#define ADC_CTRLA REG8(0x42004000u)
#define ADC_REFCTRL REG8(0x42004001u)
#define ADC_INPUTCTRL REG32(0x42004010u)
#define ADC_INTENSET REG8(0x42004017u)
#define ADC_INTFLAG REG8(0x42004018u)
#define ADC_RESULT REG16(0x4200401au)
#define ADC_SWTRIG REG8(0x4200400cu)
#define AC_CTRLA REG8(0x42004400u)
#define AC_CTRLB REG8(0x42004401u)
#define AC_STATUSA REG8(0x42004408u)
#define AC_STATUSB REG8(0x42004409u)
#define AC_COMPCTRL0 REG32(0x42004410u)
#define DAC_CTRLA REG8(0x42004800u)
#define DAC_CTRLB REG8(0x42004801u)
#define DAC_DATA REG16(0x42004808u)
#define NVIC_ISER0 REG32(0xe000e100u)

static volatile u32 timer_interrupts;
static volatile u32 dividend = 100000u;
static volatile u16 switch_input = 7u;
static volatile u8 dmac_source[4] = { 0xa1u, 0xb2u, 0xc3u, 0xd4u };
static volatile u8 dmac_destination[4];
struct dmac_descriptor {
    u16 btctrl;
    u16 btcnt;
    u32 srcaddr;
    u32 dstaddr;
    u32 descaddr;
};
static volatile struct dmac_descriptor dmac_descriptor __attribute__((aligned(64)));
static volatile u32 dmac_writeback[4] __attribute__((aligned(64)));

struct abi_record {
    u8 tag;
    u16 value;
    u32 wide;
};

enum abi_state {
    ABI_ZERO,
    ABI_LARGE = 300
};

static u16 recursive_sum(u16 value)
{
    return value == 0u ? 0u : (u16)(value + recursive_sum((u16)(value - 1u)));
}

static u16 switch_value(u16 value)
{
    switch (value) {
    case 0: return 11u;
    case 2: return 29u;
    case 7: return 47u;
    case 19: return 83u;
    default: return 101u;
    }
}

void eic_handler(void)
{
    EIC_INTFLAG = 1u << 3;
}

void tc3_handler(void)
{
    TC3_INTFLAG = 1u << 4;
    TC3_CTRLA = 0u;
    ++timer_interrupts;
}

static void uart_write(const char *text)
{
    while (*text != '\0') {
        while ((SERCOM0_INTFLAG & 1u) == 0u) {
        }
        SERCOM0_DATA = (u8)*text;
        ++text;
    }
}

int main(void)
{
    u32 failures = 0u;
    struct abi_record record = { 9u, 0x1234u, 0x89abcdefu };
    enum abi_state state = ABI_LARGE;

    failures |= (u32)((sizeof(char) != 1u) << 0);
    failures |= (u32)((sizeof(short) != 2u) << 1);
    failures |= (u32)((sizeof(int) != 4u) << 2);
    failures |= (u32)((sizeof(long) != 4u) << 3);
    failures |= (u32)((sizeof(void *) != 4u) << 4);
    failures |= (u32)((recursive_sum(12u) != 78u) << 5);
    failures |= (u32)(((dividend / 37u) != 2702u) << 6);
    failures |= (u32)(((dividend % 37u) != 26u) << 7);
    failures |= (u32)((switch_value(switch_input) != 47u) << 8);
    failures |= (u32)((record.tag != 9u || record.value != 0x1234u ||
                       record.wide != 0x89abcdefu) << 9);
    failures |= (u32)(((int)state != 300) << 10);

    PM_APBCMASK |=
        (1u << 2) | (1u << 3) | (1u << 4) | (1u << 5) | (1u << 8) | (1u << 9) |
        (1u << 10) | (1u << 11) | (1u << 12) | (1u << 13) | (1u << 16) |
        (1u << 17) | (1u << 18) | (1u << 20);
    PM_APBAMASK |= 1u << 5;
    GCLK_CLKCTRL = (u16)((0x14u << 8) | 0u | (1u << 14));

    EVSYS_CTRL = 1u << 4;
    EVSYS_CHANNEL = 2u | (0x36u << 16) | (1u << 26);
    EVSYS_USER = 0x13u | (3u << 8);
    EVSYS_INTENSET = 1u << (8u + 2u);
    EVSYS_CHANNEL_HALF = 2u | (1u << 8);
    if ((EVSYS_CHANNEL & ((0x7fu << 16) | (3u << 24) | (3u << 26) | 0xfu)) !=
        (2u | (0x36u << 16) | (1u << 26))) {
        failures |= 1u << 14;
    }
    if ((EVSYS_USER & 0x1fu) != 0x13u || (EVSYS_USER & (0x1fu << 8)) != (3u << 8)) {
        failures |= 1u << 15;
    }
    if ((EVSYS_INTFLAG & (1u << (8u + 2u))) == 0u) {
        failures |= 1u << 16;
    }
    EVSYS_INTFLAG = 1u << (8u + 2u);
    if ((EVSYS_INTFLAG & (1u << (8u + 2u))) != 0u) {
        failures |= 1u << 17;
    }

    USB_CTRLA = 1u << 1;
    USB_CTRLB = 0u;
    USB_DADD = 0x85u;
    USB_DESCADD = 0x20000103u;
    USB_EPCFG0 = 1u | (1u << 4);
    USB_EPSTATUSSET0 = (1u << 6) | (1u << 4);
    USB_EPINTENSET0 = 1u | (1u << 4);
    USB_INTENSET = 1u << 3;
    if (USB_CTRLA != (1u << 1) || USB_CTRLB != 0u || USB_DADD != 0x85u ||
        USB_STATUS != 0x40u || USB_DESCADD != 0x20000103u) {
        failures |= 1u << 18;
    }
    if (USB_EPCFG0 != (1u | (1u << 4)) || USB_EPSTATUS0 != ((1u << 6) | (1u << 4)) ||
        USB_EPINTENSET0 != (1u | (1u << 4)) || (USB_INTENSET & (1u << 3)) == 0u) {
        failures |= 1u << 19;
    }
    USB_EPSTATUSCLR0 = 1u << 6;
    if (USB_EPSTATUS0 != (1u << 4)) {
        failures |= 1u << 20;
    }

    dmac_descriptor.btctrl = (u16)((1u << 10) | (1u << 11) | (1u << 3) | 1u);
    dmac_descriptor.btcnt = 4u;
    dmac_descriptor.srcaddr = (u32)(dmac_source + 4);
    dmac_descriptor.dstaddr = (u32)dmac_destination;
    dmac_descriptor.descaddr = 0u;
    DMAC_BASEADDR = (u32)&dmac_descriptor;
    DMAC_WRBADDR = (u32)dmac_writeback;
    DMAC_CHID = 0u;
    DMAC_CHINTENSET = 1u << 1;
    DMAC_CHCTRLA = 1u << 1;
    DMAC_CTRL = (u16)((1u << 8) | (1u << 1));
    DMAC_SWTRIGCTRL = 1u;
    while ((DMAC_CHINTFLAG & (1u << 1)) == 0u) {
    }
    failures |= (u32)((dmac_destination[0] != 0xd4u || dmac_destination[1] != 0xc3u ||
                       dmac_destination[2] != 0xb2u || dmac_destination[3] != 0xa1u) << 21);
    failures |= (u32)((DMAC_CHSTATUS & 0x06u) != 0u) << 22;

    I2S_CLKCTRL0 = (1u << 7) | (1u << 5) | (1u << 2) | 1u;
    I2S_SERCTRL0 = (4u << 8) | 1u;
    I2S_CTRLA = (1u << 4) | (1u << 2) | (1u << 1);
    I2S_INTENSET = 1u << 8;
    failures |= (u32)((I2S_SYNCBUSY != 0u) << 23);
    failures |= (u32)((I2S_INTFLAG & (1u << 8)) == 0u) << 24;
    I2S_DATA0 = 0x12345678u;
    failures |= (u32)((I2S_DATA0 != 0x12345678u) << 25);
    I2S_INTFLAG = 1u << 8;

    PORT_DIRSET = 1u << 7;
    PORT_OUTSET = 1u << 7;

    EIC_CONFIG0 = 1u << (3u * 4u);
    EIC_INTENSET = 1u << 3;
    EIC_CTRLA = 1u << 1;

    SERCOM0_CTRLA = (1u << 2) | 1u;
    SERCOM0_CTRLB = 1u << 17;
    SERCOM0_CTRLA |= 1u << 1;
    uart_write("SAMD21\n");

    /* Exercise the native SERCOM0 SPI and I2C host register paths before returning to USART. */
    SERCOM0_CTRLA = 3u << 2;
    SERCOM0_CTRLB = 1u << 17;
    SERCOM0_CTRLA |= 1u << 1;
    SERCOM0_DATA = 0x3cu;
    while ((SERCOM0_INTFLAG & (1u << 2)) == 0u) {
    }
    failures |= (u32)((SERCOM0_DATA != 0x3cu) << 11);

    SERCOM0_CTRLA = 5u << 2;
    SERCOM0_CTRLB = 1u << 8;
    SERCOM0_CTRLA |= 1u << 1;
    SERCOM0_ADDR = 0xa0u;
    failures |= (u32)((SERCOM0_INTFLAG & 1u) == 0u) << 12;
    SERCOM0_DATA = 0x10u;
    SERCOM0_CTRLB = 2u << 16;
    failures |= (u32)((SERCOM0_STATUS & (3u << 4)) != (1u << 4)) << 13;

    SERCOM0_CTRLA = 1u << 2;
    SERCOM0_CTRLA |= 1u << 1;

    ADC_REFCTRL = 0u;
    ADC_INPUTCTRL = 3u;
    ADC_INTENSET = 1u;
    ADC_CTRLA = 1u << 1;
    ADC_SWTRIG = 1u << 1;
    failures |= (u32)((ADC_INTFLAG & 1u) == 0u) << 26;
    failures |= (u32)((ADC_RESULT != 0u) << 27);
    ADC_INTFLAG = 1u;

    AC_CTRLA = 1u << 1;
    AC_COMPCTRL0 = (1u << 5) | (1u << 1) | 1u;
    AC_CTRLB = 1u;
    failures |= (u32)((AC_STATUSB & 1u) == 0u) << 28;
    failures |= (u32)((AC_STATUSA & 1u) != 0u) << 29;

    DAC_CTRLB = 1u;
    DAC_CTRLA = 1u << 1;
    DAC_DATA = 0x2a5u;

    SERCOM1_CTRLA = 3u << 2;
    SERCOM1_CTRLB = 1u << 17;
    SERCOM1_CTRLA |= 1u << 1;
    SERCOM1_DATA = 0x51u;
    SERCOM2_CTRLA = 3u << 2;
    SERCOM2_CTRLB = 1u << 17;
    SERCOM2_CTRLA |= 1u << 1;
    SERCOM2_DATA = 0x52u;
    SERCOM3_CTRLA = 3u << 2;
    SERCOM3_CTRLB = 1u << 17;
    SERCOM3_CTRLA |= 1u << 1;
    SERCOM3_DATA = 0x53u;
    failures |= (u32)((SERCOM1_DATA != 0x51u || SERCOM2_DATA != 0x52u ||
                       SERCOM3_DATA != 0x53u) << 30);

    TC4_CC0 = 4u;
    TC4_INTENSET = 1u << 4;
    TC4_CTRLA = 1u << 1;
    TC5_CC0 = 5u;
    TC5_INTENSET = 1u << 4;
    TC5_CTRLA = 1u << 1;
    failures |= (u32)((TC4_CC0 != 4u || TC5_CC0 != 5u) << 31);

    TCC0_PER = 31u;
    TCC0_CC0 = 8u;
    TCC0_INTENSET = 1u << 16;
    TCC0_CTRLA = 1u << 1;
    RTC_COUNT = 0u;
    RTC_COMP0 = 64u;
    RTC_INTENSET = 1u;
    RTC_CTRL = 1u << 1;

    TC3_CC0 = 8u;
    TC3_INTENSET = 1u << 4;
    NVIC_ISER0 = 1u << 18;
    TC3_CTRLA = 1u << 1;
    while (timer_interrupts == 0u) {
    }
    while ((EIC_INTFLAG & (1u << 3)) == 0u) {
    }
    if ((PORT_IN & (1u << 3)) == 0u) {
        return 2;
    }
    EIC_INTFLAG = 1u << 3;
    PORT_OUTCLR = 1u << 7;
    return (int)failures;
}
