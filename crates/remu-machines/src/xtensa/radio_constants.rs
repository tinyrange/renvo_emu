use remu_radio::NodeId;

pub(super) const EMULATED_NODE: NodeId = NodeId(1);
pub(super) const HOST_NODE: NodeId = NodeId(0);

// Native RWBLE interrupt causes, recovered from the revision-zero ROM ISR's
// register dispatch. These are hardware status bits, not symbol hooks: bit 5
// dispatches the programmed-slot END handler and bit 6 the SKIP handler. Bits
// 1 and 2 dispatch TX and RX respectively; bit 18 updates the RX-buffer ring.
pub(super) const S3_RWBLE_SLEEP_WAKE_COMPLETE_INTERRUPT: u32 = 1;
pub(super) const S3_RWBLE_SLEEP_WAKE_INTERRUPT: u32 = 1 << 3;
pub(super) const S3_RWBLE_RX_INTERRUPT: u32 = 1 << 2;
pub(super) const S3_RWBLE_TX_INTERRUPT: u32 = 1 << 1;
pub(super) const S3_RWBLE_END_INTERRUPT: u32 = 1 << 5;
pub(super) const S3_RWBLE_SKIP_INTERRUPT: u32 = 1 << 6;
pub(super) const S3_RADIO_INTERRUPT_SOURCES: [(&str, usize); 3] = [
    ("esp32s3.wifi-mac", 0),
    ("esp32s3.bluetooth-mac", 4),
    ("esp32s3.rwble", 8),
];
pub(super) const S3_BLE_INTERFRAME_SPACE_TICKS: u64 = 2_400;
pub(super) const S3_BLE_1M_BYTE_TICKS: u64 = 8 * 16;
pub(super) const S3_BLE_FINE_POSITION_TICKS: u64 = 8;
pub(super) const S3_BLE_FINE_POSITIONS_PER_HALF_SLOT: u64 = 625;
pub(super) const S3_BLE_HALF_SLOT_TICKS: u64 =
    S3_BLE_FINE_POSITION_TICKS * S3_BLE_FINE_POSITIONS_PER_HALF_SLOT;
pub(super) const S3_BLE_COARSE_MASK: u64 = 0x0fff_ffff;
pub(super) const S3_BLE_CLOCK_CYCLE_TICKS: u64 = (S3_BLE_COARSE_MASK + 1) * S3_BLE_HALF_SLOT_TICKS;
pub(super) const BLE_ADVERTISING_ACCESS_ADDRESS: u32 = 0x8e89_bed6;
