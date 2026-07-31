#define PIO_REG(offset) (*(volatile unsigned int *)(0x50200000u + (offset)))

int main(void)
{
    PIO_REG(0x048u) = 0xe001u; /* SET PINS, 1 */
    PIO_REG(0x04cu) = 0xe000u; /* SET PINS, 0 */
    PIO_REG(0x0ccu) = 1u << 12; /* WRAP_TOP=1, WRAP_BOTTOM=0 */
    PIO_REG(0x0dcu) = (1u << 26) | (25u << 5); /* one SET pin at GPIO25 */
    PIO_REG(0x000u) = 1u; /* enable state machine 0 */
    for (volatile unsigned int ticks = 0; ticks < 16u; ++ticks) {
    }
    PIO_REG(0x000u) = 0u;
    return 0;
}
