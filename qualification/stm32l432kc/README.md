# STM32L432KC qualification

The pinned Arm GNU 13.2.Rel1 and Clang/LLD 18 lanes use the distinct Cortex-M4F Armv7E-M
profile. The smoke is compiled and executed with soft, softfp and hard-float
ABIs; disassembly must contain FPv4-SP-D16 operations. It validates vector
reset, Thumb-2/DSP/FPU compiler output, calls, stack, TIM2 interrupt handling,
external GPIO and USART2 output (`STM32L432\n`) in the documented 256 KiB flash
and 64 KiB SRAM map.

The functional register surface is RCC, FLASH/PWR startup state, GPIO A/B/C/H,
SYSCFG/EXTI, TIM1/2/6/7/15/16, LPTIM1/2, USART1/2, LPUART1, SPI1/3, I2C1/3,
IWDG, WWDG, ADC1, CRC, RTC, RNG, bxCAN, DAC1, USB FS/PMA, SAI1, QUADSPI,
SWPMI1, DMA1/2, TSC, COMP1/2 and OPAMP1. These are deterministic,
transaction-level models intended to execute and inspect firmware; they do
not claim cycle-, electrical-, or analog-level silicon fidelity. USART2
remains the compiled qualification fixture's output path. VCD records GPIO,
timer, serial, data-path, analog-result and interrupt signals.

Internal FLASH at `0x08000000` and its boot alias at `0x00000000` share one
non-volatile image. The L4 controller at `0x40022000` models KEYR unlock,
aligned 64-bit double-word programming (including paired 32-bit writes), 2 KiB
page erase, bank-one mass erase, NOR one-to-zero semantics, EOP/error flags,
controller reset locking, and direct firmware-image initialization. Operations
complete immediately; ECC correction, option-byte reload/reset effects,
write-protection enforcement, and physical program/erase timing are functional
limits rather than silicon-accurate behavior.

The official lane copies STM32CubeL4's pinned NUCLEO-L432KC
`GPIO_IOToggle/Src/main.c` unchanged and supplies only documented startup/HAL
adapters. The source revision is
`a6fd67088a77dc546a00cef2aa67ac540abf9c9f`, the selected HAL/example source is
BSD-3-Clause, and the expected result is a rising PB3 edge. Run:

```sh
scripts/qualify-stm32l432kc.sh
```

## Extended data-path and mixed-signal slice

The USB full-speed core at `0x40006800` and its PMA at `0x40006c00` model
endpoint/control registers, descriptor/data access, deterministic host reset
and OUT injection, IN extraction, and IRQ 67. The SAI1 blocks at `0x40015400`
model framing/slot configuration, enable/flush, host-injected receive samples,
captured transmit samples, FIFO status, and IRQ 74. USB PHY timing,
enumeration/class policy, audio clocks, codec synchronization, and serial-pin
waveforms are not modeled.

The QUADSPI controller at `0xA0001000` fronts a deterministic 16 MiB external
NOR window at `0x90000000`. Indirect reads, one-to-zero programming through
`DR`, transfer/status flags, IRQ 71, host image loading, and memory-mapped
reads are tested. Dual-flash operation and serial timing are outside this
slice. SWPMI1 at `0x40008800` models activation, frame-length/status registers,
host receive injection, captured transmit words, loopback, error/status
clearing, and IRQ 76 without physical single-wire timing.

DMA1 and DMA2 at `0x40020000` and `0x40020400` implement seven channels each,
the channel-selection register, byte/halfword/word transfers, increment,
direction, circular and memory-to-memory modes, half/complete/error flags, and
their native IRQ routes. The machine services transfers deterministically;
bus arbitration, request timing, and full peripheral handshake semantics are
not cycle accurate.

TSC at `0x40024000` consumes deterministic host-supplied group counts and
models acquisition completion/max-count state plus IRQ 77. COMP1/2 at
`0x40010200` compare host input codes, retain configuration/lock behavior, and
route output changes to IRQ 64. OPAMP1 at `0x40007800` models configuration,
gain, trimming storage, and a deterministic output code. These blocks expose
software-visible state only: capacitance, voltage, noise, propagation delay,
settling, calibration, and package-level analog routing are deliberately out
of scope.
