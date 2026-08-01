"""M5Stack NanoC6 board topology.

The pin mapping follows M5Stack's published NanoC6 documentation. Tests load
this file, instantiate the board, and connect external Grove devices.
"""

def m5stack_nanoc6():
    board = board_model(name = "m5stack_nanoc6", target = "esp32c6")

    board.add_connector(
        name = "grove",
        protocol = "i2c",
        data_pin = 2,
        clock_pin = 1,
        voltage_mv = 5000,
    )

    board.mount(
        push_button(name = "button", active_low = True, bounce = us(500)),
        pin = 9,
    )
    board.mount(led(name = "blue_led", active_low = True), pin = 7)
    board.mount(
        ws2812_rgb(name = "rgb", count = 1),
        pin = 20,
        enable_pin = 19,
    )
    return board
