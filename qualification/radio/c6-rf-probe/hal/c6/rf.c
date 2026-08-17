/* SPDX-License-Identifier: Apache-2.0 */
#include "mmio.h"
#include "rf.h"

#define RF_FREQUENCY_CONTROL 0x600a00c0u
#define RF_GAIN_FIRST 0x600a08ccu
#define RF_GAIN_SECOND 0x600a08d0u
#define RF_GAIN_FINAL 0x600a08d4u
#define RF_FRONTEND_FORCE 0x600a0910u
#define RF_FRONTEND_OFF 0x200u
#define RF_FREQUENCY_PREFIX 0x42844000u
#define RF_FREQUENCY_BASE 0x380u
#define RF_FREQUENCY_STRIDE 0x280u
#define RF_GAIN_COUNT 43u

struct c6_rf_gain {
    uint32_t first;
    uint32_t second;
    uint32_t final;
};

/* These complete tables are factual outputs of the pinned, public-API vendor
 * oracle. Their provenance and trace hashes live in RADIO_PLAN.html and
 * c6-rf-oracle-requirements.json. No vendor header or radio object is used. */
static const struct c6_rf_gain gain_8dbm[RF_GAIN_COUNT] = {
    {0x40200000u,0xe3c10080u,0x000000feu},{0x40200000u,0xe3c30080u,0x000000feu},
    {0x40200000u,0xe3c50080u,0x000000feu},{0x40200000u,0xe3c70080u,0x000000feu},
    {0x40200000u,0xe3c90080u,0x000000feu},{0x40200000u,0xe3cb0080u,0x000000feu},
    {0x40200000u,0xe3e10080u,0x000000feu},{0x40200000u,0xe3e30080u,0x000000feu},
    {0x40200000u,0xe3e50080u,0x000000feu},{0x40200000u,0xe3e70080u,0x000000feu},
    {0x40200000u,0xe3e90080u,0x000000feu},{0x40200000u,0x10020301u,0xfffff881u},
    {0x40200000u,0x10020301u,0xfffff681u},{0x40200000u,0x90020301u,0xfffffb80u},
    {0x40200000u,0x90020301u,0xfffff980u},{0x40200000u,0x90020301u,0xfffff780u},
    {0x40200000u,0x90020301u,0xfffff580u},{0x40200000u,0x10060100u,0xfffffb80u},
    {0x40200000u,0x10060100u,0xfffff980u},{0x40200000u,0x10060100u,0xfffff780u},
    {0x40200000u,0x10020301u,0xfffff980u},{0x40200000u,0x10020301u,0xfffff780u},
    {0x40200000u,0x10020301u,0xfffff580u},{0x40200000u,0x10020301u,0xfffff380u},
    {0x40200000u,0x10020301u,0xfffff180u},{0x40200000u,0x10020301u,0xffffef80u},
    {0x40200000u,0x10020301u,0xffffed80u},{0x40200000u,0x10020301u,0xffffeb80u},
    {0x40200000u,0x10020301u,0xffffe980u},{0x40200000u,0x10020301u,0xffffe780u},
    {0x40200000u,0x10020301u,0xffffe580u},{0x40200000u,0x10020301u,0xffffe380u},
    {0x40200000u,0x10020301u,0xffffe180u},{0x40200000u,0x10020301u,0xffffdf80u},
    {0x40200000u,0x10020301u,0xffffdd80u},{0x40200000u,0x10020301u,0xffffdb80u},
    {0x40200000u,0x10020301u,0xffffd980u},{0x40200000u,0x10020301u,0xffffd780u},
    {0x40200000u,0x10020301u,0xffffd580u},{0x40200000u,0x10020301u,0xffffd380u},
    {0x40200000u,0x10020301u,0xffffd180u},{0x40200000u,0x10020301u,0xffffcf80u},
    {0x40200000u,0x10020301u,0xffffcd80u},
};

