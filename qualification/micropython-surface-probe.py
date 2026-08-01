"""Report public hardware-facing APIs from an official MicroPython image."""

import machine
import sys


def report(name, module):
    names = sorted(value for value in dir(module) if not value.startswith("_"))
    for offset in range(0, len(names), 8):
        print("REMU_SURFACE", name, ",".join(names[offset : offset + 8]))


report("machine", machine)
for name in ("network", "bluetooth", "_thread", "rp2", "esp32"):
    try:
        module = __import__(name)
    except ImportError:
        print("REMU_SURFACE", name, "-")
    else:
        report(name, module)
print("REMU_SURFACE platform", sys.platform)
print("REMU_SURFACE_OK")
