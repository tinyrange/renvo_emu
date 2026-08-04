#include <stdint.h>

#include "soc/peri_backup_reg.h"

__attribute__((noreturn, section(".text.start"))) void _start(void)
{
    volatile uint32_t *const exit_code = (volatile uint32_t *)0xfffffff0u;
    volatile uint32_t *const rtc_store0 = (volatile uint32_t *)0x60008050u;
    volatile uint32_t *const retained = (volatile uint32_t *)0x3fc88000u;

    if (*(volatile uint32_t *)PERI_BACKUP_CONFIG_REG != 0x00006480u) {
        *exit_code = 1;
    } else if (*(volatile uint32_t *)PERI_BACKUP_DATE_REG != 0x02012300u) {
        *exit_code = 2;
    } else {
        *rtc_store0 = 0x50424b31u;
        *(volatile uint32_t *)PERI_BACKUP_APB_ADDR_REG = (uint32_t)(uintptr_t)rtc_store0;
        *(volatile uint32_t *)PERI_BACKUP_MEM_ADDR_REG = (uint32_t)(uintptr_t)retained;
        *(volatile uint32_t *)PERI_BACKUP_INT_ENA_REG = PERI_BACKUP_DONE_INT_ENA;
        *(volatile uint32_t *)PERI_BACKUP_CONFIG_REG =
            PERI_BACKUP_ENA | PERI_BACKUP_TO_MEM | PERI_BACKUP_START |
            (1u << PERI_BACKUP_SIZE_S);
        if (*retained != 0x50424b31u) {
            *exit_code = 3;
        } else if ((*(volatile uint32_t *)PERI_BACKUP_INT_ST_REG &
                    PERI_BACKUP_DONE_INT_ST) == 0) {
            *exit_code = 4;
        } else {
            *(volatile uint32_t *)PERI_BACKUP_INT_CLR_REG = PERI_BACKUP_DONE_INT_CLR;
            *exit_code = (*(volatile uint32_t *)PERI_BACKUP_INT_ST_REG == 0) ? 0 : 5;
        }
    }
    __asm__ volatile("break 0, 0");
    for (;;) {
    }
}
