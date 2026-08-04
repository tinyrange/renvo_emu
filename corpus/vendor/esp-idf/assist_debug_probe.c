#include <stdint.h>

#include "soc/assist_debug_reg.h"

__attribute__((noreturn, section(".text.start"))) void _start(void)
{
    volatile uint32_t *const exit_code = (volatile uint32_t *)0xfffffff0u;
    volatile uint32_t *const watched = (volatile uint32_t *)0x3fc88000u;
    volatile uint32_t *const trace = (volatile uint32_t *)0x3fc88100u;

    if (*(volatile uint32_t *)ASSIST_DEBUG_REG_DATE_REG != 0x02003040u) {
        *exit_code = 1;
    } else if (*(volatile uint32_t *)ASSIST_DEBUG_CORE_0_AREA_DRAM0_0_MIN_REG != 0xffffffffu ||
               *(volatile uint32_t *)ASSIST_DEBUG_CORE_1_SP_MAX_REG != 0xffffffffu ||
               *(volatile uint32_t *)ASSIST_DEBUG_LOG_SETTING_REG != ASSIST_DEBUG_LOG_MEM_LOOP_ENABLE) {
        *exit_code = 2;
    } else {
        *(volatile uint32_t *)ASSIST_DEBUG_CORE_0_AREA_DRAM0_0_MIN_REG = (uint32_t)(uintptr_t)watched;
        *(volatile uint32_t *)ASSIST_DEBUG_CORE_0_AREA_DRAM0_0_MAX_REG = (uint32_t)(uintptr_t)watched + 3u;
        *(volatile uint32_t *)ASSIST_DEBUG_CORE_0_RCD_PDEBUGENABLE_REG = 1;
        *(volatile uint32_t *)ASSIST_DEBUG_CORE_0_RCD_RECORDING_REG = 1;
        *(volatile uint32_t *)ASSIST_DEBUG_CORE_0_INTERRUPT_ENA_REG =
            ASSIST_DEBUG_CORE_0_AREA_DRAM0_0_WR_ENA;
        *(volatile uint32_t *)ASSIST_DEBUG_LOG_MIN_REG = (uint32_t)(uintptr_t)watched;
        *(volatile uint32_t *)ASSIST_DEBUG_LOG_MAX_REG = (uint32_t)(uintptr_t)watched + 3u;
        *(volatile uint32_t *)ASSIST_DEBUG_LOG_MEM_START_REG = (uint32_t)(uintptr_t)trace;
        *(volatile uint32_t *)ASSIST_DEBUG_LOG_MEM_END_REG = (uint32_t)(uintptr_t)trace + 31u;
        *(volatile uint32_t *)ASSIST_DEBUG_LOG_SETTING_REG =
            ASSIST_DEBUG_LOG_MEM_LOOP_ENABLE | (4u << ASSIST_DEBUG_LOG_ENA_S);
        *watched = 0xa55a5aa5u;
        *(volatile uint32_t *)ASSIST_DEBUG_CORE_0_RCD_RECORDING_REG = 0;
        if ((*(volatile uint32_t *)ASSIST_DEBUG_CORE_0_INTERRUPT_RAW_REG &
             ASSIST_DEBUG_CORE_0_AREA_DRAM0_0_WR_RAW) == 0) {
            *exit_code = 3;
        } else if (*(volatile uint32_t *)ASSIST_DEBUG_CORE_0_AREA_PC_REG == 0) {
            *exit_code = 4;
        } else if (trace[1] != (uint32_t)(uintptr_t)watched || trace[3] != 0xa55a5aa5u) {
            *exit_code = 5;
        } else {
            *(volatile uint32_t *)ASSIST_DEBUG_CORE_0_INTERRUPT_RLS_REG =
                ASSIST_DEBUG_CORE_0_AREA_DRAM0_0_WR_RLS;
            *(volatile uint32_t *)ASSIST_DEBUG_CORE_0_INTERRUPT_CLR_REG =
                ASSIST_DEBUG_CORE_0_AREA_DRAM0_0_WR_CLR;
            *exit_code = (*(volatile uint32_t *)ASSIST_DEBUG_CORE_0_INTERRUPT_RAW_REG == 0) ? 0 : 6;
        }
    }
    __asm__ volatile("break 0, 0");
    for (;;) {
    }
}
