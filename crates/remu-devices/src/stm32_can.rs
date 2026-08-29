use super::*;

const MCR: u64 = 0x00;
const MSR: u64 = 0x04;
const TSR: u64 = 0x08;
const RF0R: u64 = 0x0c;
const IER: u64 = 0x14;
const ESR: u64 = 0x18;
const BTR: u64 = 0x1c;
const TX_MAILBOX: u64 = 0x180;
const RX_FIFO0: u64 = 0x1b0;
const RX_FIFO0_DLC: u64 = 0x1b4;
const RX_FIFO0_LOW: u64 = 0x1b8;
const RX_FIFO0_HIGH: u64 = 0x1bc;
const MAILBOX_STRIDE: u64 = 0x10;

const MCR_INRQ: u32 = 1 << 0;
const MCR_RESET: u32 = 1 << 15;
const MSR_INAK: u32 = 1 << 0;
const TSR_RQCP0: u32 = 1 << 0;
const TSR_TXOK0: u32 = 1 << 1;
const TSR_TERR0: u32 = 1 << 3;
const RF0R_FMP0_MASK: u32 = 0x3;
const RF0R_FULL0: u32 = 1 << 3;
const RF0R_FOVR0: u32 = 1 << 4;
const RF0R_RFOM0: u32 = 1 << 5;
const IER_TMEIE: u32 = 1 << 0;
const IER_FMPIE0: u32 = 1 << 1;
const IER_FFIE0: u32 = 1 << 2;
const IER_FOVIE0: u32 = 1 << 3;
const BTR_LBKM: u32 = 1 << 30;
const TXRQ: u32 = 1;
const RTR: u32 = 1 << 1;
const IDE: u32 = 1 << 2;
const FIFO_DEPTH: usize = 3;

#[derive(Clone, Copy, Default)]
struct CanFrame {
    identifier: u32,
    remote: bool,
    extended: bool,
    length: u8,
    low: u32,
    high: u32,
}

/// Functional STM32L4 bxCAN controller slice.
///
/// The model covers the native mailbox/FIFO register contract used by
/// bare-metal and HAL loopback tests: initialization handshake, bit-timing
/// loopback selection, one transmit mailbox, receive FIFO 0, frame payloads,
/// completion/error flags, and maskable status interrupts. It is deliberately
/// not a bit-level CAN bus or arbitration model.
pub struct Stm32Can {
    name: String,
    registers: [u32; 0x400 / 4],
    tx_mailboxes: [CanFrame; 3],
    rx_fifo: VecDeque<CanFrame>,
}

impl Stm32Can {
    /// Creates a reset bxCAN controller.
    pub fn new(name: impl Into<String>) -> Self {
        let mut can = Self {
            name: name.into(),
            registers: [0; 0x400 / 4],
            tx_mailboxes: [CanFrame::default(); 3],
            rx_fifo: VecDeque::new(),
        };
        can.reset_state();
        can
    }

    fn reset_state(&mut self) {
        self.registers = [0; 0x400 / 4];
        self.tx_mailboxes = [CanFrame::default(); 3];
        self.rx_fifo.clear();
        self.registers[(MCR / 4) as usize] = MCR_INRQ;
        self.registers[(MSR / 4) as usize] = MSR_INAK;
    }

    fn index(offset: u64) -> Result<usize, DeviceError> {
        if offset & 3 != 0 {
            return Err(DeviceError::new("STM32 CAN requires aligned word access"));
        }
        let index = usize::try_from(offset / 4).expect("CAN offset fits usize");
        (index < 0x400 / 4)
            .then_some(index)
            .ok_or_else(|| DeviceError::new(format!("STM32 CAN access at {offset:#x}")))
    }

    fn loopback(&self) -> bool {
        self.registers[(BTR / 4) as usize] & BTR_LBKM != 0
    }

    fn mailbox_index(offset: u64) -> Option<(usize, u64)> {
        if !(TX_MAILBOX..TX_MAILBOX + 3 * MAILBOX_STRIDE).contains(&offset) {
            return None;
        }
        let relative = offset - TX_MAILBOX;
        Some((
            usize::try_from(relative / MAILBOX_STRIDE).ok()?,
            relative % MAILBOX_STRIDE,
        ))
    }

