#include "debug.h"
#include <stdarg.h>

#define REG32(address) (*(volatile u32 *)(address))

u32 SystemCoreClock = 48000000u;
static u32 delay_count;

static void uart_putc(unsigned char byte)
{
    REG32(0x40013804u) = byte;
}

void NVIC_PriorityGroupConfig(u32 group) { (void)group; }
void SystemCoreClockUpdate(void) { SystemCoreClock = 48000000u; }
void Delay_Init(void) {}
void SDI_Printf_Enable(void) {}
void USART_Printf_Init(u32 baud)
{
    (void)baud;
    REG32(0x40021018u) |= 1u << 14;
    REG32(0x40013808u) = 0x01a1u;
    REG32(0x4001380cu) = (1u << 13) | (1u << 3);
}
u32 DBGMCU_GetCHIPID(void) { return 0x00300500u; }

void RCC_APB2PeriphClockCmd(u32 peripheral, FunctionalState state)
{
    if (state == ENABLE) REG32(0x40021018u) |= peripheral;
}

void GPIO_Init(void *gpio, GPIO_InitTypeDef *configuration)
{
    volatile u32 *base = (volatile u32 *)gpio;
    u32 shift = 0;
    while (((configuration->GPIO_Pin >> shift) & 1u) == 0u) ++shift;
    base[0] = (base[0] & ~(0xfu << (shift * 4u))) | (1u << (shift * 4u));
}

void GPIO_WriteBit(void *gpio, u16 pin, BitAction action)
{
    volatile u32 *base = (volatile u32 *)gpio;
    base[action == Bit_SET ? 4 : 5] = pin;
}

void Delay_Ms(u32 milliseconds)
{
    (void)milliseconds;
    if (++delay_count == 3u) REG32(0xfffffff0u) = 0u;
}

int printf(const char *format, ...)
{
    va_list arguments;
    va_start(arguments, format);
    int written = 0;
    while (*format) {
        if (*format == '%') {
            ++format;
            while (*format >= '0' && *format <= '9') ++format;
            if (*format == 's') (void)va_arg(arguments, const char *);
            else if (*format == 'd' || *format == 'x' || *format == 'u') (void)va_arg(arguments, unsigned int);
            else if (*format == '%') { uart_putc('%'); ++written; }
            if (*format) ++format;
            continue;
        }
        uart_putc((unsigned char)*format++);
        ++written;
    }
    va_end(arguments);
    return written;
}
