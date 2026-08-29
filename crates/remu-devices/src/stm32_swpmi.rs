use super::SignalHub;
use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use remu_signals::{SignalId, SignalValue};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

const CR: u64 = 0x00;
const BRR: u64 = 0x04;
const ISR: u64 = 0x0c;
const ICR: u64 = 0x10;
const IER: u64 = 0x14;
const RFL: u64 = 0x18;
const TDR: u64 = 0x1c;
const RDR: u64 = 0x20;
const OR: u64 = 0x24;

const CR_RXMODE: u32 = 1 << 2;
const CR_TXMODE: u32 = 1 << 3;
const CR_LPBK: u32 = 1 << 4;
const CR_SWPACT: u32 = 1 << 5;
const CR_DEACT: u32 = 1 << 10;

const ISR_RXBFF: u32 = 1 << 0;
const ISR_TXBEF: u32 = 1 << 1;
const ISR_RXBERF: u32 = 1 << 2;
const ISR_RXOVRF: u32 = 1 << 3;
const ISR_TXUNRF: u32 = 1 << 4;
const ISR_RXNE: u32 = 1 << 5;
const ISR_TXE: u32 = 1 << 6;
const ISR_TCF: u32 = 1 << 7;
const ISR_SRF: u32 = 1 << 8;
const ISR_SUSP: u32 = 1 << 9;
const ISR_DEACTF: u32 = 1 << 10;
const IER_RXBFIE: u32 = 1 << 0;
const IER_TXBEIE: u32 = 1 << 1;
const IER_RXBERIE: u32 = 1 << 2;
const IER_RXOVRIE: u32 = 1 << 3;
const IER_TXUNRIE: u32 = 1 << 4;
const IER_RIE: u32 = 1 << 5;
const IER_TIE: u32 = 1 << 6;
const IER_TCIE: u32 = 1 << 7;
const IER_SRIE: u32 = 1 << 8;
const IER_MASK: u32 = IER_RXBFIE
    | IER_TXBEIE
    | IER_RXBERIE
    | IER_RXOVRIE
    | IER_TXUNRIE
    | IER_RIE
    | IER_TIE
    | IER_TCIE
    | IER_SRIE;

const FIFO_DEPTH: usize = 32;

/// Host-facing SWPMI endpoint for deterministic transmit and receive stimuli.
#[derive(Clone)]
pub struct Stm32SwpmiHandle {
    state: Arc<Mutex<SwpmiState>>,
}

impl Stm32SwpmiHandle {
    /// Supplies one received SWP frame to firmware.
    pub fn inject_rx(&self, word: u32, frame_bytes: u8, at: SimTime) {
        self.state
            .lock()
            .expect("SWPMI lock poisoned")
            .inject_rx(word, frame_bytes, at);
    }

    /// Returns words transmitted by firmware in bus order.
    pub fn take_tx(&self) -> Vec<u32> {
        let mut state = self.state.lock().expect("SWPMI lock poisoned");
        state.tx.drain(..).collect()
    }

    /// Returns whether an enabled SWPMI source is pending on IRQ 76.
    pub fn interrupt_pending(&self) -> bool {
        self.state
            .lock()
            .expect("SWPMI lock poisoned")
            .interrupt_pending()
    }

    /// Returns whether the one-wire bus is active.
    pub fn active(&self) -> bool {
        self.state.lock().expect("SWPMI lock poisoned").cr & CR_SWPACT != 0
    }
}

struct SwpmiState {
    cr: u32,
    brr: u32,
    raw_flags: u32,
    ier: u32,
    rfl: u32,
    or: u32,
    rx: VecDeque<(u32, u8)>,
    tx: VecDeque<u32>,
    hub: SignalHub,
    irq_signal: SignalId,
    tx_signal: SignalId,
    tx_strobe_signal: SignalId,
    rx_signal: SignalId,
    rx_strobe_signal: SignalId,
    tx_strobe: bool,
    rx_strobe: bool,
}

impl SwpmiState {
    fn set_signal(&self, signal: SignalId, value: u64, width: u16, at: SimTime) {
        self.hub
            .set(
                signal,
                SignalValue::from_u64(value, width).expect("fixed SWPMI signal width"),
                at,
            )
            .expect("SWPMI signal remains registered");
    }

    fn interrupt_pending(&self) -> bool {
        self.raw_flags & self.ier & IER_MASK != 0
    }

    fn update_irq(&self, at: SimTime) {
        self.set_signal(self.irq_signal, u64::from(self.interrupt_pending()), 1, at);
    }

    fn update_receive_flags(&mut self) {
        if self.rx.is_empty() {
            self.raw_flags &= !(ISR_RXNE | ISR_RXBFF);
            self.rfl = 0;
        } else {
            self.raw_flags |= ISR_RXNE | ISR_RXBFF;
            self.rfl = u32::from(self.rx.front().map_or(0, |(_, length)| *length & 3));
        }
    }

