"""M5StickS3 board topology.

This fixture records the published wiring and attaches the reusable ST7789
and M5PM1 behavior models. The same topology can run standalone protocol
scenarios or bind to the live ESP32-S3 SPI3, I2C1, and GPIO peripherals.
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
    board.add_connector(
        name = "imu_i2c1",
        protocol = "i2c",
        data_pin = 47,
        clock_pin = 48,
        voltage_mv = 3300,
    )
    board.add_connector(
        name = "audio_control_i2c1",
        protocol = "i2c",
        data_pin = 47,
        clock_pin = 48,
        voltage_mv = 3300,
    )
    board.add_connector(
        name = "audio_i2s",
        protocol = "digital",
        data_pin = 14,
        clock_pin = 17,
        voltage_mv = 3300,
    )
    board.add_connector(
        name = "audio_i2s_control",
        protocol = "digital",
        data_pin = 16,
        clock_pin = 15,
        voltage_mv = 3300,
    )
    board.add_connector(
        name = "infrared",
        protocol = "digital",
        data_pin = 46,
        clock_pin = 42,
        voltage_mv = 3300,
    )
    board.add_connector(
        name = "grove_port_a",
        protocol = "i2c",
        data_pin = 9,
        clock_pin = 10,
        voltage_mv = 5000,
    )
    board.add_connector(
        name = "hat2_primary",
        protocol = "digital",
        data_pin = 5,
        clock_pin = 4,
        voltage_mv = 3300,
    )
    board.add_connector(
        name = "hat2_secondary",
        protocol = "digital",
        data_pin = 43,
        clock_pin = 44,
        voltage_mv = 3300,
    )

    board.connect(
        "lcd_spi3",
        st7789(
            name = "lcd",
            width = 135,
            height = 240,
            x_offset = 52,
            y_offset = 40,
            inverted = True,
        ),
    )
    board.connect("m5pm1_i2c1", m5pm1(name = "m5pm1"))
    board.connect("imu_i2c1", bmi270(name = "bmi270"))
    board.connect("audio_control_i2c1", es8311(name = "es8311"))

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
