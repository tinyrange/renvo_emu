//! Machine-facing UART endpoint discovery.

use remu_devices::UartHandle;
use serde::Serialize;

/// Stable role of a UART exposed by a machine model.
///
/// `Compiler` is Renvo's test-only UART facade. `Native` is the first
/// memory-mapped UART implemented by the selected chip model. The role names
/// intentionally avoid exposing target-specific register addresses so board
/// composition can bind to a transport without knowing the CPU architecture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum UartEndpointId {
    /// Renvo's compiler-test UART mapped at [`crate::TEST_UART`].
    Compiler,
    /// The primary native UART in the target memory map.
    Native,
}

/// A host-facing UART endpoint discovered from a machine.
#[derive(Clone)]
pub struct UartEndpoint {
    id: UartEndpointId,
    handle: UartHandle,
}

impl UartEndpoint {
    pub(crate) fn new(id: UartEndpointId, handle: UartHandle) -> Self {
        Self { id, handle }
    }

    /// Returns the stable role used to bind this endpoint to a board.
    pub const fn id(&self) -> UartEndpointId {
        self.id
    }

    /// Returns a clone of the shared host transport handle.
    pub fn handle(&self) -> UartHandle {
        self.handle.clone()
    }
}

/// Provides typed UART endpoints without exposing machine internals.
pub trait UartEndpointProvider {
    /// Returns the compiler facade followed by the primary native UART when
    /// the selected machine implements one.
    fn uart_endpoints(&self) -> Vec<UartEndpoint>;
}

impl UartEndpointProvider for crate::RiscVMachine {
    fn uart_endpoints(&self) -> Vec<UartEndpoint> {
        let mut endpoints = vec![UartEndpoint::new(
            UartEndpointId::Compiler,
            self.uart.clone(),
        )];
        if let Some(native) = self.chip_uarts.first() {
            endpoints.push(UartEndpoint::new(UartEndpointId::Native, native.clone()));
        }
        endpoints
    }
}

impl UartEndpointProvider for crate::ArmMachine {
    fn uart_endpoints(&self) -> Vec<UartEndpoint> {
        vec![
            UartEndpoint::new(UartEndpointId::Compiler, self.uart.clone()),
            UartEndpoint::new(UartEndpointId::Native, self.chip_uart.clone()),
        ]
    }
}

impl UartEndpointProvider for crate::XtensaMachine {
    fn uart_endpoints(&self) -> Vec<UartEndpoint> {
        vec![
            UartEndpoint::new(UartEndpointId::Compiler, self.uart.clone()),
            UartEndpoint::new(UartEndpointId::Native, self.chip_uart.clone()),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ArmMachine, RiscVMachine, TargetId, XtensaMachine};

    fn ids(endpoints: &[UartEndpoint]) -> Vec<UartEndpointId> {
        endpoints.iter().map(UartEndpoint::id).collect()
    }

    #[test]
    fn endpoint_roles_are_stable_across_uart_handle_backed_machines() {
        for target in [
            TargetId::Ch32v003,
            TargetId::Ch32v006,
            TargetId::Esp32c6,
            TargetId::Rp2350,
        ] {
            let riscv = RiscVMachine::new(target).expect("RISC-V machine");
            assert_eq!(
                ids(&riscv.uart_endpoints()),
                vec![UartEndpointId::Compiler, UartEndpointId::Native]
            );
        }

        let arm = ArmMachine::new(TargetId::Rp2040).expect("RP2040 machine");
        assert_eq!(
            ids(&arm.uart_endpoints()),
            vec![UartEndpointId::Compiler, UartEndpointId::Native]
        );

        let xtensa = XtensaMachine::new(TargetId::Esp32s3).expect("ESP32-S3 machine");
        assert_eq!(
            ids(&xtensa.uart_endpoints()),
            vec![UartEndpointId::Compiler, UartEndpointId::Native]
        );
    }

    #[test]
    fn endpoint_handles_share_machine_transport_state() {
        let machine = RiscVMachine::new(TargetId::Ch32v003).expect("CH32V003 machine");
        let endpoint = machine
            .uart_endpoints()
            .into_iter()
            .find(|endpoint| endpoint.id() == UartEndpointId::Native)
            .expect("native UART endpoint");

        endpoint.handle().transmit(b"ok");
        assert_eq!(endpoint.handle().bytes(), b"ok");
        assert_eq!(machine.chip_uarts[0].bytes(), b"ok");
    }
}
