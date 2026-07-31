#include <stdint.h>

/* Keep one compiler input for each RP2350 Hazard3 extension family. The
 * generated assembly is an artifact even when GCC chooses a base instruction;
 * extensions.S supplies explicit executable opcodes for the CPU proof. */
uint32_t hazard3_compiler_cases(uint32_t left, uint32_t right) {
    uint32_t rotated = (left << (right & 31u)) | (left >> ((-right) & 31u));
    uint32_t selected = left & ~(1u << (right & 31u));
    uint32_t address = left + (right << 2u);
    return rotated ^ selected ^ address ^ __builtin_popcount(left);
}
