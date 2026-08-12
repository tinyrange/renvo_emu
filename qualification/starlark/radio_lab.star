"""Interactive, bounded laboratory for genuine C6/S3 radio firmware.

Run this entrypoint with ``remu run --agent-script ... --agent-repl``.  It
keeps the native machine live while exposing the checked evidence helpers in
the REPL.  Nothing in this script implements a peripheral or intercepts guest
execution; observations still come from the Rust LLE machine.
"""

load(
    "//qualification/starlark:agent_automation.star",
    "drain_bus",
    "drain_radio",
    "run_until",
    "wait_for_bus",
    "wait_for_radio",
)
load(
    "//qualification/starlark:c6_twt_evidence.star",
    "capture_c6_twt",
    "require_c6_twt_timer_programming",
    "summarize_c6_twt_events",
)
load(
    "//qualification/starlark:wifi_crypto_evidence.star",
    "capture_wifi_crypto",
    "require_wifi_crypto_programming",
    "summarize_wifi_crypto_events",
)

def run_slice(instructions = 100000):
    """Advances the live machine by one explicitly bounded slice."""
    return machine.run(instructions = instructions)

def wifi_crypto_capture(capacity = 8192):
    """Starts the native Wi-Fi key-table capture and returns its layout."""
    return capture_wifi_crypto(machine, capacity = capacity)

def wifi_crypto_evidence(cursor = 0, maximum = 8192):
    """Drains and decodes the current native Wi-Fi key-table capture."""
    batch = drain_bus(machine, cursor = cursor, maximum = maximum)
    return {
        "batch": batch,
        "summary": summarize_wifi_crypto_events(machine.target(), batch["events"]),
    }

def require_wifi_crypto_evidence(cursor = 0, maximum = 8192):
    """Requires a complete firmware-shaped native key programming sequence."""
    batch = drain_bus(machine, cursor = cursor, maximum = maximum)
    return {
        "batch": batch,
        "summary": require_wifi_crypto_programming(machine.target(), batch["events"]),
    }

def c6_twt_capture(capacity = 4096):
    """Starts the native C6 TWT/TSF capture."""
    return capture_c6_twt(machine, capacity = capacity)

def c6_twt_evidence(cursor = 0, maximum = 4096):
    """Drains and decodes the current native C6 TWT/TSF capture."""
    batch = drain_bus(machine, cursor = cursor, maximum = maximum)
    return {
        "batch": batch,
        "summary": summarize_c6_twt_events(batch["events"]),
    }

def require_c6_twt_evidence(cursor = 0, maximum = 4096):
    """Requires a complete firmware-shaped C6 TSF timer sequence."""
    batch = drain_bus(machine, cursor = cursor, maximum = maximum)
    return {
        "batch": batch,
        "summary": require_c6_twt_timer_programming(batch["events"]),
    }

def compact_cpu(snapshot):
    """Drops the register array when retaining a session decision artifact."""
    return {
        "architecture": snapshot["architecture"],
        "pc": snapshot["pc"],
        "halted": snapshot["halted"],
        "waiting": snapshot["waiting"],
    }

def compact_run(result):
    """Retains stop and progress evidence without duplicating UART/CPU detail."""
    return {
        "target": result["target"],
        "reason": result["reason"],
        "exit_code": result["exit_code"],
        "stats": result["stats"],
        "trace_digest": result["trace_digest"],
        "cpu": compact_cpu(result["cpu"]),
    }

def main():
    # Every agent session has an explicit native execution bound, even if the
    # first intended operation is interactive inspection.
    initial = machine.run(instructions = 1)
    print("Renvo radio lab ready for " + machine.target() + ".")
    print("Live values: machine, initial.")
    print("Run: run_slice(), drain_radio(machine), or drain_bus(machine).")
    print("Capture: wifi_crypto_capture(); C6 also supports c6_twt_capture().")
    print("Leave the REPL to emit the compact agent-session artifact.")
    repl()
    return {
        "target": machine.target(),
        "initial": compact_run(initial),
        "final_cpu": compact_cpu(machine.cpu()),
        "radio": machine.radio_events(cursor = 0, limit = 1),
        "coexistence": machine.coexistence_events(cursor = 0, limit = 1),
    }
