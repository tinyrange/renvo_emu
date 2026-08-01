# avr-libc official UART example adapter

The qualification script copies `uart.c`, `uart.h`, and `defines.h` unchanged
from the official avr-libc repository at revision
`3b40a25de8948396d0055565b791d80fbd02cab7`. The small adapter only supplies a
bounded caller and Renvo's `BREAK` stop convention. The example's UART setup,
ready polling, byte write, and newline-to-CRLF behavior execute unchanged.

Microchip's ATmega DFP 3.6.299 supplies the exact PB device header, device
specification, startup object and device library. The expected captured bytes
are `OFFICIAL\r\n`.
