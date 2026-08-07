#include <stdint.h>

#include "soc/aes_reg.h"
#include "soc/apb_saradc_reg.h"
#include "soc/ds_reg.h"
#include "soc/ecc_mult_reg.h"
#include "soc/efuse_reg.h"
#include "soc/gdma_reg.h"
#include "soc/hmac_reg.h"
#include "soc/i2c_reg.h"
#include "soc/i2s_reg.h"
#include "soc/interrupt_matrix_reg.h"
#include "soc/intpri_reg.h"
#include "soc/io_mux_reg.h"
#include "soc/ledc_reg.h"
#include "soc/lp_i2c_reg.h"
#include "soc/lp_uart_reg.h"
#include "soc/lp_wdt_reg.h"
#include "soc/mcpwm_reg.h"
#include "soc/parl_io_reg.h"
#include "soc/pcnt_reg.h"
#include "soc/rmt_reg.h"
#include "soc/rsa_reg.h"
#include "soc/sdio_hinf_reg.h"
#include "soc/sdio_slc_reg.h"
#include "soc/sha_reg.h"
#include "soc/soc_etm_reg.h"
#include "soc/spi_reg.h"
#include "soc/systimer_reg.h"
#include "soc/timer_group_reg.h"
#include "soc/twai_reg.h"
#include "soc/uhci_reg.h"

#define READ32(address) (*(volatile uint32_t *)(uintptr_t)(address))
#define WRITE32(address, value) (READ32(address) = (uint32_t)(value))
#define CHECK(address, mask, expected, code)                                  \
    do {                                                                       \
        if ((READ32(address) & (uint32_t)(mask)) != (uint32_t)(expected)) {    \
            return (code);                                                     \
        }                                                                      \
    } while (0)

int main(void)
{
    CHECK(I2C_DATE_REG(0), UINT32_MAX, 35656050u, 1);
    CHECK(SPI_DATE_REG(2), 0x0fffffffu, 35656448u, 2);
    CHECK(I2S_DATE_REG(0), 0x0fffffffu, 35655792u, 3);
    CHECK(LEDC_DATE_REG, 0x0fffffffu, 34672976u, 4);
    CHECK(RMT_DATE_REG, 0x0fffffffu, 34636307u, 5);
    CHECK(PCNT_DATE_REG, UINT32_MAX, 419898881u, 6);
    CHECK(MCPWM_VERSION_REG(0), 0x0fffffffu, 35656256u, 7);
    CHECK(PARL_IO_VERSION_REG, 0x0fffffffu, 35660352u, 8);
    CHECK(GDMA_DATE_REG, UINT32_MAX, 35660368u, 9);
    CHECK(APB_SARADC_CTRL_DATE_REG, UINT32_MAX, 35676736u, 10);
    CHECK(SOC_ETM_DATE_REG, 0x0fffffffu, 35664018u, 11);
    CHECK(SYSTIMER_DATE_REG, UINT32_MAX, 35655795u, 12);
    CHECK(LP_UART_DATE_REG, UINT32_MAX, 35656288u, 13);
    CHECK(LP_I2C_DATE_REG, UINT32_MAX, 35656003u, 14);
    CHECK(LP_WDT_DATE_REG, 0x7fffffffu, 34676864u, 15);
    CHECK(AES_DATE_REG, 0x3fffffffu, 538513936u, 16);
    CHECK(SHA_DATE_REG, 0x3fffffffu, 538972713u, 17);
    CHECK(HMAC_DATE_REG, 0x3fffffffu, 538969624u, 18);
    CHECK(RSA_DATE_REG, 0x3fffffffu, 538969624u, 19);
    CHECK(DS_DATE_REG, 0x3fffffffu, 538969624u, 20);
    CHECK(ECC_MULT_DATE_REG, 0x0fffffffu, 35656256u, 21);
    CHECK(EFUSE_DATE_REG, 0x0fffffffu, 35676928u, 22);
    CHECK(IO_MUX_DATE_REG, 0x0fffffffu, 35655776u, 23);
    CHECK(INTMTX_CORE0_INTERRUPT_REG_DATE_REG, 0x0fffffffu, 35664144u, 24);
    CHECK(INTPRI_DATE_REG, 0x0fffffffu, 35655824u, 25);
    CHECK(UHCI_DATE_REG(0), UINT32_MAX, 35655936u, 26);

    WRITE32(IO_MUX_GPIO30_REG, FUN_IE | (2u << MCU_SEL_S));
    CHECK(IO_MUX_GPIO30_REG, FUN_IE | MCU_SEL_M,
          FUN_IE | (2u << MCU_SEL_S), 27);
    WRITE32(INTMTX_CORE0_ECC_INTR_MAP_REG, 7);
    CHECK(INTMTX_CORE0_ECC_INTR_MAP_REG, 0x1f, 7, 28);
    WRITE32(INTPRI_CORE0_CPU_INT_PRI_7_REG, 3);
    CHECK(INTPRI_CORE0_CPU_INT_PRI_7_REG, 0xf, 3, 29);

    WRITE32(SPI_W0_REG(2), 0x00000201u);
    WRITE32(SPI_MS_DLEN_REG(2), 15u);
    WRITE32(SPI_MISC_REG(2), 0u);
    WRITE32(SPI_USER_REG(2), SPI_USR_MOSI | SPI_USR_MISO);
    WRITE32(SPI_CMD_REG(2), SPI_USR);
    CHECK(SPI_W0_REG(2), 0xffffu, 0x0201u, 30);

    CHECK(DR_REG_TWAI0_BASE + 0x08u, 0x0cu, 0x0cu, 31);
    CHECK(DR_REG_HINF_BASE, UINT32_MAX, 0x00926666u, 32);
    CHECK(TIMG_NTIMERS_DATE_REG(0), 0x0fffffffu, 35676274u, 33);
    return 0;
}
