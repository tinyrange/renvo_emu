"""External GPIO qualification through the official MicroPython API.

Renvo Emulator's host drives pin 0 high and pin 1 low at virtual tick zero.  This
workload is delivered through the normal raw REPL and observes those resolved
net values using only public ``machine.Pin`` operations.
"""

from machine import Pin, Signal


pin_high = Pin(0, Pin.IN)
pin_low = Pin(1, Pin.IN)

high = pin_high.value()
low = pin_low.value()
if (high, low) != (1, 0):
    raise AssertionError("external GPIO input: {} {}".format(high, low))

inverted = Signal(pin_high, invert=True)
if inverted.value() != 0:
    raise AssertionError("inverted Signal input")

output = Pin(2, Pin.OUT, value=0)
states = [output.value()]
output.on()
states.append(output.value())
output.toggle()
states.append(output.value())
if states != [0, 1, 0]:
    raise AssertionError("GPIO output helpers: {}".format(states))

print("REMU_GPIO_INPUT_OK", high, low, "".join(str(value) for value in states))
