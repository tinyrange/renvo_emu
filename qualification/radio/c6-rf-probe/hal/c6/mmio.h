/* SPDX-License-Identifier: Apache-2.0 */
#ifndef REMU_C6_MMIO_H
#define REMU_C6_MMIO_H

#include <stdint.h>

static inline uint32_t c6_read32(uint32_t address)
{
    return *(volatile uint32_t *)(uintptr_t)address;
}

static inline void c6_write32(uint32_t address, uint32_t value)
{
    *(volatile uint32_t *)(uintptr_t)address = value;
}

#endif
