//! Reusable functional microcontroller peripheral models.

use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use remu_signals::{
    DigitalNet, DriverId, Logic, SignalChange, SignalError, SignalId, SignalRegistry, SignalValue,
};
use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

mod signals;
pub use signals::*;
mod uart;
pub use uart::*;
mod wch;
pub use wch::*;
mod esp_usb_serial_jtag;
pub use esp_usb_serial_jtag::*;
mod gpio;
pub use gpio::*;
mod rp;
pub use rp::*;
mod esp;
pub use esp::*;
mod arm;
pub use arm::*;
mod esp_gpio;
pub use esp_gpio::*;
mod functional;
pub use functional::*;
mod samd;
pub use samd::*;
mod stm32;
pub use stm32::*;
mod stm32_swpmi;
pub use stm32_swpmi::*;
mod ra;
pub use ra::*;
mod avr;
pub use avr::*;
mod msp430;
pub use msp430::*;
mod pic16;
pub use pic16::*;
mod efm8;
pub use efm8::*;
mod board_components;
pub use board_components::*;

#[cfg(test)]
mod tests;
