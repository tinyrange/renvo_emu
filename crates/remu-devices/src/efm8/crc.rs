pub(super) fn crc16_ccitt(mut crc: u16, input: u8) -> u16 {
    crc ^= u16::from(input) << 8;
    for _ in 0..8 {
        crc = if crc & 0x8000 != 0 {
            (crc << 1) ^ 0x1021
        } else {
            crc << 1
        };
    }
    crc
}

pub(super) fn reverse_bits(value: u8) -> u8 {
    value.reverse_bits()
}
