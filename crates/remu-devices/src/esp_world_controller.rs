use super::{AccessWidth, Device, DeviceError, Rc, RefCell, ResetKind, SimTime};
use crate::Esp32S3World;
use remu_core::AccessKind;

const CORE_STRIDE: u64 = 0x400;
const REGISTER_WORDS: usize = 0x598 / 4;

#[derive(Clone, Copy)]
struct CoreState {
    instruction_world: Esp32S3World,
    data_world: Esp32S3World,
    world_switch_armed: bool,
    nmi_temporary_mask: bool,
    nmi_disable_armed: bool,
    last_execute_address: Option<u32>,
}

impl CoreState {
    const fn new() -> Self {
        Self {
            instruction_world: Esp32S3World::Secure,
            data_world: Esp32S3World::Secure,
            world_switch_armed: false,
            nmi_temporary_mask: false,
            nmi_disable_armed: false,
            last_execute_address: None,
        }
    }
}

struct Esp32S3WorldControllerState {
    registers: [u32; REGISTER_WORDS],
    cores: [CoreState; 2],
}

impl Esp32S3WorldControllerState {
    fn new() -> Self {
        let mut registers = [0; REGISTER_WORDS];
        registers[index(0x07c)] = 2;
        registers[index(0x47c)] = 2;
        Self {
            registers,
            cores: [CoreState::new(); 2],
        }
    }

    fn register(&self, offset: u64) -> u32 {
        self.registers[index(offset)]
    }

    fn set_register(&mut self, offset: u64, value: u32) {
        self.registers[index(offset)] = value;
    }

    fn set_instruction_world(&mut self, core: usize, world: Esp32S3World) {
        self.cores[core].instruction_world = world;
        self.set_register(core_offset(core, 0x150), world_register_value(world));
    }

    fn set_data_world(&mut self, core: usize, world: Esp32S3World) {
        self.cores[core].data_world = world;
        self.set_register(core_offset(core, 0x154), world_register_value(world));
    }

    fn update_current_alias(&mut self, core: usize) {
        let mut current = 0;
        for entry in 0..13_u64 {
            if self.register(core_offset(core, 0x080 + entry * 4)) & (1 << 5) != 0 {
                current |= 1 << (entry + 1);
            }
        }
        self.set_register(core_offset(core, 0x0fc), current);
    }

    fn write_current_alias(&mut self, core: usize, value: u32) {
        for entry in 0..13_u64 {
            let offset = core_offset(core, 0x080 + entry * 4);
            let old = self.register(offset);
            let current = ((value >> (entry + 1)) & 1) << 5;
            self.set_register(offset, (old & !(1 << 5)) | current);
        }
        self.set_register(core_offset(core, 0x0fc), value & 0x3ffe);
    }

    fn current_entry(&self, core: usize) -> u32 {
        (0..13_u64)
            .find(|entry| self.register(core_offset(core, 0x080 + entry * 4)) & (1 << 5) != 0)
            .map_or(0, |entry| {
                u32::try_from(entry + 1).expect("entry is at most 13")
            })
    }

    fn enter_secure_entry(&mut self, core: usize, entry: u32) {
        let previous_entry = self.current_entry(core);
        let from_world = u32::from(matches!(
            self.cores[core].instruction_world,
            Esp32S3World::NonSecure
        ));
        for other in 0..13_u64 {
            let offset = core_offset(core, 0x080 + other * 4);
            self.set_register(offset, self.register(offset) & !(1 << 5));
        }
        let offset = core_offset(core, 0x080 + u64::from(entry - 1) * 4);
        self.set_register(offset, (1 << 5) | (previous_entry << 1) | from_world);
        self.update_current_alias(core);
        self.set_instruction_world(core, Esp32S3World::Secure);
    }
}

/// Host-facing view of ESP32-S3 CPU world and NMI-mask state.
#[derive(Clone)]
pub struct Esp32S3WorldControllerHandle {
    state: Rc<RefCell<Esp32S3WorldControllerState>>,
}

