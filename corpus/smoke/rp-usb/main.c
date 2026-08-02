#define REG32(address) (*(volatile unsigned int *)(address))

#define USB_MAIN_CTRL REG32(0x50110040u)
#define USB_SIE_CTRL REG32(0x5011004cu)
#define USB_SIE_STATUS REG32(0x50110050u)
#define USB_INTR REG32(0x5011008cu)
#define USB_MUXING REG32(0x50110074u)

int main(void)
{
    USB_MAIN_CTRL = 1u;
    USB_SIE_CTRL = 1u << 16; /* pull-up enables the device */
    USB_MUXING = 1u;         /* connect the controller to the USB PHY */

    /* The deterministic host supplies VBUS and a bus reset on the next
       abstract instruction boundary. */
    unsigned int status = USB_INTR;
    if ((status & (1u << 11)) == 0u || (status & (1u << 12)) == 0u) {
        return 1u;
    }

    /* SIE_STATUS is write-clear; VBUS_DETECTED is read-only and remains set. */
    USB_SIE_STATUS = (1u << 19) | (1u << 16);
    status = USB_INTR;
    return (status & (1u << 11)) != 0u && (status & (1u << 12)) == 0u ? 0u : 2u;
}
