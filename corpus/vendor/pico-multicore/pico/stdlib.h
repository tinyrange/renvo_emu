#ifndef REMU_PICO_MULTICORE_STDLIB_H
#define REMU_PICO_MULTICORE_STDLIB_H

#include <stdbool.h>
#include <stdint.h>

typedef unsigned int uint;

void stdio_init_all(void);
void tight_loop_contents(void);

#endif