impl Esp32S3WorldControllerHandle {
    /// Observes an instruction fetch and applies configured address-triggered transitions.
    pub fn observe_execute(&self, core: u8, address: u32) {
        let core = usize::from(core.min(1));
        let mut state = self.state.borrow_mut();
        if state.cores[core].last_execute_address == Some(address) {
            return;
        }
        state.cores[core].last_execute_address = Some(address);
        let base = core_offset(core, 0);
        if state.cores[core].nmi_temporary_mask
            && state.cores[core].nmi_disable_armed
            && state.register(base + 0x184) == address
        {
            state.cores[core].nmi_temporary_mask = false;
            state.cores[core].nmi_disable_armed = false;
            state.set_register(base + 0x194, 0);
        }
        if state.cores[core].world_switch_armed
            && state.cores[core].instruction_world == Esp32S3World::Secure
            && state.register(base + 0x140) == address
        {
            state.cores[core].world_switch_armed = false;
            state.set_register(base + 0x158, 0);
            state.set_instruction_world(core, Esp32S3World::NonSecure);
            state.set_data_world(core, Esp32S3World::NonSecure);
            return;
        }
        let enabled = state.register(base + 0x07c) >> 1;
        if let Some(entry) = (1..=13_u32).find(|entry| {
            enabled & (1 << (entry - 1)) != 0
                && state.register(base + u64::from(entry - 1) * 4) == address
        }) {
            state.enter_secure_entry(core, entry);
        }
    }

    /// Observes a completed CPU data write for the write-buffer clearing sequence.
    pub fn observe_write(&self, core: u8, address: u32, width: AccessWidth, value: u64) {
        if width != AccessWidth::Word {
            return;
        }
        let core = usize::from(core.min(1));
        let mut state = self.state.borrow_mut();
        let base = core_offset(core, 0);
        if state.register(base + 0x100) != address {
            return;
        }
        let Some(value) = u32::try_from(value).ok() else {
            return;
        };
        let expected = (state.register(base + 0x108) >> 1) & 0xf;
        let maximum = state.register(base + 0x104) & 0xf;
        if value != expected {
            state.set_register(base + 0x108, 0);
        } else if expected == maximum {
            state.set_register(base + 0x108, 1);
            state.set_data_world(core, Esp32S3World::Secure);
        } else {
            state.set_register(base + 0x108, (1 << 5) | ((expected + 1) << 1));
        }
    }

    /// Returns the world attached to the bus used by an access.
    pub fn world_for_access(&self, core: u8, access: AccessKind) -> Esp32S3World {
        let state = self.state.borrow();
        let core = usize::from(core.min(1));
        if access == AccessKind::Execute {
            state.cores[core].instruction_world
        } else {
            state.cores[core].data_world
        }
    }

    /// Returns whether the selected CPU's NMI input is currently masked.
    pub fn nmi_masked(&self, core: u8) -> bool {
        let state = self.state.borrow();
        let core = usize::from(core.min(1));
        state.register(core_offset(core, 0x190)) & 1 != 0 || state.cores[core].nmi_temporary_mask
    }
}

/// Functional ESP32-S3 World Controller.
pub struct Esp32S3WorldController {
    name: String,
    state: Rc<RefCell<Esp32S3WorldControllerState>>,
}

