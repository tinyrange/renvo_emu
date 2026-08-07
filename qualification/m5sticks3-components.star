"""Standalone qualification for every modeled M5StickS3 board component."""

load("//boards:m5sticks3.star", "m5sticks3")

board = m5sticks3()

# M5PM1 identity, power-rail configuration, and retained register access.
board.i2c_write_read("m5pm1_i2c1", 0x6e, [0x00], read_len = 4)
board.i2c_write_read("m5pm1_i2c1", 0x6e, [0x06, 0x0f])
board.i2c_write_read("m5pm1_i2c1", 0x6e, [0x10, 0x0c])
board.i2c_write_read("m5pm1_i2c1", 0x6e, [0x11, 0x0c])

# BMI270 identity/configuration, deterministic motion sample, and power state.
board.i2c_write_read("imu_i2c1", 0x68, [0x00], read_len = 1)
board.i2c_write_read("imu_i2c1", 0x68, [0x5e, 0xa5])
board.i2c_write_read("imu_i2c1", 0x68, [0x59, 0x01])
board.i2c_write_read("imu_i2c1", 0x68, [0x7d, 0x06])
board.i2c_write_read("imu_i2c1", 0x68, [0x0c], read_len = 6)

# Official ES8311 microphone/DAC power-up register sequence.
board.i2c_write_read("audio_control_i2c1", 0x18, [0x00, 0x80])
board.i2c_write_read("audio_control_i2c1", 0x18, [0x0d, 0x01])
board.i2c_write_read("audio_control_i2c1", 0x18, [0x0e, 0x02])
board.i2c_write_read("audio_control_i2c1", 0x18, [0x12, 0x00])
board.i2c_write_read("audio_control_i2c1", 0x18, [0x13, 0x10])
board.i2c_write_read("audio_control_i2c1", 0x18, [0x17, 0xff])
board.i2c_write_read("audio_control_i2c1", 0x18, [0x32, 0xbf])

# ST7789 inversion/display enable, full visible window, and two RGB565 pixels.
board.spi_write("lcd_spi3", [0x21], data_phase = False)
board.spi_write("lcd_spi3", [0x29], data_phase = False)
board.spi_write("lcd_spi3", [0x2a], data_phase = False)
board.spi_write("lcd_spi3", [0x00, 0x34, 0x00, 0xba])
board.spi_write("lcd_spi3", [0x2b], data_phase = False)
board.spi_write("lcd_spi3", [0x00, 0x28, 0x01, 0x17])
board.spi_write("lcd_spi3", [0x2c], data_phase = False)
board.spi_write("lcd_spi3", [0xf8, 0x00, 0x07, 0xe0])

board.set_led("lcd_backlight", True)
board.press("button_a", duration = ms(20))
board.press("button_b", duration = ms(20))
board
