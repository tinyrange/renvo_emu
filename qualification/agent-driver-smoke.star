"""Small deterministic smoke workflow for the live agent-driver boundary."""

def main():
    before = machine.cpu()
    page = machine.radio_events(cursor = 0, limit = 16)
    result = machine.run(instructions = 16)
    assert_eq(result["target"], machine.target())
    assert_true(result["stats"]["instructions"] <= 16)
    return {
        "before_pc": before["pc"],
        "after_pc": result["cpu"]["pc"],
        "radio_events": page["total"],
    }
