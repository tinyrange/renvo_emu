#include "Arduino.h"

typedef volatile unsigned char reg8;
typedef volatile unsigned int reg32;

#define P111PFS (*(reg32 *)0x4004086cu)
#define PORT1_PCNTR3 (*(reg32 *)0x40040028u)
#define SCI9_SCR (*(reg8 *)0x40070122u)
#define SCI9_TDR (*(reg8 *)0x40070123u)

RenvoSerial Serial;
RenvoSerial Serial1;

void pinMode(unsigned int pin, unsigned int mode)
{
    if (pin == LED_BUILTIN && mode == OUTPUT) {
        P111PFS = (1u << 2);
    }
}

void digitalWrite(unsigned int pin, unsigned int value)
{
    if (pin == LED_BUILTIN) {
        PORT1_PCNTR3 = value ? (1u << 11) : (1u << (11u + 16u));
    }
}

void delay(unsigned long milliseconds)
{
    (void)milliseconds;
}

void RenvoSerial::begin(unsigned long baud)
{
    (void)baud;
    if (this == &Serial1) {
        hardware = 1;
        SCI9_SCR = 1u << 5;
    }
}

int RenvoSerial::available()
{
    return this == &Serial && !input_consumed;
}

int RenvoSerial::read()
{
    input_consumed = 1;
    return 'H';
}

size_t RenvoSerial::write(int value)
{
    if (hardware) {
        SCI9_TDR = (uint8_t)value;
    }
    return 1;
}
