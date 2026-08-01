"""Complete board-composition exercise for a NanoC6 and Grove SGP30."""

load("//boards:m5stack_nanoc6.star", "m5stack_nanoc6")

sensor = sgp30(name = "air_quality", eco2 = 420, tvoc = 8)
board = m5stack_nanoc6()

# The connector owns its pin mapping and protocol checks.
board.connect("grove", sensor)

# Read identity, initialise IAQ measurement, wait through conditioning, then
# sample deterministic environmental values.
board.i2c_write_read("grove", 0x58, [0x36, 0x82], read_len = 9)
board.i2c_write_read("grove", 0x58, [0x20, 0x2f], read_len = 3)
board.i2c_write_read("grove", 0x58, [0x20, 0x03])
board.run_for(seconds(15))
board.set_air_quality(sensor, eco2 = 1200, tvoc = 180)
board.i2c_write_read("grove", 0x58, [0x20, 0x08], read_len = 6)

# Exercise all onboard human-visible/input components in the same scenario.
board.press("button", duration = ms(20))
board.set_led("blue_led", True)
board.show("rgb", [0xff0000])
board.run_for(ms(1))

board
