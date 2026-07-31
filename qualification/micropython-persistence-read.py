"""Verify state written by a separate Renvo process and then clean it up."""

import os


path = "/.renvo-persistent-state"
expected = bytes([((index * 73) ^ (index >> 1) ^ 0xA5) & 0xFF for index in range(1024)])

assert path.rsplit("/", 1)[1] in os.listdir("/")
with open(path, "rb") as stream:
    observed = stream.read()
assert observed == expected

os.remove(path)
if hasattr(os, "sync"):
    os.sync()

print("RENVO_PERSIST_READ_OK", len(observed), hex(sum(observed) & 0xFFFFFFFF))
