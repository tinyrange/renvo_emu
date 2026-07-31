#include "candidate.h"

#ifndef RENVO_SOURCE_TRIGGER
#define RENVO_SOURCE_TRIGGER 0
#endif
#ifndef RENVO_FLAG_TRIGGER
#define RENVO_FLAG_TRIGGER 0
#endif

__attribute__((noreturn, section(".text.start"))) void _start(void)
{
    volatile unsigned int *const exit_code =
        (volatile unsigned int *)0xfffffff0u;
    *exit_code = RENVO_SOURCE_TRIGGER && RENVO_FLAG_TRIGGER &&
                 RENVO_INPUT_SUM == 7u;
    for (;;) {
    }
}
