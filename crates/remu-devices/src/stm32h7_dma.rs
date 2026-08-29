use super::*;
use remu_core::{AccessKind, Bus};

const STREAMS: usize = 8;
const STREAM_BASE: u64 = 0x10;
const STREAM_STRIDE: u64 = 0x18;

const CR_EN: u32 = 1 << 0;
const CR_DMEIE: u32 = 1 << 1;
const CR_TEIE: u32 = 1 << 2;
const CR_HTIE: u32 = 1 << 3;
const CR_TCIE: u32 = 1 << 4;
const CR_DIR_MASK: u32 = 0b11 << 6;
const CR_CIRC: u32 = 1 << 8;
const CR_PINC: u32 = 1 << 9;
const CR_MINC: u32 = 1 << 10;
const CR_PSIZE_MASK: u32 = 0b11 << 11;
const CR_MSIZE_MASK: u32 = 0b11 << 13;
const CR_SUPPORTED: u32 = 0x0fe7_ffff;

#[derive(Clone, Copy, Default)]
struct Stream {
    cr: u32,
    ndtr: u16,
    initial_ndtr: u16,
    par: u32,
    m0ar: u32,
    m1ar: u32,
    fcr: u32,
    initial_par: u32,
    initial_m0ar: u32,
}

#[derive(Default)]
struct State {
    streams: [Stream; STREAMS],
    lisr: u32,
    hisr: u32,
}

/// Host-facing handle for deterministic STM32H7 DMA stream progress.
#[derive(Clone)]
pub struct Stm32H7DmaHandle(Arc<Mutex<State>>);

impl Stm32H7DmaHandle {
    /// Services one transfer unit for every enabled stream.
    pub fn service(&self, bus: &mut dyn Bus, at: SimTime) -> Result<usize, DeviceError> {
        let mut serviced = 0;
        for index in 0..STREAMS {
            let Some((source, destination, width, pinc, minc)) = ({
                let state = self.0.lock().expect("STM32H7 DMA lock poisoned");
                let stream = state.streams[index];
                if stream.cr & CR_EN == 0 || stream.ndtr == 0 {
                    None
                } else if (stream.cr & CR_DIR_MASK) >> 6 != 2 {
                    drop(state);
                    self.mark_error(index);
                    None
                } else {
                    let pwidth = width(stream.cr, CR_PSIZE_MASK);
                    let mwidth = width(stream.cr, CR_MSIZE_MASK);
                    if pwidth != mwidth {
                        drop(state);
                        self.mark_error(index);
                        None
                    } else {
                        Some((
                            u64::from(stream.par),
                            u64::from(stream.m0ar),
                            pwidth,
                            stream.cr & CR_PINC != 0,
                            stream.cr & CR_MINC != 0,
                        ))
                    }
                }
            }) else {
                continue;
            };

            let value = bus
                .read(source, width, AccessKind::Read, at)
                .map_err(|error| {
                    self.mark_error(index);
                    DeviceError::new(format!("STM32H7 DMA stream {index} read: {error}"))
                })?;
            bus.write(destination, width, value, at).map_err(|error| {
                self.mark_error(index);
                DeviceError::new(format!("STM32H7 DMA stream {index} write: {error}"))
            })?;

            let mut state = self.0.lock().expect("STM32H7 DMA lock poisoned");
            let stream = &mut state.streams[index];
            let step = u32::from(width.bytes());
            if pinc {
                stream.par = stream.par.wrapping_add(step);
            }
            if minc {
                stream.m0ar = stream.m0ar.wrapping_add(step);
            }
            stream.ndtr = stream.ndtr.saturating_sub(1);
            let remaining = stream.ndtr;
            let halfway = stream.initial_ndtr / 2;
            let cr = stream.cr;
            if remaining == halfway && halfway != 0 {
                set_flag(&mut state, index, 4);
            }
            if remaining == 0 {
                set_flag(&mut state, index, 5);
                let stream = &mut state.streams[index];
                if cr & CR_CIRC != 0 {
                    stream.ndtr = stream.initial_ndtr;
                    stream.par = stream.initial_par;
                    stream.m0ar = stream.initial_m0ar;
                } else {
                    stream.cr &= !CR_EN;
                }
            }
            serviced += 1;
        }
        Ok(serviced)
    }

