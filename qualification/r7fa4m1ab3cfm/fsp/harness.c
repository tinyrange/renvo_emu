#include "common_utils.h"
#include "gpt_timer.h"
#include "uart_ep.h"

#define REG32(address) (*(volatile uint32_t *)(address))
#define GPT0_GTST REG32(0x4007803cu)
#define P111PFS REG32(0x4004086cu)
#define PORT1_PCNTR3 REG32(0x40040028u)

int main(void)
{
    if (init_gpt_timer(&g_timer_periodic_ctrl, &g_timer_periodic_cfg, PERIODIC_MODE_TIMER)) {
        return 1;
    }
    if (start_gpt_timer(&g_timer_periodic_ctrl)) {
        return 2;
    }
    while ((GPT0_GTST & (1u << 6)) == 0) {
    }
    if (uart_initialize()) {
        return 3;
    }
    if (uart_print_user_msg((uint8_t *)"FSP GPT SCI\n")) {
        return 4;
    }
    P111PFS = 1u << 2;
    PORT1_PCNTR3 = 1u << 11;
    return 0;
}
