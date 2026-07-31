int main(void)
{
    __asm__ volatile(".hword 0xde00"); /* UDF #0 */
    return 0;
}