static const struct c6_rf_gain gain_14dbm[RF_GAIN_COUNT] = {
    {0x40200000u,0xe3c10080u,0x000000feu},{0x40200000u,0xe3c30080u,0x000000feu},
    {0x40200000u,0xe3c50080u,0x000000feu},{0x40200000u,0xe3c70080u,0x000000feu},
    {0x40200000u,0xe3c90080u,0x000000feu},{0x40200000u,0xe3cb0080u,0x000000feu},
    {0x40200000u,0xe3e10080u,0x000000feu},{0x40200000u,0xe3e30080u,0x000000feu},
    {0x40200000u,0xe3e50080u,0x000000feu},{0x40200000u,0xe3e70080u,0x000000feu},
    {0x40200000u,0xe3e90080u,0x000000feu},{0x40200000u,0x90020301u,0xfffff882u},
    {0x40200000u,0x90020301u,0xfffff682u},{0x40200000u,0x90020301u,0xfffffb81u},
    {0x40200000u,0x90020301u,0xfffff981u},{0x40200000u,0x90020301u,0xfffff781u},
    {0x40200000u,0x10020301u,0xfffffa81u},{0x40200000u,0x10020301u,0xfffff881u},
    {0x40200000u,0x10020301u,0xfffff681u},{0x40200000u,0x90020301u,0xfffffb80u},
    {0x40200000u,0x90020301u,0xfffff980u},{0x40200000u,0x90020301u,0xfffff780u},
    {0x40200000u,0x90020301u,0xfffff580u},{0x40200000u,0x10060100u,0xfffffb80u},
    {0x40200000u,0x10060100u,0xfffff980u},{0x40200000u,0x10060100u,0xfffff780u},
    {0x40200000u,0x10020301u,0xfffff980u},{0x40200000u,0x10020301u,0xfffff780u},
    {0x40200000u,0x10020301u,0xfffff580u},{0x40200000u,0x10020301u,0xfffff380u},
    {0x40200000u,0x10020301u,0xfffff180u},{0x40200000u,0x10020301u,0xffffef80u},
    {0x40200000u,0x10020301u,0xffffed80u},{0x40200000u,0x10020301u,0xffffeb80u},
    {0x40200000u,0x10020301u,0xffffe980u},{0x40200000u,0x10020301u,0xffffe780u},
    {0x40200000u,0x10020301u,0xffffe580u},{0x40200000u,0x10020301u,0xffffe380u},
    {0x40200000u,0x10020301u,0xffffe180u},{0x40200000u,0x10020301u,0xffffdf80u},
    {0x40200000u,0x10020301u,0xffffdd80u},{0x40200000u,0x10020301u,0xffffdb80u},
    {0x40200000u,0x10020301u,0xffffd980u},
};

