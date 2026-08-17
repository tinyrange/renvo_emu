# ESP32-C6 RF register and observed-value reference

This is an emulator-only, evidence-bounded reference generated from the pinned
C6 public-API oracle bus trace. It is not a silicon programming manual. `high`,
`medium`, and `observed-only` are claim strengths; unknown fields remain unknown.
Program counters are provenance only and never affect runtime behavior.

The union contains **1096 registers**: **478 trace-observed** and **674 semantically implemented** (with overlap). The trace contains **6872 distinct per-register read/write value observations**; **422 observed registers retain explicit semantic unknowns** across 14 RF regions.
The companion JSON contains every value and observational PC; tables below show
all addresses but abbreviate value sets longer than eight entries.
Modeled entries also name the repository source that implements their behavior.

## Semantically implemented registers

| Address | Name | Function | Mask / reset | Confidence |
|---|---|---|---|---|
| `0x600a00c0` | `RFPLL_CHANNEL_CONTROL` | RFPLL mode/channel code and start strobe | `unknown / 0x00000000` | high |
| `0x600a00cc` | `RFPLL_CHANNEL_STATUS` | RFPLL completion status | `unknown / 0x00000000` | medium |
| `0x600a0418` | `POWER_DETECTOR_CONVERSION` | start and synchronously complete RF power conversion | `unknown / 0x00000000` | high |
| `0x600a0474` | `IQ_ESTIMATE_CONTROL` | IQ calibration start strobe | `unknown / 0x00000000` | high |
| `0x600a04a0` | `IQ_ESTIMATE_STATUS` | IQ calibration completion status | `unknown / 0x00000000` | medium |
| `0x600a0810` | `TX_TONE_CONTROL` | power-detector tone control | `unknown / 0x00000000` | medium |
| `0x600a0814` | `TX_TONE_STATUS` | power-detector tone status | `unknown / 0x00000000` | medium |
| `0x600a08cc` | `TX_GAIN_FIRST` | first word of a 43-entry TX gain tuple | `unknown / 0x00000000` | high |
| `0x600a08d0` | `TX_GAIN_SECOND` | second word of a 43-entry TX gain tuple | `unknown / 0x00000000` | high |
| `0x600a08d4` | `TX_GAIN_FINAL` | final word; completes tuple and encodes power ceiling | `unknown / 0x00000000` | high |
| `0x600a0910` | `RF_FRONTEND_FORCE` | force-off/release state for Wi-Fi frontend | `unknown / 0x00000000` | high |
| `0x600a1028` | `BLE_SCHEDULER_KICK` | submit the scheduler head descriptor | `unknown / 0x00000000` | high |
| `0x600a102c` | `BLE_SCHEDULER_STOP` | stop the current BLE schedule | `unknown / 0x00000000` | high |
| `0x600a1304` | `BLE_INTERRUPT_ENABLE0` | BLE event enable bank 0 | `unknown / 0x00000000` | high |
| `0x600a1308` | `BLE_INTERRUPT_CLEAR0` | BLE event clear bank 0 | `unknown / 0x00000000` | high |
| `0x600a130c` | `BLE_INTERRUPT_RAW0` | BLE raw event bank 0 | `unknown / 0x00000000` | high |
| `0x600a1314` | `BLE_INTERRUPT_ENABLE1` | BLE event enable bank 1 | `unknown / 0x00000000` | high |
| `0x600a1318` | `BLE_INTERRUPT_CLEAR1` | BLE event clear bank 1 | `unknown / 0x00000000` | high |
| `0x600a131c` | `BLE_INTERRUPT_RAW1` | BLE raw event bank 1 | `unknown / 0x00000000` | high |
| `0x600a18fc` | `BLE_SCHEDULER_HEAD` | first pending BLE schedule descriptor | `unknown / 0x00000000` | high |
| `0x600a1900` | `BLE_SCHEDULER_CURRENT` | active BLE schedule descriptor and ownership | `unknown / 0x00000000` | high |
| `0x600a1904` | `BLE_SCHEDULER_NEXT` | successor BLE schedule descriptor | `unknown / 0x00000000` | high |
| `0x600a1924` | `BLE_TIMER_CURRENT` | hardware-owned BLE scheduler time | `unknown / 0x00000000` | high |
| `0x600a1960` | `BLE_CURRENT_TX_BUFFER` | current BLE TX buffer descriptor | `unknown / 0x00000000` | high |
| `0x600a1964` | `BLE_CURRENT_RX_BUFFER` | current BLE RX buffer descriptor | `unknown / 0x00000000` | high |
| `0x600a1ff0` | `BLE_BASEBAND_RESET` | BLE baseband reset edge | `unknown / 0x00000000` | high |
| `0x600a3000` | `IEEE802154_COMMAND` | execute TX, RX, CCA, energy-detect, test, stop, or timer command | `0x000000ff / 0x00000000` | high |
| `0x600a3004` | `IEEE802154_REGISTER_004` | modeled writable register; field semantics are not yet established | `0xfbc058eb / 0x00000000` | medium |
| `0x600a3008` | `IEEE802154_REGISTER_008` | modeled writable register; field semantics are not yet established | `0x0000ffff / 0x00000000` | medium |
| `0x600a300c` | `IEEE802154_REGISTER_00C` | modeled writable register; field semantics are not yet established | `0x0000ffff / 0x00000000` | medium |
| `0x600a3010` | `IEEE802154_REGISTER_010` | modeled writable register; field semantics are not yet established | `0xffffffff / 0x00000000` | medium |
| `0x600a3014` | `IEEE802154_REGISTER_014` | modeled writable register; field semantics are not yet established | `0xffffffff / 0x00000000` | medium |
| `0x600a3018` | `IEEE802154_REGISTER_018` | modeled writable register; field semantics are not yet established | `0x0000ffff / 0x00000000` | medium |
| `0x600a301c` | `IEEE802154_REGISTER_01C` | modeled writable register; field semantics are not yet established | `0x0000ffff / 0x00000000` | medium |
| `0x600a3020` | `IEEE802154_REGISTER_020` | modeled writable register; field semantics are not yet established | `0xffffffff / 0x00000000` | medium |
| `0x600a3024` | `IEEE802154_REGISTER_024` | modeled writable register; field semantics are not yet established | `0xffffffff / 0x00000000` | medium |
| `0x600a3028` | `IEEE802154_REGISTER_028` | modeled writable register; field semantics are not yet established | `0x0000ffff / 0x00000000` | medium |
| `0x600a302c` | `IEEE802154_REGISTER_02C` | modeled writable register; field semantics are not yet established | `0x0000ffff / 0x00000000` | medium |
| `0x600a3030` | `IEEE802154_REGISTER_030` | modeled writable register; field semantics are not yet established | `0xffffffff / 0x00000000` | medium |
| `0x600a3034` | `IEEE802154_REGISTER_034` | modeled writable register; field semantics are not yet established | `0xffffffff / 0x00000000` | medium |
| `0x600a3038` | `IEEE802154_REGISTER_038` | modeled writable register; field semantics are not yet established | `0x0000ffff / 0x00000000` | medium |
| `0x600a303c` | `IEEE802154_REGISTER_03C` | modeled writable register; field semantics are not yet established | `0x0000ffff / 0x00000000` | medium |
| `0x600a3040` | `IEEE802154_REGISTER_040` | modeled writable register; field semantics are not yet established | `0xffffffff / 0x00000000` | medium |
| `0x600a3044` | `IEEE802154_REGISTER_044` | modeled writable register; field semantics are not yet established | `0xffffffff / 0x00000000` | medium |
| `0x600a3048` | `IEEE802154_CHANNEL` | 802.15.4 channel selection | `0x0000007f / 0x00000000` | high |
| `0x600a304c` | `IEEE802154_TX_POWER` | 802.15.4 transmit-power selection | `0x0000001f / 0x00000000` | high |
| `0x600a3050` | `IEEE802154_ED_DURATION` | energy-detection duration in symbols | `0x0f00ffff / 0x00000000` | high |
| `0x600a3054` | `IEEE802154_REGISTER_054` | modeled writable register; field semantics are not yet established | `0x0000ffff / 0x00000000` | medium |
| `0x600a3058` | `IEEE802154_REGISTER_058` | modeled writable register; field semantics are not yet established | `0x03ff00ff / 0x00000000` | medium |
| `0x600a305c` | `IEEE802154_REGISTER_05C` | modeled writable register; field semantics are not yet established | `0x0000ffff / 0x00000000` | medium |
| `0x600a3060` | `IEEE802154_EVENT_ENABLE` | 802.15.4 event enable mask | `0x00001fff / 0x00000000` | high |
| `0x600a3064` | `IEEE802154_EVENT_CLEAR` | write-one-to-clear event state | `0x00001fff / 0x00000000` | high |
| `0x600a3068` | `IEEE802154_REGISTER_068` | modeled writable register; field semantics are not yet established | `0x7fffffff / 0x00000000` | medium |
| `0x600a306c` | `IEEE802154_REGISTER_06C` | modeled writable register; field semantics are not yet established | `0xffff0001 / 0x00000000` | medium |
| `0x600a3070` | `IEEE802154_REGISTER_070` | modeled writable register; field semantics are not yet established | `0x000001ff / 0x00000000` | medium |
| `0x600a3078` | `IEEE802154_REGISTER_078` | modeled writable register; field semantics are not yet established | `0x7fffffff / 0x00000000` | medium |
| `0x600a307c` | `IEEE802154_REGISTER_07C` | modeled writable register; field semantics are not yet established | `0xffffffff / 0x00000000` | medium |
| `0x600a30a8` | `IEEE802154_TIMER0_THRESHOLD` | MAC timer 0 threshold | `0xffffffff / 0x00000000` | high |
| `0x600a30ac` | `IEEE802154_TIMER0_VALUE` | elapsed MAC timer 0 ticks | `unknown / 0x00000000` | high |
| `0x600a30b0` | `IEEE802154_TIMER1_THRESHOLD` | MAC timer 1 threshold | `0xffffffff / 0x00000000` | high |
| `0x600a30b4` | `IEEE802154_TIMER1_VALUE` | elapsed MAC timer 1 ticks | `unknown / 0x00000000` | high |
| `0x600a30b8` | `IEEE802154_REGISTER_0B8` | modeled writable register; field semantics are not yet established | `0xffffffff / 0x00000000` | medium |
| `0x600a30c4` | `IEEE802154_REGISTER_0C4` | modeled writable register; field semantics are not yet established | `0xffffffff / 0x00000000` | medium |
| `0x600a30c8` | `IEEE802154_REGISTER_0C8` | modeled writable register; field semantics are not yet established | `0xffffffff / 0x00000000` | medium |
| `0x600a30d0` | `IEEE802154_TX_DMA` | TX DMA descriptor address | `0xffffffff / 0x00000000` | high |
| `0x600a30d4` | `IEEE802154_REGISTER_0D4` | modeled writable register; field semantics are not yet established | `0x00000007 / 0x00000000` | medium |
| `0x600a30e0` | `IEEE802154_RX_DMA` | RX DMA descriptor address | `0xffffffff / 0x00000000` | high |
| `0x600a30e4` | `IEEE802154_REGISTER_0E4` | modeled writable register; field semantics are not yet established | `0x03000007 / 0x00000000` | medium |
| `0x600a30f0` | `IEEE802154_REGISTER_0F0` | modeled writable register; field semantics are not yet established | `0xffffffff / 0x00000000` | medium |
| `0x600a30f4` | `IEEE802154_REGISTER_0F4` | modeled writable register; field semantics are not yet established | `0xffffffff / 0x00000000` | medium |
| `0x600a3100` | `IEEE802154_REGISTER_100` | modeled writable register; field semantics are not yet established | `0xffffffff / 0x00000000` | medium |
| `0x600a3104` | `IEEE802154_REGISTER_104` | modeled writable register; field semantics are not yet established | `0xffffffff / 0x00000000` | medium |
| `0x600a3108` | `IEEE802154_REGISTER_108` | modeled writable register; field semantics are not yet established | `0xffffffff / 0x00000000` | medium |
| `0x600a310c` | `IEEE802154_REGISTER_10C` | modeled writable register; field semantics are not yet established | `0xffffffff / 0x00000000` | medium |
| `0x600a3110` | `IEEE802154_REGISTER_110` | modeled writable register; field semantics are not yet established | `0xffffffff / 0x00000000` | medium |
| `0x600a3114` | `IEEE802154_REGISTER_114` | modeled writable register; field semantics are not yet established | `0xffffffff / 0x00000000` | medium |
| `0x600a3118` | `IEEE802154_REGISTER_118` | modeled writable register; field semantics are not yet established | `0xffffffff / 0x00000000` | medium |
| `0x600a311c` | `IEEE802154_REGISTER_11C` | modeled writable register; field semantics are not yet established | `0xffffffff / 0x00000000` | medium |
| `0x600a3120` | `IEEE802154_REGISTER_120` | modeled writable register; field semantics are not yet established | `0xffffffff / 0x00000000` | medium |
| `0x600a3128` | `IEEE802154_SECURITY_CONTROL` | frame-security control | `0x00007f01 / 0x00000000` | high |
| `0x600a312c` | `IEEE802154_REGISTER_12C` | modeled writable register; field semantics are not yet established | `0xffffffff / 0x00000000` | medium |
| `0x600a3130` | `IEEE802154_REGISTER_130` | modeled writable register; field semantics are not yet established | `0xffffffff / 0x00000000` | medium |
| `0x600a3134` | `IEEE802154_REGISTER_134` | modeled writable register; field semantics are not yet established | `0xffffffff / 0x00000000` | medium |
| `0x600a3138` | `IEEE802154_REGISTER_138` | modeled writable register; field semantics are not yet established | `0xffffffff / 0x00000000` | medium |
| `0x600a313c` | `IEEE802154_REGISTER_13C` | modeled writable register; field semantics are not yet established | `0xffffffff / 0x00000000` | medium |
| `0x600a3140` | `IEEE802154_REGISTER_140` | modeled writable register; field semantics are not yet established | `0xffffffff / 0x00000000` | medium |
| `0x600a3144` | `IEEE802154_COUNTER_144` | modeled MAC statistic counter | `unknown / 0x00000000` | medium |
| `0x600a3148` | `IEEE802154_COUNTER_148` | modeled MAC statistic counter | `unknown / 0x00000000` | medium |
| `0x600a314c` | `IEEE802154_COUNTER_14C` | modeled MAC statistic counter | `unknown / 0x00000000` | medium |
| `0x600a3150` | `IEEE802154_COUNTER_150` | modeled MAC statistic counter | `unknown / 0x00000000` | medium |
| `0x600a3154` | `IEEE802154_COUNTER_154` | modeled MAC statistic counter | `unknown / 0x00000000` | medium |
| `0x600a3158` | `IEEE802154_COUNTER_158` | modeled MAC statistic counter | `unknown / 0x00000000` | medium |
| `0x600a315c` | `IEEE802154_COUNTER_15C` | modeled MAC statistic counter | `unknown / 0x00000000` | medium |
| `0x600a3160` | `IEEE802154_COUNTER_160` | modeled MAC statistic counter | `unknown / 0x00000000` | medium |
| `0x600a3164` | `IEEE802154_COUNTER_164` | modeled MAC statistic counter | `unknown / 0x00000000` | medium |
| `0x600a3168` | `IEEE802154_COUNTER_168` | modeled MAC statistic counter | `unknown / 0x00000000` | medium |
| `0x600a316c` | `IEEE802154_COUNTER_16C` | modeled MAC statistic counter | `unknown / 0x00000000` | medium |
| `0x600a3170` | `IEEE802154_COUNTER_170` | modeled MAC statistic counter | `unknown / 0x00000000` | medium |
| `0x600a3174` | `IEEE802154_COUNTER_174` | modeled MAC statistic counter | `unknown / 0x00000000` | medium |
| `0x600a3178` | `IEEE802154_COUNTER_178` | modeled MAC statistic counter | `unknown / 0x00000000` | medium |
| `0x600a317c` | `IEEE802154_COUNTER_17C` | modeled MAC statistic counter | `unknown / 0x00000000` | medium |
| `0x600a3180` | `IEEE802154_COUNTER_CLEAR` | write-one-to-clear statistic counters | `0x00007fff / 0x00000000` | high |
| `0x600a3184` | `IEEE802154_DATE` | hardware date/version value | `0xffffffff / 0x00220622` | high |
| `0x600a405c` | `WIFI_INTERFACE0_LOW` | station interface address bytes 0..3 | `unknown / 0x00000000` | high |
| `0x600a4060` | `WIFI_INTERFACE0_HIGH` | station address bytes 4..5 and valid bit | `unknown / 0x00000000` | high |
| `0x600a4064` | `WIFI_INTERFACE1_LOW` | interface MAC address bytes 0..3 | `unknown / 0x00000000` | high |
| `0x600a4068` | `WIFI_INTERFACE1_HIGH` | interface MAC address bytes 4..5 and valid bit | `unknown / 0x00000000` | high |
| `0x600a406c` | `WIFI_INTERFACE2_LOW` | interface MAC address bytes 0..3 | `unknown / 0x00000000` | high |
| `0x600a4070` | `WIFI_INTERFACE2_HIGH` | interface MAC address bytes 4..5 and valid bit | `unknown / 0x00000000` | high |
| `0x600a4074` | `WIFI_INTERFACE3_LOW` | interface MAC address bytes 0..3 | `unknown / 0x00000000` | high |
| `0x600a4078` | `WIFI_INTERFACE3_HIGH` | interface MAC address bytes 4..5 and valid bit | `unknown / 0x00000000` | high |
| `0x600a4080` | `WIFI_RX_CONTROL` | RX descriptor reload command | `unknown / 0x00000000` | high |
| `0x600a4084` | `WIFI_RX_BASE` | firmware-owned RX descriptor base | `unknown / 0x00000000` | high |
| `0x600a4088` | `WIFI_RX_NEXT` | next RX DMA descriptor selected by the model | `unknown / 0x00000000` | high |
| `0x600a408c` | `WIFI_RX_LAST` | last completed RX DMA descriptor | `unknown / 0x00000000` | high |
| `0x600a4178` | `WIFI_RX_BA7_CONTROL` | RX block-ack agreement control | `unknown / 0x00000000` | high |
| `0x600a417c` | `WIFI_RX_BA7_MAC_HIGH` | RX block-ack peer address high bits | `unknown / 0x00000000` | high |
| `0x600a4180` | `WIFI_RX_BA7_MAC_LOW` | RX block-ack peer address low bits | `unknown / 0x00000000` | high |
| `0x600a4188` | `WIFI_RX_BA7_SEQUENCE` | RX block-ack window origin | `unknown / 0x00000000` | high |
| `0x600a4190` | `WIFI_RX_BA7_BITMAP_LOW` | RX block-ack receive bitmap bits 0..31 | `unknown / 0x00000000` | high |
| `0x600a4198` | `WIFI_RX_BA7_BITMAP_HIGH` | RX block-ack receive bitmap bits 32..63 | `unknown / 0x00000000` | high |
| `0x600a41a0` | `WIFI_RX_BA6_CONTROL` | RX block-ack agreement control | `unknown / 0x00000000` | high |
| `0x600a41a4` | `WIFI_RX_BA6_MAC_HIGH` | RX block-ack peer address high bits | `unknown / 0x00000000` | high |
| `0x600a41a8` | `WIFI_RX_BA6_MAC_LOW` | RX block-ack peer address low bits | `unknown / 0x00000000` | high |
| `0x600a41b0` | `WIFI_RX_BA6_SEQUENCE` | RX block-ack window origin | `unknown / 0x00000000` | high |
| `0x600a41b8` | `WIFI_RX_BA6_BITMAP_LOW` | RX block-ack receive bitmap bits 0..31 | `unknown / 0x00000000` | high |
| `0x600a41c0` | `WIFI_RX_BA6_BITMAP_HIGH` | RX block-ack receive bitmap bits 32..63 | `unknown / 0x00000000` | high |
| `0x600a41c8` | `WIFI_RX_BA5_CONTROL` | RX block-ack agreement control | `unknown / 0x00000000` | high |
| `0x600a41cc` | `WIFI_RX_BA5_MAC_HIGH` | RX block-ack peer address high bits | `unknown / 0x00000000` | high |
| `0x600a41d0` | `WIFI_RX_BA5_MAC_LOW` | RX block-ack peer address low bits | `unknown / 0x00000000` | high |
| `0x600a41d8` | `WIFI_RX_BA5_SEQUENCE` | RX block-ack window origin | `unknown / 0x00000000` | high |
| `0x600a41e0` | `WIFI_RX_BA5_BITMAP_LOW` | RX block-ack receive bitmap bits 0..31 | `unknown / 0x00000000` | high |
| `0x600a41e8` | `WIFI_RX_BA5_BITMAP_HIGH` | RX block-ack receive bitmap bits 32..63 | `unknown / 0x00000000` | high |
| `0x600a41f0` | `WIFI_RX_BA4_CONTROL` | RX block-ack agreement control | `unknown / 0x00000000` | high |
| `0x600a41f4` | `WIFI_RX_BA4_MAC_HIGH` | RX block-ack peer address high bits | `unknown / 0x00000000` | high |
| `0x600a41f8` | `WIFI_RX_BA4_MAC_LOW` | RX block-ack peer address low bits | `unknown / 0x00000000` | high |
| `0x600a4200` | `WIFI_RX_BA4_SEQUENCE` | RX block-ack window origin | `unknown / 0x00000000` | high |
| `0x600a4208` | `WIFI_RX_BA4_BITMAP_LOW` | RX block-ack receive bitmap bits 0..31 | `unknown / 0x00000000` | high |
| `0x600a4210` | `WIFI_RX_BA4_BITMAP_HIGH` | RX block-ack receive bitmap bits 32..63 | `unknown / 0x00000000` | high |
| `0x600a4218` | `WIFI_RX_BA3_CONTROL` | RX block-ack agreement control | `unknown / 0x00000000` | high |
| `0x600a421c` | `WIFI_RX_BA3_MAC_HIGH` | RX block-ack peer address high bits | `unknown / 0x00000000` | high |
| `0x600a4220` | `WIFI_RX_BA3_MAC_LOW` | RX block-ack peer address low bits | `unknown / 0x00000000` | high |
| `0x600a4228` | `WIFI_RX_BA3_SEQUENCE` | RX block-ack window origin | `unknown / 0x00000000` | high |
| `0x600a4230` | `WIFI_RX_BA3_BITMAP_LOW` | RX block-ack receive bitmap bits 0..31 | `unknown / 0x00000000` | high |
| `0x600a4238` | `WIFI_RX_BA3_BITMAP_HIGH` | RX block-ack receive bitmap bits 32..63 | `unknown / 0x00000000` | high |
| `0x600a4240` | `WIFI_RX_BA2_CONTROL` | RX block-ack agreement control | `unknown / 0x00000000` | high |
| `0x600a4244` | `WIFI_RX_BA2_MAC_HIGH` | RX block-ack peer address high bits | `unknown / 0x00000000` | high |
| `0x600a4248` | `WIFI_RX_BA2_MAC_LOW` | RX block-ack peer address low bits | `unknown / 0x00000000` | high |
| `0x600a4250` | `WIFI_RX_BA2_SEQUENCE` | RX block-ack window origin | `unknown / 0x00000000` | high |
| `0x600a4258` | `WIFI_RX_BA2_BITMAP_LOW` | RX block-ack receive bitmap bits 0..31 | `unknown / 0x00000000` | high |
| `0x600a4260` | `WIFI_RX_BA2_BITMAP_HIGH` | RX block-ack receive bitmap bits 32..63 | `unknown / 0x00000000` | high |
| `0x600a4268` | `WIFI_RX_BA1_CONTROL` | RX block-ack agreement control | `unknown / 0x00000000` | high |
| `0x600a426c` | `WIFI_RX_BA1_MAC_HIGH` | RX block-ack peer address high bits | `unknown / 0x00000000` | high |
| `0x600a4270` | `WIFI_RX_BA1_MAC_LOW` | RX block-ack peer address low bits | `unknown / 0x00000000` | high |
| `0x600a4278` | `WIFI_RX_BA1_SEQUENCE` | RX block-ack window origin | `unknown / 0x00000000` | high |
| `0x600a4280` | `WIFI_RX_BA1_BITMAP_LOW` | RX block-ack receive bitmap bits 0..31 | `unknown / 0x00000000` | high |
| `0x600a4288` | `WIFI_RX_BA1_BITMAP_HIGH` | RX block-ack receive bitmap bits 32..63 | `unknown / 0x00000000` | high |
| `0x600a4290` | `WIFI_RX_BA0_CONTROL` | RX block-ack agreement control | `unknown / 0x00000000` | high |
| `0x600a4294` | `WIFI_RX_BA0_MAC_HIGH` | RX block-ack peer address high bits | `unknown / 0x00000000` | high |
| `0x600a4298` | `WIFI_RX_BA0_MAC_LOW` | RX block-ack peer address low bits | `unknown / 0x00000000` | high |
| `0x600a42a0` | `WIFI_RX_BA0_SEQUENCE` | RX block-ack window origin | `unknown / 0x00000000` | high |
| `0x600a42a8` | `WIFI_RX_BA0_BITMAP_LOW` | RX block-ack receive bitmap bits 0..31 | `unknown / 0x00000000` | high |
| `0x600a42b0` | `WIFI_RX_BA0_BITMAP_HIGH` | RX block-ack receive bitmap bits 32..63 | `unknown / 0x00000000` | high |
| `0x600a4814` | `WIFI_CRYPTO_VALID` | valid bitmap for 32 native key slots | `unknown / 0x00000000` | high |
| `0x600a4c40` | `WIFI_INTERRUPT_MASK` | Wi-Fi MAC interrupt mask | `unknown / 0x00000000` | high |
| `0x600a4c48` | `WIFI_INTERRUPT_EVENT` | latched Wi-Fi MAC interrupt events | `unknown / 0x00000000` | high |
| `0x600a4c4c` | `WIFI_INTERRUPT_CLEAR` | write-one-to-clear Wi-Fi events | `unknown / 0x00000000` | high |
| `0x600a4c70` | `WIFI_RX_ADDRESS_HIGH` | high address bits for RX DMA descriptors | `unknown / 0x00000000` | high |
| `0x600a4cb4` | `WIFI_TX_QUEUE_STATE_CLEAR` | clear completed TX queue bits | `unknown / 0x00000000` | high |
| `0x600a4cb8` | `WIFI_TX_QUEUE_STATE` | completed TX queue bitmap | `unknown / 0x00000000` | high |
| `0x600a4d10` | `WIFI_TX_QUEUE5_PROTECTION` | queue RTS/protection configuration | `unknown / 0x00000000` | high |
| `0x600a4d18` | `WIFI_TX_QUEUE5_TIMEOUT` | queue transmission timeout | `unknown / 0x00000000` | high |
| `0x600a4d1c` | `WIFI_TX_QUEUE5_CONTROL` | queue enable and TX descriptor pointer | `unknown / 0x00000000` | high |
| `0x600a4d20` | `WIFI_TX_QUEUE4_PROTECTION` | queue RTS/protection configuration | `unknown / 0x00000000` | high |
| `0x600a4d28` | `WIFI_TX_QUEUE4_TIMEOUT` | queue transmission timeout | `unknown / 0x00000000` | high |
| `0x600a4d2c` | `WIFI_TX_QUEUE4_CONTROL` | queue enable and TX descriptor pointer | `unknown / 0x00000000` | high |
| `0x600a4d30` | `WIFI_TX_QUEUE3_PROTECTION` | queue RTS/protection configuration | `unknown / 0x00000000` | high |
| `0x600a4d38` | `WIFI_TX_QUEUE3_TIMEOUT` | queue transmission timeout | `unknown / 0x00000000` | high |
| `0x600a4d3c` | `WIFI_TX_QUEUE3_CONTROL` | queue enable and TX descriptor pointer | `unknown / 0x00000000` | high |
| `0x600a4d40` | `WIFI_TX_QUEUE2_PROTECTION` | queue RTS/protection configuration | `unknown / 0x00000000` | high |
| `0x600a4d48` | `WIFI_TX_QUEUE2_TIMEOUT` | queue transmission timeout | `unknown / 0x00000000` | high |
| `0x600a4d4c` | `WIFI_TX_QUEUE2_CONTROL` | queue enable and TX descriptor pointer | `unknown / 0x00000000` | high |
| `0x600a4d50` | `WIFI_TX_QUEUE1_PROTECTION` | queue RTS/protection configuration | `unknown / 0x00000000` | high |
| `0x600a4d58` | `WIFI_TX_QUEUE1_TIMEOUT` | queue transmission timeout | `unknown / 0x00000000` | high |
| `0x600a4d5c` | `WIFI_TX_QUEUE1_CONTROL` | queue enable and TX descriptor pointer | `unknown / 0x00000000` | high |
| `0x600a4d60` | `WIFI_TX_QUEUE0_PROTECTION` | queue RTS/protection configuration | `unknown / 0x00000000` | high |
| `0x600a4d68` | `WIFI_TX_QUEUE0_TIMEOUT` | queue transmission timeout | `unknown / 0x00000000` | high |
| `0x600a4d6c` | `WIFI_TX_QUEUE0_CONTROL` | queue-0 enable and descriptor pointer | `unknown / 0x00000000` | high |
| `0x600a4ddc` | `WIFI_RESET_CONTROL` | Wi-Fi reset strobe and ready acknowledgement | `unknown / 0x00000000` | high |
| `0x600a5290` | `WIFI_TX_QUEUE5_BA_BITMAP_HIGH` | TX block-ack bitmap bits 32..63 | `unknown / 0x00000000` | high |
| `0x600a5294` | `WIFI_TX_QUEUE5_BA_BITMAP_LOW` | TX block-ack bitmap bits 0..31 | `unknown / 0x00000000` | high |
| `0x600a5298` | `WIFI_TX_QUEUE5_BA_STATUS` | TX block-ack status and starting sequence | `unknown / 0x00000000` | high |
| `0x600a52a4` | `WIFI_TX_QUEUE5_COMPLETION_COUNT` | TX completion count | `unknown / 0x00000000` | high |
| `0x600a52a8` | `WIFI_TX_QUEUE5_COMPLETION` | TX completion status | `unknown / 0x00000000` | high |
| `0x600a5304` | `WIFI_TX_QUEUE4_BA_BITMAP_HIGH` | TX block-ack bitmap bits 32..63 | `unknown / 0x00000000` | high |
| `0x600a5308` | `WIFI_TX_QUEUE4_BA_BITMAP_LOW` | TX block-ack bitmap bits 0..31 | `unknown / 0x00000000` | high |
| `0x600a530c` | `WIFI_TX_QUEUE4_BA_STATUS` | TX block-ack status and starting sequence | `unknown / 0x00000000` | high |
| `0x600a5318` | `WIFI_TX_QUEUE4_COMPLETION_COUNT` | TX completion count | `unknown / 0x00000000` | high |
| `0x600a531c` | `WIFI_TX_QUEUE4_COMPLETION` | TX completion status | `unknown / 0x00000000` | high |
| `0x600a5378` | `WIFI_TX_QUEUE3_BA_BITMAP_HIGH` | TX block-ack bitmap bits 32..63 | `unknown / 0x00000000` | high |
| `0x600a537c` | `WIFI_TX_QUEUE3_BA_BITMAP_LOW` | TX block-ack bitmap bits 0..31 | `unknown / 0x00000000` | high |
| `0x600a5380` | `WIFI_TX_QUEUE3_BA_STATUS` | TX block-ack status and starting sequence | `unknown / 0x00000000` | high |
| `0x600a538c` | `WIFI_TX_QUEUE3_COMPLETION_COUNT` | TX completion count | `unknown / 0x00000000` | high |
| `0x600a5390` | `WIFI_TX_QUEUE3_COMPLETION` | TX completion status | `unknown / 0x00000000` | high |
| `0x600a53ec` | `WIFI_TX_QUEUE2_BA_BITMAP_HIGH` | TX block-ack bitmap bits 32..63 | `unknown / 0x00000000` | high |
| `0x600a53f0` | `WIFI_TX_QUEUE2_BA_BITMAP_LOW` | TX block-ack bitmap bits 0..31 | `unknown / 0x00000000` | high |
| `0x600a53f4` | `WIFI_TX_QUEUE2_BA_STATUS` | TX block-ack status and starting sequence | `unknown / 0x00000000` | high |
| `0x600a5400` | `WIFI_TX_QUEUE2_COMPLETION_COUNT` | TX completion count | `unknown / 0x00000000` | high |
| `0x600a5404` | `WIFI_TX_QUEUE2_COMPLETION` | TX completion status | `unknown / 0x00000000` | high |
| `0x600a5460` | `WIFI_TX_QUEUE1_BA_BITMAP_HIGH` | TX block-ack bitmap bits 32..63 | `unknown / 0x00000000` | high |
| `0x600a5464` | `WIFI_TX_QUEUE1_BA_BITMAP_LOW` | TX block-ack bitmap bits 0..31 | `unknown / 0x00000000` | high |
| `0x600a5468` | `WIFI_TX_QUEUE1_BA_STATUS` | TX block-ack status and starting sequence | `unknown / 0x00000000` | high |
| `0x600a5474` | `WIFI_TX_QUEUE1_COMPLETION_COUNT` | TX completion count | `unknown / 0x00000000` | high |
| `0x600a5478` | `WIFI_TX_QUEUE1_COMPLETION` | TX completion status | `unknown / 0x00000000` | high |
| `0x600a54d4` | `WIFI_TX_QUEUE0_BA_BITMAP_HIGH` | TX block-ack bitmap bits 32..63 | `unknown / 0x00000000` | high |
| `0x600a54d8` | `WIFI_TX_QUEUE0_BA_BITMAP_LOW` | TX block-ack bitmap bits 0..31 | `unknown / 0x00000000` | high |
| `0x600a54dc` | `WIFI_TX_QUEUE0_BA_STATUS` | TX block-ack status and starting sequence | `unknown / 0x00000000` | high |
| `0x600a54e8` | `WIFI_TX_QUEUE0_COMPLETION_COUNT` | TX completion count | `unknown / 0x00000000` | high |
| `0x600a54ec` | `WIFI_TX_QUEUE0_COMPLETION` | TX completion status | `unknown / 0x00000000` | high |
| `0x600a5800` | `WIFI_CRYPTO_SLOT0` | first word of native crypto slot 0 | `unknown / 0x00000000` | high |
| `0x600a5804` | `WIFI_CRYPTO_SLOT0_WORD1` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5808` | `WIFI_CRYPTO_SLOT0_WORD2` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a580c` | `WIFI_CRYPTO_SLOT0_WORD3` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5810` | `WIFI_CRYPTO_SLOT0_WORD4` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5814` | `WIFI_CRYPTO_SLOT0_WORD5` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5818` | `WIFI_CRYPTO_SLOT0_WORD6` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a581c` | `WIFI_CRYPTO_SLOT0_WORD7` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5820` | `WIFI_CRYPTO_SLOT0_WORD8` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5824` | `WIFI_CRYPTO_SLOT0_WORD9` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5828` | `WIFI_CRYPTO_SLOT1_WORD0` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a582c` | `WIFI_CRYPTO_SLOT1_WORD1` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5830` | `WIFI_CRYPTO_SLOT1_WORD2` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5834` | `WIFI_CRYPTO_SLOT1_WORD3` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5838` | `WIFI_CRYPTO_SLOT1_WORD4` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a583c` | `WIFI_CRYPTO_SLOT1_WORD5` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5840` | `WIFI_CRYPTO_SLOT1_WORD6` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5844` | `WIFI_CRYPTO_SLOT1_WORD7` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5848` | `WIFI_CRYPTO_SLOT1_WORD8` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a584c` | `WIFI_CRYPTO_SLOT1_WORD9` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5850` | `WIFI_CRYPTO_SLOT2_WORD0` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5854` | `WIFI_CRYPTO_SLOT2_WORD1` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5858` | `WIFI_CRYPTO_SLOT2_WORD2` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a585c` | `WIFI_CRYPTO_SLOT2_WORD3` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5860` | `WIFI_CRYPTO_SLOT2_WORD4` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5864` | `WIFI_CRYPTO_SLOT2_WORD5` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5868` | `WIFI_CRYPTO_SLOT2_WORD6` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a586c` | `WIFI_CRYPTO_SLOT2_WORD7` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5870` | `WIFI_CRYPTO_SLOT2_WORD8` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5874` | `WIFI_CRYPTO_SLOT2_WORD9` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5878` | `WIFI_CRYPTO_SLOT3_WORD0` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a587c` | `WIFI_CRYPTO_SLOT3_WORD1` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5880` | `WIFI_CRYPTO_SLOT3_WORD2` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5884` | `WIFI_CRYPTO_SLOT3_WORD3` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5888` | `WIFI_CRYPTO_SLOT3_WORD4` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a588c` | `WIFI_CRYPTO_SLOT3_WORD5` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5890` | `WIFI_CRYPTO_SLOT3_WORD6` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5894` | `WIFI_CRYPTO_SLOT3_WORD7` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5898` | `WIFI_CRYPTO_SLOT3_WORD8` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a589c` | `WIFI_CRYPTO_SLOT3_WORD9` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a58a0` | `WIFI_CRYPTO_SLOT4_WORD0` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a58a4` | `WIFI_CRYPTO_SLOT4_WORD1` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a58a8` | `WIFI_CRYPTO_SLOT4_WORD2` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a58ac` | `WIFI_CRYPTO_SLOT4_WORD3` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a58b0` | `WIFI_CRYPTO_SLOT4_WORD4` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a58b4` | `WIFI_CRYPTO_SLOT4_WORD5` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a58b8` | `WIFI_CRYPTO_SLOT4_WORD6` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a58bc` | `WIFI_CRYPTO_SLOT4_WORD7` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a58c0` | `WIFI_CRYPTO_SLOT4_WORD8` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a58c4` | `WIFI_CRYPTO_SLOT4_WORD9` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a58c8` | `WIFI_CRYPTO_SLOT5_WORD0` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a58cc` | `WIFI_CRYPTO_SLOT5_WORD1` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a58d0` | `WIFI_CRYPTO_SLOT5_WORD2` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a58d4` | `WIFI_CRYPTO_SLOT5_WORD3` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a58d8` | `WIFI_CRYPTO_SLOT5_WORD4` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a58dc` | `WIFI_CRYPTO_SLOT5_WORD5` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a58e0` | `WIFI_CRYPTO_SLOT5_WORD6` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a58e4` | `WIFI_CRYPTO_SLOT5_WORD7` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a58e8` | `WIFI_CRYPTO_SLOT5_WORD8` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a58ec` | `WIFI_CRYPTO_SLOT5_WORD9` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a58f0` | `WIFI_CRYPTO_SLOT6_WORD0` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a58f4` | `WIFI_CRYPTO_SLOT6_WORD1` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a58f8` | `WIFI_CRYPTO_SLOT6_WORD2` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a58fc` | `WIFI_CRYPTO_SLOT6_WORD3` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5900` | `WIFI_CRYPTO_SLOT6_WORD4` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5904` | `WIFI_CRYPTO_SLOT6_WORD5` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5908` | `WIFI_CRYPTO_SLOT6_WORD6` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a590c` | `WIFI_CRYPTO_SLOT6_WORD7` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5910` | `WIFI_CRYPTO_SLOT6_WORD8` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5914` | `WIFI_CRYPTO_SLOT6_WORD9` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5918` | `WIFI_CRYPTO_SLOT7_WORD0` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a591c` | `WIFI_CRYPTO_SLOT7_WORD1` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5920` | `WIFI_CRYPTO_SLOT7_WORD2` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5924` | `WIFI_CRYPTO_SLOT7_WORD3` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5928` | `WIFI_CRYPTO_SLOT7_WORD4` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a592c` | `WIFI_CRYPTO_SLOT7_WORD5` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5930` | `WIFI_CRYPTO_SLOT7_WORD6` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5934` | `WIFI_CRYPTO_SLOT7_WORD7` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5938` | `WIFI_CRYPTO_SLOT7_WORD8` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a593c` | `WIFI_CRYPTO_SLOT7_WORD9` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5940` | `WIFI_CRYPTO_SLOT8_WORD0` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5944` | `WIFI_CRYPTO_SLOT8_WORD1` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5948` | `WIFI_CRYPTO_SLOT8_WORD2` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a594c` | `WIFI_CRYPTO_SLOT8_WORD3` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5950` | `WIFI_CRYPTO_SLOT8_WORD4` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5954` | `WIFI_CRYPTO_SLOT8_WORD5` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5958` | `WIFI_CRYPTO_SLOT8_WORD6` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a595c` | `WIFI_CRYPTO_SLOT8_WORD7` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5960` | `WIFI_CRYPTO_SLOT8_WORD8` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5964` | `WIFI_CRYPTO_SLOT8_WORD9` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5968` | `WIFI_CRYPTO_SLOT9_WORD0` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a596c` | `WIFI_CRYPTO_SLOT9_WORD1` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5970` | `WIFI_CRYPTO_SLOT9_WORD2` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5974` | `WIFI_CRYPTO_SLOT9_WORD3` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5978` | `WIFI_CRYPTO_SLOT9_WORD4` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a597c` | `WIFI_CRYPTO_SLOT9_WORD5` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5980` | `WIFI_CRYPTO_SLOT9_WORD6` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5984` | `WIFI_CRYPTO_SLOT9_WORD7` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5988` | `WIFI_CRYPTO_SLOT9_WORD8` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a598c` | `WIFI_CRYPTO_SLOT9_WORD9` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5990` | `WIFI_CRYPTO_SLOT10_WORD0` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5994` | `WIFI_CRYPTO_SLOT10_WORD1` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5998` | `WIFI_CRYPTO_SLOT10_WORD2` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a599c` | `WIFI_CRYPTO_SLOT10_WORD3` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a59a0` | `WIFI_CRYPTO_SLOT10_WORD4` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a59a4` | `WIFI_CRYPTO_SLOT10_WORD5` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a59a8` | `WIFI_CRYPTO_SLOT10_WORD6` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a59ac` | `WIFI_CRYPTO_SLOT10_WORD7` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a59b0` | `WIFI_CRYPTO_SLOT10_WORD8` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a59b4` | `WIFI_CRYPTO_SLOT10_WORD9` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a59b8` | `WIFI_CRYPTO_SLOT11_WORD0` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a59bc` | `WIFI_CRYPTO_SLOT11_WORD1` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a59c0` | `WIFI_CRYPTO_SLOT11_WORD2` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a59c4` | `WIFI_CRYPTO_SLOT11_WORD3` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a59c8` | `WIFI_CRYPTO_SLOT11_WORD4` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a59cc` | `WIFI_CRYPTO_SLOT11_WORD5` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a59d0` | `WIFI_CRYPTO_SLOT11_WORD6` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a59d4` | `WIFI_CRYPTO_SLOT11_WORD7` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a59d8` | `WIFI_CRYPTO_SLOT11_WORD8` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a59dc` | `WIFI_CRYPTO_SLOT11_WORD9` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a59e0` | `WIFI_CRYPTO_SLOT12_WORD0` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a59e4` | `WIFI_CRYPTO_SLOT12_WORD1` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a59e8` | `WIFI_CRYPTO_SLOT12_WORD2` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a59ec` | `WIFI_CRYPTO_SLOT12_WORD3` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a59f0` | `WIFI_CRYPTO_SLOT12_WORD4` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a59f4` | `WIFI_CRYPTO_SLOT12_WORD5` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a59f8` | `WIFI_CRYPTO_SLOT12_WORD6` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a59fc` | `WIFI_CRYPTO_SLOT12_WORD7` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5a00` | `WIFI_CRYPTO_SLOT12_WORD8` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5a04` | `WIFI_CRYPTO_SLOT12_WORD9` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5a08` | `WIFI_CRYPTO_SLOT13_WORD0` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5a0c` | `WIFI_CRYPTO_SLOT13_WORD1` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5a10` | `WIFI_CRYPTO_SLOT13_WORD2` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5a14` | `WIFI_CRYPTO_SLOT13_WORD3` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5a18` | `WIFI_CRYPTO_SLOT13_WORD4` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5a1c` | `WIFI_CRYPTO_SLOT13_WORD5` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5a20` | `WIFI_CRYPTO_SLOT13_WORD6` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5a24` | `WIFI_CRYPTO_SLOT13_WORD7` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5a28` | `WIFI_CRYPTO_SLOT13_WORD8` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5a2c` | `WIFI_CRYPTO_SLOT13_WORD9` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5a30` | `WIFI_CRYPTO_SLOT14_WORD0` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5a34` | `WIFI_CRYPTO_SLOT14_WORD1` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5a38` | `WIFI_CRYPTO_SLOT14_WORD2` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5a3c` | `WIFI_CRYPTO_SLOT14_WORD3` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5a40` | `WIFI_CRYPTO_SLOT14_WORD4` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5a44` | `WIFI_CRYPTO_SLOT14_WORD5` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5a48` | `WIFI_CRYPTO_SLOT14_WORD6` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5a4c` | `WIFI_CRYPTO_SLOT14_WORD7` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5a50` | `WIFI_CRYPTO_SLOT14_WORD8` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5a54` | `WIFI_CRYPTO_SLOT14_WORD9` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5a58` | `WIFI_CRYPTO_SLOT15_WORD0` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5a5c` | `WIFI_CRYPTO_SLOT15_WORD1` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5a60` | `WIFI_CRYPTO_SLOT15_WORD2` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5a64` | `WIFI_CRYPTO_SLOT15_WORD3` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5a68` | `WIFI_CRYPTO_SLOT15_WORD4` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5a6c` | `WIFI_CRYPTO_SLOT15_WORD5` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5a70` | `WIFI_CRYPTO_SLOT15_WORD6` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5a74` | `WIFI_CRYPTO_SLOT15_WORD7` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5a78` | `WIFI_CRYPTO_SLOT15_WORD8` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5a7c` | `WIFI_CRYPTO_SLOT15_WORD9` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5a80` | `WIFI_CRYPTO_SLOT16_WORD0` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5a84` | `WIFI_CRYPTO_SLOT16_WORD1` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5a88` | `WIFI_CRYPTO_SLOT16_WORD2` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5a8c` | `WIFI_CRYPTO_SLOT16_WORD3` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5a90` | `WIFI_CRYPTO_SLOT16_WORD4` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5a94` | `WIFI_CRYPTO_SLOT16_WORD5` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5a98` | `WIFI_CRYPTO_SLOT16_WORD6` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5a9c` | `WIFI_CRYPTO_SLOT16_WORD7` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5aa0` | `WIFI_CRYPTO_SLOT16_WORD8` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5aa4` | `WIFI_CRYPTO_SLOT16_WORD9` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5aa8` | `WIFI_CRYPTO_SLOT17_WORD0` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5aac` | `WIFI_CRYPTO_SLOT17_WORD1` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5ab0` | `WIFI_CRYPTO_SLOT17_WORD2` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5ab4` | `WIFI_CRYPTO_SLOT17_WORD3` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5ab8` | `WIFI_CRYPTO_SLOT17_WORD4` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5abc` | `WIFI_CRYPTO_SLOT17_WORD5` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5ac0` | `WIFI_CRYPTO_SLOT17_WORD6` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5ac4` | `WIFI_CRYPTO_SLOT17_WORD7` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5ac8` | `WIFI_CRYPTO_SLOT17_WORD8` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5acc` | `WIFI_CRYPTO_SLOT17_WORD9` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5ad0` | `WIFI_CRYPTO_SLOT18_WORD0` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5ad4` | `WIFI_CRYPTO_SLOT18_WORD1` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5ad8` | `WIFI_CRYPTO_SLOT18_WORD2` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5adc` | `WIFI_CRYPTO_SLOT18_WORD3` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5ae0` | `WIFI_CRYPTO_SLOT18_WORD4` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5ae4` | `WIFI_CRYPTO_SLOT18_WORD5` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5ae8` | `WIFI_CRYPTO_SLOT18_WORD6` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5aec` | `WIFI_CRYPTO_SLOT18_WORD7` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5af0` | `WIFI_CRYPTO_SLOT18_WORD8` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5af4` | `WIFI_CRYPTO_SLOT18_WORD9` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5af8` | `WIFI_CRYPTO_SLOT19_WORD0` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5afc` | `WIFI_CRYPTO_SLOT19_WORD1` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5b00` | `WIFI_CRYPTO_SLOT19_WORD2` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5b04` | `WIFI_CRYPTO_SLOT19_WORD3` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5b08` | `WIFI_CRYPTO_SLOT19_WORD4` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5b0c` | `WIFI_CRYPTO_SLOT19_WORD5` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5b10` | `WIFI_CRYPTO_SLOT19_WORD6` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5b14` | `WIFI_CRYPTO_SLOT19_WORD7` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5b18` | `WIFI_CRYPTO_SLOT19_WORD8` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5b1c` | `WIFI_CRYPTO_SLOT19_WORD9` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5b20` | `WIFI_CRYPTO_SLOT20_WORD0` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5b24` | `WIFI_CRYPTO_SLOT20_WORD1` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5b28` | `WIFI_CRYPTO_SLOT20_WORD2` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5b2c` | `WIFI_CRYPTO_SLOT20_WORD3` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5b30` | `WIFI_CRYPTO_SLOT20_WORD4` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5b34` | `WIFI_CRYPTO_SLOT20_WORD5` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5b38` | `WIFI_CRYPTO_SLOT20_WORD6` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5b3c` | `WIFI_CRYPTO_SLOT20_WORD7` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5b40` | `WIFI_CRYPTO_SLOT20_WORD8` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5b44` | `WIFI_CRYPTO_SLOT20_WORD9` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5b48` | `WIFI_CRYPTO_SLOT21_WORD0` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5b4c` | `WIFI_CRYPTO_SLOT21_WORD1` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5b50` | `WIFI_CRYPTO_SLOT21_WORD2` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5b54` | `WIFI_CRYPTO_SLOT21_WORD3` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5b58` | `WIFI_CRYPTO_SLOT21_WORD4` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5b5c` | `WIFI_CRYPTO_SLOT21_WORD5` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5b60` | `WIFI_CRYPTO_SLOT21_WORD6` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5b64` | `WIFI_CRYPTO_SLOT21_WORD7` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5b68` | `WIFI_CRYPTO_SLOT21_WORD8` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5b6c` | `WIFI_CRYPTO_SLOT21_WORD9` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5b70` | `WIFI_CRYPTO_SLOT22_WORD0` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5b74` | `WIFI_CRYPTO_SLOT22_WORD1` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5b78` | `WIFI_CRYPTO_SLOT22_WORD2` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5b7c` | `WIFI_CRYPTO_SLOT22_WORD3` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5b80` | `WIFI_CRYPTO_SLOT22_WORD4` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5b84` | `WIFI_CRYPTO_SLOT22_WORD5` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5b88` | `WIFI_CRYPTO_SLOT22_WORD6` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5b8c` | `WIFI_CRYPTO_SLOT22_WORD7` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5b90` | `WIFI_CRYPTO_SLOT22_WORD8` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5b94` | `WIFI_CRYPTO_SLOT22_WORD9` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5b98` | `WIFI_CRYPTO_SLOT23_WORD0` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5b9c` | `WIFI_CRYPTO_SLOT23_WORD1` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5ba0` | `WIFI_CRYPTO_SLOT23_WORD2` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5ba4` | `WIFI_CRYPTO_SLOT23_WORD3` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5ba8` | `WIFI_CRYPTO_SLOT23_WORD4` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5bac` | `WIFI_CRYPTO_SLOT23_WORD5` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5bb0` | `WIFI_CRYPTO_SLOT23_WORD6` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5bb4` | `WIFI_CRYPTO_SLOT23_WORD7` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5bb8` | `WIFI_CRYPTO_SLOT23_WORD8` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5bbc` | `WIFI_CRYPTO_SLOT23_WORD9` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5bc0` | `WIFI_CRYPTO_SLOT24_WORD0` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5bc4` | `WIFI_CRYPTO_SLOT24_WORD1` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5bc8` | `WIFI_CRYPTO_SLOT24_WORD2` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5bcc` | `WIFI_CRYPTO_SLOT24_WORD3` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5bd0` | `WIFI_CRYPTO_SLOT24_WORD4` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5bd4` | `WIFI_CRYPTO_SLOT24_WORD5` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5bd8` | `WIFI_CRYPTO_SLOT24_WORD6` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5bdc` | `WIFI_CRYPTO_SLOT24_WORD7` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5be0` | `WIFI_CRYPTO_SLOT24_WORD8` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5be4` | `WIFI_CRYPTO_SLOT24_WORD9` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5be8` | `WIFI_CRYPTO_SLOT25_WORD0` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5bec` | `WIFI_CRYPTO_SLOT25_WORD1` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5bf0` | `WIFI_CRYPTO_SLOT25_WORD2` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5bf4` | `WIFI_CRYPTO_SLOT25_WORD3` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5bf8` | `WIFI_CRYPTO_SLOT25_WORD4` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5bfc` | `WIFI_CRYPTO_SLOT25_WORD5` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5c00` | `WIFI_CRYPTO_SLOT25_WORD6` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5c04` | `WIFI_CRYPTO_SLOT25_WORD7` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5c08` | `WIFI_CRYPTO_SLOT25_WORD8` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5c0c` | `WIFI_CRYPTO_SLOT25_WORD9` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5c10` | `WIFI_CRYPTO_SLOT26_WORD0` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5c14` | `WIFI_CRYPTO_SLOT26_WORD1` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5c18` | `WIFI_CRYPTO_SLOT26_WORD2` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5c1c` | `WIFI_CRYPTO_SLOT26_WORD3` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5c20` | `WIFI_CRYPTO_SLOT26_WORD4` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5c24` | `WIFI_CRYPTO_SLOT26_WORD5` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5c28` | `WIFI_CRYPTO_SLOT26_WORD6` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5c2c` | `WIFI_CRYPTO_SLOT26_WORD7` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5c30` | `WIFI_CRYPTO_SLOT26_WORD8` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5c34` | `WIFI_CRYPTO_SLOT26_WORD9` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5c38` | `WIFI_CRYPTO_SLOT27_WORD0` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5c3c` | `WIFI_CRYPTO_SLOT27_WORD1` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5c40` | `WIFI_CRYPTO_SLOT27_WORD2` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5c44` | `WIFI_CRYPTO_SLOT27_WORD3` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5c48` | `WIFI_CRYPTO_SLOT27_WORD4` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5c4c` | `WIFI_CRYPTO_SLOT27_WORD5` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5c50` | `WIFI_CRYPTO_SLOT27_WORD6` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5c54` | `WIFI_CRYPTO_SLOT27_WORD7` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5c58` | `WIFI_CRYPTO_SLOT27_WORD8` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5c5c` | `WIFI_CRYPTO_SLOT27_WORD9` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5c60` | `WIFI_CRYPTO_SLOT28_WORD0` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5c64` | `WIFI_CRYPTO_SLOT28_WORD1` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5c68` | `WIFI_CRYPTO_SLOT28_WORD2` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5c6c` | `WIFI_CRYPTO_SLOT28_WORD3` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5c70` | `WIFI_CRYPTO_SLOT28_WORD4` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5c74` | `WIFI_CRYPTO_SLOT28_WORD5` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5c78` | `WIFI_CRYPTO_SLOT28_WORD6` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5c7c` | `WIFI_CRYPTO_SLOT28_WORD7` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5c80` | `WIFI_CRYPTO_SLOT28_WORD8` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5c84` | `WIFI_CRYPTO_SLOT28_WORD9` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5c88` | `WIFI_CRYPTO_SLOT29_WORD0` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5c8c` | `WIFI_CRYPTO_SLOT29_WORD1` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5c90` | `WIFI_CRYPTO_SLOT29_WORD2` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5c94` | `WIFI_CRYPTO_SLOT29_WORD3` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5c98` | `WIFI_CRYPTO_SLOT29_WORD4` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5c9c` | `WIFI_CRYPTO_SLOT29_WORD5` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5ca0` | `WIFI_CRYPTO_SLOT29_WORD6` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5ca4` | `WIFI_CRYPTO_SLOT29_WORD7` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5ca8` | `WIFI_CRYPTO_SLOT29_WORD8` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5cac` | `WIFI_CRYPTO_SLOT29_WORD9` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5cb0` | `WIFI_CRYPTO_SLOT30_WORD0` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5cb4` | `WIFI_CRYPTO_SLOT30_WORD1` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5cb8` | `WIFI_CRYPTO_SLOT30_WORD2` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5cbc` | `WIFI_CRYPTO_SLOT30_WORD3` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5cc0` | `WIFI_CRYPTO_SLOT30_WORD4` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5cc4` | `WIFI_CRYPTO_SLOT30_WORD5` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5cc8` | `WIFI_CRYPTO_SLOT30_WORD6` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5ccc` | `WIFI_CRYPTO_SLOT30_WORD7` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5cd0` | `WIFI_CRYPTO_SLOT30_WORD8` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5cd4` | `WIFI_CRYPTO_SLOT30_WORD9` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5cd8` | `WIFI_CRYPTO_SLOT31_WORD0` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5cdc` | `WIFI_CRYPTO_SLOT31_WORD1` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5ce0` | `WIFI_CRYPTO_SLOT31_WORD2` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5ce4` | `WIFI_CRYPTO_SLOT31_WORD3` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5ce8` | `WIFI_CRYPTO_SLOT31_WORD4` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5cec` | `WIFI_CRYPTO_SLOT31_WORD5` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5cf0` | `WIFI_CRYPTO_SLOT31_WORD6` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5cf4` | `WIFI_CRYPTO_SLOT31_WORD7` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5cf8` | `WIFI_CRYPTO_SLOT31_WORD8` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a5cfc` | `WIFI_CRYPTO_SLOT31_WORD9` | native Wi-Fi crypto table word | `unknown / 0x00000000` | high |
| `0x600a9404` | `BLE_ECB_START` | start AES-ECB operation | `unknown / 0x00000000` | high |
| `0x600a940c` | `BLE_ECB_LENGTH` | AES-ECB transfer length | `unknown / 0x00000000` | high |
| `0x600a9410` | `BLE_ECB_KEY_WORD0` | AES-ECB 128-bit key word | `unknown / 0x00000000` | high |
| `0x600a9414` | `BLE_ECB_KEY_WORD1` | AES-ECB 128-bit key word | `unknown / 0x00000000` | high |
| `0x600a9418` | `BLE_ECB_KEY_WORD2` | AES-ECB 128-bit key word | `unknown / 0x00000000` | high |
| `0x600a941c` | `BLE_ECB_KEY_WORD3` | AES-ECB 128-bit key word | `unknown / 0x00000000` | high |
| `0x600a9420` | `BLE_ECB_INPUT_ADDRESS` | AES-ECB input buffer address | `unknown / 0x00000000` | high |
| `0x600a9424` | `BLE_ECB_OUTPUT_ADDRESS` | AES-ECB output buffer address | `unknown / 0x00000000` | high |
| `0x600a9428` | `BLE_CCM_START` | start AES-CCM operation | `unknown / 0x00000000` | high |
| `0x600a942c` | `BLE_CCM_RESET` | reset AES-CCM state | `unknown / 0x00000000` | high |
| `0x600a9430` | `BLE_CCM_CONFIG` | AES-CCM direction and message length | `unknown / 0x00000000` | high |
| `0x600a9434` | `BLE_CCM_RESULT` | AES-CCM authentication result | `unknown / 0x00000000` | high |
| `0x600a9438` | `BLE_CCM_INPUT_ADDRESS` | AES-CCM input buffer address | `unknown / 0x00000000` | high |
| `0x600a943c` | `BLE_CCM_OUTPUT_ADDRESS` | AES-CCM output buffer address | `unknown / 0x00000000` | high |
| `0x600a9440` | `BLE_CCM_KEY_WORD0` | AES-CCM 128-bit key word | `unknown / 0x00000000` | high |
| `0x600a9444` | `BLE_CCM_KEY_WORD1` | AES-CCM 128-bit key word | `unknown / 0x00000000` | high |
| `0x600a9448` | `BLE_CCM_KEY_WORD2` | AES-CCM 128-bit key word | `unknown / 0x00000000` | high |
| `0x600a944c` | `BLE_CCM_KEY_WORD3` | AES-CCM 128-bit key word | `unknown / 0x00000000` | high |
| `0x600a9450` | `BLE_CCM_COUNTER_LOW` | AES-CCM packet counter low word | `unknown / 0x00000000` | high |
| `0x600a9454` | `BLE_CCM_COUNTER_IV0` | AES-CCM counter high byte and IV byte 0 | `unknown / 0x00000000` | high |
| `0x600a9458` | `BLE_CCM_IV1` | AES-CCM IV bytes 1..4 | `unknown / 0x00000000` | high |
| `0x600a945c` | `BLE_CCM_IV2` | AES-CCM IV bytes 5..7 | `unknown / 0x00000000` | high |
| `0x600a9460` | `BLE_CCM_AAD` | AES-CCM associated-data header | `unknown / 0x00000000` | high |
| `0x600a94c0` | `BLE_CCM_STATUS` | AES-CCM completion status | `unknown / 0x00000000` | high |
| `0x600a94c4` | `BLE_ECB_STATUS` | AES-ECB completion status | `unknown / 0x00000000` | high |
| `0x600a9800` | `MODEM_SYSCON_0` | modeled modem-control word; field semantics are not yet established | `0x00000001 / 0x00000000` | medium |
| `0x600a9804` | `MODEM_SYSCON_1` | modeled modem-control word; field semantics are not yet established | `0xffe00000 / 0x00000000` | medium |
| `0x600a9808` | `MODEM_SYSCON_2` | modeled modem-control word; field semantics are not yet established | `0xffc00000 / 0x00000000` | medium |
| `0x600a980c` | `MODEM_SYSCON_3` | modeled modem-control word; field semantics are not yet established | `0xffffff00 / 0x00000000` | medium |
| `0x600a9810` | `MODEM_RESET_CONTROL` | radio-domain reset edges | `0xefc7c500 / 0x00000000` | high |
| `0x600a9814` | `MODEM_CLOCK_ENABLE` | Wi-Fi/BLE/802.15.4 clock-domain gates | `0x00ffffff / 0x00000000` | high |
| `0x600a9818` | `MODEM_SYSCON_6` | modeled modem-control word; field semantics are not yet established | `0x00ffffff / 0x00000000` | medium |
| `0x600a981c` | `MODEM_SYSCON_7` | modeled modem-control word; field semantics are not yet established | `0xffffffff / 0x00000000` | medium |
| `0x600a9820` | `MODEM_SYSCON_8` | modeled modem-control word; field semantics are not yet established | `0x000000ff / 0x00000000` | medium |
| `0x600a9824` | `MODEM_SYSCON_9` | radio-domain reset edges | `0x0fffffff / 0x00000000` | medium |
| `0x600ad000` | `PHY_TIME` | free-running simulation time low word | `unknown / 0x00000000` | high |
| `0x600ad014` | `PHY_TSF_LATCH_CONTROL` | latch TSF into low/high registers | `unknown / 0x00000000` | high |
| `0x600ad020` | `PHY_TSF_LOW` | latched TSF low word | `unknown / 0x00000000` | high |
| `0x600ad024` | `PHY_TSF_HIGH` | latched TSF high word | `unknown / 0x00000000` | high |
| `0x600ad074` | `PHY_TSF_TIMER0_CONTROL` | TSF timer enable and wakeup control | `unknown / 0x00000000` | high |
| `0x600ad078` | `PHY_TSF_TIMER0_TARGET` | TSF timer target | `unknown / 0x00000000` | high |
| `0x600ad07c` | `PHY_TSF_TIMER1_CONTROL` | TSF timer enable and wakeup control | `unknown / 0x00000000` | high |
| `0x600ad080` | `PHY_TSF_TIMER1_TARGET` | TSF timer target | `unknown / 0x00000000` | high |
| `0x600ad084` | `PHY_TSF_TIMER2_CONTROL` | TSF timer enable and wakeup control | `unknown / 0x00000000` | high |
| `0x600ad088` | `PHY_TSF_TIMER2_TARGET` | TSF timer target | `unknown / 0x00000000` | high |
| `0x600ad08c` | `PHY_TSF_TIMER3_CONTROL` | TSF timer enable and wakeup control | `unknown / 0x00000000` | high |
| `0x600ad090` | `PHY_TSF_TIMER3_TARGET` | TSF timer target | `unknown / 0x00000000` | high |
| `0x600ad0a8` | `PHY_POWER_INTERRUPT_ENABLE` | PHY timer interrupt enables | `unknown / 0x00000000` | high |
| `0x600ad0ac` | `PHY_POWER_INTERRUPT_RAW` | raw PHY timer interrupts | `unknown / 0x00000000` | high |
| `0x600ad0b0` | `PHY_POWER_INTERRUPT_STATUS` | enabled PHY timer interrupts | `unknown / 0x00000000` | high |
| `0x600ad0b4` | `PHY_POWER_INTERRUPT_CLEAR` | write-one-to-clear PHY timer interrupts | `unknown / 0x00000000` | high |
| `0x600ae010` | `BLE_RTC_INTERRUPT_ENABLE` | arm RTC wake after a valid compare is programmed | `unknown / 0x00000000` | high |
| `0x600ae014` | `BLE_RTC_INTERRUPT_CLEAR` | clear RTC wake and return its state machine to idle | `unknown / 0x00000000` | high |
| `0x600ae01c` | `BLE_TIMER_INTERRUPT_RAW` | latched BLE synchronization-timer expiry | `unknown / 0x00000000` | high |
| `0x600ae024` | `BLE_RTC_TIMER0_PENDING` | latched RTC timer-0 pending state | `unknown / 0x00000000` | high |
| `0x600ae034` | `BLE_RTC_INTERRUPT_STATUS` | latched RTC wake interrupt status | `unknown / 0x00000000` | high |
| `0x600ae044` | `BLE_TIMER_CURRENT` | read-only 100 kHz BLE sleep counter | `unknown / 0x00000000` | high |
| `0x600ae058` | `BLE_TIMER_COMPARE` | future-only BLE synchronization compare | `unknown / 0x00000000` | high |
| `0x600ae060` | `BLE_RTC_COMPARE` | future-only RTC wake compare; must precede enable | `unknown / 0x00000000` | high |
| `0x600af000` | `MODEM_LPCON_0` | modeled modem-control word; field semantics are not yet established | `0x00000003 / 0x00000000` | medium |
| `0x600af004` | `MODEM_LPCON_1` | modeled modem-control word; field semantics are not yet established | `0x0000ffff / 0x00000000` | medium |
| `0x600af008` | `MODEM_LPCON_2` | modeled modem-control word; field semantics are not yet established | `0x0000ffff / 0x00000000` | medium |
| `0x600af00c` | `MODEM_LPCON_3` | modeled modem-control word; field semantics are not yet established | `0x0000ffff / 0x00000000` | medium |
| `0x600af010` | `MODEM_LPCON_4` | radio-domain reset edges | `0x00000001 / 0x00000000` | medium |
| `0x600af014` | `MODEM_LPCON_5` | modeled modem-control word; field semantics are not yet established | `0x00000003 / 0x00000000` | medium |
| `0x600af018` | `MODEM_LPCON_6` | modeled modem-control word; field semantics are not yet established | `0x0000000f / 0x00000000` | medium |
| `0x600af01c` | `MODEM_LPCON_7` | modeled modem-control word; field semantics are not yet established | `0x000003ff / 0x00000000` | medium |
| `0x600af020` | `MODEM_LPCON_8` | modeled modem-control word; field semantics are not yet established | `0xffff0000 / 0x00000000` | medium |
| `0x600af024` | `MODEM_LP_RESET_CONTROL` | radio-domain reset edges | `0x0000000f / 0x00000000` | high |
| `0x600af028` | `MODEM_LPCON_10` | modeled modem-control word; field semantics are not yet established | `0x000fffff / 0x00000000` | medium |
| `0x600af02c` | `MODEM_LPCON_11` | modeled modem-control word; field semantics are not yet established | `0x0fffffff / 0x00000000` | medium |
| `0x600af800` | `ANALOG_I2C_COMMAND_00` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af804` | `ANALOG_I2C_COMMAND_01` | unconditional packed analog-I2C command/result port | `unknown / 0x00000000` | medium |
| `0x600af808` | `ANALOG_I2C_COMMAND_02` | unconditional packed analog-I2C command/result port | `unknown / 0x00000000` | medium |
| `0x600af80c` | `ANALOG_I2C_COMMAND_03` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af810` | `ANALOG_I2C_COMMAND_04` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af814` | `ANALOG_I2C_COMMAND_05` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af818` | `ANALOG_I2C_COMMAND_06` | packed analog-I2C command/result slot selected by the slave byte; BBPLL calibration-done status bit 24 | `unknown / 0x00000000` | medium |
| `0x600af81c` | `ANALOG_I2C_COMMAND_07` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af820` | `ANALOG_I2C_COMMAND_08` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af824` | `ANALOG_I2C_COMMAND_09` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af828` | `ANALOG_I2C_COMMAND_0A` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af82c` | `ANALOG_I2C_COMMAND_0B` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af830` | `ANALOG_I2C_COMMAND_0C` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af834` | `ANALOG_I2C_COMMAND_0D` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af838` | `ANALOG_I2C_COMMAND_0E` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af83c` | `ANALOG_I2C_COMMAND_0F` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af840` | `ANALOG_I2C_COMMAND_10` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af844` | `ANALOG_I2C_COMMAND_11` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af848` | `ANALOG_I2C_COMMAND_12` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af84c` | `ANALOG_I2C_COMMAND_13` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af850` | `ANALOG_I2C_COMMAND_14` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af854` | `ANALOG_I2C_COMMAND_15` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af858` | `ANALOG_I2C_COMMAND_16` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af85c` | `ANALOG_I2C_COMMAND_17` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af860` | `ANALOG_I2C_COMMAND_18` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af864` | `ANALOG_I2C_COMMAND_19` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af868` | `ANALOG_I2C_COMMAND_1A` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af86c` | `ANALOG_I2C_COMMAND_1B` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af870` | `ANALOG_I2C_COMMAND_1C` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af874` | `ANALOG_I2C_COMMAND_1D` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af878` | `ANALOG_I2C_COMMAND_1E` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af87c` | `ANALOG_I2C_COMMAND_1F` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af880` | `ANALOG_I2C_COMMAND_20` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af884` | `ANALOG_I2C_COMMAND_21` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af888` | `ANALOG_I2C_COMMAND_22` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af88c` | `ANALOG_I2C_COMMAND_23` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af890` | `ANALOG_I2C_COMMAND_24` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af894` | `ANALOG_I2C_COMMAND_25` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af898` | `ANALOG_I2C_COMMAND_26` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af89c` | `ANALOG_I2C_COMMAND_27` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af8a0` | `ANALOG_I2C_COMMAND_28` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af8a4` | `ANALOG_I2C_COMMAND_29` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af8a8` | `ANALOG_I2C_COMMAND_2A` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af8ac` | `ANALOG_I2C_COMMAND_2B` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af8b0` | `ANALOG_I2C_COMMAND_2C` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af8b4` | `ANALOG_I2C_COMMAND_2D` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af8b8` | `ANALOG_I2C_COMMAND_2E` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af8bc` | `ANALOG_I2C_COMMAND_2F` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af8c0` | `ANALOG_I2C_COMMAND_30` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af8c4` | `ANALOG_I2C_COMMAND_31` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af8c8` | `ANALOG_I2C_COMMAND_32` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af8cc` | `ANALOG_I2C_COMMAND_33` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af8d0` | `ANALOG_I2C_COMMAND_34` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af8d4` | `ANALOG_I2C_COMMAND_35` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af8d8` | `ANALOG_I2C_COMMAND_36` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af8dc` | `ANALOG_I2C_COMMAND_37` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af8e0` | `ANALOG_I2C_COMMAND_38` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af8e4` | `ANALOG_I2C_COMMAND_39` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af8e8` | `ANALOG_I2C_COMMAND_3A` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af8ec` | `ANALOG_I2C_COMMAND_3B` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af8f0` | `ANALOG_I2C_COMMAND_3C` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af8f4` | `ANALOG_I2C_COMMAND_3D` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af8f8` | `ANALOG_I2C_COMMAND_3E` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |
| `0x600af8fc` | `ANALOG_I2C_COMMAND_3F` | packed analog-I2C command/result slot selected by the slave byte | `unknown / 0x00000000` | medium |

## `esp32c6.ble-baseband-registers`

| Address (offset) | Access/counts | Observed values | Meaning |
|---|---|---|---|
| `0x600a1028` (`0x00000028`) | not-observed R0/W0 | — | submit the scheduler head descriptor |
| `0x600a102c` (`0x0000002c`) | not-observed R0/W0 | — | stop the current BLE schedule |
| `0x600a1304` (`0x00000304`) | not-observed R0/W0 | — | BLE event enable bank 0 |
| `0x600a1308` (`0x00000308`) | not-observed R0/W0 | — | BLE event clear bank 0 |
| `0x600a130c` (`0x0000030c`) | not-observed R0/W0 | — | BLE raw event bank 0 |
| `0x600a1314` (`0x00000314`) | not-observed R0/W0 | — | BLE event enable bank 1 |
| `0x600a1318` (`0x00000318`) | not-observed R0/W0 | — | BLE event clear bank 1 |
| `0x600a131c` (`0x0000031c`) | not-observed R0/W0 | — | BLE raw event bank 1 |
| `0x600a18fc` (`0x000008fc`) | not-observed R0/W0 | — | first pending BLE schedule descriptor |
| `0x600a1900` (`0x00000900`) | not-observed R0/W0 | — | active BLE schedule descriptor and ownership |
| `0x600a1904` (`0x00000904`) | not-observed R0/W0 | — | successor BLE schedule descriptor |
| `0x600a1924` (`0x00000924`) | not-observed R0/W0 | — | hardware-owned BLE scheduler time |
| `0x600a1960` (`0x00000960`) | not-observed R0/W0 | — | current BLE TX buffer descriptor |
| `0x600a1964` (`0x00000964`) | not-observed R0/W0 | — | current BLE RX buffer descriptor |
| `0x600a1ff0` (`0x00000ff0`) | not-observed R0/W0 | — | BLE baseband reset edge |

## `esp32c6.ble-control-registers`

| Address (offset) | Access/counts | Observed values | Meaning |
|---|---|---|---|
| `0x600a9404` (`0x00000404`) | not-observed R0/W0 | — | start AES-ECB operation |
| `0x600a940c` (`0x0000040c`) | not-observed R0/W0 | — | AES-ECB transfer length |
| `0x600a9410` (`0x00000410`) | not-observed R0/W0 | — | AES-ECB 128-bit key word |
| `0x600a9414` (`0x00000414`) | not-observed R0/W0 | — | AES-ECB 128-bit key word |
| `0x600a9418` (`0x00000418`) | not-observed R0/W0 | — | AES-ECB 128-bit key word |
| `0x600a941c` (`0x0000041c`) | not-observed R0/W0 | — | AES-ECB 128-bit key word |
| `0x600a9420` (`0x00000420`) | not-observed R0/W0 | — | AES-ECB input buffer address |
| `0x600a9424` (`0x00000424`) | not-observed R0/W0 | — | AES-ECB output buffer address |
| `0x600a9428` (`0x00000428`) | not-observed R0/W0 | — | start AES-CCM operation |
| `0x600a942c` (`0x0000042c`) | not-observed R0/W0 | — | reset AES-CCM state |
| `0x600a9430` (`0x00000430`) | not-observed R0/W0 | — | AES-CCM direction and message length |
| `0x600a9434` (`0x00000434`) | not-observed R0/W0 | — | AES-CCM authentication result |
| `0x600a9438` (`0x00000438`) | not-observed R0/W0 | — | AES-CCM input buffer address |
| `0x600a943c` (`0x0000043c`) | not-observed R0/W0 | — | AES-CCM output buffer address |
| `0x600a9440` (`0x00000440`) | not-observed R0/W0 | — | AES-CCM 128-bit key word |
| `0x600a9444` (`0x00000444`) | not-observed R0/W0 | — | AES-CCM 128-bit key word |
| `0x600a9448` (`0x00000448`) | not-observed R0/W0 | — | AES-CCM 128-bit key word |
| `0x600a944c` (`0x0000044c`) | not-observed R0/W0 | — | AES-CCM 128-bit key word |
| `0x600a9450` (`0x00000450`) | not-observed R0/W0 | — | AES-CCM packet counter low word |
| `0x600a9454` (`0x00000454`) | not-observed R0/W0 | — | AES-CCM counter high byte and IV byte 0 |
| `0x600a9458` (`0x00000458`) | not-observed R0/W0 | — | AES-CCM IV bytes 1..4 |
| `0x600a945c` (`0x0000045c`) | not-observed R0/W0 | — | AES-CCM IV bytes 5..7 |
| `0x600a9460` (`0x00000460`) | not-observed R0/W0 | — | AES-CCM associated-data header |
| `0x600a94c0` (`0x000004c0`) | not-observed R0/W0 | — | AES-CCM completion status |
| `0x600a94c4` (`0x000004c4`) | not-observed R0/W0 | — | AES-ECB completion status |

## `esp32c6.ble-modem-registers`

| Address (offset) | Access/counts | Observed values | Meaning |
|---|---|---|---|
| `0x600ae010` (`0x00000010`) | not-observed R0/W0 | — | arm RTC wake after a valid compare is programmed |
| `0x600ae014` (`0x00000014`) | not-observed R0/W0 | — | clear RTC wake and return its state machine to idle |
| `0x600ae01c` (`0x0000001c`) | not-observed R0/W0 | — | latched BLE synchronization-timer expiry |
| `0x600ae024` (`0x00000024`) | not-observed R0/W0 | — | latched RTC timer-0 pending state |
| `0x600ae034` (`0x00000034`) | not-observed R0/W0 | — | latched RTC wake interrupt status |
| `0x600ae044` (`0x00000044`) | not-observed R0/W0 | — | read-only 100 kHz BLE sleep counter |
| `0x600ae058` (`0x00000058`) | not-observed R0/W0 | — | future-only BLE synchronization compare |
| `0x600ae060` (`0x00000060`) | not-observed R0/W0 | — | future-only RTC wake compare; must precede enable |

## `esp32c6.i2c-ana-mst`

| Address (offset) | Access/counts | Observed values | Meaning |
|---|---|---|---|
| `0x600af800` (`0x00000000`) | read-write R1423/W819 | `0x00000669`, `0x0102056b`, `0x010f0669`, `0x013b036b`, `0x0172026b`, `0x0181086b`, `0x0188066b`, `0x01a4046b`, … (26 total; see JSON) | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af804` (`0x00000004`) | read-write R8219/W4216 | `0x00000061`, `0x00000361`, `0x00000561`, `0x00000566`, `0x0000056d`, `0x0000076d`, `0x00000966`, `0x00000d6d`, … (138 total; see JSON) | unconditional packed analog-I2C command/result port |
| `0x600af808` (`0x00000008`) | not-observed R0/W0 | — | unconditional packed analog-I2C command/result port |
| `0x600af80c` (`0x0000000c`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af810` (`0x00000010`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af814` (`0x00000014`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af818` (`0x00000018`) | read-write R214/W213 | `0x00000000`, `0x00000008`, `0x01000000`, `0x01000004`, `0x01000008`, `0x01000408`, `0x01000444`, `0x01000448` | packed analog-I2C command/result slot selected by the slave byte; BBPLL calibration-done status bit 24 |
| `0x600af81c` (`0x0000001c`) | write R0/W3145 | `0x00fffbff`, `0x00fffdff`, `0x00fffeff`, `0x00ffff7f`, `0xfffffdff`, `0xfffffeff`, `0xffffff7f`, `0xffffffbf`, … (12 total; see JSON) | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af820` (`0x00000020`) | read-write R4555/W4502 | `0x00000020`, `0x00001f00` | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af824` (`0x00000024`) | read-write R74/W74 | `0x00000080`, `0x00000084` | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af828` (`0x00000028`) | read-write R74/W74 | `0x00000080`, `0x00000084` | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af82c` (`0x0000002c`) | read-write R74/W74 | `0x00000080`, `0x00000084` | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af830` (`0x00000030`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af834` (`0x00000034`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af838` (`0x00000038`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af83c` (`0x0000003c`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af840` (`0x00000040`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af844` (`0x00000044`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af848` (`0x00000048`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af84c` (`0x0000004c`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af850` (`0x00000050`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af854` (`0x00000054`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af858` (`0x00000058`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af85c` (`0x0000005c`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af860` (`0x00000060`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af864` (`0x00000064`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af868` (`0x00000068`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af86c` (`0x0000006c`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af870` (`0x00000070`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af874` (`0x00000074`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af878` (`0x00000078`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af87c` (`0x0000007c`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af880` (`0x00000080`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af884` (`0x00000084`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af888` (`0x00000088`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af88c` (`0x0000008c`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af890` (`0x00000090`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af894` (`0x00000094`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af898` (`0x00000098`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af89c` (`0x0000009c`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af8a0` (`0x000000a0`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af8a4` (`0x000000a4`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af8a8` (`0x000000a8`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af8ac` (`0x000000ac`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af8b0` (`0x000000b0`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af8b4` (`0x000000b4`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af8b8` (`0x000000b8`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af8bc` (`0x000000bc`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af8c0` (`0x000000c0`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af8c4` (`0x000000c4`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af8c8` (`0x000000c8`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af8cc` (`0x000000cc`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af8d0` (`0x000000d0`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af8d4` (`0x000000d4`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af8d8` (`0x000000d8`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af8dc` (`0x000000dc`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af8e0` (`0x000000e0`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af8e4` (`0x000000e4`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af8e8` (`0x000000e8`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af8ec` (`0x000000ec`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af8f0` (`0x000000f0`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af8f4` (`0x000000f4`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af8f8` (`0x000000f8`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |
| `0x600af8fc` (`0x000000fc`) | not-observed R0/W0 | — | packed analog-I2C command/result slot selected by the slave byte |

## `esp32c6.ieee802154`

| Address (offset) | Access/counts | Observed values | Meaning |
|---|---|---|---|
| `0x600a3000` (`0x00000000`) | not-observed R0/W0 | — | execute TX, RX, CCA, energy-detect, test, stop, or timer command |
| `0x600a3004` (`0x00000004`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a3008` (`0x00000008`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a300c` (`0x0000000c`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a3010` (`0x00000010`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a3014` (`0x00000014`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a3018` (`0x00000018`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a301c` (`0x0000001c`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a3020` (`0x00000020`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a3024` (`0x00000024`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a3028` (`0x00000028`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a302c` (`0x0000002c`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a3030` (`0x00000030`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a3034` (`0x00000034`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a3038` (`0x00000038`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a303c` (`0x0000003c`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a3040` (`0x00000040`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a3044` (`0x00000044`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a3048` (`0x00000048`) | not-observed R0/W0 | — | 802.15.4 channel selection |
| `0x600a304c` (`0x0000004c`) | not-observed R0/W0 | — | 802.15.4 transmit-power selection |
| `0x600a3050` (`0x00000050`) | not-observed R0/W0 | — | energy-detection duration in symbols |
| `0x600a3054` (`0x00000054`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a3058` (`0x00000058`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a305c` (`0x0000005c`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a3060` (`0x00000060`) | not-observed R0/W0 | — | 802.15.4 event enable mask |
| `0x600a3064` (`0x00000064`) | not-observed R0/W0 | — | write-one-to-clear event state |
| `0x600a3068` (`0x00000068`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a306c` (`0x0000006c`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a3070` (`0x00000070`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a3078` (`0x00000078`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a307c` (`0x0000007c`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a30a8` (`0x000000a8`) | not-observed R0/W0 | — | MAC timer 0 threshold |
| `0x600a30ac` (`0x000000ac`) | not-observed R0/W0 | — | elapsed MAC timer 0 ticks |
| `0x600a30b0` (`0x000000b0`) | not-observed R0/W0 | — | MAC timer 1 threshold |
| `0x600a30b4` (`0x000000b4`) | not-observed R0/W0 | — | elapsed MAC timer 1 ticks |
| `0x600a30b8` (`0x000000b8`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a30c4` (`0x000000c4`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a30c8` (`0x000000c8`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a30d0` (`0x000000d0`) | not-observed R0/W0 | — | TX DMA descriptor address |
| `0x600a30d4` (`0x000000d4`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a30e0` (`0x000000e0`) | not-observed R0/W0 | — | RX DMA descriptor address |
| `0x600a30e4` (`0x000000e4`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a30f0` (`0x000000f0`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a30f4` (`0x000000f4`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a3100` (`0x00000100`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a3104` (`0x00000104`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a3108` (`0x00000108`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a310c` (`0x0000010c`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a3110` (`0x00000110`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a3114` (`0x00000114`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a3118` (`0x00000118`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a311c` (`0x0000011c`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a3120` (`0x00000120`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a3128` (`0x00000128`) | not-observed R0/W0 | — | frame-security control |
| `0x600a312c` (`0x0000012c`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a3130` (`0x00000130`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a3134` (`0x00000134`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a3138` (`0x00000138`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a313c` (`0x0000013c`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a3140` (`0x00000140`) | not-observed R0/W0 | — | modeled writable register; field semantics are not yet established |
| `0x600a3144` (`0x00000144`) | not-observed R0/W0 | — | modeled MAC statistic counter |
| `0x600a3148` (`0x00000148`) | not-observed R0/W0 | — | modeled MAC statistic counter |
| `0x600a314c` (`0x0000014c`) | not-observed R0/W0 | — | modeled MAC statistic counter |
| `0x600a3150` (`0x00000150`) | not-observed R0/W0 | — | modeled MAC statistic counter |
| `0x600a3154` (`0x00000154`) | not-observed R0/W0 | — | modeled MAC statistic counter |
| `0x600a3158` (`0x00000158`) | not-observed R0/W0 | — | modeled MAC statistic counter |
| `0x600a315c` (`0x0000015c`) | not-observed R0/W0 | — | modeled MAC statistic counter |
| `0x600a3160` (`0x00000160`) | not-observed R0/W0 | — | modeled MAC statistic counter |
| `0x600a3164` (`0x00000164`) | not-observed R0/W0 | — | modeled MAC statistic counter |
| `0x600a3168` (`0x00000168`) | not-observed R0/W0 | — | modeled MAC statistic counter |
| `0x600a316c` (`0x0000016c`) | not-observed R0/W0 | — | modeled MAC statistic counter |
| `0x600a3170` (`0x00000170`) | not-observed R0/W0 | — | modeled MAC statistic counter |
| `0x600a3174` (`0x00000174`) | not-observed R0/W0 | — | modeled MAC statistic counter |
| `0x600a3178` (`0x00000178`) | not-observed R0/W0 | — | modeled MAC statistic counter |
| `0x600a317c` (`0x0000017c`) | not-observed R0/W0 | — | modeled MAC statistic counter |
| `0x600a3180` (`0x00000180`) | not-observed R0/W0 | — | write-one-to-clear statistic counters |
| `0x600a3184` (`0x00000184`) | not-observed R0/W0 | — | hardware date/version value |

## `esp32c6.modem-lpcon`

| Address (offset) | Access/counts | Observed values | Meaning |
|---|---|---|---|
| `0x600af000` (`0x00000000`) | not-observed R0/W0 | — | modeled modem-control word; field semantics are not yet established |
| `0x600af004` (`0x00000004`) | read-write R4/W4 | `0x00000000` | modeled modem-control word; field semantics are not yet established |
| `0x600af008` (`0x00000008`) | read-write R6/W6 | `0x00000000`, `0x00000004`, `0x00000314` | modeled modem-control word; field semantics are not yet established |
| `0x600af00c` (`0x0000000c`) | read-write R10/W10 | `0x00000000`, `0x00000001` | modeled modem-control word; field semantics are not yet established |
| `0x600af010` (`0x00000010`) | not-observed R0/W0 | — | radio-domain reset edges |
| `0x600af014` (`0x00000014`) | not-observed R0/W0 | — | modeled modem-control word; field semantics are not yet established |
| `0x600af018` (`0x00000018`) | read-write R222/W169 | `0x00000000`, `0x00000001`, `0x00000003`, `0x00000004`, `0x00000005`, `0x00000007` | modeled modem-control word; field semantics are not yet established |
| `0x600af01c` (`0x0000001c`) | not-observed R0/W0 | — | modeled modem-control word; field semantics are not yet established |
| `0x600af020` (`0x00000020`) | read-write R904/W452 | `0x60000000`, `0x66000000`, `0x66600000`, `0x66660000` | modeled modem-control word; field semantics are not yet established |
| `0x600af024` (`0x00000024`) | not-observed R0/W0 | — | radio-domain reset edges |
| `0x600af028` (`0x00000028`) | not-observed R0/W0 | — | modeled modem-control word; field semantics are not yet established |
| `0x600af02c` (`0x0000002c`) | not-observed R0/W0 | — | modeled modem-control word; field semantics are not yet established |
| `0x600af400` (`0x00000400`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af404` (`0x00000404`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af408` (`0x00000408`) | read-write R35/W36 | `0x00000000` | unknown; factual observed values only |
| `0x600af40c` (`0x0000040c`) | read-write R35/W36 | `0x00000000`, `0x00000001` | unknown; factual observed values only |
| `0x600af410` (`0x00000410`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af414` (`0x00000414`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af418` (`0x00000418`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af41c` (`0x0000041c`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af420` (`0x00000420`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af424` (`0x00000424`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af428` (`0x00000428`) | read-write R35/W36 | `0x00000000` | unknown; factual observed values only |
| `0x600af42c` (`0x0000042c`) | read-write R35/W36 | `0x00000000`, `0x00000001` | unknown; factual observed values only |
| `0x600af430` (`0x00000430`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af434` (`0x00000434`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af438` (`0x00000438`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af43c` (`0x0000043c`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af440` (`0x00000440`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af444` (`0x00000444`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af448` (`0x00000448`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af44c` (`0x0000044c`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af450` (`0x00000450`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af454` (`0x00000454`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af458` (`0x00000458`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af45c` (`0x0000045c`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af460` (`0x00000460`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af464` (`0x00000464`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af468` (`0x00000468`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af46c` (`0x0000046c`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af470` (`0x00000470`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af474` (`0x00000474`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af478` (`0x00000478`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af47c` (`0x0000047c`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af480` (`0x00000480`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af484` (`0x00000484`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af488` (`0x00000488`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af48c` (`0x0000048c`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af490` (`0x00000490`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af494` (`0x00000494`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af498` (`0x00000498`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af49c` (`0x0000049c`) | write R0/W1 | `0x40000000` | unknown; factual observed values only |
| `0x600af4a0` (`0x000004a0`) | write R0/W1 | `0x00000001` | unknown; factual observed values only |
| `0x600af4a4` (`0x000004a4`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af4a8` (`0x000004a8`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af4ac` (`0x000004ac`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af4b0` (`0x000004b0`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af4b4` (`0x000004b4`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af4b8` (`0x000004b8`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af4bc` (`0x000004bc`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af4c0` (`0x000004c0`) | write R0/W1 | `0x00000001` | unknown; factual observed values only |
| `0x600af4c4` (`0x000004c4`) | write R0/W1 | `0x00000002` | unknown; factual observed values only |
| `0x600af4c8` (`0x000004c8`) | write R0/W1 | `0x00000003` | unknown; factual observed values only |
| `0x600af4cc` (`0x000004cc`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af4d0` (`0x000004d0`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af4d4` (`0x000004d4`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |
| `0x600af4d8` (`0x000004d8`) | write R0/W1 | `0x00000000` | unknown; factual observed values only |

## `esp32c6.modem-syscon`

| Address (offset) | Access/counts | Observed values | Meaning |
|---|---|---|---|
| `0x600a9800` (`0x00000000`) | not-observed R0/W0 | — | modeled modem-control word; field semantics are not yet established |
| `0x600a9804` (`0x00000004`) | not-observed R0/W0 | — | modeled modem-control word; field semantics are not yet established |
| `0x600a9808` (`0x00000008`) | not-observed R0/W0 | — | modeled modem-control word; field semantics are not yet established |
| `0x600a980c` (`0x0000000c`) | read-write R1356/W678 | `0x60000000`, `0x64000000`, `0x64600000`, `0x64640000`, `0x64646000`, `0x64646400` | modeled modem-control word; field semantics are not yet established |
| `0x600a9810` (`0x00000010`) | read-write R6/W6 | `0x00000000`, `0x00000400` | radio-domain reset edges |
| `0x600a9814` (`0x00000014`) | read-write R530/W493 | `0x00000000`, `0x000001ff`, `0x000003ff`, `0x00000400`, `0x00000600`, `0x000007ff`, `0x000107ff`, `0x000127ff`, … (18 total; see JSON) | Wi-Fi/BLE/802.15.4 clock-domain gates |
| `0x600a9818` (`0x00000018`) | not-observed R0/W0 | — | modeled modem-control word; field semantics are not yet established |
| `0x600a981c` (`0x0000001c`) | read-write R422/W349 | `0x00000000`, `0x00000800`, `0x10000800`, `0x10000802` | modeled modem-control word; field semantics are not yet established |
| `0x600a9820` (`0x00000020`) | not-observed R0/W0 | — | modeled modem-control word; field semantics are not yet established |
| `0x600a9824` (`0x00000024`) | not-observed R0/W0 | — | radio-domain reset edges |

## `esp32c6.phy-baseband-registers`

| Address (offset) | Access/counts | Observed values | Meaning |
|---|---|---|---|
| `0x600a7018` (`0x00000018`) | read-write R76/W76 | `0x00800000`, `0x10800000` | unknown; factual observed values only |
| `0x600a702c` (`0x0000002c`) | read-write R563/W563 | `0x00004f00`, `0x32004f00`, `0x32004fe0`, `0x32004fe9`, `0x32804f00`, `0x32804fe0`, `0x32804fe9`, `0x46004f00`, … (9 total; see JSON) | unknown; factual observed values only |
| `0x600a7030` (`0x00000030`) | read-write R136/W136 | `0x0001a000`, `0x2001a000` | unknown; factual observed values only |
| `0x600a7044` (`0x00000044`) | read-write R76/W76 | `0x003f0000`, `0x003f2100` | unknown; factual observed values only |
| `0x600a705c` (`0x0000005c`) | read-write R4/W4 | `0x00000000`, `0x00000400`, `0xd1080400`, `0xd1080800` | unknown; factual observed values only |
| `0x600a7064` (`0x00000064`) | write R0/W38 | `0x00081825` | unknown; factual observed values only |
| `0x600a7068` (`0x00000068`) | write R0/W40 | `0x00000404` | unknown; factual observed values only |
| `0x600a708c` (`0x0000008c`) | read R100/W0 | `0x00000000` | unknown; factual observed values only |
| `0x600a7094` (`0x00000094`) | read-write R38/W38 | `0x00000138` | unknown; factual observed values only |
| `0x600a70a0` (`0x000000a0`) | read-write R96/W96 | `0xda000000`, `0xe9000000` | unknown; factual observed values only |
| `0x600a7104` (`0x00000104`) | read-write R38/W38 | `0x000001c8` | unknown; factual observed values only |
| `0x600a7114` (`0x00000114`) | write R0/W38 | `0x00081825` | unknown; factual observed values only |
| `0x600a711c` (`0x0000011c`) | read-write R38/W38 | `0x00000000` | unknown; factual observed values only |
| `0x600a7120` (`0x00000120`) | read-write R38/W38 | `0x1e001e00` | unknown; factual observed values only |
| `0x600a7124` (`0x00000124`) | read-write R76/W76 | `0x00008400`, `0x00008403` | unknown; factual observed values only |
| `0x600a7128` (`0x00000128`) | read-write R44/W41 | `0xd2000000` | unknown; factual observed values only |
| `0x600a713c` (`0x0000013c`) | read-write R39/W39 | `0x01300000`, `0x01380000` | unknown; factual observed values only |
| `0x600a7400` (`0x00000400`) | read-write R38/W38 | `0x00006000` | unknown; factual observed values only |
| `0x600a7424` (`0x00000424`) | read-write R38/W38 | `0x01000000` | unknown; factual observed values only |
| `0x600a7428` (`0x00000428`) | read-write R39/W39 | `0x00001500` | unknown; factual observed values only |
| `0x600a7438` (`0x00000438`) | read-write R76/W76 | `0x00000000` | unknown; factual observed values only |
| `0x600a7808` (`0x00000808`) | read-write R38/W38 | `0x00003000` | unknown; factual observed values only |
| `0x600a7848` (`0x00000848`) | read-write R238/W157 | `0x00000000`, `0x000433af`, `0x17041f54`, `0x17042473`, `0x170428c2`, `0x17042d1a`, `0x1704317b`, `0x170433ae`, … (12 total; see JSON) | unknown; factual observed values only |
| `0x600a7890` (`0x00000890`) | read-write R76/W76 | `0x00000000`, `0x01000000` | unknown; factual observed values only |
| `0x600a78dc` (`0x000008dc`) | read-write R38/W38 | `0x00000100` | unknown; factual observed values only |
| `0x600a78e4` (`0x000008e4`) | read-write R38/W38 | `0x00000000` | unknown; factual observed values only |
| `0x600a790c` (`0x0000090c`) | read-write R38/W38 | `0x00000000` | unknown; factual observed values only |
| `0x600a7980` (`0x00000980`) | read-write R38/W38 | `0x00000000` | unknown; factual observed values only |
| `0x600a7a28` (`0x00000a28`) | read-write R38/W38 | `0x00000000` | unknown; factual observed values only |
| `0x600a7c00` (`0x00000c00`) | read-write R76/W76 | `0x00000200`, `0x0000a200` | unknown; factual observed values only |
| `0x600a7c30` (`0x00000c30`) | read-write R76/W76 | `0x00000000`, `0x0000001e` | unknown; factual observed values only |
| `0x600a7c3c` (`0x00000c3c`) | read-write R38/W38 | `0x400000aa` | unknown; factual observed values only |
| `0x600a7c40` (`0x00000c40`) | read-write R38/W38 | `0x80000000` | unknown; factual observed values only |
| `0x600a7c6c` (`0x00000c6c`) | write R0/W38 | `0x0140c81e` | unknown; factual observed values only |
| `0x600a7ca8` (`0x00000ca8`) | read-write R38/W38 | `0x00100000` | unknown; factual observed values only |
| `0x600a7cd0` (`0x00000cd0`) | read-write R76/W76 | `0x000f000f` | unknown; factual observed values only |

## `esp32c6.phy-front-end-registers`

| Address (offset) | Access/counts | Observed values | Meaning |
|---|---|---|---|
| `0x600a8004` (`0x00000004`) | read-write R38/W38 | `0x00009000` | unknown; factual observed values only |
| `0x600a8060` (`0x00000060`) | read-write R530/W12 | `0x00000000` | unknown; factual observed values only |
| `0x600a807c` (`0x0000007c`) | read-write R6/W6 | `0x00000000` | unknown; factual observed values only |

## `esp32c6.phy-i2c-command-memory`

| Address (offset) | Access/counts | Observed values | Meaning |
|---|---|---|---|
| `0x600afc00` (`0x00000000`) | write R0/W1 | `0x00060267` | unknown; factual observed values only |
| `0x600afc04` (`0x00000004`) | write R0/W13 | `0x0072026b`, `0x00d7026b` | unknown; factual observed values only |
| `0x600afc08` (`0x00000008`) | write R0/W1 | `0x003b036b` | unknown; factual observed values only |
| `0x600afc0c` (`0x0000000c`) | write R0/W1 | `0x00a4046b` | unknown; factual observed values only |
| `0x600afc10` (`0x00000010`) | write R0/W1 | `0x0002056b` | unknown; factual observed values only |
| `0x600afc14` (`0x00000014`) | write R0/W1 | `0x0088066b` | unknown; factual observed values only |
| `0x600afc18` (`0x00000018`) | write R0/W1 | `0x00b8076b` | unknown; factual observed values only |
| `0x600afc1c` (`0x0000001c`) | write R0/W1 | `0x0081086b` | unknown; factual observed values only |
| `0x600afc20` (`0x00000020`) | write R0/W1 | `0x00680062` | unknown; factual observed values only |
| `0x600afc24` (`0x00000024`) | write R0/W1 | `0x00280462` | unknown; factual observed values only |
| `0x600afc28` (`0x00000028`) | write R0/W1 | `0x00690f62` | unknown; factual observed values only |
| `0x600afc2c` (`0x0000002c`) | write R0/W1 | `0x00260267` | unknown; factual observed values only |
| `0x600afc30` (`0x00000030`) | write R0/W1 | `0x000e0467` | unknown; factual observed values only |
| `0x600afc34` (`0x00000034`) | write R0/W1 | `0x000e0567` | unknown; factual observed values only |
| `0x600afc38` (`0x00000038`) | write R0/W1 | `0x00030667` | unknown; factual observed values only |
| `0x600afc3c` (`0x0000003c`) | write R0/W1 | `0x00030767` | unknown; factual observed values only |
| `0x600afc40` (`0x00000040`) | write R0/W1 | `0x000e0c67` | unknown; factual observed values only |
| `0x600afc44` (`0x00000044`) | write R0/W1 | `0x000e0d67` | unknown; factual observed values only |
| `0x600afc48` (`0x00000048`) | write R0/W1 | `0x00030e67` | unknown; factual observed values only |
| `0x600afc4c` (`0x0000004c`) | write R0/W1 | `0x00030f67` | unknown; factual observed values only |
| `0x600afc50` (`0x00000050`) | write R0/W1 | `0x00041467` | unknown; factual observed values only |
| `0x600afc54` (`0x00000054`) | write R0/W1 | `0x00041567` | unknown; factual observed values only |
| `0x600afc58` (`0x00000058`) | write R0/W1 | `0x00001667` | unknown; factual observed values only |
| `0x600afc5c` (`0x0000005c`) | write R0/W1 | `0x00001767` | unknown; factual observed values only |
| `0x600afc60` (`0x00000060`) | write R0/W1 | `0x00041867` | unknown; factual observed values only |
| `0x600afc64` (`0x00000064`) | write R0/W1 | `0x00041967` | unknown; factual observed values only |
| `0x600afc68` (`0x00000068`) | write R0/W1 | `0x00051c67` | unknown; factual observed values only |
| `0x600afc6c` (`0x0000006c`) | write R0/W1 | `0x00051d67` | unknown; factual observed values only |
| `0x600afc70` (`0x00000070`) | write R0/W1 | `0x00451e67` | unknown; factual observed values only |
| `0x600afc74` (`0x00000074`) | write R0/W1 | `0x00051f67` | unknown; factual observed values only |

## `esp32c6.phy-mac-registers`

| Address (offset) | Access/counts | Observed values | Meaning |
|---|---|---|---|
| `0x600a2848` (`0x00000848`) | read-write R2/W2 | `0x50000000`, `0x50500000` | unknown; factual observed values only |
| `0x600a2868` (`0x00000868`) | read-write R2/W2 | `0x50000000`, `0x50500000` | unknown; factual observed values only |

## `esp32c6.phy-registers`

| Address (offset) | Access/counts | Observed values | Meaning |
|---|---|---|---|
| `0x600ad000` (`0x00000000`) | read R2353/W0 | `0x0012a6e2`, `0x0012a6ed`, `0x0012a79a`, `0x0012a847`, `0x0012a8f4`, `0x0012a9a1`, `0x0012aa4e`, `0x0012aafb`, … (2353 total; see JSON) | free-running simulation time low word |
| `0x600ad010` (`0x00000010`) | read-write R2/W2 | `0x00000001` | unknown; factual observed values only |
| `0x600ad014` (`0x00000014`) | not-observed R0/W0 | — | latch TSF into low/high registers |
| `0x600ad020` (`0x00000020`) | not-observed R0/W0 | — | latched TSF low word |
| `0x600ad024` (`0x00000024`) | not-observed R0/W0 | — | latched TSF high word |
| `0x600ad030` (`0x00000030`) | read-write R3/W3 | `0x08000000` | unknown; factual observed values only |
| `0x600ad034` (`0x00000034`) | read-write R2/W2 | `0x40000000` | unknown; factual observed values only |
| `0x600ad038` (`0x00000038`) | read-write R6/W6 | `0x00000000` | unknown; factual observed values only |
| `0x600ad03c` (`0x0000003c`) | read-write R10/W10 | `0x00000000`, `0x00010000` | unknown; factual observed values only |
| `0x600ad050` (`0x00000050`) | read-write R19/W19 | `0x00000000`, `0x00080000`, `0x20000000`, `0x88000000`, `0x88080000` | unknown; factual observed values only |
| `0x600ad070` (`0x00000070`) | read-write R3/W3 | `0x000075a5` | unknown; factual observed values only |
| `0x600ad074` (`0x00000074`) | not-observed R0/W0 | — | TSF timer enable and wakeup control |
| `0x600ad078` (`0x00000078`) | not-observed R0/W0 | — | TSF timer target |
| `0x600ad07c` (`0x0000007c`) | not-observed R0/W0 | — | TSF timer enable and wakeup control |
| `0x600ad080` (`0x00000080`) | not-observed R0/W0 | — | TSF timer target |
| `0x600ad084` (`0x00000084`) | not-observed R0/W0 | — | TSF timer enable and wakeup control |
| `0x600ad088` (`0x00000088`) | not-observed R0/W0 | — | TSF timer target |
| `0x600ad08c` (`0x0000008c`) | not-observed R0/W0 | — | TSF timer enable and wakeup control |
| `0x600ad090` (`0x00000090`) | not-observed R0/W0 | — | TSF timer target |
| `0x600ad0a8` (`0x000000a8`) | read-write R8/W8 | `0x00000000` | PHY timer interrupt enables |
| `0x600ad0ac` (`0x000000ac`) | read R16/W0 | `0x00000000` | raw PHY timer interrupts |
| `0x600ad0b0` (`0x000000b0`) | read R16/W0 | `0x00000000` | enabled PHY timer interrupts |
| `0x600ad0b4` (`0x000000b4`) | read-write R4/W20 | `0x00000000` | write-one-to-clear PHY timer interrupts |

## `esp32c6.power-detector`

| Address (offset) | Access/counts | Observed values | Meaning |
|---|---|---|---|
| `0x600a00c0` (`0x000000c0`) | read-write R2311/W2311 | `0x00000000`, `0x40000000`, `0x42000000`, `0x42840000`, `0x42840030`, `0x42840060`, `0x42840090`, `0x428400c0`, … (422 total; see JSON) | RFPLL mode/channel code and start strobe |
| `0x600a00c8` (`0x000000c8`) | read-write R37/W37 | `0x19800249` | unknown; factual observed values only |
| `0x600a00cc` (`0x000000cc`) | read-write R561/W37 | `0x25824e50` | RFPLL completion status |
| `0x600a00d0` (`0x000000d0`) | write R0/W614 | `0x03000162`, `0x03000262`, `0x03000363`, `0x03000463`, `0x03000563`, `0x03000663`, `0x07000063`, `0x07000367`, … (104 total; see JSON) | unknown; factual observed values only |
| `0x600a00d4` (`0x000000d4`) | read-write R74/W74 | `0x00000300`, `0x00052300` | unknown; factual observed values only |
| `0x600a00d8` (`0x000000d8`) | write R0/W37 | `0x04941cc1` | unknown; factual observed values only |
| `0x600a00dc` (`0x000000dc`) | write R0/W37 | `0x00000003` | unknown; factual observed values only |
| `0x600a00e0` (`0x000000e0`) | write R0/W37 | `0x00000000` | unknown; factual observed values only |
| `0x600a0410` (`0x00000410`) | read-write R58/W37 | `0xa0000000` | unknown; factual observed values only |
| `0x600a0414` (`0x00000414`) | read-write R93/W93 | `0x00000004`, `0x00000007` | unknown; factual observed values only |
| `0x600a0418` (`0x00000418`) | read-write R518/W266 | `0x00000002`, `0x00000003`, `0x00400000`, `0x00400001`, `0x00400002`, `0x00400003` | start and synchronously complete RF power conversion |
| `0x600a0420` (`0x00000420`) | read-write R2462/W2462 | `0x00000020`, `0x00022020`, `0x00022096`, `0x00027008`, `0x00027020`, `0x00027820`, `0x0002c020`, `0x00034020`, … (26 total; see JSON) | unknown; factual observed values only |
| `0x600a0424` (`0x00000424`) | read-write R56/W56 | `0x00000000` | unknown; factual observed values only |
| `0x600a042c` (`0x0000042c`) | read-write R104/W104 | `0x00000000` | unknown; factual observed values only |
| `0x600a044c` (`0x0000044c`) | read-write R113/W113 | `0x02000000`, `0x0a000000` | unknown; factual observed values only |
| `0x600a0450` (`0x00000450`) | read-write R178/W178 | `0x60006000`, `0xa000a000`, `0xa000bf00`, `0xa000e000`, `0xc000ff00`, `0xe000a000`, `0xe000bf00`, `0xe000e000`, … (9 total; see JSON) | unknown; factual observed values only |
| `0x600a0460` (`0x00000460`) | read-write R72/W72 | `0x00000000`, `0x06000000` | unknown; factual observed values only |
| `0x600a0468` (`0x00000468`) | read-write R74/W74 | `0x00000000` | unknown; factual observed values only |
| `0x600a046c` (`0x0000046c`) | read-write R39/W39 | `0x00000000` | unknown; factual observed values only |
| `0x600a0470` (`0x00000470`) | read-write R110/W110 | `0x04000000` | unknown; factual observed values only |
| `0x600a0474` (`0x00000474`) | read-write R660/W660 | `0x00100000`, `0x00100ffc`, `0x00100ffd`, `0x00100fff`, `0x00102000`, `0x00102001`, `0x00102003`, `0x00103e80`, … (13 total; see JSON) | IQ calibration start strobe |
| `0x600a0478` (`0x00000478`) | read R4/W0 | `0x00000000` | unknown; factual observed values only |
| `0x600a047c` (`0x0000047c`) | read R4/W0 | `0x00000000` | unknown; factual observed values only |
| `0x600a0480` (`0x00000480`) | read R4/W0 | `0x00000000` | unknown; factual observed values only |
| `0x600a0484` (`0x00000484`) | read R4/W0 | `0x00000000` | unknown; factual observed values only |
| `0x600a0488` (`0x00000488`) | read R104/W0 | `0x00000000` | unknown; factual observed values only |
| `0x600a048c` (`0x0000048c`) | read R104/W0 | `0x00000000` | unknown; factual observed values only |
| `0x600a0490` (`0x00000490`) | read R106/W0 | `0x00000000` | unknown; factual observed values only |
| `0x600a04a0` (`0x000004a0`) | read R110/W0 | `0x00010000` | IQ calibration completion status |
| `0x600a0810` (`0x00000810`) | read-write R4883/W4882 | `0x00000500`, `0x00200500`, `0x00210500`, `0x00210501`, `0x00210780`, `0x00210781` | power-detector tone control |
| `0x600a0814` (`0x00000814`) | read-write R2443/W127 | `0x00000008`, `0x00003008`, `0x0001f008` | power-detector tone status |
| `0x600a0818` (`0x00000818`) | write R0/W37 | `0x0f0f0fff` | unknown; factual observed values only |
| `0x600a081c` (`0x0000081c`) | read-write R3/W39 | `0x00bf0f64`, `0x00bf0ff0`, `0x00ff0f64` | unknown; factual observed values only |
| `0x600a0820` (`0x00000820`) | write R0/W88 | `0x00000000`, `0x0000016a`, `0x00005555`, `0x0000aaaa` | unknown; factual observed values only |
| `0x600a0824` (`0x00000824`) | read R2316/W0 | `0x00000000` | unknown; factual observed values only |
| `0x600a0828` (`0x00000828`) | read R2316/W0 | `0x00000000` | unknown; factual observed values only |
| `0x600a082c` (`0x0000082c`) | read R2316/W0 | `0x00000000` | unknown; factual observed values only |
| `0x600a0830` (`0x00000830`) | read R2316/W0 | `0x00000000` | unknown; factual observed values only |
| `0x600a08c8` (`0x000008c8`) | read-write R1271/W1271 | `0x000e0000`, `0x000e0800`, `0x000e1000`, `0x000e1800`, `0x000e2000`, `0x000e2800`, `0x000e3000`, `0x000e3800`, … (260 total; see JSON) | unknown; factual observed values only |
| `0x600a08cc` (`0x000008cc`) | write R0/W1271 | `0x000401ff`, `0x000487ff`, `0x000501ff`, `0x000801ff`, `0x00097fff`, `0x001401ff`, `0x0014f9ff`, `0x0014fdff`, … (27 total; see JSON) | first word of a 43-entry TX gain tuple |
| `0x600a08d0` (`0x000008d0`) | write R0/W1211 | `0x00010080`, `0x1001ffff`, `0x10020301`, `0x10060100`, `0x9001ffff`, `0x90020301`, `0xe0010080`, `0xe0030080`, … (41 total; see JSON) | second word of a 43-entry TX gain tuple |
| `0x600a08d4` (`0x000008d4`) | write R0/W1211 | `0x00000000`, `0x00000002`, `0x00000006`, `0x0000000e`, `0x000000ae`, `0x000000de`, `0x000000e6`, `0x000000ee`, … (53 total; see JSON) | final word; completes tuple and encodes power ceiling |
| `0x600a08e0` (`0x000008e0`) | read-write R3/W38 | `0x00000700`, `0x0c080700` | unknown; factual observed values only |
| `0x600a08e4` (`0x000008e4`) | read-write R3/W38 | `0x0000140d`, `0x1915140d` | unknown; factual observed values only |
| `0x600a08e8` (`0x000008e8`) | read-write R3/W38 | `0x00001c1a`, `0x1d1d1c1a` | unknown; factual observed values only |
| `0x600a08ec` (`0x000008ec`) | read-write R3/W38 | `0x0000251e`, `0x2a26251e` | unknown; factual observed values only |
| `0x600a08f0` (`0x000008f0`) | read-write R3/W38 | `0x0000322b`, `0x3733322b` | unknown; factual observed values only |
| `0x600a08f4` (`0x000008f4`) | read-write R3/W38 | `0x00003a38`, `0x3b3b3a38` | unknown; factual observed values only |
| `0x600a08f8` (`0x000008f8`) | read-write R77/W77 | `0x00000004`, `0x00005004` | unknown; factual observed values only |
| `0x600a08fc` (`0x000008fc`) | read-write R76/W76 | `0x0a0e0000`, `0x0a0ec800` | unknown; factual observed values only |
| `0x600a0900` (`0x00000900`) | read-write R258/W258 | `0x02000000` | unknown; factual observed values only |
| `0x600a0904` (`0x00000904`) | read-write R11798/W11750 | `0x00000001`, `0x00008000`, `0x00008001`, `0x00008003`, `0x00008005`, `0x00008007`, `0x00008009`, `0x0000800b`, … (548 total; see JSON) | unknown; factual observed values only |
| `0x600a090c` (`0x0000090c`) | read-write R146/W146 | `0x00000000`, `0x08000000` | unknown; factual observed values only |
| `0x600a0910` (`0x00000910`) | read-write R6302/W500 | `0x00000000`, `0x00000200`, `0x00000800`, `0x00000a00`, `0x00004000`, `0x00004a00`, `0x0000c000`, `0x0000ca00`, … (13 total; see JSON) | force-off/release state for Wi-Fi frontend |
| `0x600a0914` (`0x00000914`) | read-write R69/W37 | `0x00100000` | unknown; factual observed values only |
| `0x600a0958` (`0x00000958`) | read-write R76/W76 | `0x000004f0`, `0x000974f0`, `0x0009f4f0` | unknown; factual observed values only |

## `esp32c6.wifi-mac-registers`

| Address (offset) | Access/counts | Observed values | Meaning |
|---|---|---|---|
| `0x600a4000` (`0x00000000`) | write R0/W5 | `0x00024552` | unknown; factual observed values only |
| `0x600a4004` (`0x00000004`) | read-write R45/W45 | `0x00000000`, `0x80000000` | unknown; factual observed values only |
| `0x600a400c` (`0x0000000c`) | read-write R16/W16 | `0x40000000` | unknown; factual observed values only |
| `0x600a4010` (`0x00000010`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a4014` (`0x00000014`) | read-write R21/W21 | `0x00000000`, `0x80000000` | unknown; factual observed values only |
| `0x600a4020` (`0x00000020`) | read-write R24/W21 | `0x00000000`, `0x0001fe00` | unknown; factual observed values only |
| `0x600a4028` (`0x00000028`) | read-write R3/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a402c` (`0x0000002c`) | read-write R9/W9 | `0x00000000`, `0x0001fe00` | unknown; factual observed values only |
| `0x600a4034` (`0x00000034`) | read-write R3/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a4038` (`0x00000038`) | read-write R6/W6 | `0x00400000`, `0x007fe800` | unknown; factual observed values only |
| `0x600a403c` (`0x0000003c`) | read-write R12/W12 | `0x00000800`, `0x00800800` | unknown; factual observed values only |
| `0x600a4048` (`0x00000048`) | read-write R3/W3 | `0x000000f0` | unknown; factual observed values only |
| `0x600a405c` (`0x0000005c`) | write R0/W8 | `0x00024552` | station interface address bytes 0..3 |
| `0x600a4060` (`0x00000060`) | read-write R23/W31 | `0x00000000`, `0x00010000` | station address bytes 4..5 and valid bit |
| `0x600a4064` (`0x00000064`) | write R0/W5 | `0x00024552` | interface MAC address bytes 0..3 |
| `0x600a4068` (`0x00000068`) | read-write R13/W18 | `0x00000000`, `0x00000100`, `0x00010100` | interface MAC address bytes 4..5 and valid bit |
| `0x600a406c` (`0x0000006c`) | write R0/W3 | `0x00000000` | interface MAC address bytes 0..3 |
| `0x600a4070` (`0x00000070`) | read-write R9/W12 | `0x00000000`, `0x00010000` | interface MAC address bytes 4..5 and valid bit |
| `0x600a4074` (`0x00000074`) | not-observed R0/W0 | — | interface MAC address bytes 0..3 |
| `0x600a4078` (`0x00000078`) | not-observed R0/W0 | — | interface MAC address bytes 4..5 and valid bit |
| `0x600a407c` (`0x0000007c`) | read-write R3/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a4080` (`0x00000080`) | read-write R19/W19 | `0x00000000`, `0x08000000`, `0x88000000` | RX descriptor reload command |
| `0x600a4084` (`0x00000084`) | write R0/W3 | `0x408287fc`, `0x4082d50c` | firmware-owned RX descriptor base |
| `0x600a4088` (`0x00000088`) | not-observed R0/W0 | — | next RX DMA descriptor selected by the model |
| `0x600a408c` (`0x0000008c`) | not-observed R0/W0 | — | last completed RX DMA descriptor |
| `0x600a4098` (`0x00000098`) | read-write R9/W9 | `0x08000000`, `0x08000001`, `0x08000101` | unknown; factual observed values only |
| `0x600a409c` (`0x0000009c`) | read-write R6/W6 | `0x00000000`, `0x00000008` | unknown; factual observed values only |
| `0x600a40d8` (`0x000000d8`) | read-write R66/W66 | `0x00000280`, `0x00000285` | unknown; factual observed values only |
| `0x600a40dc` (`0x000000dc`) | read-write R28/W28 | `0x00000280`, `0x00000285` | unknown; factual observed values only |
| `0x600a40e0` (`0x000000e0`) | read-write R30/W30 | `0x00000280`, `0x00000285` | unknown; factual observed values only |
| `0x600a40e4` (`0x000000e4`) | read-write R12/W12 | `0x00000280`, `0x00000285` | unknown; factual observed values only |
| `0x600a40f8` (`0x000000f8`) | read-write R9/W9 | `0x00000000`, `0x01000000`, `0x05000000` | unknown; factual observed values only |
| `0x600a40fc` (`0x000000fc`) | read-write R9/W9 | `0x00000000`, `0x01000000`, `0x05000000` | unknown; factual observed values only |
| `0x600a4100` (`0x00000100`) | read-write R3/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a4104` (`0x00000104`) | read-write R3/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a410c` (`0x0000010c`) | read-write R3/W3 | `0x00080000` | unknown; factual observed values only |
| `0x600a4110` (`0x00000110`) | read-write R6/W6 | `0x00000001`, `0x00000011` | unknown; factual observed values only |
| `0x600a4114` (`0x00000114`) | read-write R6/W6 | `0x80000000`, `0x81b00000` | unknown; factual observed values only |
| `0x600a4118` (`0x00000118`) | read-write R11/W11 | `0x00000000` | unknown; factual observed values only |
| `0x600a411c` (`0x0000011c`) | read-write R6/W6 | `0x00003f00`, `0x00003f7e` | unknown; factual observed values only |
| `0x600a4120` (`0x00000120`) | write R0/W3 | `0x00023006` | unknown; factual observed values only |
| `0x600a4124` (`0x00000124`) | write R0/W3 | `0x00023006` | unknown; factual observed values only |
| `0x600a4128` (`0x00000128`) | write R0/W3 | `0x00023006` | unknown; factual observed values only |
| `0x600a412c` (`0x0000012c`) | write R0/W3 | `0x0002301c` | unknown; factual observed values only |
| `0x600a4130` (`0x00000130`) | write R0/W3 | `0x0002301c` | unknown; factual observed values only |
| `0x600a4134` (`0x00000134`) | write R0/W3 | `0x00023011` | unknown; factual observed values only |
| `0x600a413c` (`0x0000013c`) | write R0/W3 | `0x00000608` | unknown; factual observed values only |
| `0x600a4140` (`0x00000140`) | write R0/W3 | `0x00000808` | unknown; factual observed values only |
| `0x600a4144` (`0x00000144`) | write R0/W3 | `0x00008e88` | unknown; factual observed values only |
| `0x600a4148` (`0x00000148`) | write R0/W3 | `0x44004300` | unknown; factual observed values only |
| `0x600a414c` (`0x0000014c`) | write R0/W3 | `0x43004400` | unknown; factual observed values only |
| `0x600a4150` (`0x00000150`) | write R0/W3 | `0x00000001` | unknown; factual observed values only |
| `0x600a4158` (`0x00000158`) | write R0/W3 | `0x0000ffff` | unknown; factual observed values only |
| `0x600a415c` (`0x0000015c`) | write R0/W3 | `0x0000ffff` | unknown; factual observed values only |
| `0x600a4160` (`0x00000160`) | write R0/W3 | `0x0000ffff` | unknown; factual observed values only |
| `0x600a4164` (`0x00000164`) | write R0/W3 | `0xffffffff` | unknown; factual observed values only |
| `0x600a4168` (`0x00000168`) | write R0/W3 | `0xffffffff` | unknown; factual observed values only |
| `0x600a416c` (`0x0000016c`) | write R0/W3 | `0x000000ff` | unknown; factual observed values only |
| `0x600a4178` (`0x00000178`) | not-observed R0/W0 | — | RX block-ack agreement control |
| `0x600a417c` (`0x0000017c`) | not-observed R0/W0 | — | RX block-ack peer address high bits |
| `0x600a4180` (`0x00000180`) | not-observed R0/W0 | — | RX block-ack peer address low bits |
| `0x600a4188` (`0x00000188`) | not-observed R0/W0 | — | RX block-ack window origin |
| `0x600a4190` (`0x00000190`) | not-observed R0/W0 | — | RX block-ack receive bitmap bits 0..31 |
| `0x600a4198` (`0x00000198`) | not-observed R0/W0 | — | RX block-ack receive bitmap bits 32..63 |
| `0x600a41a0` (`0x000001a0`) | not-observed R0/W0 | — | RX block-ack agreement control |
| `0x600a41a4` (`0x000001a4`) | not-observed R0/W0 | — | RX block-ack peer address high bits |
| `0x600a41a8` (`0x000001a8`) | not-observed R0/W0 | — | RX block-ack peer address low bits |
| `0x600a41b0` (`0x000001b0`) | not-observed R0/W0 | — | RX block-ack window origin |
| `0x600a41b8` (`0x000001b8`) | not-observed R0/W0 | — | RX block-ack receive bitmap bits 0..31 |
| `0x600a41c0` (`0x000001c0`) | not-observed R0/W0 | — | RX block-ack receive bitmap bits 32..63 |
| `0x600a41c8` (`0x000001c8`) | not-observed R0/W0 | — | RX block-ack agreement control |
| `0x600a41cc` (`0x000001cc`) | not-observed R0/W0 | — | RX block-ack peer address high bits |
| `0x600a41d0` (`0x000001d0`) | not-observed R0/W0 | — | RX block-ack peer address low bits |
| `0x600a41d8` (`0x000001d8`) | not-observed R0/W0 | — | RX block-ack window origin |
| `0x600a41e0` (`0x000001e0`) | not-observed R0/W0 | — | RX block-ack receive bitmap bits 0..31 |
| `0x600a41e8` (`0x000001e8`) | not-observed R0/W0 | — | RX block-ack receive bitmap bits 32..63 |
| `0x600a41f0` (`0x000001f0`) | not-observed R0/W0 | — | RX block-ack agreement control |
| `0x600a41f4` (`0x000001f4`) | not-observed R0/W0 | — | RX block-ack peer address high bits |
| `0x600a41f8` (`0x000001f8`) | not-observed R0/W0 | — | RX block-ack peer address low bits |
| `0x600a4200` (`0x00000200`) | not-observed R0/W0 | — | RX block-ack window origin |
| `0x600a4208` (`0x00000208`) | not-observed R0/W0 | — | RX block-ack receive bitmap bits 0..31 |
| `0x600a4210` (`0x00000210`) | not-observed R0/W0 | — | RX block-ack receive bitmap bits 32..63 |
| `0x600a4218` (`0x00000218`) | not-observed R0/W0 | — | RX block-ack agreement control |
| `0x600a421c` (`0x0000021c`) | not-observed R0/W0 | — | RX block-ack peer address high bits |
| `0x600a4220` (`0x00000220`) | not-observed R0/W0 | — | RX block-ack peer address low bits |
| `0x600a4228` (`0x00000228`) | not-observed R0/W0 | — | RX block-ack window origin |
| `0x600a4230` (`0x00000230`) | not-observed R0/W0 | — | RX block-ack receive bitmap bits 0..31 |
| `0x600a4238` (`0x00000238`) | not-observed R0/W0 | — | RX block-ack receive bitmap bits 32..63 |
| `0x600a4240` (`0x00000240`) | not-observed R0/W0 | — | RX block-ack agreement control |
| `0x600a4244` (`0x00000244`) | not-observed R0/W0 | — | RX block-ack peer address high bits |
| `0x600a4248` (`0x00000248`) | not-observed R0/W0 | — | RX block-ack peer address low bits |
| `0x600a4250` (`0x00000250`) | not-observed R0/W0 | — | RX block-ack window origin |
| `0x600a4258` (`0x00000258`) | not-observed R0/W0 | — | RX block-ack receive bitmap bits 0..31 |
| `0x600a4260` (`0x00000260`) | not-observed R0/W0 | — | RX block-ack receive bitmap bits 32..63 |
| `0x600a4268` (`0x00000268`) | not-observed R0/W0 | — | RX block-ack agreement control |
| `0x600a426c` (`0x0000026c`) | not-observed R0/W0 | — | RX block-ack peer address high bits |
| `0x600a4270` (`0x00000270`) | not-observed R0/W0 | — | RX block-ack peer address low bits |
| `0x600a4278` (`0x00000278`) | not-observed R0/W0 | — | RX block-ack window origin |
| `0x600a4280` (`0x00000280`) | not-observed R0/W0 | — | RX block-ack receive bitmap bits 0..31 |
| `0x600a4288` (`0x00000288`) | not-observed R0/W0 | — | RX block-ack receive bitmap bits 32..63 |
| `0x600a4290` (`0x00000290`) | not-observed R0/W0 | — | RX block-ack agreement control |
| `0x600a4294` (`0x00000294`) | not-observed R0/W0 | — | RX block-ack peer address high bits |
| `0x600a4298` (`0x00000298`) | not-observed R0/W0 | — | RX block-ack peer address low bits |
| `0x600a42a0` (`0x000002a0`) | not-observed R0/W0 | — | RX block-ack window origin |
| `0x600a42a8` (`0x000002a8`) | not-observed R0/W0 | — | RX block-ack receive bitmap bits 0..31 |
| `0x600a42b0` (`0x000002b0`) | not-observed R0/W0 | — | RX block-ack receive bitmap bits 32..63 |
| `0x600a42cc` (`0x000002cc`) | read-write R6/W6 | `0x00000000`, `0x00000020` | unknown; factual observed values only |
| `0x600a42d4` (`0x000002d4`) | read-write R3/W3 | `0x40000000` | unknown; factual observed values only |
| `0x600a42fc` (`0x000002fc`) | read-write R10/W10 | `0x00000000`, `0x00000070` | unknown; factual observed values only |
| `0x600a4308` (`0x00000308`) | read-write R5/W5 | `0x00000002`, `0x00000003` | unknown; factual observed values only |
| `0x600a4400` (`0x00000400`) | read-write R54/W54 | `0x00000000`, `0x00000350`, `0x00020350` | unknown; factual observed values only |
| `0x600a4408` (`0x00000408`) | read-write R42/W42 | `0x00000000`, `0x00000800`, `0x00000e00`, `0x00001400` | unknown; factual observed values only |
| `0x600a440c` (`0x0000040c`) | read-write R42/W42 | `0x00000000`, `0x00010000`, `0x00010800`, `0x00010e00`, `0x00011400` | unknown; factual observed values only |
| `0x600a4410` (`0x00000410`) | read-write R42/W42 | `0x00000000`, `0x00050000`, `0x00050800`, `0x00050e00`, `0x00051400` | unknown; factual observed values only |
| `0x600a4414` (`0x00000414`) | read-write R42/W42 | `0x00000000`, `0x000b0000`, `0x000b0800`, `0x000b0e00`, `0x000b1400` | unknown; factual observed values only |
| `0x600a4418` (`0x00000418`) | read-write R42/W42 | `0x00000000`, `0x000a0000`, `0x000a0800`, `0x000a0e00`, `0x000a1400` | unknown; factual observed values only |
| `0x600a441c` (`0x0000041c`) | read-write R42/W42 | `0x00000000`, `0x00090000`, `0x00090800`, `0x00090e00`, `0x00091400` | unknown; factual observed values only |
| `0x600a4420` (`0x00000420`) | read-write R42/W42 | `0x00800000`, `0x00900000`, `0x00900800`, `0x00900e00`, `0x00901300` | unknown; factual observed values only |
| `0x600a4424` (`0x00000424`) | read-write R42/W42 | `0x00800000`, `0x00910000`, `0x00910800`, `0x00910e00`, `0x00911300` | unknown; factual observed values only |
| `0x600a4428` (`0x00000428`) | read-write R42/W42 | `0x00800000`, `0x00920000`, `0x00920800`, `0x00920e00`, `0x00921300` | unknown; factual observed values only |
| `0x600a442c` (`0x0000042c`) | read-write R42/W42 | `0x00800000`, `0x00920000`, `0x00920800`, `0x00920e00`, `0x00921300` | unknown; factual observed values only |
| `0x600a4430` (`0x00000430`) | read-write R56/W56 | `0x00000013`, `0x00001313`, `0x00131313`, `0x08080808`, `0x0808080e`, `0x08080e0e`, `0x080e0e0e`, `0x0e080808`, … (18 total; see JSON) | unknown; factual observed values only |
| `0x600a4434` (`0x00000434`) | read-write R56/W56 | `0x00000013`, `0x00001313`, `0x00121313`, `0x08080808`, `0x0808080e`, `0x08080e0e`, `0x080e0e0e`, `0x0e080808`, … (18 total; see JSON) | unknown; factual observed values only |
| `0x600a4438` (`0x00000438`) | read-write R56/W56 | `0x00000011`, `0x00000f11`, `0x000f0f11`, `0x08080808`, `0x0808080e`, `0x08080e0e`, `0x080e0e0e`, `0x0e080808`, … (18 total; see JSON) | unknown; factual observed values only |
| `0x600a4440` (`0x00000440`) | read-write R21/W9 | `0x00090a0b` | unknown; factual observed values only |
| `0x600a4444` (`0x00000444`) | write R0/W9 | `0x00050100` | unknown; factual observed values only |
| `0x600a444c` (`0x0000044c`) | read-write R21/W9 | `0x00090a0b` | unknown; factual observed values only |
| `0x600a4450` (`0x00000450`) | write R0/W9 | `0x00050100` | unknown; factual observed values only |
| `0x600a4458` (`0x00000458`) | read-write R12/W12 | `0x04000000`, `0x04080000`, `0x04081000`, `0x04081020` | unknown; factual observed values only |
| `0x600a4470` (`0x00000470`) | read-write R3/W3 | `0x00801000` | unknown; factual observed values only |
| `0x600a4474` (`0x00000474`) | read-write R3/W3 | `0x80000000` | unknown; factual observed values only |
| `0x600a4800` (`0x00000800`) | write R0/W6 | `0x00030000` | unknown; factual observed values only |
| `0x600a4804` (`0x00000804`) | write R0/W3 | `0x00030000` | unknown; factual observed values only |
| `0x600a4808` (`0x00000808`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a480c` (`0x0000080c`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a4810` (`0x00000810`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a4814` (`0x00000814`) | read-write R3/W3 | `0x00000000` | valid bitmap for 32 native key slots |
| `0x600a4c1c` (`0x00000c1c`) | read-write R15/W15 | `0x00000011`, `0x80000011`, `0xc0000011` | unknown; factual observed values only |
| `0x600a4c20` (`0x00000c20`) | read-write R3/W3 | `0x000000f0` | unknown; factual observed values only |
| `0x600a4c24` (`0x00000c24`) | read-write R3/W3 | `0x000000f0` | unknown; factual observed values only |
| `0x600a4c2c` (`0x00000c2c`) | read-write R3/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a4c34` (`0x00000c34`) | read R16/W0 | `0x00000000` | unknown; factual observed values only |
| `0x600a4c38` (`0x00000c38`) | read-write R8/W8 | `0x00001000` | unknown; factual observed values only |
| `0x600a4c40` (`0x00000c40`) | read-write R2/W10 | `0x00000000`, `0x19a879e0` | Wi-Fi MAC interrupt mask |
| `0x600a4c48` (`0x00000c48`) | read R16/W0 | `0x00000000`, `0x00000080` | latched Wi-Fi MAC interrupt events |
| `0x600a4c4c` (`0x00000c4c`) | write R0/W13 | `0x00000080`, `0xffffffff` | write-one-to-clear Wi-Fi events |
| `0x600a4c54` (`0x00000c54`) | read-write R6/W6 | `0x14000000`, `0x1409d800` | unknown; factual observed values only |
| `0x600a4c58` (`0x00000c58`) | read-write R9/W9 | `0x00123400`, `0x001234a0`, `0x0bd234a0` | unknown; factual observed values only |
| `0x600a4c60` (`0x00000c60`) | read-write R6/W6 | `0x7fff0000`, `0xffff0000` | unknown; factual observed values only |
| `0x600a4c70` (`0x00000c70`) | not-observed R0/W0 | — | high address bits for RX DMA descriptors |
| `0x600a4c78` (`0x00000c78`) | read-write R18/W18 | `0x00000000`, `0x00080000`, `0x00087100`, `0x00087120`, `0x000d7120` | unknown; factual observed values only |
| `0x600a4c7c` (`0x00000c7c`) | read-write R3/W3 | `0x00000400` | unknown; factual observed values only |
| `0x600a4c80` (`0x00000c80`) | read-write R9/W9 | `0x00000000`, `0x00000be0` | unknown; factual observed values only |
| `0x600a4c84` (`0x00000c84`) | read-write R6/W6 | `0x0e000000`, `0x0e7c0000` | unknown; factual observed values only |
| `0x600a4c88` (`0x00000c88`) | read-write R6/W6 | `0x00000002`, `0x00000003` | unknown; factual observed values only |
| `0x600a4c8c` (`0x00000c8c`) | read-write R15/W15 | `0x8080a000`, `0x8080b000`, `0x9080b000`, `0x9080b200` | unknown; factual observed values only |
| `0x600a4c98` (`0x00000c98`) | read-write R6/W6 | `0x00000000`, `0x00000004` | unknown; factual observed values only |
| `0x600a4c9c` (`0x00000c9c`) | read-write R3/W3 | `0x00000003` | unknown; factual observed values only |
| `0x600a4ca4` (`0x00000ca4`) | read-write R3/W3 | `0x00000040` | unknown; factual observed values only |
| `0x600a4ca8` (`0x00000ca8`) | read-write R240/W96 | `0x00000000`, `0x00ff1000` | unknown; factual observed values only |
| `0x600a4cb4` (`0x00000cb4`) | read-write R8/W8 | `0x00000001` | clear completed TX queue bits |
| `0x600a4cb8` (`0x00000cb8`) | read R8/W0 | `0x00000001` | completed TX queue bitmap |
| `0x600a4cbc` (`0x00000cbc`) | read-write R3/W3 | `0x80000000` | unknown; factual observed values only |
| `0x600a4cf4` (`0x00000cf4`) | read-write R3/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a4d04` (`0x00000d04`) | read-write R3/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a4d10` (`0x00000d10`) | not-observed R0/W0 | — | queue RTS/protection configuration |
| `0x600a4d14` (`0x00000d14`) | read-write R3/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a4d18` (`0x00000d18`) | not-observed R0/W0 | — | queue transmission timeout |
| `0x600a4d1c` (`0x00000d1c`) | not-observed R0/W0 | — | queue enable and TX descriptor pointer |
| `0x600a4d20` (`0x00000d20`) | not-observed R0/W0 | — | queue RTS/protection configuration |
| `0x600a4d24` (`0x00000d24`) | read-write R3/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a4d28` (`0x00000d28`) | not-observed R0/W0 | — | queue transmission timeout |
| `0x600a4d2c` (`0x00000d2c`) | not-observed R0/W0 | — | queue enable and TX descriptor pointer |
| `0x600a4d30` (`0x00000d30`) | read-write R3/W3 | `0x00000000` | queue RTS/protection configuration |
| `0x600a4d34` (`0x00000d34`) | read-write R3/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a4d38` (`0x00000d38`) | not-observed R0/W0 | — | queue transmission timeout |
| `0x600a4d3c` (`0x00000d3c`) | not-observed R0/W0 | — | queue enable and TX descriptor pointer |
| `0x600a4d40` (`0x00000d40`) | read-write R3/W3 | `0x00000000` | queue RTS/protection configuration |
| `0x600a4d44` (`0x00000d44`) | read-write R3/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a4d48` (`0x00000d48`) | not-observed R0/W0 | — | queue transmission timeout |
| `0x600a4d4c` (`0x00000d4c`) | not-observed R0/W0 | — | queue enable and TX descriptor pointer |
| `0x600a4d50` (`0x00000d50`) | read-write R3/W3 | `0x00000000` | queue RTS/protection configuration |
| `0x600a4d54` (`0x00000d54`) | read-write R3/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a4d58` (`0x00000d58`) | not-observed R0/W0 | — | queue transmission timeout |
| `0x600a4d5c` (`0x00000d5c`) | not-observed R0/W0 | — | queue enable and TX descriptor pointer |
| `0x600a4d60` (`0x00000d60`) | read-write R11/W11 | `0x00000000` | queue RTS/protection configuration |
| `0x600a4d64` (`0x00000d64`) | read-write R19/W11 | `0x00000000` | unknown; factual observed values only |
| `0x600a4d68` (`0x00000d68`) | read-write R40/W40 | `0x000003fe`, `0x020003fe`, `0x020013fe`, `0x020033fe` | queue transmission timeout |
| `0x600a4d6c` (`0x00000d6c`) | read-write R8/W16 | `0x0062c96c`, `0x0062d19c`, `0xc062c96c`, `0xc062d19c` | queue-0 enable and descriptor pointer |
| `0x600a4db8` (`0x00000db8`) | read R8/W0 | `0x00000000` | unknown; factual observed values only |
| `0x600a4dd0` (`0x00000dd0`) | read-write R27/W24 | `0x05000000`, `0x05700000`, `0x05770000`, `0x05777000`, `0x05777500`, `0x05777550`, `0x05777555`, `0x55777555` | unknown; factual observed values only |
| `0x600a4dd4` (`0x00000dd4`) | read-write R15/W12 | `0x00000070`, `0x00000077`, `0x00000377`, `0x00003377` | unknown; factual observed values only |
| `0x600a4dd8` (`0x00000dd8`) | read-write R8/W8 | `0x00000020`, `0x00000021` | unknown; factual observed values only |
| `0x600a4ddc` (`0x00000ddc`) | read-write R10/W5 | `0x00000002`, `0x00000003` | Wi-Fi reset strobe and ready acknowledgement |
| `0x600a4de0` (`0x00000de0`) | read-write R6/W3 | `0x00100000`, `0x00200000`, `0x00300000` | unknown; factual observed values only |
| `0x600a4df8` (`0x00000df8`) | read-write R6/W6 | `0x00000000` | unknown; factual observed values only |
| `0x600a5190` (`0x00001190`) | read-write R12/W12 | `0x00000000`, `0x00000020` | unknown; factual observed values only |
| `0x600a5204` (`0x00001204`) | read-write R12/W12 | `0x00000000`, `0x00000020` | unknown; factual observed values only |
| `0x600a5278` (`0x00001278`) | read-write R12/W12 | `0x00000000`, `0x00000020` | unknown; factual observed values only |
| `0x600a5290` (`0x00001290`) | not-observed R0/W0 | — | TX block-ack bitmap bits 32..63 |
| `0x600a5294` (`0x00001294`) | not-observed R0/W0 | — | TX block-ack bitmap bits 0..31 |
| `0x600a5298` (`0x00001298`) | not-observed R0/W0 | — | TX block-ack status and starting sequence |
| `0x600a52a4` (`0x000012a4`) | not-observed R0/W0 | — | TX completion count |
| `0x600a52a8` (`0x000012a8`) | not-observed R0/W0 | — | TX completion status |
| `0x600a52ec` (`0x000012ec`) | read-write R12/W12 | `0x00000000`, `0x00000020` | unknown; factual observed values only |
| `0x600a5304` (`0x00001304`) | not-observed R0/W0 | — | TX block-ack bitmap bits 32..63 |
| `0x600a5308` (`0x00001308`) | not-observed R0/W0 | — | TX block-ack bitmap bits 0..31 |
| `0x600a530c` (`0x0000130c`) | not-observed R0/W0 | — | TX block-ack status and starting sequence |
| `0x600a5318` (`0x00001318`) | not-observed R0/W0 | — | TX completion count |
| `0x600a531c` (`0x0000131c`) | not-observed R0/W0 | — | TX completion status |
| `0x600a5360` (`0x00001360`) | read-write R12/W12 | `0x00000000`, `0x00000020` | unknown; factual observed values only |
| `0x600a5378` (`0x00001378`) | not-observed R0/W0 | — | TX block-ack bitmap bits 32..63 |
| `0x600a537c` (`0x0000137c`) | not-observed R0/W0 | — | TX block-ack bitmap bits 0..31 |
| `0x600a5380` (`0x00001380`) | not-observed R0/W0 | — | TX block-ack status and starting sequence |
| `0x600a538c` (`0x0000138c`) | not-observed R0/W0 | — | TX completion count |
| `0x600a5390` (`0x00001390`) | not-observed R0/W0 | — | TX completion status |
| `0x600a53d4` (`0x000013d4`) | read-write R12/W12 | `0x00000000`, `0x00000020` | unknown; factual observed values only |
| `0x600a53ec` (`0x000013ec`) | not-observed R0/W0 | — | TX block-ack bitmap bits 32..63 |
| `0x600a53f0` (`0x000013f0`) | not-observed R0/W0 | — | TX block-ack bitmap bits 0..31 |
| `0x600a53f4` (`0x000013f4`) | not-observed R0/W0 | — | TX block-ack status and starting sequence |
| `0x600a5400` (`0x00001400`) | not-observed R0/W0 | — | TX completion count |
| `0x600a5404` (`0x00001404`) | not-observed R0/W0 | — | TX completion status |
| `0x600a5448` (`0x00001448`) | read-write R12/W12 | `0x00000000`, `0x00000020` | unknown; factual observed values only |
| `0x600a5460` (`0x00001460`) | not-observed R0/W0 | — | TX block-ack bitmap bits 32..63 |
| `0x600a5464` (`0x00001464`) | not-observed R0/W0 | — | TX block-ack bitmap bits 0..31 |
| `0x600a5468` (`0x00001468`) | not-observed R0/W0 | — | TX block-ack status and starting sequence |
| `0x600a5474` (`0x00001474`) | not-observed R0/W0 | — | TX completion count |
| `0x600a5478` (`0x00001478`) | not-observed R0/W0 | — | TX completion status |
| `0x600a5488` (`0x00001488`) | write R0/W8 | `0x00000030` | unknown; factual observed values only |
| `0x600a5490` (`0x00001490`) | read-write R40/W40 | `0x00000000` | unknown; factual observed values only |
| `0x600a54ac` (`0x000014ac`) | write R0/W8 | `0x08080008`, `0x0e0e000e`, `0x14140014` | unknown; factual observed values only |
| `0x600a54bc` (`0x000014bc`) | read-write R20/W28 | `0x00000000`, `0x00000020`, `0x00400000`, `0x00400004`, `0x00400020` | unknown; factual observed values only |
| `0x600a54d0` (`0x000014d0`) | read R24/W0 | `0x00000000` | unknown; factual observed values only |
| `0x600a54d4` (`0x000014d4`) | not-observed R0/W0 | — | TX block-ack bitmap bits 32..63 |
| `0x600a54d8` (`0x000014d8`) | not-observed R0/W0 | — | TX block-ack bitmap bits 0..31 |
| `0x600a54dc` (`0x000014dc`) | not-observed R0/W0 | — | TX block-ack status and starting sequence |
| `0x600a54e0` (`0x000014e0`) | read R24/W0 | `0x00000000` | unknown; factual observed values only |
| `0x600a54e8` (`0x000014e8`) | read R40/W0 | `0x00010000` | TX completion count |
| `0x600a54ec` (`0x000014ec`) | read R8/W0 | `0x00000000` | TX completion status |
| `0x600a54f4` (`0x000014f4`) | read R8/W0 | `0x00000000` | unknown; factual observed values only |
| `0x600a55f0` (`0x000015f0`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a55f4` (`0x000015f4`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a55f8` (`0x000015f8`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a55fc` (`0x000015fc`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5600` (`0x00001600`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5604` (`0x00001604`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5608` (`0x00001608`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a560c` (`0x0000160c`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5610` (`0x00001610`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5614` (`0x00001614`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5618` (`0x00001618`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a561c` (`0x0000161c`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5620` (`0x00001620`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5624` (`0x00001624`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5628` (`0x00001628`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a562c` (`0x0000162c`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5630` (`0x00001630`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5634` (`0x00001634`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5638` (`0x00001638`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a563c` (`0x0000163c`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5640` (`0x00001640`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5644` (`0x00001644`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5648` (`0x00001648`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a564c` (`0x0000164c`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5650` (`0x00001650`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5654` (`0x00001654`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5658` (`0x00001658`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a565c` (`0x0000165c`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5660` (`0x00001660`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5664` (`0x00001664`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5668` (`0x00001668`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a566c` (`0x0000166c`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5670` (`0x00001670`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5674` (`0x00001674`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5678` (`0x00001678`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a567c` (`0x0000167c`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5680` (`0x00001680`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5684` (`0x00001684`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5688` (`0x00001688`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a568c` (`0x0000168c`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5690` (`0x00001690`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5694` (`0x00001694`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5698` (`0x00001698`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a569c` (`0x0000169c`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a56a0` (`0x000016a0`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a56a4` (`0x000016a4`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a56a8` (`0x000016a8`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a56ac` (`0x000016ac`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a56b0` (`0x000016b0`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a56b4` (`0x000016b4`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a56b8` (`0x000016b8`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a56bc` (`0x000016bc`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a56c0` (`0x000016c0`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a56c4` (`0x000016c4`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a56c8` (`0x000016c8`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a56cc` (`0x000016cc`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a56d0` (`0x000016d0`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a56d4` (`0x000016d4`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a56d8` (`0x000016d8`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a56dc` (`0x000016dc`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a56e0` (`0x000016e0`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a56e4` (`0x000016e4`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a56e8` (`0x000016e8`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a56ec` (`0x000016ec`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a56f0` (`0x000016f0`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a56f4` (`0x000016f4`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a56f8` (`0x000016f8`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a56fc` (`0x000016fc`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5700` (`0x00001700`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5704` (`0x00001704`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5708` (`0x00001708`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a570c` (`0x0000170c`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5710` (`0x00001710`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5714` (`0x00001714`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5718` (`0x00001718`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a571c` (`0x0000171c`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5720` (`0x00001720`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5724` (`0x00001724`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5728` (`0x00001728`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a572c` (`0x0000172c`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5730` (`0x00001730`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5734` (`0x00001734`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5738` (`0x00001738`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a573c` (`0x0000173c`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5740` (`0x00001740`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5744` (`0x00001744`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5748` (`0x00001748`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a574c` (`0x0000174c`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5750` (`0x00001750`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5754` (`0x00001754`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5758` (`0x00001758`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a575c` (`0x0000175c`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5760` (`0x00001760`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5764` (`0x00001764`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5768` (`0x00001768`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a576c` (`0x0000176c`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5770` (`0x00001770`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5774` (`0x00001774`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5778` (`0x00001778`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a577c` (`0x0000177c`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5780` (`0x00001780`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5784` (`0x00001784`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5788` (`0x00001788`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a578c` (`0x0000178c`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5790` (`0x00001790`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5794` (`0x00001794`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5798` (`0x00001798`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a579c` (`0x0000179c`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a57a0` (`0x000017a0`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a57a4` (`0x000017a4`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a57a8` (`0x000017a8`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a57ac` (`0x000017ac`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a57b0` (`0x000017b0`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a57b4` (`0x000017b4`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a57b8` (`0x000017b8`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a57bc` (`0x000017bc`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a57c0` (`0x000017c0`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a57c4` (`0x000017c4`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a57c8` (`0x000017c8`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a57cc` (`0x000017cc`) | write R0/W3 | `0x00000000` | unknown; factual observed values only |
| `0x600a5800` (`0x00001800`) | not-observed R0/W0 | — | first word of native crypto slot 0 |
| `0x600a5804` (`0x00001804`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5808` (`0x00001808`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a580c` (`0x0000180c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5810` (`0x00001810`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5814` (`0x00001814`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5818` (`0x00001818`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a581c` (`0x0000181c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5820` (`0x00001820`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5824` (`0x00001824`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5828` (`0x00001828`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a582c` (`0x0000182c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5830` (`0x00001830`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5834` (`0x00001834`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5838` (`0x00001838`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a583c` (`0x0000183c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5840` (`0x00001840`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5844` (`0x00001844`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5848` (`0x00001848`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a584c` (`0x0000184c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5850` (`0x00001850`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5854` (`0x00001854`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5858` (`0x00001858`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a585c` (`0x0000185c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5860` (`0x00001860`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5864` (`0x00001864`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5868` (`0x00001868`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a586c` (`0x0000186c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5870` (`0x00001870`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5874` (`0x00001874`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5878` (`0x00001878`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a587c` (`0x0000187c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5880` (`0x00001880`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5884` (`0x00001884`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5888` (`0x00001888`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a588c` (`0x0000188c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5890` (`0x00001890`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5894` (`0x00001894`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5898` (`0x00001898`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a589c` (`0x0000189c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a58a0` (`0x000018a0`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a58a4` (`0x000018a4`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a58a8` (`0x000018a8`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a58ac` (`0x000018ac`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a58b0` (`0x000018b0`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a58b4` (`0x000018b4`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a58b8` (`0x000018b8`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a58bc` (`0x000018bc`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a58c0` (`0x000018c0`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a58c4` (`0x000018c4`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a58c8` (`0x000018c8`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a58cc` (`0x000018cc`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a58d0` (`0x000018d0`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a58d4` (`0x000018d4`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a58d8` (`0x000018d8`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a58dc` (`0x000018dc`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a58e0` (`0x000018e0`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a58e4` (`0x000018e4`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a58e8` (`0x000018e8`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a58ec` (`0x000018ec`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a58f0` (`0x000018f0`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a58f4` (`0x000018f4`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a58f8` (`0x000018f8`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a58fc` (`0x000018fc`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5900` (`0x00001900`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5904` (`0x00001904`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5908` (`0x00001908`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a590c` (`0x0000190c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5910` (`0x00001910`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5914` (`0x00001914`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5918` (`0x00001918`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a591c` (`0x0000191c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5920` (`0x00001920`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5924` (`0x00001924`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5928` (`0x00001928`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a592c` (`0x0000192c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5930` (`0x00001930`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5934` (`0x00001934`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5938` (`0x00001938`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a593c` (`0x0000193c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5940` (`0x00001940`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5944` (`0x00001944`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5948` (`0x00001948`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a594c` (`0x0000194c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5950` (`0x00001950`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5954` (`0x00001954`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5958` (`0x00001958`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a595c` (`0x0000195c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5960` (`0x00001960`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5964` (`0x00001964`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5968` (`0x00001968`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a596c` (`0x0000196c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5970` (`0x00001970`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5974` (`0x00001974`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5978` (`0x00001978`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a597c` (`0x0000197c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5980` (`0x00001980`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5984` (`0x00001984`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5988` (`0x00001988`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a598c` (`0x0000198c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5990` (`0x00001990`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5994` (`0x00001994`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5998` (`0x00001998`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a599c` (`0x0000199c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a59a0` (`0x000019a0`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a59a4` (`0x000019a4`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a59a8` (`0x000019a8`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a59ac` (`0x000019ac`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a59b0` (`0x000019b0`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a59b4` (`0x000019b4`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a59b8` (`0x000019b8`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a59bc` (`0x000019bc`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a59c0` (`0x000019c0`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a59c4` (`0x000019c4`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a59c8` (`0x000019c8`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a59cc` (`0x000019cc`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a59d0` (`0x000019d0`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a59d4` (`0x000019d4`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a59d8` (`0x000019d8`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a59dc` (`0x000019dc`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a59e0` (`0x000019e0`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a59e4` (`0x000019e4`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a59e8` (`0x000019e8`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a59ec` (`0x000019ec`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a59f0` (`0x000019f0`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a59f4` (`0x000019f4`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a59f8` (`0x000019f8`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a59fc` (`0x000019fc`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5a00` (`0x00001a00`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5a04` (`0x00001a04`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5a08` (`0x00001a08`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5a0c` (`0x00001a0c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5a10` (`0x00001a10`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5a14` (`0x00001a14`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5a18` (`0x00001a18`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5a1c` (`0x00001a1c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5a20` (`0x00001a20`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5a24` (`0x00001a24`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5a28` (`0x00001a28`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5a2c` (`0x00001a2c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5a30` (`0x00001a30`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5a34` (`0x00001a34`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5a38` (`0x00001a38`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5a3c` (`0x00001a3c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5a40` (`0x00001a40`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5a44` (`0x00001a44`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5a48` (`0x00001a48`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5a4c` (`0x00001a4c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5a50` (`0x00001a50`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5a54` (`0x00001a54`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5a58` (`0x00001a58`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5a5c` (`0x00001a5c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5a60` (`0x00001a60`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5a64` (`0x00001a64`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5a68` (`0x00001a68`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5a6c` (`0x00001a6c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5a70` (`0x00001a70`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5a74` (`0x00001a74`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5a78` (`0x00001a78`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5a7c` (`0x00001a7c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5a80` (`0x00001a80`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5a84` (`0x00001a84`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5a88` (`0x00001a88`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5a8c` (`0x00001a8c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5a90` (`0x00001a90`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5a94` (`0x00001a94`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5a98` (`0x00001a98`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5a9c` (`0x00001a9c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5aa0` (`0x00001aa0`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5aa4` (`0x00001aa4`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5aa8` (`0x00001aa8`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5aac` (`0x00001aac`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5ab0` (`0x00001ab0`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5ab4` (`0x00001ab4`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5ab8` (`0x00001ab8`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5abc` (`0x00001abc`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5ac0` (`0x00001ac0`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5ac4` (`0x00001ac4`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5ac8` (`0x00001ac8`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5acc` (`0x00001acc`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5ad0` (`0x00001ad0`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5ad4` (`0x00001ad4`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5ad8` (`0x00001ad8`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5adc` (`0x00001adc`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5ae0` (`0x00001ae0`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5ae4` (`0x00001ae4`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5ae8` (`0x00001ae8`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5aec` (`0x00001aec`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5af0` (`0x00001af0`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5af4` (`0x00001af4`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5af8` (`0x00001af8`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5afc` (`0x00001afc`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5b00` (`0x00001b00`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5b04` (`0x00001b04`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5b08` (`0x00001b08`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5b0c` (`0x00001b0c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5b10` (`0x00001b10`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5b14` (`0x00001b14`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5b18` (`0x00001b18`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5b1c` (`0x00001b1c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5b20` (`0x00001b20`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5b24` (`0x00001b24`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5b28` (`0x00001b28`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5b2c` (`0x00001b2c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5b30` (`0x00001b30`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5b34` (`0x00001b34`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5b38` (`0x00001b38`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5b3c` (`0x00001b3c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5b40` (`0x00001b40`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5b44` (`0x00001b44`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5b48` (`0x00001b48`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5b4c` (`0x00001b4c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5b50` (`0x00001b50`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5b54` (`0x00001b54`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5b58` (`0x00001b58`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5b5c` (`0x00001b5c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5b60` (`0x00001b60`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5b64` (`0x00001b64`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5b68` (`0x00001b68`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5b6c` (`0x00001b6c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5b70` (`0x00001b70`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5b74` (`0x00001b74`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5b78` (`0x00001b78`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5b7c` (`0x00001b7c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5b80` (`0x00001b80`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5b84` (`0x00001b84`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5b88` (`0x00001b88`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5b8c` (`0x00001b8c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5b90` (`0x00001b90`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5b94` (`0x00001b94`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5b98` (`0x00001b98`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5b9c` (`0x00001b9c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5ba0` (`0x00001ba0`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5ba4` (`0x00001ba4`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5ba8` (`0x00001ba8`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5bac` (`0x00001bac`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5bb0` (`0x00001bb0`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5bb4` (`0x00001bb4`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5bb8` (`0x00001bb8`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5bbc` (`0x00001bbc`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5bc0` (`0x00001bc0`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5bc4` (`0x00001bc4`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5bc8` (`0x00001bc8`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5bcc` (`0x00001bcc`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5bd0` (`0x00001bd0`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5bd4` (`0x00001bd4`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5bd8` (`0x00001bd8`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5bdc` (`0x00001bdc`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5be0` (`0x00001be0`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5be4` (`0x00001be4`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5be8` (`0x00001be8`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5bec` (`0x00001bec`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5bf0` (`0x00001bf0`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5bf4` (`0x00001bf4`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5bf8` (`0x00001bf8`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5bfc` (`0x00001bfc`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5c00` (`0x00001c00`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5c04` (`0x00001c04`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5c08` (`0x00001c08`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5c0c` (`0x00001c0c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5c10` (`0x00001c10`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5c14` (`0x00001c14`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5c18` (`0x00001c18`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5c1c` (`0x00001c1c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5c20` (`0x00001c20`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5c24` (`0x00001c24`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5c28` (`0x00001c28`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5c2c` (`0x00001c2c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5c30` (`0x00001c30`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5c34` (`0x00001c34`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5c38` (`0x00001c38`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5c3c` (`0x00001c3c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5c40` (`0x00001c40`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5c44` (`0x00001c44`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5c48` (`0x00001c48`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5c4c` (`0x00001c4c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5c50` (`0x00001c50`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5c54` (`0x00001c54`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5c58` (`0x00001c58`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5c5c` (`0x00001c5c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5c60` (`0x00001c60`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5c64` (`0x00001c64`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5c68` (`0x00001c68`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5c6c` (`0x00001c6c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5c70` (`0x00001c70`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5c74` (`0x00001c74`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5c78` (`0x00001c78`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5c7c` (`0x00001c7c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5c80` (`0x00001c80`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5c84` (`0x00001c84`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5c88` (`0x00001c88`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5c8c` (`0x00001c8c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5c90` (`0x00001c90`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5c94` (`0x00001c94`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5c98` (`0x00001c98`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5c9c` (`0x00001c9c`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5ca0` (`0x00001ca0`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5ca4` (`0x00001ca4`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5ca8` (`0x00001ca8`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5cac` (`0x00001cac`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5cb0` (`0x00001cb0`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5cb4` (`0x00001cb4`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5cb8` (`0x00001cb8`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5cbc` (`0x00001cbc`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5cc0` (`0x00001cc0`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5cc4` (`0x00001cc4`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5cc8` (`0x00001cc8`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5ccc` (`0x00001ccc`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5cd0` (`0x00001cd0`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5cd4` (`0x00001cd4`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5cd8` (`0x00001cd8`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5cdc` (`0x00001cdc`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5ce0` (`0x00001ce0`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5ce4` (`0x00001ce4`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5ce8` (`0x00001ce8`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5cec` (`0x00001cec`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5cf0` (`0x00001cf0`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5cf4` (`0x00001cf4`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5cf8` (`0x00001cf8`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
| `0x600a5cfc` (`0x00001cfc`) | not-observed R0/W0 | — | native Wi-Fi crypto table word |
