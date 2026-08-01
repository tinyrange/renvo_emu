"""Exercise filesystem retention and USB raw-REPL recovery across soft reset."""

import machine
import os


path = "/.remu-soft-reset-state"
payload = b"remu-soft-reset-v1:" + bytes(range(64))
try:
    os.remove(path)
except OSError:
    pass
with open(path, "wb") as stream:
    assert stream.write(payload) == len(payload)
    stream.flush()
if hasattr(os, "sync"):
    os.sync()
print("REMU_SOFT_RESET_BEFORE", len(payload))
machine.soft_reset()

# REMU_CHUNK
import os


path = "/.remu-soft-reset-state"
expected = b"remu-soft-reset-v1:" + bytes(range(64))
with open(path, "rb") as stream:
    observed = stream.read()
assert observed == expected
os.remove(path)
if hasattr(os, "sync"):
    os.sync()
print("REMU_SOFT_RESET_OK", len(observed))
