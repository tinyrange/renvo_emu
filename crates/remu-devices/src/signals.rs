use super::*;

/// Shared signal registry and append-only pending-change stream.
#[derive(Clone, Default)]
pub struct SignalHub {
    inner: Rc<RefCell<SignalHubState>>,
}

#[derive(Default)]
struct SignalHubState {
    registry: SignalRegistry,
    changes: Vec<SignalChange>,
}

impl SignalHub {
    /// Creates an empty signal hub.
    pub fn new() -> Self {
        Self::default()
    }

    /// Declares a signal.
    pub fn declare(
        &self,
        path: impl Into<String>,
        initial: SignalValue,
        description: Option<String>,
    ) -> Result<SignalId, SignalError> {
        self.inner
            .borrow_mut()
            .registry
            .declare(path, initial, description)
    }

    /// Sets a value and queues a real transition.
    pub fn set(
        &self,
        signal: SignalId,
        value: SignalValue,
        at: SimTime,
    ) -> Result<(), SignalError> {
        let mut state = self.inner.borrow_mut();
        if let Some(change) = state.registry.set(signal, value, at)? {
            state.changes.push(change);
        }
        Ok(())
    }

    /// Runs a read-only operation against the registry.
    pub fn with_registry<T>(&self, operation: impl FnOnce(&SignalRegistry) -> T) -> T {
        let state = self.inner.borrow();
        operation(&state.registry)
    }

    /// Returns whether any signal changes are waiting to be consumed.
    pub fn has_changes(&self) -> bool {
        !self.inner.borrow().changes.is_empty()
    }

    /// Removes all pending changes in chronological insertion order.
    pub fn drain_changes(&self) -> Vec<SignalChange> {
        let mut state = self.inner.borrow_mut();
        core::mem::take(&mut state.changes)
    }
}
