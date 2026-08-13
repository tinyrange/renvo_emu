"""Bounded evidence helpers for the ESP32-C6 native TWT/TSF timer block.

The register layout is recovered from the pinned ESP32-C6 vendor HAL. These
helpers only configure and interpret bus capture; they do not schedule timers,
intercept firmware, or implement TWT behavior in Starlark.
"""

_C6_TWT_LAYOUT = {
    "start": 0x600ad068,
    "end": 0x600ad0b8,
    "region": "esp32c6.phy-registers",
    "btwt_tsf": 0x600ad068,
    "timer_interrupt_enable": 0x600ad0a8,
    "timer_interrupt_clear": 0x600ad0b4,
    "itwt_clear": 0x600ad094,
    "itwt_enable": 0x600ad09c,
    "itwt_priority": 0x600ad0a0,
}

_C6_TSF_TIMER_CONTROL = [
    0x600ad074,
    0x600ad07c,
    0x600ad084,
    0x600ad08c,
]

_C6_TSF_TIMER_TARGET = [
    0x600ad078,
    0x600ad080,
    0x600ad088,
    0x600ad090,
]

def c6_twt_layout(target):
    """Returns the pinned C6 TWT/TSF capture layout."""
    if target != "esp32c6":
        fail("C6 TWT evidence is supported only for esp32c6")
    return _C6_TWT_LAYOUT

def capture_c6_twt(machine, capacity = 4096):
    """Starts a bounded write-only capture of the native C6 TWT/TSF window."""
    layout = c6_twt_layout(machine.target())
    machine.capture_bus(
        start = layout["start"],
        end = layout["end"],
        regions = [layout["region"]],
        kinds = ["write"],
        capacity = capacity,
    )
    return layout

def _timer_index(address, addresses):
    for index in range(len(addresses)):
        if address == addresses[index]:
            return index
    return None

def _timer_mask(value):
    mask = 0
    for timer in range(4):
        if value & (0x80 >> timer):
            mask |= 1 << timer
    return mask

def summarize_c6_twt_events(events):
    """Decodes captured writes using the four-timer C6 vendor HAL layout."""
    timer_targets = []
    timer_controls = []
    interrupt_enable_writes = []
    interrupt_clear_writes = []
    btwt_tsf_writes = []
    itwt_writes = []
    for event in events:
        access = event["access"]
        if access["kind"] != "Write":
            continue
        address = access["address"]
        value = access["value"]
        timer = _timer_index(address, _C6_TSF_TIMER_TARGET)
        if timer != None:
            timer_targets.append({"at": access["at"], "timer": timer, "target": value})
            continue
        timer = _timer_index(address, _C6_TSF_TIMER_CONTROL)
        if timer != None:
            timer_controls.append({
                "at": access["at"],
                "timer": timer,
                "control": value,
                "enabled": bool(value & (1 << 31)),
                "wakeup": bool(value & (1 << 30)),
                "mode": value & 7,
            })
            continue
        if address == _C6_TWT_LAYOUT["timer_interrupt_enable"]:
            interrupt_enable_writes.append({"at": access["at"], "mask": _timer_mask(value), "value": value})
        elif address == _C6_TWT_LAYOUT["timer_interrupt_clear"]:
            interrupt_clear_writes.append({"at": access["at"], "mask": _timer_mask(value), "value": value})
        elif address == _C6_TWT_LAYOUT["btwt_tsf"]:
            btwt_tsf_writes.append({"at": access["at"], "value": value})
        elif address in [_C6_TWT_LAYOUT["itwt_clear"], _C6_TWT_LAYOUT["itwt_enable"], _C6_TWT_LAYOUT["itwt_priority"]]:
            itwt_writes.append({"at": access["at"], "address": address, "value": value})
    return {
        "target": "esp32c6",
        "timer_targets": timer_targets,
        "timer_controls": timer_controls,
        "interrupt_enable_writes": interrupt_enable_writes,
        "interrupt_clear_writes": interrupt_clear_writes,
        "btwt_tsf_writes": btwt_tsf_writes,
        "itwt_writes": itwt_writes,
    }

def require_c6_twt_timer_programming(events):
    """Requires one complete vendor-shaped target/enable programming path."""
    summary = summarize_c6_twt_events(events)
    targets = {}
    enabled = {}
    for write in summary["timer_targets"]:
        targets[write["timer"]] = True
    for write in summary["timer_controls"]:
        if write["enabled"]:
            enabled[write["timer"]] = True
    matched = []
    for timer in range(4):
        bit = 1 << timer
        has_interrupt = False
        has_clear = False
        for write in summary["interrupt_enable_writes"]:
            has_interrupt = has_interrupt or bool(write["mask"] & bit)
        for write in summary["interrupt_clear_writes"]:
            has_clear = has_clear or bool(write["mask"] & bit)
        if targets.get(timer, False) and enabled.get(timer, False) and has_interrupt and has_clear:
            matched.append(timer)
    if not matched:
        fail("firmware did not complete a native C6 TSF target/enable programming path")
    summary["programmed_timers"] = matched
    return summary