    fn inject_rx(&mut self, word: u32, frame_bytes: u8, at: SimTime) {
        let frame_bytes = frame_bytes.clamp(1, 4);
        if self.rx.len() >= FIFO_DEPTH {
            self.raw_flags |= ISR_RXOVRF;
        } else {
            self.rx.push_back((word, frame_bytes));
            self.update_receive_flags();
            self.set_signal(self.rx_signal, u64::from(word), 32, at);
            self.rx_strobe = !self.rx_strobe;
            self.set_signal(self.rx_strobe_signal, u64::from(self.rx_strobe), 1, at);
        }
        self.update_irq(at);
    }

    fn transmit(&mut self, word: u32, at: SimTime) {
        if self.cr & CR_SWPACT == 0 {
            self.raw_flags |= ISR_TXUNRF;
            self.update_irq(at);
            return;
        }
        self.raw_flags &= !(ISR_TXE | ISR_TXBEF | ISR_TCF | ISR_SUSP);
        self.tx.push_back(word);
        self.set_signal(self.tx_signal, u64::from(word), 32, at);
        self.tx_strobe = !self.tx_strobe;
        self.set_signal(self.tx_strobe_signal, u64::from(self.tx_strobe), 1, at);
        if self.cr & CR_LPBK != 0 {
            self.inject_rx(word, 4, at);
        }
        // Functional transfers complete at the end of this register access;
        // protocol bit timing is deliberately outside the initial slice.
        self.raw_flags |= ISR_TXE | ISR_TXBEF | ISR_TCF;
        self.update_irq(at);
    }

    fn read_rx(&mut self, at: SimTime) -> u32 {
        let value = self.rx.pop_front().map_or(0, |(word, _)| word);
        self.update_receive_flags();
        self.set_signal(self.rx_signal, u64::from(value), 32, at);
        self.rx_strobe = !self.rx_strobe;
        self.set_signal(self.rx_strobe_signal, u64::from(self.rx_strobe), 1, at);
        self.update_irq(at);
        value
    }

    fn reset(&mut self, at: SimTime) {
        self.cr = 0;
        self.brr = 0;
        self.raw_flags = ISR_TXE;
        self.ier = 0;
        self.rfl = 0;
        self.or = 0;
        self.rx.clear();
        self.tx.clear();
        self.tx_strobe = false;
        self.rx_strobe = false;
        self.set_signal(self.irq_signal, 0, 1, at);
        self.set_signal(self.tx_signal, 0, 32, at);
        self.set_signal(self.tx_strobe_signal, 0, 1, at);
        self.set_signal(self.rx_signal, 0, 32, at);
        self.set_signal(self.rx_strobe_signal, 0, 1, at);
    }
}

/// Functional STM32L432 SWPMI1 master interface.
pub struct Stm32Swpmi {
    name: String,
    state: Arc<Mutex<SwpmiState>>,
}

impl Stm32Swpmi {
    /// Creates an inactive SWPMI bus with an empty receive queue.
    pub fn new(
        name: impl Into<String>,
        hub: SignalHub,
    ) -> Result<(Self, Stm32SwpmiHandle), remu_signals::SignalError> {
        let irq_signal = hub.declare(
            "board.stm32l432kc.swpmi.irq",
            SignalValue::from_u64(0, 1)?,
            Some("enabled SWPMI interrupt request".to_owned()),
        )?;
        let tx_signal = hub.declare(
            "board.stm32l432kc.swpmi.tx_word",
            SignalValue::from_u64(0, 32)?,
            Some("last SWPMI transmitted frame".to_owned()),
        )?;
        let tx_strobe_signal = hub.declare(
            "board.stm32l432kc.swpmi.tx_strobe",
            SignalValue::from_u64(0, 1)?,
            Some("toggles for SWPMI transmissions".to_owned()),
        )?;
        let rx_signal = hub.declare(
            "board.stm32l432kc.swpmi.rx_word",
            SignalValue::from_u64(0, 32)?,
            Some("last SWPMI received frame".to_owned()),
        )?;
        let rx_strobe_signal = hub.declare(
            "board.stm32l432kc.swpmi.rx_strobe",
            SignalValue::from_u64(0, 1)?,
            Some("toggles for SWPMI receptions".to_owned()),
        )?;
        let state = Arc::new(Mutex::new(SwpmiState {
            cr: 0,
            brr: 0,
            raw_flags: ISR_TXE,
            ier: 0,
            rfl: 0,
            or: 0,
            rx: VecDeque::new(),
            tx: VecDeque::new(),
            hub,
            irq_signal,
            tx_signal,
            tx_strobe_signal,
            rx_signal,
            rx_strobe_signal,
            tx_strobe: false,
            rx_strobe: false,
        }));
        Ok((
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Stm32SwpmiHandle { state },
        ))
    }
}