static const struct c6_rf_gain gain_20dbm[RF_GAIN_COUNT] = {
    {0x40200000u,0xe3c10080u,0x000000feu},{0x40200000u,0xe3c30080u,0x000000feu},
    {0x40200000u,0xe3c50080u,0x000000feu},{0x40200000u,0xe3c70080u,0x000000feu},
    {0x40200000u,0xe3c90080u,0x000000feu},{0x40200000u,0xe3cb0080u,0x000000feu},
    {0x40200000u,0xe3e10080u,0x000000feu},{0x40200000u,0xe3e30080u,0x000000feu},
    {0x40200000u,0xe3e50080u,0x000000feu},{0x40200000u,0xe3e70080u,0x000000feu},
    {0x40200000u,0xe3e90080u,0x000000feu},{0x40200000u,0x10020301u,0xfffff607u},
    {0x40200000u,0x90020301u,0xfffff706u},{0x40200000u,0x10020301u,0xfffff806u},
    {0x40200000u,0x10020301u,0xfffff606u},{0x40200000u,0x90020301u,0xfffff783u},
    {0x40200000u,0x90020301u,0xfffffa82u},{0x40200000u,0x90020301u,0xfffff882u},
    {0x40200000u,0x90020301u,0xfffff682u},{0x40200000u,0x90020301u,0xfffffb81u},
    {0x40200000u,0x90020301u,0xfffff981u},{0x40200000u,0x90020301u,0xfffff781u},
    {0x40200000u,0x10020301u,0xfffffa81u},{0x40200000u,0x10020301u,0xfffff881u},
    {0x40200000u,0x10020301u,0xfffff681u},{0x40200000u,0x90020301u,0xfffffb80u},
    {0x40200000u,0x90020301u,0xfffff980u},{0x40200000u,0x90020301u,0xfffff780u},
    {0x40200000u,0x90020301u,0xfffff580u},{0x40200000u,0x10060100u,0xfffffb80u},
    {0x40200000u,0x10060100u,0xfffff980u},{0x40200000u,0x10060100u,0xfffff780u},
    {0x40200000u,0x10020301u,0xfffff980u},{0x40200000u,0x10020301u,0xfffff780u},
    {0x40200000u,0x10020301u,0xfffff580u},{0x40200000u,0x10020301u,0xfffff380u},
    {0x40200000u,0x10020301u,0xfffff180u},{0x40200000u,0x10020301u,0xffffef80u},
    {0x40200000u,0x10020301u,0xffffed80u},{0x40200000u,0x10020301u,0xffffeb80u},
    {0x40200000u,0x10020301u,0xffffe980u},{0x40200000u,0x10020301u,0xffffe780u},
    {0x40200000u,0x10020301u,0xffffe580u},
};

static uint8_t active_channel;
static uint8_t active_power;
static uint8_t rx_enabled;

static const struct c6_rf_gain *power_table(uint8_t power_dbm)
{
    if (power_dbm == 8u) return gain_8dbm;
    if (power_dbm == 14u) return gain_14dbm;
    if (power_dbm == 20u) return gain_20dbm;
    return 0;
}

void c6_rf_invalidate(void)
{
    active_channel = 0;
    active_power = 0;
    rx_enabled = 0;
}

int c6_rf_configure(uint8_t channel, uint8_t power_dbm)
{
    const struct c6_rf_gain *table = power_table(power_dbm);
    if (channel < 1u || channel > 13u) return C6_RF_INVALID_CHANNEL;
    if (table == 0) return C6_RF_INVALID_POWER;

    c6_write32(RF_FRONTEND_FORCE, RF_FRONTEND_OFF);
    c6_write32(RF_FREQUENCY_CONTROL,
               RF_FREQUENCY_PREFIX + RF_FREQUENCY_BASE +
                   (uint32_t)channel * RF_FREQUENCY_STRIDE);
    for (uint32_t index = 0; index < RF_GAIN_COUNT; ++index) {
        c6_write32(RF_GAIN_FIRST, table[index].first);
        c6_write32(RF_GAIN_SECOND, table[index].second);
        c6_write32(RF_GAIN_FINAL, table[index].final);
    }
    c6_write32(RF_FRONTEND_FORCE, 0);
    active_channel = channel;
    active_power = power_dbm;
    rx_enabled = 1;
    return C6_RF_OK;
}

int c6_rf_set_channel(uint8_t channel)
{
    if (active_power == 0) return C6_RF_NOT_INITIALIZED;
    return c6_rf_configure(channel, active_power);
}

int c6_rf_set_power(uint8_t power_dbm)
{
    if (active_channel == 0) return C6_RF_NOT_INITIALIZED;
    return c6_rf_configure(active_channel, power_dbm);
}

void c6_rf_rx_enable(int enabled)
{
    c6_write32(RF_FRONTEND_FORCE, enabled ? 0u : RF_FRONTEND_OFF);
    rx_enabled = enabled != 0;
}

uint8_t c6_rf_channel(void) { return active_channel; }
uint8_t c6_rf_power_dbm(void) { return active_power; }
int c6_rf_ready(void)
{
    return active_channel != 0 && active_power != 0 && rx_enabled != 0;
}
