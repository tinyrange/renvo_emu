"""Portable official-firmware qualification workload for Renvo Emulator.

This file is delivered unchanged through the board's ordinary USB CDC REPL.
It deliberately uses only public MicroPython APIs and leaves a compact,
deterministic transcript suitable for cross-profile comparison.
"""

import gc
import hashlib
import json
import math
import os
import struct
import sys


evidence = []


def record(name, value):
    evidence.append((name, value))
    print("REMU_CASE", name, value)


def require(condition, message):
    if not condition:
        raise AssertionError(message)


record("implementation", sys.implementation.name)
require(sys.implementation.name == "micropython", "official MicroPython required")
record("version", ".".join(str(value) for value in sys.implementation.version[:3]))


def make_closure(seed):
    def apply(values):
        return [seed + value * value for value in values]

    return apply


closure_result = make_closure(3)(range(8))
require(closure_result == [3, 4, 7, 12, 19, 28, 39, 52], "closure/comprehension")
record("closure", sum(closure_result))


def fibonacci(limit):
    left, right = 0, 1
    while left < limit:
        sent = yield left
        left, right = right, left + right + (sent or 0)


generator = fibonacci(100)
sequence = [next(generator), generator.send(0), generator.send(1)]
sequence.extend(list(generator))
require(sequence == [0, 1, 1, 3, 4, 7, 11, 18, 29, 47, 76], "generator send")
record("generator", sum(sequence))


# REMU_CHUNK
class Managed:
    def __init__(self):
        self.events = []

    def __enter__(self):
        self.events.append("enter")
        return self

    def __exit__(self, kind, value, traceback):
        self.events.append(kind.__name__ if kind else "exit")
        return kind is ValueError


managed = Managed()
with managed:
    raise ValueError("expected")
require(managed.events == ["enter", "ValueError"], "context manager")
record("exceptions", ":".join(managed.events))

try:
    try:
        raise KeyError("inner")
    except KeyError as error:
        raise RuntimeError("outer") from error
except RuntimeError as error:
    require(str(error) == "outer", "exception chaining")
else:
    raise AssertionError("exception was not raised")


# REMU_CHUNK
large = (1 << 131) + (1 << 67) + 0x12345678
require((large // 97) * 97 + large % 97 == large, "long integer divmod")
require(pow(large, 17, 0x1FFFF_FFFB) == 8291659504, "modular exponentiation")
record("longint", hex((large ^ (large >> 64)) & 0xFFFFFFFF))

float_result = math.fsum([0.1, 0.2, 0.3, -0.6]) if hasattr(math, "fsum") else sum(
    [0.1, 0.2, 0.3, -0.6]
)
require(abs(float_result) < 1e-12, "floating point")
require(abs(math.sin(math.pi / 6) - 0.5) < 1e-7, "transcendental")
record("float", "{:.9f}".format(math.sqrt(2)))


# REMU_CHUNK
mapping = {key: key * key for key in range(17)}
mapping.update({-1: 7, "snowman": "\u2603"})
require(sum(mapping[key] for key in range(17)) == 1496, "dict lookup")
require(set(mapping) >= {0, 8, 16, -1, "snowman"}, "set relation")
record("collections", len(mapping))

payload = bytearray(range(64))
view = memoryview(payload)[7:55]
for offset in range(0, len(view), 3):
    view[offset] = 0xA5
packed = struct.pack("<Bhiqf", 0xA5, -1234, 0x12345678, -0x123456789, 1.25)
unpacked = struct.unpack("<Bhiqf", packed)
require(unpacked[:4] == (0xA5, -1234, 0x12345678, -0x123456789), "struct integer")
require(abs(unpacked[4] - 1.25) < 1e-6, "struct float")
record("buffer", sum(payload) & 0xFFFF)

unicode_text = "Renvo Emulator \u2603 \U0001f680 caf\u00e9"
encoded = unicode_text.encode("utf-8")
require(encoded.decode("utf-8") == unicode_text, "unicode round trip")
require(unicode_text[6:9] == "\u2603 \U0001f680", "unicode indexing")
record("unicode", len(encoded))

document = {
    "bool": True,
    "none": None,
    "numbers": [1, -2, 3.5],
    "text": unicode_text,
    "nested": {"value": 0x1234},
}
encoded_json = json.dumps(document)
decoded_json = json.loads(encoded_json)
require(decoded_json == document, "JSON round trip")
record("json", len(encoded_json))


# REMU_CHUNK
class Node:
    pass


gc.collect()
before = gc.mem_free()
for index in range(250):
    left = Node()
    right = Node()
    left.other = right
    right.other = left
    left.data = bytearray((index % 31) + 17)
left = right = None
gc.collect()
after = gc.mem_free()
require(after > 0 and before > 0, "GC accounting")
record("gc", after >= before // 2)

# REMU_CHUNK
temporary = "/.remu-qualification.tmp"
renamed = "/.remu-qualification.done"
try:
    try:
        os.remove(temporary)
    except OSError:
        pass
    try:
        os.remove(renamed)
    except OSError:
        pass
    content = b"remu-storage-" + bytes(range(32))
    with open(temporary, "wb") as stream:
        require(stream.write(content) == len(content), "filesystem write")
    os.rename(temporary, renamed)
    with open(renamed, "rb") as stream:
        require(stream.read() == content, "filesystem read")
    require(".remu-qualification.done" in os.listdir("/"), "filesystem directory")
    record("filesystem", len(content))
finally:
    try:
        os.remove(temporary)
    except OSError:
        pass
    try:
        os.remove(renamed)
    except OSError:
        pass


# REMU_CHUNK
import machine

identity = bytes(machine.unique_id())
require(len(identity) >= 4, "machine unique_id")
frequency = machine.freq()
require(isinstance(frequency, int) and frequency > 0, "machine frequency")
record("machine", "{}:{}".format(len(identity), frequency))

pin = machine.Pin(0, machine.Pin.OUT, value=0)
pin_states = [pin.value()]
pin.value(1)
pin_states.append(pin.value())
pin.value(0)
pin_states.append(pin.value())
require(pin_states == [0, 1, 0], "GPIO output state")
record("gpio", ":".join(str(value) for value in pin_states))

digest_input = repr(evidence).encode()
digest = hashlib.sha256(digest_input).digest().hex()
print("REMU_QUAL_DIGEST", digest)
print("REMU_QUAL_OK", len(evidence))
