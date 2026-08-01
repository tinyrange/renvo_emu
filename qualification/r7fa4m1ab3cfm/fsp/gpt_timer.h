#pragma once
#include "common_utils.h"

#define GPT_MAX_PERCENT 100U
#define BUF_SIZE 16U
#define PERIODIC_MODE_TIMER 1U
#define PWM_MODE_TIMER 2U
#define ONE_SHOT_MODE_TIMER 3U
#define INITIAL_VALUE '\0'
#define PERIODIC_MODE 1U
#define PWM_MODE 2U
#define ONE_SHOT_MODE 3U
#define TIMER_PIN GPT_IO_PIN_GTIOCB

fsp_err_t init_gpt_timer(timer_ctrl_t * const ctrl, timer_cfg_t const * const cfg, uint8_t mode);
fsp_err_t start_gpt_timer(timer_ctrl_t * const ctrl);
fsp_err_t set_timer_duty_cycle(uint8_t percent);
uint32_t process_input_data(void);
void deinit_gpt_timer(timer_ctrl_t * const ctrl);
void print_timer_menu(void);
