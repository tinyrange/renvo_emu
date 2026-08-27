use super::*;

pub(super) const P0MAT: usize = 0xfd;
pub(super) const P0MASK: usize = 0xfe;
pub(super) const P1MAT: usize = 0xed;
pub(super) const P1MASK: usize = 0xee;
pub(super) const P2MAT: usize = (PAGE3 << 8) | 0xfb;
pub(super) const P2MASK: usize = (PAGE3 << 8) | 0xfc;
pub(super) const EIE1_EMAT: u8 = 0x02;

impl Efm8State {
    pub(super) fn port_match_active(&self) -> bool {
        let masks = [
            self.registers[P0MASK],
            self.registers[P1MASK],
            self.registers[P2MASK],
        ];
        let matches = [
            self.registers[P0MAT],
            self.registers[P1MAT],
            self.registers[P2MAT],
        ];
        (0..3).any(|port| {
            let mask = masks[port] & PORT_MASKS[port];
            let input = self.resolved_port(port);
            (input & mask) != (matches[port] & mask)
        })
    }

    pub(super) fn refresh_port_match(&mut self, at: SimTime) {
        self.port_match_event = self.port_match_active();
        self.set_signal(
            self.port_match_signal,
            u64::from(self.port_match_event),
            1,
            at,
        );
    }
}
