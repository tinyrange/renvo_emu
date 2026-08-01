#pragma once
#include "common_utils.h"

#define CARRIAGE_ASCII 13U
#define ZERO_ASCII 48U
#define NINE_ASCII 57U
#define DATA_LENGTH 4U
#define UART_ERROR_EVENTS (UART_EVENT_BREAK_DETECT | UART_EVENT_ERR_OVERFLOW | UART_EVENT_ERR_FRAMING | UART_EVENT_ERR_PARITY)

fsp_err_t uart_ep_demo(void);
fsp_err_t uart_print_user_msg(uint8_t *message);
fsp_err_t uart_initialize(void);
void deinit_uart(void);
