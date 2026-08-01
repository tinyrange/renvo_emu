#ifndef REMU_WCH_DEBUG_H
#define REMU_WCH_DEBUG_H

typedef unsigned char u8;
typedef unsigned short u16;
typedef unsigned int u32;

typedef enum { DISABLE = 0, ENABLE = 1 } FunctionalState;
typedef enum { Bit_RESET = 0, Bit_SET = 1 } BitAction;
typedef enum {
    GPIO_Mode_Out_PP = 0x01,
} GPIOMode_TypeDef;
typedef enum {
    GPIO_Speed_30MHz = 0x03,
} GPIOSpeed_TypeDef;

typedef struct {
    u16 GPIO_Pin;
    GPIOSpeed_TypeDef GPIO_Speed;
    GPIOMode_TypeDef GPIO_Mode;
} GPIO_InitTypeDef;

#define GPIOD ((void *)0x40011400u)
#define GPIO_Pin_0 ((u16)0x0001u)
#define RCC_APB2Periph_GPIOD (1u << 5)
#define NVIC_PriorityGroup_1 1u
#define SDI_PR_OPEN 1
#define SDI_PRINT 0

extern u32 SystemCoreClock;
void NVIC_PriorityGroupConfig(u32 group);
void SystemCoreClockUpdate(void);
void Delay_Init(void);
void SDI_Printf_Enable(void);
void USART_Printf_Init(u32 baud);
u32 DBGMCU_GetCHIPID(void);
void RCC_APB2PeriphClockCmd(u32 peripheral, FunctionalState state);
void GPIO_Init(void *gpio, GPIO_InitTypeDef *configuration);
void GPIO_WriteBit(void *gpio, u16 pin, BitAction action);
void Delay_Ms(u32 milliseconds);
int printf(const char *format, ...);

#endif
