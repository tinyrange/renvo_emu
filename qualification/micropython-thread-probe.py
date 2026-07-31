"""Exercise the official MicroPython thread API through its public surface."""

import _thread
import time


lock = _thread.allocate_lock()
state = [0, 0]


def worker(seed):
    value = seed
    for index in range(200):
        value = ((value << 5) ^ (value >> 3) ^ index) & 0xFFFFFFFF
    with lock:
        state[0] = value
        state[1] = _thread.get_ident()


thread_id = _thread.start_new_thread(worker, (0x12345678,))
deadline = time.ticks_add(time.ticks_ms(), 2000)
while state[1] == 0 and time.ticks_diff(deadline, time.ticks_ms()) > 0:
    time.sleep_ms(1)

assert state[0] == 0xD062B2B8, hex(state[0])
assert state[1] != 0
assert thread_id is None or isinstance(thread_id, int)
print("RENVO_THREAD_OK", hex(state[0]), state[1] != _thread.get_ident())
