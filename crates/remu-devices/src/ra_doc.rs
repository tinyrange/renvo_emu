//! RA4M1 Data Operation Circuit peripheral.

use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use std::sync::{Arc, Mutex};

const DOCR: u64 = 0x00;
const DODIR: u64 = 0x02;
const DODSR: u64 = 0x04;
const DOPCF: u16 = 1 << 5;
const DOPCFCL: u16 = 1 << 6;

#[derive(Default)]
struct DocState {
    control: u16,
    input: u16,
    data: u16,
    flag: bool,
}

impl DocState {
    fn operate(&mut self) {
        match self.control & 0x03 {
            0 => {
                let matched = self.input == self.data;
                self.flag = if self.control & (1 << 2) != 0 {
                    matched
                } else {
                    !matched
                };
            }
            1 => {
                let (result, carry) = self.data.overflowing_add(self.input);
                self.data = result;
                self.flag = carry;
            }
            2 => {
                let (result, borrow) = self.data.overflowing_sub(self.input);
                self.data = result;
                self.flag = borrow;
            }
            _ => self.flag = false,
        }
    }

    fn control_read(&self) -> u16 {
        (self.control & 0x07) | if self.flag { DOPCF } else { 0 }
    }
}

/// Host-facing RA4M1 DOC state.
#[derive(Clone)]
pub struct RaDocHandle(Arc<Mutex<DocState>>);

impl RaDocHandle {
    /// Returns the current operation result/reference register.
    pub fn result(&self) -> u16 {
        self.0.lock().expect("RA DOC lock poisoned").data
    }

    /// Returns the current operation flag.
    pub fn flag(&self) -> bool {
        self.0.lock().expect("RA DOC lock poisoned").flag
    }
}

/// Functional RA4M1 compare/add/subtract data operation circuit.
pub struct RaDoc {
    name: String,
    state: Arc<Mutex<DocState>>,
}

impl RaDoc {
    /// Creates a reset DOC block.
    pub fn new(name: impl Into<String>) -> (Self, RaDocHandle) {
        let state = Arc::new(Mutex::new(DocState::default()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            RaDocHandle(state),
        )
    }
}

impl Device for RaDoc {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::HalfWord {
            return Err(DeviceError::new("RA DOC requires half-word accesses"));
        }
        let state = self.state.lock().expect("RA DOC lock poisoned");
        let value = match offset {
            DOCR => state.control_read(),
            DODIR => state.input,
            DODSR => state.data,
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled RA DOC read at {offset:#x}"
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
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::HalfWord {
            return Err(DeviceError::new("RA DOC requires half-word accesses"));
        }
        let mut state = self.state.lock().expect("RA DOC lock poisoned");
        match offset {
            DOCR => {
                let value = value as u16;
                if value & DOPCFCL != 0 {
                    state.flag = false;
                }
                state.control = value & 0x07;
            }
            DODIR => {
                state.input = value as u16;
                state.operate();
            }
            DODSR => state.data = value as u16,
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled RA DOC write at {offset:#x}"
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("RA DOC lock poisoned") = DocState::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_comparison_sets_and_clears_match_flag() {
        let (mut doc, handle) = RaDoc::new("doc");
        doc.write(DODSR, AccessWidth::HalfWord, 0x1234, SimTime::ZERO)
            .unwrap();
        doc.write(DOCR, AccessWidth::HalfWord, 1 << 2, SimTime::ZERO)
            .unwrap();
        doc.write(DODIR, AccessWidth::HalfWord, 0x1234, SimTime::ZERO)
            .unwrap();
        assert!(handle.flag());
        doc.write(DOCR, AccessWidth::HalfWord, DOPCFCL.into(), SimTime::ZERO)
            .unwrap();
        assert!(!handle.flag());
    }

    #[test]
    fn doc_add_and_subtract_store_result() {
        let (mut doc, handle) = RaDoc::new("doc");
        doc.write(DODSR, AccessWidth::HalfWord, 10, SimTime::ZERO)
            .unwrap();
        doc.write(DOCR, AccessWidth::HalfWord, 1, SimTime::ZERO)
            .unwrap();
        doc.write(DODIR, AccessWidth::HalfWord, 5, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.result(), 15);
        doc.write(DOCR, AccessWidth::HalfWord, 2, SimTime::ZERO)
            .unwrap();
        doc.write(DODIR, AccessWidth::HalfWord, 3, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.result(), 12);
    }
}
