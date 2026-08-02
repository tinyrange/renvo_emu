# STM32L432KC qualification

The pinned Arm GNU 13.2.Rel1 and Clang/LLD 18 lanes use the distinct Cortex-M4F Armv7E-M
profile. The smoke is compiled and executed with soft, softfp and hard-float
ABIs; disassembly must contain FPv4-SP-D16 operations. It validates vector
reset, Thumb-2/DSP/FPU compiler output, calls, stack, TIM2 interrupt handling,
external GPIO and USART2 output (`STM32L432\n`) in the documented 256 KiB flash
and 64 KiB SRAM map.

The functional register surface is RCC, FLASH/PWR startup state, GPIO A/B/C/H,
SYSCFG/EXTI, TIM2, USART2, and the seven-channel DMA1/DMA2 controllers. DMA
request selection uses the STM32L4 CSELR registers. Firmware and host tests can
run deterministic memory transfers with byte, halfword, or word widths,
incrementing/circular modes, half/complete/error flags, and NVIC routing for
all DMA channels. Peripheral request timing, arbitration, and exact clock/low-
power behaviour remain unsupported. VCD records GPIO, timer, UART and interrupt
signals.

The official lane copies STM32CubeL4's pinned NUCLEO-L432KC
`GPIO_IOToggle/Src/main.c` unchanged and supplies only documented startup/HAL
adapters. The source revision is
`a6fd67088a77dc546a00cef2aa67ac540abf9c9f`, the selected HAL/example source is
BSD-3-Clause, and the expected result is a rising PB3 edge. Run:

```sh
scripts/qualify-stm32l432kc.sh
```
