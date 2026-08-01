# Arduino qualification adapter

The qualification script copies the canonical `Blink.ino` and `MultiSerial.ino`
sources unchanged from `arduino/arduino-examples` revision
`ad14bc44cb95555e5df7c16e6605559cad860d29`. The board identity and pin mapping
come from `arduino/ArduinoCore-renesas` revision
`424e86eff92d37f72123c2b641dd8bbf06a38b47`: UNO R4 Minima D13 is P111 and
`Serial1` is the board hardware serial API.

`Arduino.h`, `adapter.cpp`, `harness.cpp`, startup, and linker files are the
allowed freestanding test harness. They invoke each unchanged sketch once,
map `LED_BUILTIN` to P111, feed one deterministic host byte to `Serial`, and
route the sketch's `Serial1.write` to the selected functional SCI9 trace sink.
USB and the Wi-Fi-board bridge are intentionally absent.
