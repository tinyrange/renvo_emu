#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "device/dcd.h"
#include "esp_private/usb_phy.h"
#include "tinyusb.h"
#include "tusb.h"

#define REG32(address) (*(volatile uint32_t *)(address))
#define RTC_USB_CONF 0x60008120u
#define USB_BASE 0x60080000u
#define USB_GINTSTS 0x14u
#define USB_GINTMSK 0x18u
#define TEST_EXIT 0xfffffff0u

static volatile bool mounted;
static volatile bool sent;
static volatile uint32_t written;
static volatile uint32_t flushed;
static uint32_t milliseconds;

void *memcpy(void *destination, const void *source, size_t length)
{
    uint8_t *out = destination;
    const uint8_t *in = source;
    while (length--) {
        *out++ = *in++;
    }
    return destination;
}

void *memset(void *destination, int value, size_t length)
{
    uint8_t *out = destination;
    while (length--) {
        *out++ = (uint8_t)value;
    }
    return destination;
}

void *memmove(void *destination, const void *source, size_t length)
{
    uint8_t *out = destination;
    const uint8_t *in = source;
    if (out < in) {
        return memcpy(destination, source, length);
    }
    while (length--) {
        out[length] = in[length];
    }
    return destination;
}

int memcmp(const void *left, const void *right, size_t length)
{
    const uint8_t *a = left;
    const uint8_t *b = right;
    while (length--) {
        if (*a != *b) {
            return *a - *b;
        }
        ++a;
        ++b;
    }
    return 0;
}

size_t strlen(const char *text)
{
    const char *end = text;
    while (*end) {
        ++end;
    }
    return (size_t)(end - text);
}

uint32_t tusb_time_millis_api(void)
{
    return milliseconds++;
}

static const tusb_desc_device_t device_descriptor = {
    .bLength = sizeof(tusb_desc_device_t),
    .bDescriptorType = TUSB_DESC_DEVICE,
    .bcdUSB = 0x0200,
    .bDeviceClass = TUSB_CLASS_MISC,
    .bDeviceSubClass = MISC_SUBCLASS_COMMON,
    .bDeviceProtocol = MISC_PROTOCOL_IAD,
    .bMaxPacketSize0 = CFG_TUD_ENDPOINT0_SIZE,
    .idVendor = 0xcafe,
    .idProduct = 0x4001,
    .bcdDevice = 0x0100,
    .iManufacturer = 1,
    .iProduct = 2,
    .iSerialNumber = 3,
    .bNumConfigurations = 1,
};

enum {
    INTERFACE_CDC_CONTROL,
    INTERFACE_CDC_DATA,
    INTERFACE_COUNT,
};

#define CONFIG_TOTAL_LENGTH (TUD_CONFIG_DESC_LEN + TUD_CDC_DESC_LEN)

static const uint8_t configuration_descriptor[] = {
    TUD_CONFIG_DESCRIPTOR(1, INTERFACE_COUNT, 0, CONFIG_TOTAL_LENGTH, 0, 100),
    TUD_CDC_DESCRIPTOR(INTERFACE_CDC_CONTROL, 4, 0x81, 8, 0x02, 0x82, 64),
};

const uint8_t *tud_descriptor_device_cb(void)
{
    return (const uint8_t *)&device_descriptor;
}

const uint8_t *tud_descriptor_configuration_cb(uint8_t index)
{
    (void)index;
    return configuration_descriptor;
}

const uint16_t *tud_descriptor_string_cb(uint8_t index, uint16_t language)
{
    static uint16_t descriptor[32];
    static const char *const strings[] = {
        "", "Renvo Emulator", "TinyUSB qualification", "0001",
    };
    (void)language;
    if (index == 0) {
        descriptor[1] = 0x0409;
        descriptor[0] = (TUSB_DESC_STRING << 8) | 4;
        return descriptor;
    }
    if (index >= sizeof(strings) / sizeof(strings[0])) {
        return NULL;
    }
    size_t length = strlen(strings[index]);
    if (length > 31) {
        length = 31;
    }
    for (size_t i = 0; i < length; ++i) {
        descriptor[1 + i] = (uint8_t)strings[index][i];
    }
    descriptor[0] = (uint16_t)((TUSB_DESC_STRING << 8) | (2 * length + 2));
    return descriptor;
}

static void tinyusb_event(tinyusb_event_t *event, void *argument)
{
    (void)argument;
    if (event->id == TINYUSB_EVENT_ATTACHED) {
        mounted = true;
    }
}

esp_err_t usb_new_phy(const usb_phy_config_t *config, usb_phy_handle_t *handle)
{
    if (config == NULL || config->controller != USB_PHY_CTRL_OTG
        || config->target != USB_PHY_TARGET_INT || config->otg_mode != USB_OTG_MODE_DEVICE
        || config->otg_speed != USB_PHY_SPEED_FULL) {
        return ESP_ERR_INVALID_ARG;
    }
    REG32(RTC_USB_CONF) |= (1u << 20) | (1u << 19);
    *handle = (void *)1;
    return ESP_OK;
}

esp_err_t usb_del_phy(usb_phy_handle_t handle)
{
    (void)handle;
    return ESP_OK;
}

esp_err_t tinyusb_task_check_config(const tinyusb_task_config_t *config)
{
    return config != NULL && config->size != 0 && config->priority != 0 ? ESP_OK
                                                                        : ESP_ERR_INVALID_ARG;
}

esp_err_t tinyusb_task_start(tinyusb_port_t port, const tinyusb_task_config_t *task,
                             const tinyusb_desc_config_t *descriptor)
{
    (void)task;
    (void)descriptor;
    const tusb_rhport_init_t rhport = {
        .role = TUSB_ROLE_DEVICE,
        .speed = TUSB_SPEED_FULL,
    };
    return tusb_rhport_init((uint8_t)port, &rhport) ? ESP_OK : ESP_FAIL;
}

esp_err_t tinyusb_task_stop(void)
{
    return ESP_OK;
}

void app_main(void)
{
    mounted = false;
    sent = false;
    written = 0;
    flushed = 0;
    milliseconds = 0;

    const tinyusb_config_t config = {
        .port = TINYUSB_PORT_FULL_SPEED_0,
        .phy = {.skip_setup = false, .self_powered = false, .vbus_monitor_io = -1},
        .task = {.size = 2048, .priority = 5, .xCoreID = 0},
        .descriptor = {
            .device = &device_descriptor,
            .full_speed_config = configuration_descriptor,
        },
        .event_cb = tinyusb_event,
    };
    if (tinyusb_driver_install(&config) != ESP_OK) {
        REG32(TEST_EXIT) = 10;
        return;
    }

    uint32_t after_send = 0;
    for (uint32_t iteration = 0; iteration < 1000000u; ++iteration) {
        if (REG32(USB_BASE + USB_GINTSTS) & REG32(USB_BASE + USB_GINTMSK)) {
            dcd_int_handler(0);
        }
        tud_task();
        if (!sent && mounted && tud_cdc_n_connected(0)) {
            static const uint8_t pass[] = {'P', 'A', 'S', 'S', '\n'};
            written = tud_cdc_n_write(0, pass, sizeof(pass));
            flushed = tud_cdc_n_write_flush(0);
            sent = written == sizeof(pass) && flushed == sizeof(pass);
        }
        if (sent && ++after_send == 4096u) {
            REG32(TEST_EXIT) = written == 5 && flushed == 5
                                   ? 0
                                   : 20 + (written & 0xffu) + ((flushed & 0xffu) << 8);
            return;
        }
    }
    REG32(TEST_EXIT) = 11;
}
