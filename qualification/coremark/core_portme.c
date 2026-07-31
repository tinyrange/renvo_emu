#include "coremark.h"
#include "core_portme.h"

#if VALIDATION_RUN
volatile ee_s32 seed1_volatile = 0x3415;
volatile ee_s32 seed2_volatile = 0x3415;
volatile ee_s32 seed3_volatile = 0x66;
#elif PROFILE_RUN
volatile ee_s32 seed1_volatile = 0x8;
volatile ee_s32 seed2_volatile = 0x8;
volatile ee_s32 seed3_volatile = 0x8;
#else
volatile ee_s32 seed1_volatile = 0x0;
volatile ee_s32 seed2_volatile = 0x0;
volatile ee_s32 seed3_volatile = 0x66;
#endif

volatile ee_s32 seed4_volatile = ITERATIONS;
volatile ee_s32 seed5_volatile = 0;
ee_u32 default_num_contexts = 1;

static CORETIMETYPE start_ticks;
static CORETIMETYPE stop_ticks;

static CORETIMETYPE renvo_ticks(void)
{
    return *(volatile ee_u32 *)0xffff0200u;
}

void start_time(void)
{
    start_ticks = renvo_ticks();
}

void stop_time(void)
{
    stop_ticks = renvo_ticks();
}

CORE_TICKS get_time(void)
{
    return stop_ticks - start_ticks;
}

secs_ret time_in_secs(CORE_TICKS ticks)
{
    /*
     * One "second" in the emitted upstream report is one million Renvo
     * abstract instruction ticks. Documentation deliberately labels the
     * resulting metric as iterations/Mtick, not hardware CoreMark/s.
     */
    return ticks / 1000000u;
}

void portable_init(core_portable *portable, int *argc, char *argv[])
{
    (void)argc;
    (void)argv;
    if (sizeof(ee_ptr_int) != sizeof(ee_u8 *)) {
        ee_printf("ERROR! ee_ptr_int cannot hold a pointer\n");
    }
    if (sizeof(ee_u32) != 4u) {
        ee_printf("ERROR! ee_u32 is not 32 bits\n");
    }
    portable->portable_id = 1;
}

void portable_fini(core_portable *portable)
{
    portable->portable_id = 0;
}
