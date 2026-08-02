# ATmega328PB qualification

The compiler lane uses AVR-GCC 7.3.0 with Microchip ATmega Device Family Pack
3.6.299. The compiler is selected with `-mmcu=atmega328pb` and the pack's own
device specs, startup object, device library and `iom328pb.h`; it is never
compiled as the older ATmega328P.

The smoke firmware checks the native AVR ABI (`int` and data pointers are 16
bits), startup/data initialization, calls and recursion, switch lowering,
16-bit division helpers, volatile MMIO and interrupt prologues. Timer0 overflow
and a PORTB pin-change interrupt each toggle PB0. USART0 emits `AVR8-PB\n`, and
an EEPROM byte is written and read back. VCD contains PORT B/C/D plus explicit
Timer0, Timer1, USART0, pin-change, external-interrupt, watchdog, and
analog-comparator output signals.

Implemented functionally: clock/power register storage, PORT B/C/D, INT0 and
pin-change group 0, Timer0 overflow, Timer1 compare A, USART0 transmit, EEPROM,
watchdog reset, reset/vectors and AVR Harvard program/data separation. The
analog comparator exposes deterministic boolean AIN0/AIN1 host inputs, ACO
state, selectable toggle/falling/rising ACI interrupt modes, and a VCD output.
It does not model analog voltages, the bandgap reference, synchronization
delay, ADC coupling, touch, SPI or TWI. Timer prescalers and serial bit timing
are deterministic approximations.

Run from the repository root:

```sh
scripts/qualify-atmega328pb.sh
```

The same command also compiles and runs avr-libc's official `stdiodemo/uart.c`
unchanged at pinned revision `3b40a25de8948396d0055565b791d80fbd02cab7`.
That file carries the Beer-Ware License Revision 42. A local link adapter only
provides bounded input/exit behavior; the unchanged UART logic must emit
`OFFICIAL\r\n` through USART0.
