#include "core_portme.h"

#include <stdarg.h>

static void put_character(char character)
{
    *(volatile ee_u8 *)0xffff0100u = (ee_u8)character;
}

static int put_string(const char *text)
{
    int written = 0;
    while (*text != '\0') {
        put_character(*text++);
        ++written;
    }
    return written;
}

static int put_unsigned(ee_u32 value, ee_u32 radix, int width, char pad)
{
    char digits[16];
    int count = 0;
    int written = 0;
    do {
        ee_u32 digit = value % radix;
        digits[count++] = (char)(digit < 10u ? '0' + digit : 'a' + digit - 10u);
        value /= radix;
    } while (value != 0u);
    while (width > count) {
        put_character(pad);
        --width;
        ++written;
    }
    while (count != 0) {
        put_character(digits[--count]);
        ++written;
    }
    return written;
}

int ee_printf(const char *format, ...)
{
    va_list arguments;
    int written = 0;
    va_start(arguments, format);
    while (*format != '\0') {
        if (*format != '%') {
            put_character(*format++);
            ++written;
            continue;
        }
        ++format;
        char pad = ' ';
        int width = 0;
        if (*format == '0') {
            pad = '0';
            ++format;
        }
        while (*format >= '0' && *format <= '9') {
            width = width * 10 + (*format++ - '0');
        }
        int is_long = *format == 'l';
        if (is_long) {
            ++format;
        }
        switch (*format++) {
        case '%':
            put_character('%');
            ++written;
            break;
        case 'c':
            put_character((char)va_arg(arguments, int));
            ++written;
            break;
        case 's':
            written += put_string(va_arg(arguments, const char *));
            break;
        case 'd': {
            ee_s32 value = is_long
                ? (ee_s32)va_arg(arguments, long)
                : (ee_s32)va_arg(arguments, int);
            if (value < 0) {
                put_character('-');
                ++written;
                written += put_unsigned(0u - (ee_u32)value, 10u, width, pad);
            } else {
                written += put_unsigned((ee_u32)value, 10u, width, pad);
            }
            break;
        }
        case 'u':
            written += put_unsigned(
                is_long ? (ee_u32)va_arg(arguments, unsigned long)
                        : va_arg(arguments, unsigned int),
                10u,
                width,
                pad);
            break;
        case 'x':
            written += put_unsigned(
                is_long ? (ee_u32)va_arg(arguments, unsigned long)
                        : va_arg(arguments, unsigned int),
                16u,
                width,
                pad);
            break;
        default:
            put_character('?');
            ++written;
            break;
        }
    }
    va_end(arguments);
    return written;
}
