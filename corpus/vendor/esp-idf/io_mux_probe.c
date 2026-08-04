#include <stdint.h>

#include "soc/io_mux_reg.h"

__attribute__((noreturn, section(".text.start"))) void _start(void)
{
    volatile uint32_t *const gpio2 = (volatile uint32_t *)IO_MUX_GPIO2_REG;
    volatile uint32_t *const date = (volatile uint32_t *)IO_MUX_DATE_REG;
    volatile uint32_t *const exit_code = (volatile uint32_t *)0xfffffff0u;
    const uint32_t config = FUN_PU | FUN_IE | (PIN_FUNC_GPIO << MCU_SEL_S);

    if (*date != IO_MUX_DATE_VERSION) {
        *exit_code = 1;
    } else {
        *gpio2 = config;
        *exit_code = (*gpio2 == config) ? 0 : 2;
    }
    __asm__ volatile("break 0, 0");
    for (;;) {
    }
}
