"""Bounded, event-driven helpers for a live Renvo machine session."""

def run_until(machine, accept, instructions = 100000, max_slices = 1000):
    """Runs bounded slices until `accept(machine, result)` returns a value."""
    if instructions < 1 or instructions > 100000000:
        fail("instructions must be in 1..100000000")
    if max_slices < 1 or max_slices > 1000000:
        fail("max_slices must be in 1..1000000")
    for index in range(max_slices):
        result = machine.run(instructions = instructions)
        value = accept(machine, result)
        if value:
            return {
                "result": result,
                "slices": index + 1,
                "value": value,
            }
        if result["reason"] != "InstructionLimit":
            fail("machine stopped before predicate accepted: " + str(result["reason"]))
    fail("machine predicate exceeded its slice budget")

def wait_for_radio(machine, accept, instructions = 100000, max_slices = 1000, cursor = 0, page_size = 256, max_events_per_slice = 4096):
    """Runs and pages RF evidence until `accept(event)` returns a value."""
    if instructions < 1 or instructions > 100000000:
        fail("instructions must be in 1..100000000")
    if max_slices < 1 or max_slices > 1000000:
        fail("max_slices must be in 1..1000000")
    if page_size < 1 or page_size > 4096:
        fail("page_size must be in 1..4096")
    if max_events_per_slice < 1 or max_events_per_slice > 65536:
        fail("max_events_per_slice must be in 1..65536")
    scanned = 0
    for index in range(max_slices):
        result = machine.run(instructions = instructions)
        batch = drain_radio(
            machine,
            cursor = cursor,
            page_size = page_size,
            maximum = max_events_per_slice,
        )
        matched_cursor = cursor
        for event in batch["events"]:
            scanned += 1
            matched_cursor += 1
            value = accept(event)
            if value:
                return {
                    "cursor": matched_cursor,
                    "event": event,
                    "result": result,
                    "scanned": scanned,
                    "slices": index + 1,
                    "value": value,
                }
        cursor = batch["cursor"]
        if not batch["complete"]:
            fail("radio events exceeded the per-slice evidence budget")
        if result["reason"] != "InstructionLimit":
            fail("machine stopped before radio predicate accepted: " + str(result["reason"]))
    fail("radio predicate exceeded its slice budget")

def drain_radio(machine, cursor = 0, page_size = 256, maximum = 4096):
    """Returns a bounded RF page sequence and its continuation cursor."""
    if page_size < 1 or page_size > 4096:
        fail("page_size must be in 1..4096")
    if maximum < 1 or maximum > 1000000:
        fail("maximum must be in 1..1000000")
    events = []
    pages = (maximum + page_size - 1) // page_size
    for _index in range(pages):
        limit = min(page_size, maximum - len(events))
        page = machine.radio_events(cursor = cursor, limit = limit)
        events.extend(page["events"])
        cursor = page["next_cursor"]
        if page["complete"]:
            return {"complete": True, "cursor": cursor, "events": events}
    return {"complete": False, "cursor": cursor, "events": events}

def wait_for_bus(machine, accept, instructions = 100000, max_slices = 1000, cursor = 0, page_size = 256, max_events_per_slice = 4096):
    """Runs and pages a configured bus capture until `accept(access)` returns a value."""
    if instructions < 1 or instructions > 100000000:
        fail("instructions must be in 1..100000000")
    if max_slices < 1 or max_slices > 1000000:
        fail("max_slices must be in 1..1000000")
    if page_size < 1 or page_size > 4096:
        fail("page_size must be in 1..4096")
    if max_events_per_slice < 1 or max_events_per_slice > 65536:
        fail("max_events_per_slice must be in 1..65536")
    scanned = 0
    for index in range(max_slices):
        result = machine.run(instructions = instructions)
        batch = drain_bus(
            machine,
            cursor = cursor,
            page_size = page_size,
            maximum = max_events_per_slice,
        )
        matched_cursor = cursor
        for event in batch["events"]:
            scanned += 1
            matched_cursor = event["cursor"] + 1
            value = accept(event["access"])
            if value:
                return {
                    "cursor": matched_cursor,
                    "event": event,
                    "result": result,
                    "scanned": scanned,
                    "slices": index + 1,
                    "value": value,
                }
        cursor = batch["cursor"]
        if not batch["complete"]:
            fail("bus events exceeded the per-slice evidence budget")
        if result["reason"] != "InstructionLimit":
            fail("machine stopped before bus predicate accepted: " + str(result["reason"]))
    fail("bus predicate exceeded its slice budget")

def drain_bus(machine, cursor = 0, page_size = 256, maximum = 4096):
    """Returns bounded pages from a configured bus capture without hiding ring loss."""
    if page_size < 1 or page_size > 4096:
        fail("page_size must be in 1..4096")
    if maximum < 1 or maximum > 1000000:
        fail("maximum must be in 1..1000000")
    events = []
    pages = (maximum + page_size - 1) // page_size
    for _index in range(pages):
        limit = min(page_size, maximum - len(events))
        page = machine.bus_events(cursor = cursor, limit = limit)
        if page["missed_before_cursor"]:
            fail("bus capture ring dropped %s requested events" % page["missed_before_cursor"])
        events.extend(page["events"])
        cursor = page["next_cursor"]
        if page["complete"]:
            return {"complete": True, "cursor": cursor, "events": events}
    return {"complete": False, "cursor": cursor, "events": events}
