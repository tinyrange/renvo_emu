"""Small deterministic smoke workflow for the live agent-driver boundary."""

load("//qualification/starlark:agent_automation.star", "run_until")

def _one_slice(machine, result):
    return result

def main():
    before = machine.cpu()
    page = machine.radio_events(cursor = 0, limit = 16)
    outcome = run_until(machine, _one_slice, instructions = 16, max_slices = 1)
    result = outcome["result"]
    assert_eq(result["target"], machine.target())
    assert_true(result["stats"]["instructions"] <= 16)
    return {
        "before_pc": before["pc"],
        "after_pc": result["cpu"]["pc"],
        "radio_events": page["total"],
        "slices": outcome["slices"],
    }
