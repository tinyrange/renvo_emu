#define REG32(address) (*(volatile unsigned int *)(address))

#define ADC_STATR   REG32(0x40012400u)
#define ADC_CTLR1   REG32(0x40012404u)
#define ADC_CTLR2   REG32(0x40012408u)
#define ADC_RSQR3   REG32(0x40012434u)
#define TKEY_CHG    REG32(0x4001243cu)
#define TKEY_DISCHG REG32(0x4001244cu)
#define TKEY_DR     REG32(0x4001244cu)

int main(void)
{
    ADC_CTLR2 = 1u;                 /* ADON */
    ADC_CTLR1 = (1u << 24);         /* TKENABLE */
    ADC_RSQR3 = 3u;                 /* SQ1 = touch channel 3 */
    TKEY_CHG = 4u;
    TKEY_DISCHG = 5u;               /* starts the sample */
    while ((ADC_STATR & (1u << 1)) == 0u) {
    }
    return TKEY_DR == 0x0800u ? 0 : 1;
}
