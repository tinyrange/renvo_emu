"""Agent-owned genuine-firmware C6 iTWT experiment and qualification."""

load("//qualification/starlark:agent_automation.star", "drain_bus")
load(
    "//qualification/starlark:c6_twt_evidence.star",
    "capture_c6_twt",
    "require_c6_twt_timer_programming",
)

_INSTRUCTION_BUDGET = 60000000

def main():
    if machine.target() != "esp32c6":
        fail("native iTWT qualification supports only esp32c6")
    capture_c6_twt(machine, capacity = 4096)
    result = machine.run(instructions = _INSTRUCTION_BUDGET)
    if result["reason"] != "InstructionLimit":
        fail("C6 iTWT firmware did not reach its instruction budget: " + str(result["reason"]))
    bus = drain_bus(machine, maximum = 4096)
    if not bus["complete"]:
        fail("C6 iTWT evidence exceeded its bounded capture")
    twt = require_c6_twt_timer_programming(bus["events"])
    return {
        "schema": "remu.radio-c6-itwt-agent.v1",
        "target": machine.target(),
        "run": {
            "reason": result["reason"],
            "instructions": result["stats"]["instructions"],
        },
        "twt": twt,
        "evidence": {
            "bus_events": len(bus["events"]),
            "bus_loss": False,
        },
    }
