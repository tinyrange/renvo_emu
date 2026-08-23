#pragma once

typedef void *intr_handle_t;
typedef void (*intr_handler_t)(void *);

#define ESP_INTR_FLAG_LOWMED 0

static inline int esp_intr_alloc(int source, int flags, intr_handler_t handler, void *argument,
                                intr_handle_t *handle)
{
    (void)source;
    (void)flags;
    (void)handler;
    (void)argument;
    *handle = (void *)1;
    return 0;
}

static inline int esp_intr_free(intr_handle_t handle)
{
    (void)handle;
    return 0;
}
