#pragma once

#include "esp_err.h"

typedef enum { USB_PHY_TARGET_INT, USB_PHY_TARGET_UTMI, USB_PHY_TARGET_EXT } usb_phy_target_t;
typedef enum { USB_PHY_CTRL_OTG, USB_PHY_CTRL_SERIAL_JTAG } usb_phy_controller_t;
typedef enum { USB_PHY_MODE_DEFAULT, USB_OTG_MODE_HOST, USB_OTG_MODE_DEVICE } usb_otg_mode_t;
typedef enum {
    USB_PHY_SPEED_UNDEFINED,
    USB_PHY_SPEED_LOW,
    USB_PHY_SPEED_FULL,
    USB_PHY_SPEED_HIGH,
} usb_phy_speed_t;

typedef struct {
    int iddig_io_num;
    int avalid_io_num;
    int vbusvalid_io_num;
    int idpullup_io_num;
    int dppulldown_io_num;
    int dmpulldown_io_num;
    int drvvbus_io_num;
    int bvalid_io_num;
    int sessend_io_num;
    int chrgvbus_io_num;
    int dischrgvbus_io_num;
} usb_phy_otg_io_conf_t;

#define USB_PHY_SELF_POWERED_DEVICE(vbus_io) \
    {                                        \
        .iddig_io_num = -1,                  \
        .avalid_io_num = -1,                 \
        .vbusvalid_io_num = -1,              \
        .idpullup_io_num = -1,               \
        .dppulldown_io_num = -1,             \
        .dmpulldown_io_num = -1,             \
        .drvvbus_io_num = -1,                \
        .bvalid_io_num = (vbus_io),          \
        .sessend_io_num = -1,                \
        .chrgvbus_io_num = -1,               \
        .dischrgvbus_io_num = -1,            \
    }

typedef struct {
    usb_phy_controller_t controller;
    usb_phy_target_t target;
    usb_otg_mode_t otg_mode;
    usb_phy_speed_t otg_speed;
    const void *ext_io_conf;
    const usb_phy_otg_io_conf_t *otg_io_conf;
} usb_phy_config_t;

typedef void *usb_phy_handle_t;

esp_err_t usb_new_phy(const usb_phy_config_t *config, usb_phy_handle_t *handle);
esp_err_t usb_del_phy(usb_phy_handle_t handle);
