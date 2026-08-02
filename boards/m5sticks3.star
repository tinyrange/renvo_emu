"""M5StickS3 board topology.

This fixture records the published M5StickS3 wiring so board scenarios can
share stable connector and button names. The ST7789 and M5PM1 behavior models
are separate components; this definition intentionally keeps topology
assembly independent from the later live-firmware MMIO bridge.
"""

def m5sticks3():
    board = board_model(name = "m5sticks3", target = "esp32s3")

    board.add_connector(
        name = "lcd_spi3",
        protocol = "spi",
        data_pin = 39,
        clock_pin = 40,
        voltage_mv = 3300,
    )
    board.add_connector(
        name = "lcd_control",
        protocol = "digital",
        data_pin = 45,
        clock_pin = 41,
        voltage_mv = 3300,
    )
    # A single-wire digital connector records the reset line in data_pin; the
    # duplicated clock pin is ignored by the digital protocol.
    board.add_connector(
        name = "lcd_reset",
        protocol = "digital",
        data_pin = 21,
        clock_pin = 21,
        voltage_mv = 3300,
    )
    board.add_connector(
        name = "m5pm1_i2c1",
        protocol = "i2c",
        data_pin = 47,
        clock_pin = 48,
        voltage_mv = 3300,
    )

    board.mount(
        push_button(name = "button_a", active_low = True, bounce = us(500)),
        pin = 11,
    )
    board.mount(
        push_button(name = "button_b", active_low = True, bounce = us(500)),
        pin = 12,
    )
    board.mount(led(name = "lcd_backlight", active_low = False), pin = 38)
    return board
