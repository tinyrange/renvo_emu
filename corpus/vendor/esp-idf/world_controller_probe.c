#include <stdint.h>

#include "soc/world_controller_reg.h"

static volatile uint32_t message;
static volatile uint32_t failure;

__attribute__((noreturn, noinline)) static void nmi_unmasked(void)
{
    volatile uint32_t *const exit_code = (volatile uint32_t *)0xfffffff0u;
    if (*(volatile uint32_t *)WCL_CORE_0_NMI_MASK_PHASE_REG != 0u) {
        failure = 5;
    }
    *exit_code = failure;
    __asm__ volatile("break 0, 0");
    for (;;) {
    }
}

__attribute__((noreturn, noinline)) static void secure_entry(void)
{
    if (*(volatile uint32_t *)WCL_CORE_0_WORLD_IRAM0_REG != 1u ||
        *(volatile uint32_t *)WCL_CORE_0_WORLD_DRAM0_PIF_REG != 1u ||
        *(volatile uint32_t *)WCL_CORE_0_MESSAGE_PHASE_REG != 1u ||
        *(volatile uint32_t *)WCL_CORE_0_STATUSTABLE1_REG != 0x21u ||
        *(volatile uint32_t *)WCL_CORE_0_STATUSTABLE_CURRENT_REG != 2u) {
        failure = 3;
    }

    *(volatile uint32_t *)WCL_CORE_0_NMI_MASK_TRIGGER_ADDR_REG =
        (uint32_t)(uintptr_t)nmi_unmasked;
    *(volatile uint32_t *)WCL_CORE_0_NMI_MASK_DISABLE_REG = 0u;
    *(volatile uint32_t *)WCL_CORE_0_NMI_MASK_ENABLE_REG = 0u;
    if (*(volatile uint32_t *)WCL_CORE_0_NMI_MASK_PHASE_REG != 1u) {
        failure = 4;
    }
    nmi_unmasked();
}

__attribute__((noreturn, noinline)) static void world1_entry(void)
{
    if (*(volatile uint32_t *)WCL_CORE_0_WORLD_IRAM0_REG != 2u ||
        *(volatile uint32_t *)WCL_CORE_0_WORLD_DRAM0_PIF_REG != 2u ||
        *(volatile uint32_t *)WCL_CORE_0_WORLD_PHASE_REG != 0u) {
        failure = 2;
    }
    message = 0;
    message = 1;
    message = 2;
    message = 3;
    secure_entry();
}

__attribute__((noreturn, section(".text.start"))) void _start(void)
{
    volatile uint32_t *const exit_code = (volatile uint32_t *)0xfffffff0u;
    failure = 0;

    if (*(volatile uint32_t *)WCL_CORE_0_ENTRY_CHECK_REG != 2u ||
        *(volatile uint32_t *)WCL_CORE_1_ENTRY_CHECK_REG != 2u) {
        *exit_code = 1;
        __asm__ volatile("break 0, 0");
    }

    *(volatile uint32_t *)WCL_CORE_0_ENTRY_1_ADDR_REG =
        (uint32_t)(uintptr_t)secure_entry;
    *(volatile uint32_t *)WCL_CORE_0_ENTRY_CHECK_REG = 2u;
    *(volatile uint32_t *)WCL_CORE_0_MESSAGE_ADDR_REG =
        (uint32_t)(uintptr_t)&message;
    *(volatile uint32_t *)WCL_CORE_0_MESSAGE_MAX_REG = 3u;
    *(volatile uint32_t *)WCL_CORE_0_WORLD_TRIGGER_ADDR_REG =
        (uint32_t)(uintptr_t)world1_entry;
    *(volatile uint32_t *)WCL_CORE_0_WORLD_PREPARE_REG = 2u;
    *(volatile uint32_t *)WCL_CORE_0_WORLD_UPDATE_REG = 0u;
    __asm__ volatile("memw" ::: "memory");
    world1_entry();
}
