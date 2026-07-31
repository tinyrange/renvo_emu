#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>
#include <setjmp.h>

void *memcpy(void *destination, const void *source, size_t length)
{
    unsigned char *out = destination;
    const unsigned char *in = source;
    while (length-- != 0) {
        *out++ = *in++;
    }
    return destination;
}

void *memmove(void *destination, const void *source, size_t length)
{
    unsigned char *out = destination;
    const unsigned char *in = source;
    if (out < in) {
        return memcpy(destination, source, length);
    }
    while (length-- != 0) {
        out[length] = in[length];
    }
    return destination;
}

void *memset(void *destination, int value, size_t length)
{
    unsigned char *out = destination;
    while (length-- != 0) {
        *out++ = (unsigned char)value;
    }
    return destination;
}

int memcmp(const void *left, const void *right, size_t length)
{
    const unsigned char *a = left;
    const unsigned char *b = right;
    while (length-- != 0) {
        if (*a != *b) {
            return (int)*a - (int)*b;
        }
        ++a;
        ++b;
    }
    return 0;
}

size_t strlen(const char *text)
{
    const char *end = text;
    while (*end != '\0') {
        ++end;
    }
    return (size_t)(end - text);
}

int strcmp(const char *left, const char *right)
{
    while (*left != '\0' && *left == *right) {
        ++left;
        ++right;
    }
    return (unsigned char)*left - (unsigned char)*right;
}

int abs(int value)
{
    return value < 0 ? -value : value;
}

double fabs(double value)
{
    union {
        double number;
        uint64_t bits;
    } converted = { .number = value };
    converted.bits &= UINT64_C(0x7fffffffffffffff);
    return converted.number;
}

double copysign(double magnitude, double sign)
{
    union {
        double number;
        uint64_t bits;
    } left = { .number = magnitude }, right = { .number = sign };
    left.bits = (left.bits & UINT64_C(0x7fffffffffffffff))
        | (right.bits & UINT64_C(0x8000000000000000));
    return left.number;
}

char *strchr(const char *text, int character)
{
    do {
        if (*text == (char)character) {
            return (char *)text;
        }
    } while (*text++ != '\0');
    return NULL;
}

static unsigned char fallback_heap[32 * 1024];
static size_t fallback_heap_used;

void *malloc(size_t length)
{
    length = (length + 7) & ~(size_t)7;
    if (length > sizeof(fallback_heap) - fallback_heap_used) {
        return NULL;
    }
    void *result = fallback_heap + fallback_heap_used;
    fallback_heap_used += length;
    return result;
}

void free(void *pointer)
{
    (void)pointer;
}

void *realloc(void *pointer, size_t length)
{
    if (pointer == NULL) {
        return malloc(length);
    }
    return NULL;
}

int printf(const char *format, ...)
{
    (void)format;
    return 0;
}

int snprintf(char *buffer, size_t length, const char *format, ...)
{
    (void)format;
    if (length != 0) {
        buffer[0] = '\0';
    }
    return 0;
}

/*
 * MQuickJS uses this recovery point only to escape a fatal parse/runtime
 * allocation error.  The qualification workload must not take that path:
 * turn it into the harness's explicit failure exit instead of pulling a
 * target-specific C library into otherwise identical freestanding images.
 */
__attribute__((returns_twice)) int setjmp(jmp_buf environment)
{
    (void)environment;
    return 0;
}

__attribute__((noreturn)) void longjmp(jmp_buf environment, int value)
{
    (void)environment;
    (void)value;
    *(volatile uint32_t *)0xfffffff0u = 98;
    for (;;) {
    }
}

__attribute__((noreturn)) void abort(void)
{
    *(volatile uint32_t *)0xfffffff0u = 99;
    for (;;) {
    }
}
