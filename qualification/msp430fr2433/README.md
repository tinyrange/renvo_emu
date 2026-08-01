# MSP430FR2433 qualification

The compiler lane uses TI MSP430 GCC 9.3.1.11 and TI MSP430 support files
1.212. Firmware is selected with `-mmcu=msp430fr2433`. `-mhwmult=none` keeps
runtime arithmetic on the CPU because the frozen acceptance slice does not
include the optional memory-mapped hardware multiplier.

The smoke firmware checks the native MSP430 ABI (`int` and data pointers are
16 bits), startup and data initialization, calls and recursion, switch
lowering, 32-bit division helpers, volatile MMIO, CPUX instructions, interrupt
entry/return and FRAM persistence. GPIO edge input, Timer_A low-power wake, RTC
modulo overflow, eUSCI_A0 transmit and watchdog configuration are exercised.
The same source is
compiled at `-O0`, `-Os` and `-O2`; each binary emits `MSP430X-FR2433\n` and
halts with R12 equal to zero.

Implemented functionally: the CPUXv2 integer/interruption subset emitted by the
pinned toolchain, the 20-bit unified address space, reset vectors, FRAM and
SRAM, PM5 GPIO lock behavior, Ports 1–3, Port 1 edge interrupts, all four
Timer_A blocks (TA0/TA1 with CCR0..CCR2 and TA2/TA3 with CCR0..CCR1), TAxIV
arbitration, RTC modulo counting and overflow, eUSCI_A0 UART transmit/receive loopback,
WDT_A reset and persistent FRAM. Clock-tree, timer prescaling and UART bit
timing are deterministic approximations. Timer compare/overflow flags and
CCR0/A1 vectors are deterministic; capture pin routing, RTC calendar/alarm
features, analog peripherals and unlisted serial modes remain outside this
acceptance slice.

Run from the repository root:

```sh
scripts/qualify-msp430fr2433.sh
```

That command also acquires, hashes, compiles and runs the three unchanged TI
SLAC700 examples documented under [`slac700`](slac700/README.md). The archive
is version `01.00.00.0E`, SHA-256
`9d8b339b98949afd26a31609d16baed06257eeb06126043ac0a3eda09397e12d`;
its original TI notices are retained. Only startup/link and a UART loopback
wrapper replace board wiring. Expected results are the documented P1.0 GPIO
edges and incrementing eUSCI bytes.
