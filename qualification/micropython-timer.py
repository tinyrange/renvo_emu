"""Timer callback qualification for official MicroPython firmware."""

import sys
import time
from machine import Timer


events = []


def timer_callback(timer):
    events.append(time.ticks_us())


timer_id = -1 if sys.platform == "rp2" else 0
timer = Timer(timer_id)
timer.init(mode=Timer.PERIODIC, period=3, callback=timer_callback)

deadline = time.ticks_add(time.ticks_ms(), 1000)
while len(events) < 4 and time.ticks_diff(deadline, time.ticks_ms()) > 0:
    time.sleep_ms(1)
timer.deinit()

if len(events) < 4:
    raise AssertionError("periodic Timer callback count: {}".format(len(events)))
if any(time.ticks_diff(right, left) <= 0 for left, right in zip(events, events[1:])):
    raise AssertionError("periodic Timer callback ordering")
print("RENVO_TIMER_PERIODIC_OK", len(events))


# RENVO_CHUNK
events = []
timer.init(mode=Timer.ONE_SHOT, period=3, callback=timer_callback)

deadline = time.ticks_add(time.ticks_ms(), 1000)
while not events and time.ticks_diff(deadline, time.ticks_ms()) > 0:
    time.sleep_ms(1)
time.sleep_ms(12)
timer.deinit()

if len(events) != 1:
    raise AssertionError("one-shot Timer callback count: {}".format(len(events)))
print("RENVO_TIMER_ONE_SHOT_OK", len(events))
