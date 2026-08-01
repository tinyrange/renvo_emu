#pragma once
#include "common_utils.h"
#define MAX_INTENISTY 100U
#define TIMER_PIN GPT_IO_PIN_GTIOCB
fsp_err_t set_intensity(uint32_t raw_count, uint8_t pin);
