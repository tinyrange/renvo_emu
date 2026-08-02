# ATmega328PB qualification

The compiler lane uses AVR-GCC 7.3.0 with Microchip ATmega Device Family Pack
3.6.299. The compiler is selected with `-mmcu=atmega328pb` and the pack's own
device specs, startup object, device library and `iom328pb.h`; it is never
compiled as the older ATmega328P.

The smoke firmware checks the native AVR ABI (`int` and data pointers are 16
bits), startup/data initialization, calls and recursion, switch lowering,
16-bit division helpers, volatile MMIO and interrupt prologues. Timer0 overflow,
Timer3 compare-A, Timer4 compare-A and a PORTB pin-change interrupt are serviced;
USART0 emits `AVR8-PB\n`, and an EEPROM byte is written and read back. VCD
contains PORT B/C/D plus explicit Timer0, Timer1, Timer2, Timer3, Timer4, USART0,
pin-change, external-interrupt, watchdog, ADC, and analog-comparator signals.

Implemented functionally: clock/power register storage, PORT B/C/D, INT0 and
pin-change groups, Timer0/1/2, Timer3/4 compare A and overflow, USART0/1 transmit,
SPI0, TWI0, ADC, EEPROM, watchdog reset, reset/vectors and AVR Harvard
program/data separation. The comparator exposes deterministic boolean AIN0/AIN1
host inputs, ACO state and selectable edge interrupts. Analog voltage/noise,
touch, Timer3/4 PWM and capture waveforms, second SPI/TWI controllers, and
cycle-accurate prescalers/serial timing remain outside this functional slice.

Run from the repository root:

```sh
scripts/qualify-atmega328pb.sh
```

The same command also compiles and runs avr-libc's official `stdiodemo/uart.c`
unchanged at pinned revision `3b40a25de8948396d0055565b791d80fbd02cab7`.
That file carries the Beer-Ware License Revision 42. A local link adapter only
provides bounded input/exit behavior; the unchanged UART logic must emit
`OFFICIAL\r\n` through USART0.
