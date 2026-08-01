extern void isa_vectors(void);

void main(void)
{
    /* Keep the assembler psect reachable; qualification never calls it. */
    if (*(volatile unsigned char *)0x70u == 0xffu) {
        isa_vectors();
    }
    for (;;) {
    }
}
