#include <stdint.h>

static volatile float input_left = 3.5f;
static volatile float input_right = -1.25f;
static volatile float input_bias = 2.0f;
static volatile int32_t dsp_left = 300;
static volatile int32_t dsp_right = -7;

__attribute__((noinline)) static int32_t dsp_case(void) {
    int32_t result;
    int32_t left = dsp_left;
    int32_t right = dsp_right;
    __asm__ volatile("smlabb %0, %1, %2, %3"
                     : "=r"(result)
                     : "r"(left), "r"(right), "r"(11));
    return result;
}

__attribute__((noinline)) static int32_t float_case(void) {
    float result = input_left * input_right + input_bias;
    return result == -2.375f;
}

int main(void) {
    return (float_case() && dsp_case() == -2089) ? 0 : 1;
}