impl Esp32S3WorldController {
    /// Creates reset world-controller state and its machine-facing handle.
    pub fn new(name: impl Into<String>) -> (Self, Esp32S3WorldControllerHandle) {
        let state = Rc::new(RefCell::new(Esp32S3WorldControllerState::new()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Esp32S3WorldControllerHandle { state },
        )
    }
}

impl Device for Esp32S3WorldController {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        validate_access(&self.name, offset, width)?;
        Ok(u64::from(
            self.state.borrow().register(offset) & read_mask(offset),
        ))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        validate_access(&self.name, offset, width)?;
        let value = u32::try_from(value)
            .map_err(|_| DeviceError::new(format!("{} word write exceeds 32 bits", self.name)))?;
        let (core, local) = decode_offset(offset).expect("validated WCL offset decodes");
        let base = core_offset(core, 0);
        let mut state = self.state.borrow_mut();
        let old = state.register(offset);
        let value = (old & !write_mask(offset)) | (value & write_mask(offset));
        match local {
            0x080..=0x0b0 => {
                state.set_register(offset, value);
                state.update_current_alias(core);
            }
            0x0fc => state.write_current_alias(core, value),
            0x100 | 0x104 => {
                state.set_register(offset, value);
                state.set_register(base + 0x108, 0);
            }
            0x148 => {
                let armed = state.register(base + 0x144) == 2;
                state.cores[core].world_switch_armed = armed;
                state.set_register(base + 0x158, u32::from(armed));
            }
            0x14c => {
                state.cores[core].world_switch_armed = false;
                state.set_register(base + 0x158, 0);
            }
            0x180 => {
                state.cores[core].nmi_temporary_mask = true;
                state.set_register(base + 0x194, 1);
            }
            0x188 => state.cores[core].nmi_disable_armed = true,
            0x18c => state.cores[core].nmi_disable_armed = false,
            0x190 => {
                state.set_register(offset, value);
                let masked = value & 1 != 0 || state.cores[core].nmi_temporary_mask;
                state.set_register(base + 0x194, u32::from(masked));
            }
            _ => state.set_register(offset, value),
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.borrow_mut() = Esp32S3WorldControllerState::new();
    }
}

const fn core_offset(core: usize, local: u64) -> u64 {
    local + core as u64 * CORE_STRIDE
}

const fn world_register_value(world: Esp32S3World) -> u32 {
    match world {
        Esp32S3World::Secure => 1,
        Esp32S3World::NonSecure => 2,
    }
}

const fn index(offset: u64) -> usize {
    (offset / 4) as usize
}

fn decode_offset(offset: u64) -> Option<(usize, u64)> {
    let (core, local) = if offset >= CORE_STRIDE {
        (1, offset.checked_sub(CORE_STRIDE)?)
    } else {
        (0, offset)
    };
    documented_local_offset(local).then_some((core, local))
}

fn documented_local_offset(offset: u64) -> bool {
    matches!(
        offset,
        0x000..=0x030
            | 0x07c..=0x0b0
            | 0x0fc..=0x108
            | 0x140..=0x158
            | 0x180..=0x194
    ) && offset.is_multiple_of(4)
}

fn validate_access(name: &str, offset: u64, width: AccessWidth) -> Result<(), DeviceError> {
    if width != AccessWidth::Word || decode_offset(offset).is_none() {
        return Err(DeviceError::new(format!(
            "{name} requires aligned word access to a documented WCL register at {offset:#x}"
        )));
    }
    Ok(())
}

fn read_mask(offset: u64) -> u32 {
    let (_, local) = decode_offset(offset).expect("documented WCL offset");
    match local {
        0x07c | 0x0fc => 0x3ffe,
        0x080..=0x0b0 => 0x3f,
        0x104 => 0xf,
        0x108 => 0x7f,
        0x144 | 0x150 | 0x154 => 3,
        0x148 | 0x14c | 0x180 | 0x188 | 0x18c => 0,
        0x158 | 0x190 | 0x194 => 1,
        _ => u32::MAX,
    }
}

fn write_mask(offset: u64) -> u32 {
    let (_, local) = decode_offset(offset).expect("documented WCL offset");
    match local {
        0x07c | 0x0fc => 0x3ffe,
        0x080..=0x0b0 => 0x3f,
        0x104 => 0xf,
        0x108 | 0x158 | 0x194 => 0,
        0x144 | 0x150 | 0x154 => 3,
        0x190 => 1,
        _ => u32::MAX,
    }
}

#[cfg(test)]
fn reset_value(offset: u64) -> u32 {
    let (_, local) = decode_offset(offset).expect("documented WCL offset");
    u32::from(local == 0x07c) << 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(device: &mut Esp32S3WorldController, offset: u64) -> u32 {
        device
            .read(offset, AccessWidth::Word, SimTime::ZERO)
            .unwrap() as u32
    }

    fn write(device: &mut Esp32S3WorldController, offset: u64, value: u32) {
        device
            .write(offset, AccessWidth::Word, u64::from(value), SimTime::ZERO)
            .unwrap();
    }

    #[test]
    fn all_88_vendor_registers_have_exact_access_masks_and_resets() {
        let (mut device, _) = Esp32S3WorldController::new("wcl");
        let mut count = 0;
        for offset in (0..0x1000).step_by(4) {
            if decode_offset(offset).is_some() {
                count += 1;
                assert_eq!(read(&mut device, offset), reset_value(offset));
                let (mut isolated, _) = Esp32S3WorldController::new("wcl");
                write(&mut isolated, offset, u32::MAX);
                assert_eq!(
                    read(&mut isolated, offset),
                    write_mask(offset) & read_mask(offset)
                );
            } else {
                assert!(
                    device
                        .read(offset, AccessWidth::Word, SimTime::ZERO)
                        .is_err()
                );
            }
        }
        assert_eq!(count, 88);
        assert!(device.read(0, AccessWidth::Byte, SimTime::ZERO).is_err());
    }

    #[test]
    fn secure_to_nonsecure_switch_is_one_shot_and_cancelable() {
        let (mut device, handle) = Esp32S3WorldController::new("wcl");
        write(&mut device, 0x140, 0x4037_1000);
        write(&mut device, 0x144, 2);
        write(&mut device, 0x148, 0);
        assert_eq!(read(&mut device, 0x158), 1);
        handle.observe_execute(0, 0x4037_1000);
        assert_eq!(
            handle.world_for_access(0, AccessKind::Execute),
            Esp32S3World::NonSecure
        );
        assert_eq!(
            handle.world_for_access(0, AccessKind::Read),
            Esp32S3World::NonSecure
        );
        assert_eq!(read(&mut device, 0x150), 2);
        assert_eq!(read(&mut device, 0x154), 2);
        assert_eq!(read(&mut device, 0x158), 0);

        device.reset(ResetKind::Software);
        write(&mut device, 0x140, 0x4037_1000);
        write(&mut device, 0x144, 2);
        write(&mut device, 0x148, 0);
        write(&mut device, 0x14c, 0);
        handle.observe_execute(0, 0x4037_1000);
        assert_eq!(
            handle.world_for_access(0, AccessKind::Execute),
            Esp32S3World::Secure
        );
    }

    #[test]
    fn message_sequence_and_entry_restore_independent_data_and_instruction_worlds() {
        let (mut device, handle) = Esp32S3WorldController::new("wcl");
        write(&mut device, 0x000, 0x4037_2000);
        write(&mut device, 0x07c, 2);
        write(&mut device, 0x100, 0x3fc8_9000);
        write(&mut device, 0x104, 3);
        write(&mut device, 0x140, 0x4037_1000);
        write(&mut device, 0x144, 2);
        write(&mut device, 0x148, 0);
        handle.observe_execute(0, 0x4037_1000);

        for value in 0..=3 {
            handle.observe_write(0, 0x3fc8_9000, AccessWidth::Word, value);
        }
        assert_eq!(read(&mut device, 0x108), 1);
        assert_eq!(
            handle.world_for_access(0, AccessKind::Read),
            Esp32S3World::Secure
        );
        assert_eq!(
            handle.world_for_access(0, AccessKind::Execute),
            Esp32S3World::NonSecure
        );
        handle.observe_execute(0, 0x4037_2000);
        handle.observe_execute(0, 0x4037_2000);
        assert_eq!(
            handle.world_for_access(0, AccessKind::Execute),
            Esp32S3World::Secure
        );
        assert_eq!(read(&mut device, 0x080), 0x21);
        assert_eq!(read(&mut device, 0x0fc), 2);
    }

    #[test]
    fn direct_and_address_terminated_nmi_masks_are_reported() {
        let (mut device, handle) = Esp32S3WorldController::new("wcl");
        write(&mut device, 0x190, 1);
        assert!(handle.nmi_masked(0));
        assert_eq!(read(&mut device, 0x194), 1);
        write(&mut device, 0x190, 0);
        write(&mut device, 0x184, 0x4037_3000);
        write(&mut device, 0x188, 0);
        write(&mut device, 0x180, 0);
        assert!(handle.nmi_masked(0));
        handle.observe_execute(0, 0x4037_3000);
        assert!(!handle.nmi_masked(0));
        assert_eq!(read(&mut device, 0x194), 0);
    }
}
