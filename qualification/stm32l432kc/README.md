# STM32L432KC qualification

The pinned Arm GNU 13.2.Rel1 and Clang/LLD 18 lanes use the distinct Cortex-M4F Armv7E-M
profile. The smoke is compiled and executed with soft, softfp and hard-float
ABIs; disassembly must contain FPv4-SP-D16 operations. It validates vector
reset, Thumb-2/DSP/FPU compiler output, calls, stack, TIM2 interrupt handling,
external GPIO and USART2 output (`STM32L432\n`) in the documented 256 KiB flash
and 64 KiB SRAM map.

The functional register surface is RCC, FLASH/PWR startup state, GPIO A/B/C/H,
SYSCFG/EXTI, TIM2 and USART2. Analog, USB, DMA, exact clocks and low-power
timing are unsupported. VCD records GPIO, timer, UART and interrupt signals.

The official lane copies STM32CubeL4's pinned NUCLEO-L432KC
`GPIO_IOToggle/Src/main.c` unchanged and supplies only documented startup/HAL
adapters. The source revision is
`a6fd67088a77dc546a00cef2aa67ac540abf9c9f`, the selected HAL/example source is
BSD-3-Clause, and the expected result is a rising PB3 edge. Run:

```sh
scripts/qualify-stm32l432kc.sh
```

The machine also exposes the STM32L432 QUADSPI register block at
`0xA0001000` and a deterministic 16 MiB external NOR window at `0x90000000`.
Indirect reads and one-to-zero NOR programming through `DR`, transfer/status
flags, the IRQ 71 route, and memory-mapped reads are covered by the
`remu-machines` unit gate. Dual-flash, DMA, and serial timing remain outside
this functional slice.
