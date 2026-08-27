#include <stdint.h>

volatile uint32_t exception_count;
static volatile uint32_t atomic_word = 0x11223344u;
static volatile float float_left = 3.25f;
static volatile float float_right = 1.5f;
static volatile uint32_t abi_seed = 13u;
__attribute__((section(".drom")))
static volatile const uint32_t drom_value = 0x2468ace0u;

__attribute__((noinline, section(".irom")))
static uint32_t irom_probe(uint32_t value)
{
    return value ^ drom_value;
}

__attribute__((noinline)) static uint32_t leaf(uint32_t a, uint32_t b,
                                                uint32_t c, uint32_t d,
                                                uint32_t e, uint32_t f)
{
    return (a * 3u) + (b * 5u) + (c ^ d) + (e / 7u) + (f % 11u);
}

__attribute__((noinline)) static uint32_t middle(uint32_t seed)
{
    return leaf(seed, seed + 1u, seed + 2u, seed + 3u,
                seed * 17u, seed * 19u) ^ 0x5a5aa5a5u;
}

__attribute__((noinline)) static uint32_t windowed_abi_probe(uint32_t seed)
{
    uint32_t first = middle(seed);
    uint32_t second = middle(seed + 9u);
    return (first << 7) | (first >> 25) ^ second;
}

static int atomic_probe(void)
{
    uint32_t expected = 0x11223344u;
    int exchanged = __atomic_compare_exchange_n(
        &atomic_word, &expected, 0xa5a55a5au, 0,
        __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST);
    if (!exchanged || expected != 0x11223344u || atomic_word != 0xa5a55a5au) {
        return 0;
    }

    expected = 0x01020304u;
    exchanged = __atomic_compare_exchange_n(
        &atomic_word, &expected, 0xffffffffu, 0,
        __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST);
    return !exchanged && expected == 0xa5a55a5au && atomic_word == 0xa5a55a5au;
}

static int floating_point_probe(void)
{
    volatile float result = (float_left * float_right) + 2.125f;
    return result == 7.0f;
}

static void trigger_level_one_exception(void)
{
    uint32_t vector_base = 0x40378000u;
    uint32_t interrupt = 1u;
    __asm__ volatile(
        "wsr %0, vecbase\n"
        "wsr %1, intenable\n"
        "wsr %1, intset\n"
        "rsync\n"
        :
        : "a"(vector_base), "a"(interrupt)
        : "memory");
}

__attribute__((noreturn, section(".text.start"))) void _start(void)
{
    volatile uint32_t *const exit_code = (volatile uint32_t *)0xfffffff0u;
    uint32_t abi = windowed_abi_probe(abi_seed);
    int atomic_ok = atomic_probe();
    int float_ok = floating_point_probe();
    uint32_t mapped = irom_probe(0x13579bdfu);

    // Direct ELF qualification deliberately poisons unmaterialized BSS so
    // firmware cannot accidentally depend on startup code that was skipped.
    exception_count = 0u;
    trigger_level_one_exception();

    if (abi != 0x7f5aafe3u) {
        *exit_code = 1u;
    } else if (!atomic_ok) {
        *exit_code = 2u;
    } else if (!float_ok) {
        *exit_code = 3u;
    } else if (exception_count != 1u) {
        *exit_code = 4u;
    } else if (mapped != 0x373f373fu) {
        *exit_code = 5u;
    } else {
        *exit_code = 0u;
    }
    __asm__ volatile("break 0, 0");
    for (;;) {
    }
}
