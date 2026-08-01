#include <msp430.h>

extern int __real_main(void);

/*
 * The TI example specifies a physical jumper from P1.4/UCA0TXD to
 * P1.5/UCA0RXD. The qualification board supplies the same connection through
 * the eUSCI listen-mode loopback bit, without changing the vendor source.
 */
int __wrap_main(void)
{
    UCA0STATW |= UCLISTEN;
    return __real_main();
}
