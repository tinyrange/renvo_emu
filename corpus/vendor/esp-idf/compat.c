#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include "esp_chip_info.h"
#include "esp_flash.h"
#include "esp_system.h"

#define REG32(address) (*(volatile uint32_t *)(address))

static void put_character(char character) { REG32(0x60000000u) = (unsigned char)character; }

static void put_unsigned(uint32_t value, unsigned int base)
{
    char digits[16];
    unsigned int count = 0;
    do {
        unsigned int digit = value % base;
        digits[count++] = (char)(digit < 10 ? '0' + digit : 'a' + digit - 10);
        value /= base;
    } while (value != 0u);
    while (count != 0u) put_character(digits[--count]);
}

int printf(const char *format, ...)
{
    va_list arguments;
    va_start(arguments, format);
    int written = 0;
    while (*format) {
        if (*format != '%') { put_character(*format++); ++written; continue; }
        ++format;
        while (*format >= '0' && *format <= '9') ++format;
        if (*format == 's') {
            const char *text = va_arg(arguments, const char *);
            while (*text) { put_character(*text++); ++written; }
        } else if (*format == 'd') {
            int value = va_arg(arguments, int);
            if (value < 0) { put_character('-'); value = -value; }
            put_unsigned((uint32_t)value, 10);
        } else if (*format == 'u') put_unsigned(va_arg(arguments, uint32_t), 10);
        else if (*format == 'x') put_unsigned(va_arg(arguments, uint32_t), 16);
        else if (*format == '%') put_character('%');
        if (*format) ++format;
    }
    va_end(arguments);
    return written;
}

int puts(const char *text)
{
    while (*text) put_character(*text++);
    put_character('\n');
    return 0;
}
int fflush(FILE *stream) { (void)stream; return 0; }
void vTaskDelay(unsigned int ticks) { (void)ticks; }
void esp_chip_info(esp_chip_info_t *information)
{
    information->features = CHIP_FEATURE_WIFI_BGN | CHIP_FEATURE_BLE;
    information->cores = 1;
    information->revision = 100;
}
esp_err_t esp_flash_get_size(void *chip, uint32_t *size)
{
    (void)chip; *size = 4u * 1024u * 1024u; return ESP_OK;
}
uint32_t esp_get_minimum_free_heap_size(void) { return 262144u; }
void esp_restart(void)
{
    REG32(0xfffffff0u) = 0u;
    for (;;) {}
}
