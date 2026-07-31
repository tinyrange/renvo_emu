int main(void)
{
    __asm__ volatile(".word 0xffffffff");
    return 0;
}
