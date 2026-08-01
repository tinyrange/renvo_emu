#pragma once

typedef unsigned char uint8_t;
typedef unsigned int uint32_t;
typedef unsigned long long uint64_t;
typedef int int32_t;
typedef unsigned int size_t;
typedef int bool;

#define true 1
#define false 0
#define NULL ((void *)0)
#define RESET_VALUE 0U
#define FSP_SUCCESS 0
#define FSP_INVALID_ARGUMENT 1
#define FSP_ERR_TRANSFER_ABORTED 2
#define FSP_ERR_TIMEOUT 3
#define UINT16_MAX 65535U
#define UINT32_MAX 4294967295U
#define GPT_IO_PIN_GTIOCB 1U
#define TIMER_EVENT_CYCLE_END 1U

typedef int fsp_err_t;
typedef int timer_direction_t;
typedef struct { uint32_t marker; } timer_ctrl_t;
typedef struct { uint32_t source_div; } timer_cfg_t;
typedef struct {
    timer_direction_t count_direction;
    uint32_t period_counts;
    uint32_t clock_frequency;
} timer_info_t;
typedef struct { uint32_t event; } timer_callback_args_t;

typedef struct { uint32_t marker; } uart_ctrl_t;
typedef struct { uint32_t marker; } uart_cfg_t;
typedef struct { uint32_t event; uint32_t data; } uart_callback_args_t;

#define UART_EVENT_TX_COMPLETE 1U
#define UART_EVENT_RX_CHAR 2U
#define UART_EVENT_BREAK_DETECT 4U
#define UART_EVENT_ERR_OVERFLOW 8U
#define UART_EVENT_ERR_FRAMING 16U
#define UART_EVENT_ERR_PARITY 32U

extern timer_ctrl_t g_timer_pwm_ctrl;
extern timer_ctrl_t g_timer_periodic_ctrl;
extern timer_ctrl_t g_timer_one_shot_mode_ctrl;
extern timer_cfg_t g_timer_pwm_cfg;
extern timer_cfg_t g_timer_periodic_cfg;
extern timer_cfg_t g_timer_one_shot_mode_cfg;
extern uart_ctrl_t g_uart_ctrl;
extern uart_cfg_t g_uart_cfg;

fsp_err_t R_GPT_Open(timer_ctrl_t *ctrl, timer_cfg_t const *cfg);
fsp_err_t R_GPT_Start(timer_ctrl_t *ctrl);
fsp_err_t R_GPT_Close(timer_ctrl_t *ctrl);
fsp_err_t R_GPT_InfoGet(timer_ctrl_t *ctrl, timer_info_t *info);
fsp_err_t R_GPT_DutyCycleSet(timer_ctrl_t *ctrl, uint32_t counts, uint8_t pin);
fsp_err_t R_SCI_UART_Open(uart_ctrl_t *ctrl, uart_cfg_t const *cfg);
fsp_err_t R_SCI_UART_Write(uart_ctrl_t *ctrl, uint8_t const *bytes, uint32_t length);
fsp_err_t R_SCI_UART_Close(uart_ctrl_t *ctrl);
void user_uart_callback(uart_callback_args_t *args);

size_t strlen(char const *text);
void *memset(void *destination, int value, size_t length);
int atoi(char const *text);

#define APP_PRINT(...) ((void)0)
#define APP_ERR_PRINT(...) ((void)0)
#define APP_CHECK_DATA 0
#define APP_READ(buffer) (0U)