    fn tx_status(&self) -> u32 {
        let mut status = self.registers[(TSR / 4) as usize];
        for mailbox in 0..3 {
            status |= 1 << (26 + mailbox);
        }
        status
    }

    fn rx_status(&self) -> u32 {
        let count = self.rx_fifo.len().min(FIFO_DEPTH) as u32;
        let mut status = count & RF0R_FMP0_MASK;
        if self.rx_fifo.len() >= FIFO_DEPTH {
            status |= RF0R_FULL0;
        }
        status | (self.registers[(RF0R / 4) as usize] & RF0R_FOVR0)
    }

    fn rx_frame(&self) -> CanFrame {
        self.rx_fifo.front().copied().unwrap_or_default()
    }

    fn write_mailbox(&mut self, mailbox: usize, register: u64, value: u32) {
        let submit = {
            let frame = &mut self.tx_mailboxes[mailbox];
            match register {
                0x00 => {
                    frame.identifier = if value & IDE != 0 {
                        (value >> 3) & 0x1fff_ffff
                    } else {
                        (value >> 21) & 0x7ff
                    };
                    frame.remote = value & RTR != 0;
                    frame.extended = value & IDE != 0;
                    value & TXRQ != 0
                }
                0x04 => {
                    frame.length = (value & 0xf).min(8) as u8;
                    false
                }
                0x08 => {
                    frame.low = value;
                    false
                }
                0x0c => {
                    frame.high = value;
                    false
                }
                _ => false,
            }
        };
        if submit {
            self.submit_mailbox(mailbox);
        }
    }

    fn submit_mailbox(&mut self, mailbox: usize) {
        let frame = self.tx_mailboxes[mailbox];
        let shift = mailbox * 8;
        self.registers[(TSR / 4) as usize] |= TSR_RQCP0 << shift;
        if self.registers[(MSR / 4) as usize] & MSR_INAK != 0 {
            self.registers[(TSR / 4) as usize] |= TSR_TERR0 << shift;
            return;
        }
        self.registers[(TSR / 4) as usize] |= TSR_TXOK0 << shift;
        if self.loopback() {
            if self.rx_fifo.len() < FIFO_DEPTH {
                self.rx_fifo.push_back(frame);
            } else {
                self.registers[(RF0R / 4) as usize] |= RF0R_FOVR0;
            }
        }
    }

    fn interrupt_pending(&self) -> bool {
        let ier = self.registers[(IER / 4) as usize];
        let tsr = self.tx_status();
        let rfr = self.rx_status();
        (ier & IER_TMEIE != 0 && tsr & (TSR_RQCP0 | TSR_TXOK0 | TSR_TERR0) != 0)
            || (ier & IER_FMPIE0 != 0 && rfr & RF0R_FMP0_MASK != 0)
            || (ier & IER_FFIE0 != 0 && rfr & RF0R_FULL0 != 0)
            || (ier & IER_FOVIE0 != 0 && rfr & RF0R_FOVR0 != 0)
    }
}

