use super::*;

#[test]
fn exti_routes_afio_selected_edges_and_clears_flags() {
    let (mut exti, handle, mut afio) = WchExti::new("exti", "afio");
    // EXTICR line 2 selects PC (the WCH encoding is PA=0, PB=1, PC=2).
    afio.write(0x08, AccessWidth::Word, 2 << (2 * 2), SimTime::ZERO)
        .unwrap();
    exti.write(0x00, AccessWidth::Word, 1 << 2, SimTime::ZERO)
        .unwrap();
    exti.write(0x08, AccessWidth::Word, 1 << 2, SimTime::ZERO)
        .unwrap();

    assert!(!handle.pending([0, 0, 0]));
    assert!(handle.pending([0, 1 << 2, 0]));
    assert_eq!(
        exti.read(0x14, AccessWidth::Word, SimTime::ZERO).unwrap(),
        1 << 2
    );
    exti.write(0x14, AccessWidth::Word, 1 << 2, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        exti.read(0x14, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0
    );
}
