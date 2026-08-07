#ifndef REMU_PICO_MULTICORE_H
#define REMU_PICO_MULTICORE_H

#include <stdint.h>

void multicore_launch_core1(void (*entry)(void));
void multicore_fifo_push_blocking(uint32_t value);
uint32_t multicore_fifo_pop_blocking(void);

#endif