impl Device for Stm32Can {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("STM32 CAN requires word accesses"));
        }
        let index = Self::index(offset)?;
        let frame = self.rx_frame();
        let value = match offset {
            MSR => self.registers[index] | u32::from(self.interrupt_pending()) << 2,
            TSR => self.tx_status(),
            RF0R => self.rx_status(),
            RX_FIFO0 => frame.identifier << 3 | u32::from(frame.remote) << 1,
            RX_FIFO0_DLC => u32::from(frame.length),
            RX_FIFO0_LOW => frame.low,
            RX_FIFO0_HIGH => frame.high,
            _ => {
                Self::mailbox_index(offset).map_or(self.registers[index], |(mailbox, register)| {
                    match register {
                        0x00 => {
                            self.tx_mailboxes[mailbox].identifier << 21
                                | u32::from(self.tx_mailboxes[mailbox].remote) << 1
                                | u32::from(self.tx_mailboxes[mailbox].extended) << 2
                        }
                        0x04 => u32::from(self.tx_mailboxes[mailbox].length),
                        0x08 => self.tx_mailboxes[mailbox].low,
                        0x0c => self.tx_mailboxes[mailbox].high,
                        _ => 0,
                    }
                })
            }
        };
        Ok(u64::from(value))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("STM32 CAN requires word accesses"));
        }
        let index = Self::index(offset)?;
        let value = value as u32;
        match offset {
            MCR => {
                if value & MCR_RESET != 0 {
                    self.reset_state();
                } else {
                    self.registers[index] = value;
                    self.registers[(MSR / 4) as usize] =
                        u32::from(value & MCR_INRQ != 0) * MSR_INAK;
                }
            }
            TSR => self.registers[index] &= !(value & 0x0000_00ff),
            RF0R => {
                if value & RF0R_RFOM0 != 0 {
                    self.rx_fifo.pop_front();
                }
                self.registers[index] &= !(value & (RF0R_FULL0 | RF0R_FOVR0));
            }
            IER | ESR | BTR => self.registers[index] = value,
            offset if Self::mailbox_index(offset).is_some() => {
                let (mailbox, register) = Self::mailbox_index(offset).expect("matched mailbox");
                self.write_mailbox(mailbox, register, value);
            }
            RX_FIFO0..=0x1bc => {}
            _ => self.registers[index] = value,
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.reset_state();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_mailbox_reaches_receive_fifo() {
        let mut can = Stm32Can::new("can1");
        can.write(MCR, AccessWidth::Word, 0, SimTime::ZERO).unwrap();
        can.write(BTR, AccessWidth::Word, BTR_LBKM as u64, SimTime::ZERO)
            .unwrap();
        can.write(
            IER,
            AccessWidth::Word,
            u64::from(IER_TMEIE | IER_FMPIE0),
            SimTime::ZERO,
        )
        .unwrap();
        can.write(TX_MAILBOX + 4, AccessWidth::Word, 8, SimTime::ZERO)
            .unwrap();
        can.write(
            TX_MAILBOX + 8,
            AccessWidth::Word,
            0x4433_2211,
            SimTime::ZERO,
        )
        .unwrap();
        can.write(
            TX_MAILBOX + 12,
            AccessWidth::Word,
            0x8877_6655,
            SimTime::ZERO,
        )
        .unwrap();
        can.write(
            TX_MAILBOX,
            AccessWidth::Word,
            u64::from((0x123 << 21) | TXRQ),
            SimTime::ZERO,
        )
        .unwrap();

        assert_eq!(can.read(MSR, AccessWidth::Word, SimTime::ZERO), Ok(4));
        assert_eq!(can.read(RF0R, AccessWidth::Word, SimTime::ZERO), Ok(1));
        assert_eq!(
            can.read(RX_FIFO0, AccessWidth::Word, SimTime::ZERO),
            Ok(0x123 << 3)
        );
        assert_eq!(
            can.read(RX_FIFO0 + 4, AccessWidth::Word, SimTime::ZERO),
            Ok(8)
        );
        assert_eq!(
            can.read(RX_FIFO0 + 8, AccessWidth::Word, SimTime::ZERO),
            Ok(0x4433_2211)
        );
        assert_eq!(
            can.read(RX_FIFO0 + 12, AccessWidth::Word, SimTime::ZERO),
            Ok(0x8877_6655)
        );
        can.write(
            RF0R,
            AccessWidth::Word,
            u64::from(RF0R_RFOM0),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(can.read(RF0R, AccessWidth::Word, SimTime::ZERO), Ok(0));
    }

    #[test]
    fn transmit_in_init_mode_sets_mailbox_error() {
        let mut can = Stm32Can::new("can1");
        can.write(TX_MAILBOX + 4, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        can.write(TX_MAILBOX + 8, AccessWidth::Word, 0xaa, SimTime::ZERO)
            .unwrap();
        can.write(
            TX_MAILBOX,
            AccessWidth::Word,
            u64::from(TXRQ),
            SimTime::ZERO,
        )
        .unwrap();
        assert_ne!(can.read(TSR, AccessWidth::Word, SimTime::ZERO).unwrap(), 0);
    }
}
