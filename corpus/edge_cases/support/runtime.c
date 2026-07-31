typedef unsigned int u32;
typedef unsigned char u8;

/*
 * Minimal freestanding runtime support. Some GCC targets lower an unoptimized
 * structure copy to memcpy even with no hosted C library linked.
 */
void *memcpy(void *destination, const void *source, u32 length)
{
    u8 *output = destination;
    const u8 *input = source;
    while (length != 0u) {
        *output++ = *input++;
        --length;
    }
    return destination;
}

void *memset(void *destination, int value, u32 length)
{
    u8 *output = destination;
    while (length != 0u) {
        *output++ = (u8)value;
        --length;
    }
    return destination;
}

void *memmove(void *destination, const void *source, u32 length)
{
    u8 *output = destination;
    const u8 *input = source;
    if (output < input) {
        while (length != 0u) {
            *output++ = *input++;
            --length;
        }
    } else {
        while (length != 0u) {
            --length;
            output[length] = input[length];
        }
    }
    return destination;
}

/*
 * Clang follows the Arm EABI and may select these helpers for aligned aggregate
 * initialization and copies. Keep the bodies explicit and volatile so the
 * freestanding compiler cannot lower them back into calls to themselves.
 */
__attribute__((noinline))
void __aeabi_memclr4(void *destination, u32 length)
{
    volatile u8 *output = destination;
    while (length != 0u) {
        *output++ = 0u;
        --length;
    }
}

__attribute__((noinline))
void __aeabi_memclr8(void *destination, u32 length)
{
    __aeabi_memclr4(destination, length);
}

__attribute__((noinline))
void __aeabi_memclr(void *destination, u32 length)
{
    __aeabi_memclr4(destination, length);
}

__attribute__((noinline))
void __aeabi_memcpy4(void *destination, const void *source, u32 length)
{
    volatile u8 *output = destination;
    const volatile u8 *input = source;
    while (length != 0u) {
        *output++ = *input++;
        --length;
    }
}

__attribute__((noinline))
void __aeabi_memcpy8(void *destination, const void *source, u32 length)
{
    __aeabi_memcpy4(destination, source, length);
}

__attribute__((noinline))
void __aeabi_memcpy(void *destination, const void *source, u32 length)
{
    __aeabi_memcpy4(destination, source, length);
}

__attribute__((noinline))
void __aeabi_memmove(void *destination, const void *source, u32 length)
{
    memmove(destination, source, length);
}

__attribute__((noinline))
void __aeabi_memmove4(void *destination, const void *source, u32 length)
{
    memmove(destination, source, length);
}

__attribute__((noinline))
void __aeabi_memmove8(void *destination, const void *source, u32 length)
{
    memmove(destination, source, length);
}
