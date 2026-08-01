#include <stdint.h>
#include "definitions.h"

#define REG32(address) (*(volatile uint32_t *)(address))
#define PORT_DIRSET REG32(0x41004408u)
#define PORT_OUTCLR REG32(0x41004414u)
#define PORT_OUTSET REG32(0x41004418u)
#define PORT_IN REG32(0x41004420u)

void SYS_Initialize(void *data)
{
    (void)data;
    PORT_DIRSET = 1u << 7;
}

uint32_t renvo_switch_get(void)
{
    return (PORT_IN >> 3) & 1u;
}

void renvo_led_clear(void)
{
    PORT_OUTCLR = 1u << 7;
}

void renvo_led_set(void)
{
    PORT_OUTSET = 1u << 7;
}
