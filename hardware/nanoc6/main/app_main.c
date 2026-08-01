#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>

#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

#define REMU_CASES(X) \
    X(0000) X(0025) X(0050) X(0075) X(0100) X(0125) X(0150) X(0175) \
    X(0200) X(0225) X(0250) X(0275) X(0300) X(0325) X(0350) X(0375) \
    X(0400) X(0425) X(0450) X(0475) X(0500) X(0525) X(0550) X(0575) \
    X(0600) X(0625) X(0650) X(0675) X(0700) X(0725) X(0750) X(0775) \
    X(0800) X(0825) X(0850) X(0875) X(0900) X(0925) X(0950) X(0975)

#define DECLARE_CASE(id) extern uint32_t remu_case_##id(void);
REMU_CASES(DECLARE_CASE)

struct remu_case_entry {
    const char *name;
    uint32_t (*run)(void);
};

#define CASE_ENTRY(id) { "case_" #id, remu_case_##id },
static const struct remu_case_entry CASES[] = {
    REMU_CASES(CASE_ENTRY)
};

volatile uint32_t remu_hw_ready;
volatile uint32_t remu_hw_results[sizeof(CASES) / sizeof(CASES[0])];

void app_main(void)
{
    setvbuf(stdout, NULL, _IONBF, 0);
    uint32_t run = 0;

    for (;;) {
        ++run;
        printf("REMU_HW_BEGIN %" PRIu32 " %zu\n", run, sizeof(CASES) / sizeof(CASES[0]));
        for (size_t index = 0; index < sizeof(CASES) / sizeof(CASES[0]); ++index) {
            const uint32_t result = CASES[index].run();
            remu_hw_results[index] = result;
            printf("REMU_HW %s %08" PRIx32 "\n", CASES[index].name, result);
        }
        remu_hw_ready = 0x52454e56;
        printf("REMU_HW_END %" PRIu32 "\n", run);
        vTaskDelay(pdMS_TO_TICKS(2000));
    }
}
