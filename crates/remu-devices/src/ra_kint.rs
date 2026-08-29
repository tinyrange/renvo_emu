use super::*;

#[derive(Default)]
struct KintState {
    krctl: u8,
    krf: u8,
    krm: u8,
    previous_inputs: u8,
}

/// Host-facing RA4M1 key interrupt state.
#[derive(Clone)]
pub struct RaKintHandle(Arc<Mutex<KintState>>);

impl RaKintHandle {
    /// Samples KR00..KR07 and reports the KEY_INTKR request level.
    pub fn poll(&self, inputs: u8) -> bool {
        let mut state = self.0.lock().expect("RA KINT lock poisoned");
        let active_level = if state.krctl & 1 != 0 {
            inputs & state.krm
        } else {
            (!inputs) & state.krm
        };
        let valid_edges = if state.krctl & 1 != 0 {
            (!state.previous_inputs) & inputs
        } else {
            state.previous_inputs & (!inputs)
        };
        if state.krctl & 0x80 != 0 {
            state.krf |= valid_edges & state.krm;
        }
        state.previous_inputs = inputs;
        if state.krctl & 0x80 != 0 {
            state.krf & state.krm != 0
        } else {
            active_level != 0
        }
    }

    /// Returns the latched per-channel key flags.
    pub fn flags(&self) -> u8 {
        self.0.lock().expect("RA KINT lock poisoned").krf
    }
}

/// Functional RA4M1 key interrupt register and edge-detection slice.
pub struct RaKint {
    name: String,
    state: Arc<Mutex<KintState>>,
}

impl RaKint {
    /// Creates KINT and its host-facing input sampler.
    pub fn new(name: impl Into<String>) -> (Self, RaKintHandle) {
        let state = Arc::new(Mutex::new(KintState::default()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            RaKintHandle(state),
        )
    }
}

impl Device for RaKint {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Byte {
            return Err(DeviceError::new("RA KINT requires byte accesses"));
        }
        let state = self.state.lock().expect("RA KINT lock poisoned");
        let value = match RaKintRegister::from_offset(offset) {
            Some(RaKintRegister::Krctl) => state.krctl,
            Some(RaKintRegister::Krf) => state.krf,
            Some(RaKintRegister::Krm) => state.krm,
            None => 0,
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
        if width != AccessWidth::Byte {
            return Err(DeviceError::new("RA KINT requires byte accesses"));
        }
        let mut state = self.state.lock().expect("RA KINT lock poisoned");
        match RaKintRegister::from_offset(offset) {
            Some(RaKintRegister::Krctl) => state.krctl = (value as u8) & 0x81,
            Some(RaKintRegister::Krf) => state.krf &= value as u8,
            Some(RaKintRegister::Krm) => state.krm = value as u8,
            None => {}
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("RA KINT lock poisoned") = KintState::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_selected_edges_and_clears_latched_flags() {
        let (mut kint, handle) = RaKint::new("kint");
        kint.write(0, AccessWidth::Byte, 0x81, SimTime::ZERO)
            .unwrap();
        kint.write(8, AccessWidth::Byte, 1, SimTime::ZERO).unwrap();
        assert!(!handle.poll(0));
        assert!(handle.poll(1));
        assert_eq!(handle.flags(), 1);
        kint.write(4, AccessWidth::Byte, 0xfe, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.flags(), 0);
        assert!(!handle.poll(1));
    }
}
