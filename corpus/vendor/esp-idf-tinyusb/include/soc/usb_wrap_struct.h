#pragma once

#include <stdint.h>

typedef union {
    struct {
        uint32_t srp_sessend_override : 1;
        uint32_t srp_sessend_value : 1;
        uint32_t phy_sel : 1;
        uint32_t dfifo_force_pd : 1;
        uint32_t dbnce_fltr_bypass : 1;
        uint32_t exchg_pins_override : 1;
        uint32_t exchg_pins : 1;
        uint32_t vrefh : 2;
        uint32_t vrefl : 2;
        uint32_t vref_override : 1;
        uint32_t pad_pull_override : 1;
        uint32_t dp_pullup : 1;
        uint32_t dp_pulldown : 1;
        uint32_t dm_pullup : 1;
        uint32_t dm_pulldown : 1;
        uint32_t pullup_value : 1;
        uint32_t pad_enable : 1;
        uint32_t ahb_clk_force_on : 1;
        uint32_t phy_clk_force_on : 1;
        uint32_t phy_tx_edge_sel : 1;
        uint32_t dfifo_force_pu : 1;
        uint32_t reserved_23 : 8;
        uint32_t clk_en : 1;
    };
    uint32_t val;
} usb_wrap_otg_conf_reg_t;

typedef struct {
    volatile usb_wrap_otg_conf_reg_t otg_conf;
} usb_wrap_dev_t;

#define USB_WRAP (*(usb_wrap_dev_t *)0x60039000u)
