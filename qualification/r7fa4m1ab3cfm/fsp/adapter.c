#include "common_utils.h"

#define REG8(address) (*(volatile uint8_t *)(address))
#define REG32(address) (*(volatile uint32_t *)(address))
#define GPT0_GTCR REG32(0x4007802cu)
#define GPT0_GTINTAD REG32(0x40078038u)
#define GPT0_GTPR REG32(0x40078064u)
#define SCI9_SCR REG8(0x40070122u)
#define SCI9_TDR REG8(0x40070123u)

timer_ctrl_t g_timer_pwm_ctrl;
timer_ctrl_t g_timer_periodic_ctrl;
timer_ctrl_t g_timer_one_shot_mode_ctrl;
timer_cfg_t g_timer_pwm_cfg;
timer_cfg_t g_timer_periodic_cfg;
timer_cfg_t g_timer_one_shot_mode_cfg;
uart_ctrl_t g_uart_ctrl;
uart_cfg_t g_uart_cfg;

fsp_err_t R_GPT_Open(timer_ctrl_t *ctrl, timer_cfg_t const *cfg)
{
    (void)ctrl;
    (void)cfg;
    GPT0_GTPR = 7;
    GPT0_GTINTAD = 1u << 6;
    return FSP_SUCCESS;
}

fsp_err_t R_GPT_Start(timer_ctrl_t *ctrl)
{
    (void)ctrl;
    GPT0_GTCR = 1;
    return FSP_SUCCESS;
}

fsp_err_t R_GPT_Close(timer_ctrl_t *ctrl)
{
    (void)ctrl;
    GPT0_GTCR = 0;
    return FSP_SUCCESS;
}

fsp_err_t R_GPT_InfoGet(timer_ctrl_t *ctrl, timer_info_t *info)
{
    (void)ctrl;
    info->period_counts = 8;
    return FSP_SUCCESS;
}

fsp_err_t R_GPT_DutyCycleSet(timer_ctrl_t *ctrl, uint32_t counts, uint8_t pin)
{
    (void)ctrl;
    (void)counts;
    (void)pin;
    return FSP_SUCCESS;
}

fsp_err_t R_SCI_UART_Open(uart_ctrl_t *ctrl, uart_cfg_t const *cfg)
{
    (void)ctrl;
    (void)cfg;
    SCI9_SCR = 1u << 5;
    return FSP_SUCCESS;
}

fsp_err_t R_SCI_UART_Write(uart_ctrl_t *ctrl, uint8_t const *bytes, uint32_t length)
{
    (void)ctrl;
    for (uint32_t index = 0; index < length; ++index) {
        SCI9_TDR = bytes[index];
    }
    uart_callback_args_t args = {UART_EVENT_TX_COMPLETE, 0};
    user_uart_callback(&args);
    return FSP_SUCCESS;
}

fsp_err_t R_SCI_UART_Close(uart_ctrl_t *ctrl)
{
    (void)ctrl;
    return FSP_SUCCESS;
}

size_t strlen(char const *text)
{
    size_t length = 0;
    while (text[length]) { ++length; }
    return length;
}

void *memset(void *destination, int value, size_t length)
{
    uint8_t *bytes = destination;
    for (size_t index = 0; index < length; ++index) { bytes[index] = (uint8_t)value; }
    return destination;
}

int atoi(char const *text)
{
    int value = 0;
    while (*text >= '0' && *text <= '9') { value = value * 10 + *text++ - '0'; }
    return value;
}

fsp_err_t set_intensity(uint32_t raw_count, uint8_t pin)
{
    (void)raw_count;
    (void)pin;
    return FSP_SUCCESS;
}
