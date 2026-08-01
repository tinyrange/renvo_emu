# Renesas FSP example qualification adapter

The qualification script copies `gpt_timer.c` and `uart_ep.c` unchanged from
the EK-RA4M1 projects in `renesas/ra-fsp-examples` revision
`01a411dfc2e9808f489070c780a554a5bead6714`. The compact headers and adapter are
the documented linker/startup test harness: FSP calls are translated to the
same RA4M1 GPT0 and SCI9 registers modeled by Renvo Emulator. The official helper logic
for open/start, UART write completion, validation, and callbacks is not edited.
