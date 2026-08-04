#include <stdint.h>

#include "soc/sensitive_reg.h"

__attribute__((noreturn, section(".text.start"))) void _start(void)
{
    volatile uint32_t *const exit_code = (volatile uint32_t *)0xfffffff0u;
    volatile uint32_t *const pif_lock =
        (volatile uint32_t *)SENSITIVE_CORE_0_PIF_PMS_CONSTRAIN_0_REG;
    volatile uint32_t *const pif_permissions =
        (volatile uint32_t *)SENSITIVE_CORE_0_PIF_PMS_CONSTRAIN_1_REG;
    const uint32_t uart_read_only =
        (0xff33cfffu & ~SENSITIVE_CORE_0_PIF_PMS_CONSTRAIN_WORLD_0_UART_M) |
        (2u << SENSITIVE_CORE_0_PIF_PMS_CONSTRAIN_WORLD_0_UART_S);

    if (*(volatile uint32_t *)SENSITIVE_DATE_REG != 0x02101280u) {
        *exit_code = 1;
    } else if (*pif_lock != 0 || *pif_permissions != 0xff33cfffu) {
        *exit_code = 2;
    } else if (*(volatile uint32_t *)SENSITIVE_EDMA_BOUNDARY_0_REG != 0 ||
               *(volatile uint32_t *)SENSITIVE_EDMA_BOUNDARY_1_REG != 0x2000u ||
               *(volatile uint32_t *)SENSITIVE_EDMA_BOUNDARY_2_REG != 0x2000u ||
               *(volatile uint32_t *)SENSITIVE_EDMA_PMS_SPI2_REG != 0x0fu) {
        *exit_code = 3;
    } else {
        *pif_permissions = uart_read_only;
        *pif_lock = SENSITIVE_CORE_0_PIF_PMS_CONSTRAIN_LOCK;
        *pif_permissions = 0xff33cfffu;
        *pif_lock = 0;
        if (*pif_permissions != uart_read_only ||
            *pif_lock != SENSITIVE_CORE_0_PIF_PMS_CONSTRAIN_LOCK) {
            *exit_code = 4;
        } else {
            *exit_code = 0;
        }
    }
    __asm__ volatile("break 0, 0");
    for (;;) {
    }
}
