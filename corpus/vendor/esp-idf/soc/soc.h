#pragma once

#include <stdint.h>

#define BIT(number) (1U << (number))
#define DR_REG_IO_MUX_BASE 0x60009000U
#define DR_REG_UHCI0_BASE 0x60014000U
#define DR_REG_SYSCON_BASE 0x60026000U
#define DR_REG_USB_WRAP_BASE 0x60039000U
#define REG_UHCI_BASE(instance) (DR_REG_UHCI0_BASE - (instance) * 0x8000U)
