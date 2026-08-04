#define REG32(address) (*(volatile unsigned int *)(address))

#define USB_BASE 0x60080000u
#define GAHBCFG 0x08u
#define GINTSTS 0x14u
#define GINTMSK 0x18u
#define GRXSTSP 0x20u
#define DIEPMSK 0x810u
#define DOEPMSK 0x814u
#define DAINTMSK 0x81cu
#define RX_FIFO 0x1000u
#define DOEPINT0 0xb08u
#define RESET_EVENTS ((1u << 12) | (1u << 13))

static int wait_for(unsigned int mask)
{
    for (unsigned int attempt = 0; attempt < 8000u; ++attempt) {
        if ((REG32(USB_BASE + GINTSTS) & mask) == mask) {
            return 0;
        }
    }
    return 1;
}

__attribute__((noreturn, section(".text.start"))) void _start(void)
{
    REG32(USB_BASE + GAHBCFG) = 1u;
    REG32(USB_BASE + GINTMSK) = RESET_EVENTS | (1u << 4) | (1u << 18) | (1u << 19);
    REG32(USB_BASE + DIEPMSK) = 1u;
    REG32(USB_BASE + DOEPMSK) = 1u << 3;
    REG32(USB_BASE + DAINTMSK) = 1u | (1u << 16);

    int result = wait_for(RESET_EVENTS);
    if (result == 0) {
        REG32(USB_BASE + GINTSTS) = RESET_EVENTS;
        result = wait_for(1u << 4);
    }
    if (result == 0) {
        unsigned int status = REG32(USB_BASE + GRXSTSP);
        unsigned int packet_status = (status >> 17) & 0xfu;
        unsigned int byte_count = (status >> 4) & 0x7ffu;
        unsigned int setup_word0 = REG32(USB_BASE + RX_FIFO);
        unsigned int setup_word1 = REG32(USB_BASE + RX_FIFO);
        unsigned int complete = REG32(USB_BASE + GRXSTSP);
        if ((status & 0xfu) != 0u || packet_status != 6u || byte_count != 8u
            || setup_word0 != 0x01000680u || setup_word1 != 0x00120000u
            || complete != (4u << 17)) {
            result = 2;
        }
    }
    if (result == 0) {
        if ((REG32(USB_BASE + GINTSTS) & (1u << 19)) == 0u) {
            result = 3;
        } else {
            REG32(USB_BASE + DOEPINT0) = 1u << 3;
        }
    }

    REG32(0xfffffff0u) = (unsigned int)result;
    __asm__ volatile("break 0, 0");
    for (;;) {
    }
}
