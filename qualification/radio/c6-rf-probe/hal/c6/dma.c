/* SPDX-License-Identifier: Apache-2.0 */
#include "dma.h"

void c6_dma_publish(void)
{
    __asm__ volatile("fence rw, rw" ::: "memory");
}

void c6_dma_tx_descriptor(struct c6_dma_descriptor *descriptor,
                          const void *buffer)
{
    descriptor->control = 0xc0000000u;
    descriptor->buffer = (uint32_t)(uintptr_t)buffer;
    descriptor->next = 0;
    c6_dma_publish();
}

void c6_dma_rx_descriptor(struct c6_dma_descriptor *descriptor,
                          void *buffer, uint32_t capacity)
{
    descriptor->control = (1u << 31) | (capacity << 14) | capacity;
    descriptor->buffer = (uint32_t)(uintptr_t)buffer;
    descriptor->next = 0;
    c6_dma_publish();
}
