"""Live-firmware M5StickS3 fixture with Button A initially pressed."""

load("//boards:m5sticks3.star", "m5sticks3")

board = m5sticks3()
board.press("button_a", duration = ms(20))
board
