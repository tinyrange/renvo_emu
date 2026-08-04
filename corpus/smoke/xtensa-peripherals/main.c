#include <stdint.h>

#define READ32(address) (*(volatile uint32_t *)(uintptr_t)(address))
#define WRITE32(address, value) (READ32(address) = (uint32_t)(value))
#define CHECK(address, mask, expected, code)                                      \
    do {                                                                           \
        if ((READ32(address) & (uint32_t)(mask)) != (uint32_t)(expected)) {        \
            failure = (code);                                                       \
        }                                                                           \
    } while (0)

/*
 * This probe is compiled by Espressif's xtensa-esp32s3-elf-gcc. It verifies
 * every peripheral brought in by the ESP32-S3 merge train through native
 * addresses and reset/version contracts. Device-level tests cover masks,
 * commands, interrupts, host injection, and reset behavior in greater depth.
 */
__attribute__((noreturn, section(".text.start"))) void _start(void)
{
    volatile uint32_t *const exit_code = (volatile uint32_t *)0xfffffff0u;
    uint32_t failure = 0;

    CHECK(0x60010000u, 0xffffffffu, 0x00000000u, 1);  /* UART1 */
    CHECK(0x6002e000u, 0xffffffffu, 0x00000000u, 2);  /* UART2 */
    CHECK(0x600130f8u, 0xffffffffu, 0x02201172u, 3);  /* I2C0 */
    CHECK(0x600270f8u, 0xffffffffu, 0x02201172u, 4);  /* I2C1 */
    CHECK(0x600240f0u, 0xffffffffu, 0x02101190u, 5);  /* SPI2 */
    CHECK(0x600250f0u, 0xffffffffu, 0x02101190u, 6);  /* SPI3 */
    CHECK(0x6000f080u, 0xffffffffu, 0x02009070u, 7);  /* I2S0 */
    CHECK(0x6002d080u, 0xffffffffu, 0x02009070u, 8);  /* I2S1 */
    CHECK(0x600160ccu, 0xffffffffu, 0x02101181u, 9);  /* RMT */
    CHECK(0x600190fcu, 0xffffffffu, 0x19040200u, 10); /* LEDC */
    CHECK(0x600170fcu, 0xffffffffu, 0x19072601u, 11); /* PCNT */
    CHECK(0x6001e124u, 0xffffffffu, 0x02107230u, 12); /* MCPWM0 */
    CHECK(0x6002c124u, 0xffffffffu, 0x02107230u, 13); /* MCPWM1 */
    CHECK(0x6002b000u, 0x0000000fu, 0x00000001u, 14); /* TWAI */
    CHECK(0x6003f40cu, 0xffffffffu, 0x02101180u, 15); /* GDMA */
    CHECK(0x600403fcu, 0xffffffffu, 0x02101180u, 16); /* SAR ADC */
    CHECK(0x600089fcu, 0xffffffffu, 0x02101180u, 17); /* temperature */
    CHECK(0x600410fcu, 0xffffffffu, 0x02003020u, 18); /* LCD/CAM */
    CHECK(0x6002806cu, 0xffffffffu, 0x3430322au, 19); /* SD/MMC */
    CHECK(0x6003b02cu, 0xffffffffu, 0x20190402u, 20); /* SHA */
    CHECK(0x6003a0b4u, 0xffffffffu, 0x00000000u, 21); /* AES */
    CHECK(0x600071fcu, 0xffffffffu, 0x02101290u, 22); /* eFuse */
    CHECK(0x6003e1fcu, 0xffffffffu, 0x20190402u, 23); /* HMAC */
    CHECK(0x6003c830u, 0xffffffffu, 0x20190425u, 24); /* RSA */
    CHECK(0x6003de20u, 0xffffffffu, 0x20191217u, 25); /* digital signature */
    CHECK(0x600081fcu, 0xffffffffu, 0x02101271u, 26); /* RTC control */
    CHECK(0x600c27fcu, 0xffffffffu, 0x02012300u, 27); /* interrupt matrix */
    CHECK(0x600090fcu, 0xffffffffu, 0x01907160u, 28); /* IO MUX */
    CHECK(0x60014084u, 0xffffffffu, 0x02010090u, 29); /* UHCI0 */
    WRITE32(0x50000120u, 0x52544353u);
    CHECK(0x60021120u, 0xffffffffu, 0x52544353u, 30); /* RTC slow alias */
    WRITE32(0x60021124u, 0x414c4941u);
    CHECK(0x50000124u, 0xffffffffu, 0x414c4941u, 31); /* reverse alias */

    *exit_code = failure;
    __asm__ volatile("break 0, 0");
    for (;;) {
    }
}
