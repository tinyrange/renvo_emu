#include "candidate.h"

#ifndef REMU_SOURCE_TRIGGER
#define REMU_SOURCE_TRIGGER 0
#endif
#ifndef REMU_FLAG_TRIGGER
#define REMU_FLAG_TRIGGER 0
#endif

__attribute__((noreturn, section(".text.start"))) void _start(void)
{
    volatile unsigned int *const exit_code =
        (volatile unsigned int *)0xfffffff0u;
    *exit_code = REMU_SOURCE_TRIGGER && REMU_FLAG_TRIGGER &&
                 REMU_INPUT_SUM == 7u;
    for (;;) {
    }
}
