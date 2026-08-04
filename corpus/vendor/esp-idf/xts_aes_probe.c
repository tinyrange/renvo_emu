#include <stdint.h>

#include "soc/system_reg.h"

#define READ32(address) (*(volatile uint32_t *)(uintptr_t)(address))
#define WRITE32(address, value) (READ32(address) = (uint32_t)(value))
#define XTS_REG(offset) (DR_REG_EXT_MEM_ENC + (offset))

/* ESP32-S3 TRM v1.8, chapter 23 manual-encryption register block. ESP-IDF
 * exposes the vendor base address but intentionally has no public XTS header. */
#define XTS_AES_LINESIZE_REG XTS_REG(0x40)
#define XTS_AES_DESTINATION_REG XTS_REG(0x44)
#define XTS_AES_PHYSICAL_ADDRESS_REG XTS_REG(0x48)
#define XTS_AES_TRIGGER_REG XTS_REG(0x4c)
#define XTS_AES_RELEASE_REG XTS_REG(0x50)
#define XTS_AES_DESTROY_REG XTS_REG(0x54)
#define XTS_AES_STATE_REG XTS_REG(0x58)
#define XTS_AES_DATE_REG XTS_REG(0x5c)

__attribute__((noreturn, section(".text.start"))) void _start(void)
{
    volatile uint32_t *const exit_code = (volatile uint32_t *)0xfffffff0u;
    uint32_t failure = 0;

    if (READ32(XTS_REG(0)) != 0u || READ32(XTS_AES_LINESIZE_REG) != 0u ||
        READ32(XTS_AES_STATE_REG) != 0u ||
        READ32(XTS_AES_DATE_REG) != 0x20200111u) {
        failure = 1;
    }

    WRITE32(XTS_AES_TRIGGER_REG, 1u);
    if (READ32(XTS_AES_STATE_REG) != 0u) {
        failure = 2;
    }
    WRITE32(SYSTEM_EXTERNAL_DEVICE_ENCRYPT_DECRYPT_CONTROL_REG,
            SYSTEM_ENABLE_SPI_MANUAL_ENCRYPT);
    WRITE32(XTS_AES_DESTINATION_REG, 0u);
    WRITE32(XTS_AES_PHYSICAL_ADDRESS_REG, 0u);
    WRITE32(XTS_AES_LINESIZE_REG, 0u);
    WRITE32(XTS_REG(0x00), 0x03020100u);
    WRITE32(XTS_REG(0x04), 0x07060504u);
    WRITE32(XTS_REG(0x08), 0x0b0a0908u);
    WRITE32(XTS_REG(0x0c), 0x0f0e0d0cu);
    WRITE32(XTS_AES_TRIGGER_REG, 1u);
    if (READ32(XTS_AES_STATE_REG) != 2u) {
        failure = 3;
    }
    WRITE32(XTS_AES_RELEASE_REG, 1u);
    if (READ32(XTS_AES_STATE_REG) != 3u) {
        failure = 4;
    }
    WRITE32(XTS_AES_DESTROY_REG, 1u);
    if (READ32(XTS_AES_STATE_REG) != 0u) {
        failure = 5;
    }

    *exit_code = failure;
    __asm__ volatile("break 0, 0");
    for (;;) {
    }
}
