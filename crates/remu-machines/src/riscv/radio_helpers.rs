fn decode_cca_mode(mode: u8) -> Ieee802154CcaMode {
    match mode & 3 {
        0 => Ieee802154CcaMode::Carrier,
        1 => Ieee802154CcaMode::Energy,
        2 => Ieee802154CcaMode::CarrierOrEnergy,
        _ => Ieee802154CcaMode::CarrierAndEnergy,
    }
}

fn decode_tx_power(encoded: u8) -> i16 {
    let encoded = encoded & 0x1f;
    if encoded & 0x10 != 0 {
        i16::from(encoded) - 32
    } else {
        i16::from(encoded)
    }
}
