# New-target register contract

This is the behavioral contract for the first STM32F411RE, nRF52840, and
ESP32-P4 slices. Addresses are native byte addresses and all listed accesses
are 32-bit unless stated otherwise. “Stored” means masked software-visible
state only; it does not imply the physical clock, analog, bus, or pin-routing
effect of the silicon register.

## STM32F411RE

| Block | Base | Modeled registers and behavior |
|---|---:|---|
| TIM2 | `0x40000000` | `CR1 +0x00` CEN, `DIER +0x0c` UIE, `SR +0x10` UIF, `EGR +0x14` UG, `CNT +0x24`, `PSC +0x28`, `ARR +0x2c`; instruction-ordered update events and NVIC IRQ 28 |
| USART2 | `0x40004400` | `SR +0x00` reports TXE bit 7 and RXNE bit 5; `DR +0x04` transmits/receives one byte; other aligned registers, including `CR1 +0x0c`, are startup-compatible stored state; receive service maps to NVIC IRQ 38 |
| SYSCFG | `0x40013800` | `MEMRMP +0x00`, `PMC +0x04`, `EXTICR1-4 +0x08..+0x14` stored with documented masks; EXTI port muxing is not yet applied electrically |
| EXTI | `0x40013c00` | `IMR +0x00`, `EMR +0x04`, `RTSR +0x08`, `FTSR +0x0c`, `SWIER +0x10`, `PR +0x14`; GPIO edges latch pending bits, PR is write-one-to-clear, and lines 0-15 route to the native grouped NVIC lines |
| GPIOA/B/C/H | `0x40020000`, `0x40020400`, `0x40020800`, `0x40021c00` | `MODER +0x00`, `OTYPER +0x04`, `OSPEEDR +0x08`, `PUPDR +0x0c`, `IDR +0x10`, `ODR +0x14`, `BSRR +0x18`, `LCKR +0x1c`, `AFRL/H +0x20/+0x24`; MODER output mode drives the digital net, but AF selection is stored only |
| RCC | `0x40023800` | Reset/stored words at `CR +0x00=0x00000083`, `PLLCFGR +0x04=0x24003010`, `CFGR +0x08`, `CIR +0x0c`, `AHB1ENR +0x30`, `APB1ENR +0x40=0x00100000`, `APB2ENR +0x44`, `CSR +0x70=0x0e000000`, `SSCGR +0x74`; clock timing and gating are not enforced |
| FLASH interface | `0x40023c00` | Masked startup state for `ACR +0x00`, `KEYR +0x04`, `OPTKEYR +0x08`, `SR +0x0c`, `CR +0x10=0x80000000`, `OPTCR +0x14=0x0fffaaed`; program/erase and protection are not modeled |

Flash is executable at `0x08000000..0x0807ffff` and is mirrored at reset
address `0x00000000`; SRAM is `0x20000000..0x2001ffff`.

## nRF52840

| Block | Base | Modeled registers and behavior |
|---|---:|---|
| CLOCK/POWER | `0x40000000` | Stored startup/event words at offsets `0x100`, `0x104`, `0x108`, `0x40c`, `0x418`, `0x518`, and `0x51c`; these provide deterministic startup compatibility, not oscillator or power-state timing |
| UART0 | `0x40002000` | `TASKS_STARTRX +0x000`, `STOPRX +0x004`, `STARTTX +0x008`, `STOPTX +0x00c`; `EVENTS_RXDRDY +0x108`, `TXDRDY +0x11c`; `ENABLE +0x500` (legacy UART value 4); `PSELRTS/TXD/CTS/RXD +0x508..+0x514`; `RXD +0x518`, `TXD +0x51c`, `BAUDRATE +0x524`, `CONFIG +0x56c`. Tasks gate byte movement and events; IRQ 2 is reserved for the slice |
| TIMER0 | `0x40008000` | `TASKS_START +0x000`, `STOP +0x004`, `COUNT +0x008`, `CLEAR +0x00c`, `CAPTURE[0..3] +0x040..+0x04c`; `EVENTS_COMPARE[0] +0x140`; `SHORTS +0x200` compare0-clear/stop; `INTENSET/CLR +0x304/+0x308` bit 16; `MODE +0x504`, `BITMODE +0x508`, `PRESCALER +0x510`, `CC[0..3] +0x540..+0x54c`; compare0 routes to NVIC IRQ 8 |
| GPIO P0 | `0x50000500` | `OUT +0x04`, `OUTSET +0x08`, `OUTCLR +0x0c`, `IN +0x10`, `DIR +0x14`, `DIRSET +0x18`, `DIRCLR +0x1c`, `PIN_CNF[0..31] +0x200..+0x27c` |
| GPIO P1 | `0x50000800` | Same offsets for package pins 32-47; accesses to P1.16-P1.31 are rejected |

`PIN_CNF` stores DIR, INPUT, PULL, DRIVE, and SENSE fields under mask
`0x0007030f`; DIR controls the digital output driver. Flash is
`0x00000000..0x000fffff`, SRAM is `0x20000000..0x2003ffff`. RADIO/NFC and RF
state are not mapped, scheduled, or exercised.

## ESP32-P4

| Block | Base | Modeled registers and behavior |
|---|---:|---|
| TIMG0/1 | `0x500c2000`, `0x500c3000` | Timer 0 `CONFIG +0x00`, latched `LO/HI +0x04/+0x08`, `UPDATE +0x0c`, `ALARMLO/HI +0x10/+0x14`, `LOADLO/HI +0x18/+0x1c`, `LOAD +0x20`; watchdog/config/calibration state `+0x48..+0x6c`; interrupt enable/raw/status/clear `+0x70..+0x7c`. Counting is deterministic and instruction ordered, not clock accurate |
| UART0 | `0x500ca000` | `FIFO +0x00` byte transmit/receive and receive-count field in `STATUS +0x1c`; other aligned words are lenient startup-compatible storage |
| GPIO | `0x500e0000` | Low-bank `OUT/SET/CLR +0x04/+0x08/+0x0c`, high-bank `+0x10/+0x14/+0x18`; low-bank `ENABLE/SET/CLR +0x20/+0x24/+0x28`, high-bank `+0x2c/+0x30/+0x34`; `STRAP +0x38`, low/high `IN +0x3c/+0x40`; pins 0-54 are exposed as digital nets |
| HP system | `0x500e5000` | Offsets `+0x00` and `+0x04` stored; `DATE +0xffc=0x24071000` read-only compatibility value |
| HP clock/reset | `0x500e6000` | Offsets `+0x00` and `+0x04` stored; `DATE +0xffc=0x24071000` read-only compatibility value |

The memory slice maps TCM at `0x30100000..0x30101fff`, a bounded executable
IROM/DROM window at `0x40000000..0x40ffffff`, mask ROM at
`0x4fc00000..0x4fc1ffff`, L2 RAM at `0x4ff00000..0x4ffbffff`, and LP SRAM at
`0x50108000..0x5010ffff`. Native addressless binaries are rooted at
`0x40000000`. CPU1 is parked. The current CPU contract is RV32IMAC plus the
existing machine-mode CSR slice; FPU, PIE/AI instructions, hardware loops,
cache/MMU boot, and full ESP image validation are outside this tranche.
