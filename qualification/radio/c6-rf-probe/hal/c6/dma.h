/* SPDX-License-Identifier: Apache-2.0 */
#ifndef REMU_C6_DMA_H
#define REMU_C6_DMA_H

#include <stdint.h>

struct c6_dma_descriptor {
    volatile uint32_t control;
    volatile uint32_t buffer;
    volatile uint32_t next;
};

void c6_dma_publish(void);
void c6_dma_tx_descriptor(struct c6_dma_descriptor *descriptor,
                          const void *buffer);
void c6_dma_rx_descriptor(struct c6_dma_descriptor *descriptor,
                          void *buffer, uint32_t capacity);

#endif
