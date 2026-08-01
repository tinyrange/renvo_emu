"""Create durable qualification state using only public MicroPython APIs."""

import os


path = "/.remu-persistent-state"
payload = bytes([((index * 73) ^ (index >> 1) ^ 0xA5) & 0xFF for index in range(1024)])

try:
    os.remove(path)
except OSError:
    pass

with open(path, "wb") as stream:
    assert stream.write(payload) == len(payload)
    stream.flush()

if hasattr(os, "sync"):
    os.sync()

with open(path, "rb") as stream:
    assert stream.read() == payload

print("REMU_PERSIST_WRITE_OK", len(payload), hex(sum(payload) & 0xFFFFFFFF))