    /// Returns whether one stream has an enabled pending interrupt flag.
    pub fn stream_pending(&self, index: usize) -> bool {
        let state = self.0.lock().expect("STM32H7 DMA lock poisoned");
        let Some(stream) = state.streams.get(index) else {
            return false;
        };
        let flags = status(&state, index);
        (stream.cr & CR_DMEIE != 0 && flags & (1 << 2) != 0)
            || (stream.cr & CR_TEIE != 0 && flags & (1 << 3) != 0)
            || (stream.cr & CR_HTIE != 0 && flags & (1 << 4) != 0)
            || (stream.cr & CR_TCIE != 0 && flags & (1 << 5) != 0)
    }

    fn mark_error(&self, index: usize) {
        let mut state = self.0.lock().expect("STM32H7 DMA lock poisoned");
        set_flag(&mut state, index, 3);
        state.streams[index].cr &= !CR_EN;
    }
}

/// Functional STM32H7 DMA1/DMA2 stream controller.
pub struct Stm32H7Dma {
    name: String,
    state: Arc<Mutex<State>>,
}

impl Stm32H7Dma {
    /// Creates a reset eight-stream controller and service handle.
    pub fn new(name: impl Into<String>) -> (Self, Stm32H7DmaHandle) {
        let state = Arc::new(Mutex::new(State::default()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Stm32H7DmaHandle(state),
        )
    }

    fn stream(offset: u64) -> Option<(usize, u64)> {
        let relative = offset.checked_sub(STREAM_BASE)?;
        let index = usize::try_from(relative / STREAM_STRIDE).ok()?;
        (index < STREAMS).then_some((index, relative % STREAM_STRIDE))
    }
}

impl Device for Stm32H7Dma {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        require_word(offset, width)?;
        let state = self.state.lock().expect("STM32H7 DMA lock poisoned");
        let value = match offset {
            0x00 => state.lisr,
            0x04 => state.hisr,
            0x08 | 0x0c => 0,
            _ => {
                let (index, register) = Self::stream(offset).ok_or_else(|| {
                    DeviceError::new(format!("{} invalid register {offset:#x}", self.name))
                })?;
                let stream = state.streams[index];
                match register {
                    0x00 => stream.cr,
                    0x04 => u32::from(stream.ndtr),
                    0x08 => stream.par,
                    0x0c => stream.m0ar,
                    0x10 => stream.m1ar,
                    0x14 => stream.fcr,
                    _ => return Err(DeviceError::new("STM32H7 DMA invalid stream register")),
                }
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
        require_word(offset, width)?;
        let value = value as u32;
        let mut state = self.state.lock().expect("STM32H7 DMA lock poisoned");
        match offset {
            0x00 | 0x04 => return Err(DeviceError::new("STM32H7 DMA status is read-only")),
            0x08 => state.lisr &= !value,
            0x0c => state.hisr &= !value,
            _ => {
                let (index, register) = Self::stream(offset).ok_or_else(|| {
                    DeviceError::new(format!("{} invalid register {offset:#x}", self.name))
                })?;
                let stream = &mut state.streams[index];
                match register {
                    0x00 => {
                        let enabling = stream.cr & CR_EN == 0 && value & CR_EN != 0;
                        stream.cr = value & CR_SUPPORTED;
                        if enabling {
                            stream.initial_ndtr = stream.ndtr;
                            stream.initial_par = stream.par;
                            stream.initial_m0ar = stream.m0ar;
                        }
                    }
                    0x04 if stream.cr & CR_EN == 0 => stream.ndtr = value as u16,
                    0x08 if stream.cr & CR_EN == 0 => stream.par = value,
                    0x0c if stream.cr & CR_EN == 0 => stream.m0ar = value,
                    0x10 if stream.cr & CR_EN == 0 => stream.m1ar = value,
                    0x14 if stream.cr & CR_EN == 0 => stream.fcr = value & 0x87,
                    0x04 | 0x08 | 0x0c | 0x10 | 0x14 => {}
                    _ => return Err(DeviceError::new("STM32H7 DMA invalid stream register")),
                }
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("STM32H7 DMA lock poisoned") = State::default();
    }
}

fn require_word(offset: u64, width: AccessWidth) -> Result<(), DeviceError> {
    if width != AccessWidth::Word || offset & 3 != 0 {
        return Err(DeviceError::new(
            "STM32H7 DMA requires aligned word accesses",
        ));
    }
    Ok(())
}

fn width(control: u32, mask: u32) -> AccessWidth {
    match (control & mask) >> mask.trailing_zeros() {
        0 => AccessWidth::Byte,
        1 => AccessWidth::HalfWord,
        _ => AccessWidth::Word,
    }
}

fn flag_shift(index: usize) -> u32 {
    [0, 6, 16, 22][index % 4]
}

fn status(state: &State, index: usize) -> u32 {
    let register = if index < 4 { state.lisr } else { state.hisr };
    (register >> flag_shift(index)) & 0x3f
}

fn set_flag(state: &mut State, index: usize, bit: u32) {
    let mask = 1 << (flag_shift(index) + bit);
    if index < 4 {
        state.lisr |= mask;
    } else {
        state.hisr |= mask;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use remu_bus::{AddressSpace, Endianness};

    #[test]
    fn memory_to_memory_stream_sets_half_and_complete_flags() {
        let mut bus = AddressSpace::new(Endianness::Little);
        bus.map_ram("ram", 0x2400_0000, 0x100, true).unwrap();
        let (dma, handle) = Stm32H7Dma::new("dma1");
        bus.map_device("dma", 0x4002_0000, 0x400, Box::new(dma))
            .unwrap();
        for (offset, value) in [(0, 0x1234_5678), (4, 0x9abc_def0)] {
            bus.write(
                0x2400_0000 + offset,
                AccessWidth::Word,
                value,
                SimTime::ZERO,
            )
            .unwrap();
        }
        bus.write(0x4002_0018, AccessWidth::Word, 0x2400_0000, SimTime::ZERO)
            .unwrap();
        bus.write(0x4002_001c, AccessWidth::Word, 0x2400_0008, SimTime::ZERO)
            .unwrap();
        bus.write(0x4002_0014, AccessWidth::Word, 2, SimTime::ZERO)
            .unwrap();
        bus.write(
            0x4002_0010,
            AccessWidth::Word,
            u64::from(
                CR_EN | CR_HTIE | CR_TCIE | (2 << 6) | CR_PINC | CR_MINC | (2 << 11) | (2 << 13),
            ),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(handle.service(&mut bus, SimTime::ZERO).unwrap(), 1);
        assert_eq!(handle.service(&mut bus, SimTime::ZERO).unwrap(), 1);
        assert_eq!(
            bus.read(
                0x2400_0008,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO
            )
            .unwrap(),
            0x1234_5678
        );
        assert_eq!(
            bus.read(
                0x2400_000c,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO
            )
            .unwrap(),
            0x9abc_def0
        );
        assert_eq!(
            bus.read(
                0x4002_0000,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO
            )
            .unwrap()
                & 0x30,
            0x30
        );
        assert!(handle.stream_pending(0));
        bus.write(0x4002_0008, AccessWidth::Word, 0x30, SimTime::ZERO)
            .unwrap();
        assert!(!handle.stream_pending(0));
    }
}