impl Device for Stm32Swpmi {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        let mut state = self.state.lock().expect("SWPMI lock poisoned");
        if offset == RDR {
            if width != AccessWidth::Word {
                return Err(DeviceError::new("SWPMI RDR requires word reads"));
            }
            return Ok(u64::from(state.read_rx(at)));
        }
        if width != AccessWidth::Word {
            return Err(DeviceError::new(
                "STM32 SWPMI registers require word accesses",
            ));
        }
        let value = match offset {
            CR => state.cr,
            BRR => state.brr,
            ISR => state.raw_flags,
            ICR => 0,
            IER => state.ier,
            RFL => state.rfl,
            TDR => 0,
            OR => state.or,
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled SWPMI read at {offset:#x}"
                )));
            }
        };
        Ok(u64::from(value))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        let mut state = self.state.lock().expect("SWPMI lock poisoned");
        if offset == TDR {
            if width != AccessWidth::Word {
                return Err(DeviceError::new("SWPMI TDR requires word writes"));
            }
            state.transmit(value as u32, at);
            return Ok(());
        }
        if width != AccessWidth::Word {
            return Err(DeviceError::new(
                "STM32 SWPMI registers require word accesses",
            ));
        }
        let value = value as u32;
        match offset {
            CR => {
                if value & CR_DEACT != 0 {
                    state.cr &= !CR_SWPACT;
                    state.raw_flags |= ISR_DEACTF;
                } else {
                    state.cr = value & (CR_RXMODE | CR_TXMODE | CR_LPBK | CR_SWPACT);
                    if state.cr & CR_SWPACT != 0 {
                        state.raw_flags &= !ISR_DEACTF;
                    }
                }
                state.update_irq(at);
            }
            BRR => state.brr = value & 0x3f,
            ICR => {
                let clear = value
                    & (ISR_RXBFF
                        | ISR_TXBEF
                        | ISR_RXBERF
                        | ISR_RXOVRF
                        | ISR_TXUNRF
                        | ISR_TCF
                        | ISR_SRF);
                state.raw_flags &= !clear;
                state.update_receive_flags();
                state.update_irq(at);
            }
            IER => {
                state.ier = value & IER_MASK;
                state.update_irq(at);
            }
            RFL => {}
            OR => state.or = value & 0x3,
            ISR | TDR | RDR => {
                return Err(DeviceError::new(format!(
                    "SWPMI register at {offset:#x} is not writable"
                )));
            }
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled SWPMI write at {offset:#x}"
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state
            .lock()
            .expect("SWPMI lock poisoned")
            .reset(SimTime::ZERO);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use remu_bus::Device;

    fn peripheral() -> (Stm32Swpmi, Stm32SwpmiHandle) {
        let (device, handle) = Stm32Swpmi::new("swpmi", SignalHub::new()).unwrap();
        (device, handle)
    }

    #[test]
    fn loopback_transmits_and_receives_a_frame() {
        let (mut swpmi, handle) = peripheral();
        swpmi
            .write(
                CR,
                AccessWidth::Word,
                u64::from(CR_SWPACT | CR_LPBK),
                SimTime::ZERO,
            )
            .unwrap();
        swpmi
            .write(IER, AccessWidth::Word, u64::from(IER_RIE), SimTime::ZERO)
            .unwrap();
        swpmi
            .write(TDR, AccessWidth::Word, 0x1122_3344, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.take_tx(), vec![0x1122_3344]);
        assert_ne!(
            swpmi.read(ISR, AccessWidth::Word, SimTime::ZERO).unwrap() & u64::from(ISR_RXNE),
            0
        );
        assert_eq!(
            swpmi.read(RDR, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0x1122_3344
        );
        assert!(!handle.interrupt_pending());
    }

    #[test]
    fn host_receive_sets_frame_length_and_clearable_irq() {
        let (mut swpmi, handle) = peripheral();
        swpmi
            .write(IER, AccessWidth::Word, u64::from(IER_RIE), SimTime::ZERO)
            .unwrap();
        handle.inject_rx(0xaabb_ccdd, 3, SimTime::ZERO);
        assert_eq!(
            swpmi.read(RFL, AccessWidth::Word, SimTime::ZERO).unwrap(),
            3
        );
        assert!(handle.interrupt_pending());
        assert_eq!(
            swpmi.read(RDR, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0xaabb_ccdd
        );
        assert!(!handle.interrupt_pending());
    }

    #[test]
    fn deactivation_and_flag_clear_follow_register_contract() {
        let (mut swpmi, handle) = peripheral();
        swpmi
            .write(CR, AccessWidth::Word, u64::from(CR_SWPACT), SimTime::ZERO)
            .unwrap();
        swpmi
            .write(
                IER,
                AccessWidth::Word,
                u64::from(IER_TCIE | IER_TXUNRIE),
                SimTime::ZERO,
            )
            .unwrap();
        swpmi
            .write(TDR, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert!(handle.interrupt_pending());
        swpmi
            .write(ICR, AccessWidth::Word, u64::from(ISR_TCF), SimTime::ZERO)
            .unwrap();
        assert!(!handle.interrupt_pending());
        swpmi
            .write(CR, AccessWidth::Word, u64::from(CR_DEACT), SimTime::ZERO)
            .unwrap();
        assert!(!handle.active());
    }
}
