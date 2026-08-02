use super::{GpioHandle, GpioState, SignalHub, refresh_gpio, vendor_gpio};
use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use remu_signals::{Logic, SignalId, SignalValue};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

const DATA_BYTES: usize = 0x2000;
const INTCON: usize = 0x00b;
const PORT_BASE: usize = 0x00c;
const TRIS_BASE: usize = 0x012;
const LAT_BASE: usize = 0x018;
const RC1REG: usize = 0x119;
const TX1REG: usize = 0x11a;
const RC1STA: usize = 0x11d;
const TX1STA: usize = 0x11e;
/// Native PIC16F15376 MSSP1 register identifiers.
///
/// The enum keeps the peripheral register window typed at the API boundary;
/// callers should not have to carry unlabelled data-space offsets when
/// inspecting or driving the functional I²C host model.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(usize)]
pub enum Pic16Mssp1Register {
    /// SSP1BUF synchronous serial input/output buffer.
    Buffer = 0x18c,
    /// SSP1ADD baud divider/address register.
    Address = 0x18d,
    /// SSP1MSK address-mask register.
    Mask = 0x18e,
    /// SSP1STAT status register.
    Status = 0x18f,
    /// SSP1CON1 mode and enable control.
    Control1 = 0x190,
    /// SSP1CON2 I²C command and acknowledge control.
    Control2 = 0x191,
    /// SSP1CON3 I²C auxiliary control.
    Control3 = 0x192,
}

impl Pic16Mssp1Register {
    /// Native banked data-space offset for this MSSP1 register.
    pub const fn offset(self) -> usize {
        self as usize
    }

    /// Converts a native data-space offset into a typed MSSP1 identifier.
    pub const fn from_offset(offset: usize) -> Option<Self> {
        match offset {
            0x18c => Some(Self::Buffer),
            0x18d => Some(Self::Address),
            0x18e => Some(Self::Mask),
            0x18f => Some(Self::Status),
            0x190 => Some(Self::Control1),
            0x191 => Some(Self::Control2),
            0x192 => Some(Self::Control3),
            _ => None,
        }
    }
}

// Keep the internal register-array indexing readable while deriving every
// offset from the named enum above rather than duplicating numeric IDs.
const SSP1BUF: usize = Pic16Mssp1Register::Buffer.offset();
const SSP1ADD: usize = Pic16Mssp1Register::Address.offset();
const SSP1MSK: usize = Pic16Mssp1Register::Mask.offset();
const SSP1STAT: usize = Pic16Mssp1Register::Status.offset();
const SSP1CON1: usize = Pic16Mssp1Register::Control1.offset();
const SSP1CON2: usize = Pic16Mssp1Register::Control2.offset();
const SSP1CON3: usize = Pic16Mssp1Register::Control3.offset();
const TMR1L: usize = 0x20c;
const TMR1H: usize = 0x20d;
const T1CON: usize = 0x20e;
const TMR0L: usize = 0x59c;
const TMR0H: usize = 0x59d;
const T0CON0: usize = 0x59e;
const PIR0: usize = 0x70c;
const PIR1: usize = 0x70d;
const PIR3: usize = 0x70f;
const PIE0: usize = 0x716;
const PIE1: usize = 0x717;
const PIE3: usize = 0x719;
const WDTCON0: usize = 0x80c;
const OSCSTAT: usize = 0x890;
const ANSEL: [usize; 5] = [0x1f38, 0x1f43, 0x1f4e, 0x1f59, 0x1f64];
const ADRESL: usize = 0x09b;
const ADRESH: usize = 0x09c;
const ADCON0: usize = 0x09d;
const ADCON1: usize = 0x09e;

const PORT_WIDTHS: [u8; 5] = [8, 8, 8, 8, 4];
const PORT_MASKS: [u8; 5] = [0xff, 0xff, 0xff, 0xff, 0x0f];
const INTCON_GIE: u8 = 1 << 7;
const INTCON_PEIE: u8 = 1 << 6;
const TMR0IF: u8 = 1 << 5;
const TMR1IF: u8 = 1;
const TMR2IF: u8 = 1 << 1;
const TX1IF: u8 = 1 << 4;
const RC1IF: u8 = 1 << 5;
const SSP1IF: u8 = 1;
const SSP1IE: u8 = 1;
const ADIF: u8 = 1 << 6;
const ADIE: u8 = 1 << 6;
const ADCON0_GO: u8 = 1 << 1;
const ADCON0_ADON: u8 = 1;
const TXEN: u8 = 1 << 5;
const SPEN: u8 = 1 << 7;
const NCO1IF: u8 = 1 << 4;
const NCO1IE: u8 = 1 << 4;
const NCO1EN: u8 = 1 << 7;
const NCO1OUT: u8 = 1 << 5;
const NCO1POL: u8 = 1 << 4;
const NCO1PFM: u8 = 1;
const NCO_ACC_MASK: u32 = 0x0f_ffff;
const SSP1STAT_BF: u8 = 1;
const SSP1CON1_WCOL: u8 = 1 << 7;
const SSP1CON1_SSPEN: u8 = 1 << 5;
const T2ON: u8 = 1 << 7;
const T2CKPS_MASK: u8 = 0x70;
const T2OUTPS_MASK: u8 = 0x0f;

/// Named PIC16F15376 Timer2 and interrupt register identifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[repr(usize)]
pub enum Pic16Timer2Register {
    /// Timer2 count register (T2TMR).
    T2Tmr = 0x28c,
    /// Timer2 period register (T2PR).
    T2Pr = 0x28d,
    /// Timer2 control register (T2CON).
    T2Con = 0x28e,
    /// Timer2 clock selection register (T2CLKCON).
    T2ClkCon = 0x290,
    /// Peripheral interrupt request register 4 (PIR4).
    Pir4 = 0x710,
    /// Peripheral interrupt enable register 4 (PIE4).
    Pie4 = 0x71a,
}

impl Pic16Timer2Register {
    /// Stable list of modeled Timer2-related register IDs.
    pub const ALL: [Self; 6] = [
        Self::T2Tmr,
        Self::T2Pr,
        Self::T2Con,
        Self::T2ClkCon,
        Self::Pir4,
        Self::Pie4,
    ];

    /// Returns the native PIC16 data-space address.
    pub const fn offset(self) -> usize {
        self as usize
    }

    /// Returns the register-array index used by `Pic16Peripherals`.
    pub const fn index(self) -> usize {
        self.offset()
    }

    /// Returns the vendor register name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::T2Tmr => "t2tmr",
            Self::T2Pr => "t2pr",
            Self::T2Con => "t2con",
            Self::T2ClkCon => "t2clkcon",
            Self::Pir4 => "pir4",
            Self::Pie4 => "pie4",
        }
    }

    /// Resolves a native data-space address to a named Timer2-related register.
    pub const fn from_data_address(address: usize) -> Option<Self> {
        match address {
            0x28c => Some(Self::T2Tmr),
            0x28d => Some(Self::T2Pr),
            0x28e => Some(Self::T2Con),
            0x290 => Some(Self::T2ClkCon),
            0x710 => Some(Self::Pir4),
            0x71a => Some(Self::Pie4),
            _ => None,
        }
    }
}
const DAC1EN: u8 = 1 << 7;
// DAC1NSS (bit 0) reads as zero on this device and is not writable.
const DAC1CON0_MASK: u8 = 0xbc;
const DAC1R_MASK: u8 = 0x1f;

/// PIC16F15376 DAC1 register identifiers.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
#[repr(usize)]
pub enum Pic16DacRegister {
    /// DAC1 enable, output-enable and source-selection control.
    Dac1Con0 = 0x90e,
    /// DAC1 five-bit code register.
    Dac1Con1 = 0x90f,
}

impl Pic16DacRegister {
    /// All registers modelled by this peripheral slice, in address order.
    pub const ALL: [Self; 2] = [Self::Dac1Con0, Self::Dac1Con1];

    /// Data-space address of this register.
    pub const fn offset(self) -> usize {
        self as usize
    }

    /// Backing register-array index for this register.
    pub const fn index(self) -> usize {
        self.offset()
    }

    /// Lowercase datasheet register name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Dac1Con0 => "dac1con0",
            Self::Dac1Con1 => "dac1con1",
        }
    }

    /// Converts a data-space address into a known DAC1 register.
    pub const fn from_data_address(address: usize) -> Option<Self> {
        match address {
            0x90e => Some(Self::Dac1Con0),
            0x90f => Some(Self::Dac1Con1),
            _ => None,
        }
    }
}
const C1IF: u8 = 1;
const CM1CON0_OUT: u8 = 1 << 6;
const CMOUT_C1OUT: u8 = 1;
const C1ON: u8 = 1 << 7;
const C1POL: u8 = 1 << 4;
const CM1CON0_WRITE_MASK: u8 = C1ON | C1POL | (1 << 1) | 1;
const CM1CON1_MASK: u8 = 0x03;
const CM1_CHANNEL_MASK: u8 = 0x07;

/// PIC16F15376 Comparator C1 and interrupt register identifiers.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
#[repr(usize)]
pub enum Pic16ComparatorRegister {
    /// Comparator interrupt flag register (C1IF is bit 0).
    Pir2 = 0x70e,
    /// Comparator interrupt enable register (C1IE is bit 0).
    Pie2 = 0x718,
    /// Read-only mirror of comparator outputs.
    Cmout = 0x98f,
    /// Comparator C1 enable, output, polarity, hysteresis and sync control.
    Cm1Con0 = 0x990,
    /// Comparator C1 edge interrupt enables.
    Cm1Con1 = 0x991,
    /// Comparator C1 negative input selection.
    Cm1Nch = 0x992,
    /// Comparator C1 positive input selection.
    Cm1Pch = 0x993,
}

impl Pic16ComparatorRegister {
    /// All comparator-related registers modelled by this peripheral slice.
    pub const ALL: [Self; 7] = [
        Self::Pir2,
        Self::Pie2,
        Self::Cmout,
        Self::Cm1Con0,
        Self::Cm1Con1,
        Self::Cm1Nch,
        Self::Cm1Pch,
    ];

    /// Data-space address of this register.
    pub const fn offset(self) -> usize {
        self as usize
    }

    /// Backing register-array index for this register.
    pub const fn index(self) -> usize {
        self.offset()
    }

    /// Lowercase datasheet register name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Pir2 => "pir2",
            Self::Pie2 => "pie2",
            Self::Cmout => "cmout",
            Self::Cm1Con0 => "cm1con0",
            Self::Cm1Con1 => "cm1con1",
            Self::Cm1Nch => "cm1nch",
            Self::Cm1Pch => "cm1pch",
        }
    }

    /// Converts a data-space address into a known comparator register.
    pub const fn from_data_address(address: usize) -> Option<Self> {
        match address {
            0x70e => Some(Self::Pir2),
            0x718 => Some(Self::Pie2),
            0x98f => Some(Self::Cmout),
            0x990 => Some(Self::Cm1Con0),
            0x991 => Some(Self::Cm1Con1),
            0x992 => Some(Self::Cm1Nch),
            0x993 => Some(Self::Cm1Pch),
            _ => None,
        }
    }
}
const PPS_OUTPUT_TX1: u8 = 0x0f;
const PPS_OUTPUT_TMR0: u8 = 0x19;
const PPS_OUTPUT_MASK: u8 = 0x1f;
const PPSLOCKED: u8 = 1;

/// Named PIC16F15376 PPS registers used by the functional output-routing model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
#[allow(missing_docs)]
pub enum Pic16PpsRegister {
    /// PPS lock state register.
    Ppslock,
    /// PORTA PPS output registers.
    Ra0Pps,
    Ra1Pps,
    Ra2Pps,
    Ra3Pps,
    Ra4Pps,
    Ra5Pps,
    Ra6Pps,
    Ra7Pps,
    /// PORTB PPS output registers.
    Rb0Pps,
    Rb1Pps,
    Rb2Pps,
    Rb3Pps,
    Rb4Pps,
    Rb5Pps,
    Rb6Pps,
    Rb7Pps,
    /// PORTC PPS output registers.
    Rc0Pps,
    Rc1Pps,
    Rc2Pps,
    Rc3Pps,
    Rc4Pps,
    Rc5Pps,
    Rc6Pps,
    Rc7Pps,
    /// PORTD PPS output registers.
    Rd0Pps,
    Rd1Pps,
    Rd2Pps,
    Rd3Pps,
    Rd4Pps,
    Rd5Pps,
    Rd6Pps,
    Rd7Pps,
    /// PORTE PPS output registers.
    Re0Pps,
    Re1Pps,
    Re2Pps,
    Re3Pps,
}

impl Pic16PpsRegister {
    /// Stable register order.
    pub const ALL: [Self; 37] = [
        Self::Ppslock,
        Self::Ra0Pps,
        Self::Ra1Pps,
        Self::Ra2Pps,
        Self::Ra3Pps,
        Self::Ra4Pps,
        Self::Ra5Pps,
        Self::Ra6Pps,
        Self::Ra7Pps,
        Self::Rb0Pps,
        Self::Rb1Pps,
        Self::Rb2Pps,
        Self::Rb3Pps,
        Self::Rb4Pps,
        Self::Rb5Pps,
        Self::Rb6Pps,
        Self::Rb7Pps,
        Self::Rc0Pps,
        Self::Rc1Pps,
        Self::Rc2Pps,
        Self::Rc3Pps,
        Self::Rc4Pps,
        Self::Rc5Pps,
        Self::Rc6Pps,
        Self::Rc7Pps,
        Self::Rd0Pps,
        Self::Rd1Pps,
        Self::Rd2Pps,
        Self::Rd3Pps,
        Self::Rd4Pps,
        Self::Rd5Pps,
        Self::Rd6Pps,
        Self::Rd7Pps,
        Self::Re0Pps,
        Self::Re1Pps,
        Self::Re2Pps,
        Self::Re3Pps,
    ];

    /// Canonical data-space address.
    pub const fn offset(self) -> usize {
        match self as u8 {
            0 => 0x1e8f,
            1..=8 => 0x1f10 + (self as usize - 1),
            9..=16 => 0x1f18 + (self as usize - 9),
            17..=24 => 0x1f20 + (self as usize - 17),
            25..=32 => 0x1f28 + (self as usize - 25),
            33..=36 => 0x1f30 + (self as usize - 33),
            _ => unreachable!(),
        }
    }

    /// Stable numeric index for metadata tables.
    pub const fn index(self) -> usize {
        self as usize
    }

    /// Stable lowercase register name.
    pub const fn name(self) -> &'static str {
        const NAMES: [&str; 37] = [
            "ppslock", "ra0pps", "ra1pps", "ra2pps", "ra3pps", "ra4pps", "ra5pps", "ra6pps",
            "ra7pps", "rb0pps", "rb1pps", "rb2pps", "rb3pps", "rb4pps", "rb5pps", "rb6pps",
            "rb7pps", "rc0pps", "rc1pps", "rc2pps", "rc3pps", "rc4pps", "rc5pps", "rc6pps",
            "rc7pps", "rd0pps", "rd1pps", "rd2pps", "rd3pps", "rd4pps", "rd5pps", "rd6pps",
            "rd7pps", "re0pps", "re1pps", "re2pps", "re3pps",
        ];
        NAMES[self.index()]
    }

    /// Returns the output port/pin for an RxyPPS register.
    pub const fn port_pin(self) -> Option<(usize, usize)> {
        match self as u8 {
            1..=8 => Some((0, self as usize - 1)),
            9..=16 => Some((1, self as usize - 9)),
            17..=24 => Some((2, self as usize - 17)),
            25..=32 => Some((3, self as usize - 25)),
            33..=36 => Some((4, self as usize - 33)),
            _ => None,
        }
    }

    /// Returns a named output register for a port/pin pair.
    pub fn output(port: usize, pin: usize) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|register| register.port_pin() == Some((port, pin)))
    }

    /// Resolves a raw data-space address to its named PPS register.
    pub fn from_data_address(address: usize) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|register| register.offset() == address)
    }
}

/// Named PIC16F15376 NCO1 and associated interrupt registers.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[repr(u8)]
#[allow(missing_docs)]
pub enum Pic16NcoRegister {
    Nco1Accl,
    Nco1Acch,
    Nco1Accu,
    Nco1Incl,
    Nco1Inch,
    Nco1Incu,
    Nco1Con,
    Nco1Clk,
    Pir7,
    Pie7,
}

impl Pic16NcoRegister {
    /// Every register in the implemented NCO1 block, in data-space order.
    pub const ALL: [Self; 10] = [
        Self::Nco1Accl,
        Self::Nco1Acch,
        Self::Nco1Accu,
        Self::Nco1Incl,
        Self::Nco1Inch,
        Self::Nco1Incu,
        Self::Nco1Con,
        Self::Nco1Clk,
        Self::Pir7,
        Self::Pie7,
    ];

    /// Returns the canonical data-space address.
    pub const fn offset(self) -> usize {
        match self {
            Self::Nco1Accl => 0x58c,
            Self::Nco1Acch => 0x58d,
            Self::Nco1Accu => 0x58e,
            Self::Nco1Incl => 0x58f,
            Self::Nco1Inch => 0x590,
            Self::Nco1Incu => 0x591,
            Self::Nco1Con => 0x592,
            Self::Nco1Clk => 0x593,
            Self::Pir7 => 0x713,
            Self::Pie7 => 0x71d,
        }
    }

    /// Returns the stable zero-based index in [`Self::ALL`].
    pub const fn index(self) -> usize {
        self as usize
    }

    /// Returns a stable human-readable register name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Nco1Accl => "NCO1ACCL",
            Self::Nco1Acch => "NCO1ACCH",
            Self::Nco1Accu => "NCO1ACCU",
            Self::Nco1Incl => "NCO1INCL",
            Self::Nco1Inch => "NCO1INCH",
            Self::Nco1Incu => "NCO1INCU",
            Self::Nco1Con => "NCO1CON",
            Self::Nco1Clk => "NCO1CLK",
            Self::Pir7 => "PIR7",
            Self::Pie7 => "PIE7",
        }
    }

    /// Resolves a canonical data-space address to a named register.
    pub const fn from_data_address(address: usize) -> Option<Self> {
        match address {
            0x58c => Some(Self::Nco1Accl),
            0x58d => Some(Self::Nco1Acch),
            0x58e => Some(Self::Nco1Accu),
            0x58f => Some(Self::Nco1Incl),
            0x590 => Some(Self::Nco1Inch),
            0x591 => Some(Self::Nco1Incu),
            0x592 => Some(Self::Nco1Con),
            0x593 => Some(Self::Nco1Clk),
            0x713 => Some(Self::Pir7),
            0x71d => Some(Self::Pie7),
            _ => None,
        }
    }
}

const SSP1STAT_RW: u8 = 1 << 2;
const SSP1STAT_S: u8 = 1 << 3;
const SSP1STAT_P: u8 = 1 << 4;
const SSP1STAT_DA: u8 = 1 << 5;
const SSP1_I2C_MASTER_7BIT: u8 = 0x08;
const SSP1_I2C_MASTER_10BIT: u8 = 0x09;
const SSP1CON2_SEN: u8 = 1 << 0;
const SSP1CON2_RSEN: u8 = 1 << 1;
const SSP1CON2_PEN: u8 = 1 << 2;
const SSP1CON2_RCEN: u8 = 1 << 3;
const SSP1CON2_ACKEN: u8 = 1 << 4;
const SSP1CON2_ACKDT: u8 = 1 << 5;
const SSP1CON2_ACKSTAT: u8 = 1 << 6;
const SSP1CON3_BOEN: u8 = 1 << 4;

/// A functional MSSP1 I²C host transaction observed by the emulator.
///
/// Addresses are represented as 7-bit addresses. The model deliberately
/// reports byte-level transactions rather than SCL edges; it is intended for
/// deterministic firmware tests, not electrical or cycle-accurate simulation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pic16I2cEvent {
    /// A normal bus START condition.
    Start,
    /// A repeated START condition without releasing the bus.
    RepeatedStart,
    /// A byte transmitted after the address byte.
    Write {
        /// Seven-bit slave address.
        address: u8,
        /// Transmitted data byte.
        value: u8,
    },
    /// A byte returned by the queued slave response.
    Read {
        /// Seven-bit slave address.
        address: u8,
        /// Received data byte.
        value: u8,
    },
    /// An acknowledge or not-acknowledge bit emitted after a host read.
    Ack {
        /// `true` for ACK (`ACKDT = 0`), `false` for NACK (`ACKDT = 1`).
        acknowledge: bool,
    },
    /// A normal bus STOP condition.
    Stop,
}

struct Pic16State {
    registers: Vec<u8>,
    ports: [Arc<Mutex<GpioState>>; 5],
    port_signals: [Vec<SignalId>; 5],
    hub: SignalHub,
    uart: Vec<u8>,
    spi: Vec<u8>,
    spi_incoming: VecDeque<u8>,
    i2c_events: Vec<Pic16I2cEvent>,
    i2c_responses: BTreeMap<u8, VecDeque<u8>>,
    i2c_acknowledgements: BTreeMap<u8, bool>,
    i2c_address: Option<u8>,
    i2c_read: bool,
    i2c_byte_signal: SignalId,
    i2c_strobe_signal: SignalId,
    timer0_epoch: u64,
    timer1_epoch: u64,
    timer2_epoch: u64,
    timer2_postscale: u8,
    nco_epoch: u64,
    nco_increment_active: u32,
    nco_increment_pending: bool,
    nco_raw_output: bool,
    nco_pulse_remaining: u64,
    watchdog_epoch: u64,
    watchdog_reset: bool,
    adc_inputs: [u16; 64],
    adc_started: Option<(u8, u64)>,
    uart_byte_signal: SignalId,
    uart_strobe_signal: SignalId,
    spi_byte_signal: SignalId,
    spi_strobe_signal: SignalId,
    spi_irq_signal: SignalId,
    timer0_irq_signal: SignalId,
    timer1_irq_signal: SignalId,
    timer2_irq_signal: SignalId,
    nco1_output_signal: SignalId,
    dac1_value_signal: SignalId,
    dac1_active_signal: SignalId,
    comparator1_output_signal: SignalId,
    interrupt_signal: SignalId,
    watchdog_reset_signal: SignalId,
}

impl Pic16State {
    fn set_signal(&self, signal: SignalId, value: u64, width: u16, at: SimTime) {
        self.hub
            .set(
                signal,
                SignalValue::from_u64(value, width).expect("fixed PIC16 signal width is valid"),
                at,
            )
            .expect("PIC16 signal identity is fixed at construction");
    }

    fn resolved_port(&self, port: usize) -> u8 {
        self.ports[port]
            .lock()
            .expect("PIC16 GPIO lock poisoned")
            .nets
            .iter()
            .enumerate()
            .fold(0_u8, |value, (pin, net)| {
                value | (u8::from(net.resolved() == Logic::One) << pin)
            })
            & PORT_MASKS[port]
    }

    fn nco_accumulator(&self) -> u32 {
        u32::from(self.registers[Pic16NcoRegister::Nco1Accl.offset()])
            | (u32::from(self.registers[Pic16NcoRegister::Nco1Acch.offset()]) << 8)
            | (u32::from(self.registers[Pic16NcoRegister::Nco1Accu.offset()] & 0x0f) << 16)
    }

    fn nco_increment(&self) -> u32 {
        self.nco_increment_active
    }

    fn nco_enabled(&self) -> bool {
        self.registers[Pic16NcoRegister::Nco1Con.offset()] & NCO1EN != 0
    }

    fn nco_output(&self) -> bool {
        self.registers[Pic16NcoRegister::Nco1Con.offset()] & NCO1OUT != 0
    }

    fn nco_pulse_width(&self) -> u64 {
        1_u64 << u32::from((self.registers[Pic16NcoRegister::Nco1Clk.offset()] >> 5) & 0x07)
    }

    fn publish_nco_output(&mut self, at: SimTime) {
        let control = Pic16NcoRegister::Nco1Con.offset();
        let visible =
            self.nco_enabled() && (self.nco_raw_output ^ (self.registers[control] & NCO1POL != 0));
        self.registers[control] =
            (self.registers[control] & !NCO1OUT) | (u8::from(visible) * NCO1OUT);
        self.set_signal(self.nco1_output_signal, u64::from(visible), 1, at);
    }

    fn update_nco(&mut self, now: SimTime) {
        let control = Pic16NcoRegister::Nco1Con.offset();
        if self.nco_increment_pending {
            self.nco_increment_active = self.nco_increment_registers();
            self.nco_increment_pending = false;
        }
        let elapsed = now.ticks().saturating_sub(self.nco_epoch);
        self.nco_epoch = now.ticks();
        if !self.nco_enabled() {
            self.nco_raw_output = false;
            self.nco_pulse_remaining = 0;
            self.publish_nco_output(now);
            return;
        }
        if elapsed == 0 {
            self.publish_nco_output(now);
            return;
        }
        let increment = u64::from(self.nco_increment());
        let total = u64::from(self.nco_accumulator()) + increment.saturating_mul(elapsed);
        let overflows = total >> 20;
        let accumulator = (total as u32) & NCO_ACC_MASK;
        self.registers[Pic16NcoRegister::Nco1Accl.offset()] = accumulator as u8;
        self.registers[Pic16NcoRegister::Nco1Acch.offset()] = (accumulator >> 8) as u8;
        self.registers[Pic16NcoRegister::Nco1Accu.offset()] = (accumulator >> 16) as u8 & 0x0f;
        if overflows != 0 {
            self.registers[Pic16NcoRegister::Pir7.offset()] |= NCO1IF;
            if self.registers[control] & NCO1PFM == 0 {
                if overflows & 1 != 0 {
                    self.nco_raw_output = !self.nco_raw_output;
                }
            } else {
                self.nco_pulse_remaining = self
                    .nco_pulse_remaining
                    .saturating_sub(elapsed)
                    .max(self.nco_pulse_width());
            }
        } else if self.registers[control] & NCO1PFM != 0 {
            self.nco_pulse_remaining = self.nco_pulse_remaining.saturating_sub(elapsed);
        }
        if self.registers[control] & NCO1PFM != 0 {
            self.nco_raw_output = self.nco_pulse_remaining != 0;
        }
        self.publish_nco_output(now);
    }

    fn nco_increment_registers(&self) -> u32 {
        u32::from(self.registers[Pic16NcoRegister::Nco1Incl.offset()])
            | (u32::from(self.registers[Pic16NcoRegister::Nco1Inch.offset()]) << 8)
            | (u32::from(self.registers[Pic16NcoRegister::Nco1Incu.offset()] & 0x0f) << 16)
    }

    fn update_dac_signals(&self, at: SimTime) {
        let enabled = self.registers[Pic16DacRegister::Dac1Con0.index()] & DAC1EN != 0;
        let code = if enabled {
            self.registers[Pic16DacRegister::Dac1Con1.index()] & DAC1R_MASK
        } else {
            0
        };
        self.set_signal(self.dac1_value_signal, u64::from(code), 5, at);
        self.set_signal(self.dac1_active_signal, u64::from(enabled), 1, at);
    }

    fn comparator_pin(&self, channel: u8, positive: bool) -> Option<Logic> {
        let (port, pin) = if positive {
            match channel {
                0 => (0, 2), // C1IN0+
                1 => (0, 3), // C1IN1+
                _ => return None,
            }
        } else {
            match channel {
                0 => (0, 0), // C1IN0-
                1 => (0, 1), // C1IN1-
                2 => (3, 3), // C1IN2- on RB3
                3 => (1, 1), // C1IN3- on RB1
                _ => return None,
            }
        };
        Some(
            self.ports[port]
                .lock()
                .expect("PIC16 GPIO lock poisoned")
                .nets[pin]
                .resolved(),
        )
    }

    fn comparator_input(&self, channel: u8, positive: bool) -> Logic {
        match channel {
            5 if positive => Logic::Zero, // DAC output is not part of this slice.
            6 => Logic::One,              // FVR buffer 2 is a deterministic high reference.
            7 => Logic::Zero,             // AVSS.
            _ => self
                .comparator_pin(channel, positive)
                .unwrap_or(Logic::Zero),
        }
    }

    fn update_comparator(&mut self, at: SimTime) {
        let enabled = self.registers[Pic16ComparatorRegister::Cm1Con0.index()] & C1ON != 0;
        let previous = self.registers[Pic16ComparatorRegister::Cm1Con0.index()] & CM1CON0_OUT != 0;
        let positive = self.comparator_input(
            self.registers[Pic16ComparatorRegister::Cm1Pch.index()] & CM1_CHANNEL_MASK,
            true,
        ) == Logic::One;
        let negative = self.comparator_input(
            self.registers[Pic16ComparatorRegister::Cm1Nch.index()] & CM1_CHANNEL_MASK,
            false,
        ) == Logic::One;
        let raw_output = enabled && (positive != negative) && positive;
        let output =
            if enabled && self.registers[Pic16ComparatorRegister::Cm1Con0.index()] & C1POL != 0 {
                !raw_output
            } else {
                raw_output
            };
        let cm1con0 = Pic16ComparatorRegister::Cm1Con0.index();
        self.registers[cm1con0] =
            (self.registers[cm1con0] & !CM1CON0_OUT) | (u8::from(output) * CM1CON0_OUT);
        let cmout = Pic16ComparatorRegister::Cmout.index();
        self.registers[cmout] =
            (self.registers[cmout] & !CMOUT_C1OUT) | (u8::from(output) * CMOUT_C1OUT);
        // C1IF is edge-triggered even when a transition is caused by changing
        // C1ON or C1POL; the data sheet explicitly calls out those cases.
        if output != previous {
            let edge_enable = if output {
                self.registers[Pic16ComparatorRegister::Cm1Con1.index()] & (1 << 1) != 0
            } else {
                self.registers[Pic16ComparatorRegister::Cm1Con1.index()] & 1 != 0
            };
            if edge_enable {
                self.registers[Pic16ComparatorRegister::Pir2.index()] |= C1IF;
            }
        }
        self.set_signal(self.comparator1_output_signal, u64::from(output), 1, at);
    }

    fn signal_level(&self, signal: SignalId) -> Logic {
        self.hub.with_registry(|registry| {
            registry
                .value(signal)
                .and_then(|value| value.bit(0))
                .unwrap_or(Logic::Zero)
        })
    }

    fn pps_output_level(&self, source: u8) -> Logic {
        match source {
            0 => Logic::Zero,
            PPS_OUTPUT_TX1 => self.signal_level(self.uart_strobe_signal),
            PPS_OUTPUT_TMR0 => self.signal_level(self.timer0_irq_signal),
            _ => Logic::Zero,
        }
    }

    fn refresh_port(&mut self, port: usize, at: SimTime) -> Result<(), DeviceError> {
        let direction = (!self.registers[TRIS_BASE + port]) & PORT_MASKS[port];
        let latch = self.registers[LAT_BASE + port] & PORT_MASKS[port];
        let mut output = latch;
        for pin in 0..usize::from(PORT_WIDTHS[port]) {
            let register = Pic16PpsRegister::output(port, pin).expect("PIC16 PPS pin is mapped");
            let source = self.registers[register.offset()] & PPS_OUTPUT_MASK;
            if source != 0 {
                output = (output & !(1 << pin))
                    | (u8::from(self.pps_output_level(source) == Logic::One) << pin);
            }
        }
        {
            let mut gpio = self.ports[port].lock().expect("PIC16 GPIO lock poisoned");
            gpio.direction = u32::from(direction);
            gpio.output = u32::from(output);
        }
        refresh_gpio(
            &self.ports[port],
            &self.port_signals[port],
            &self.hub,
            PORT_WIDTHS[port],
            at,
        )?;
        let digital = !self.registers[ANSEL[port]];
        self.registers[PORT_BASE + port] = self.resolved_port(port) & digital & PORT_MASKS[port];
        Ok(())
    }

    fn i2c_master_enabled(&self) -> bool {
        self.registers[SSP1CON1] & SSP1CON1_SSPEN != 0
            && matches!(
                self.registers[SSP1CON1] & 0x0f,
                SSP1_I2C_MASTER_7BIT | SSP1_I2C_MASTER_10BIT
            )
    }

    fn emit_i2c_byte(&mut self, value: u8, at: SimTime) {
        self.set_signal(self.i2c_byte_signal, u64::from(value), 8, at);
        let previous = self.hub.with_registry(|registry| {
            registry
                .value(self.i2c_strobe_signal)
                .and_then(|signal| signal.bit(0))
                .map_or(0, |logic| u64::from(logic == Logic::One))
        });
        self.set_signal(self.i2c_strobe_signal, previous ^ 1, 1, at);
    }

    fn i2c_command(&mut self, value: u8, at: SimTime) {
        const COMMANDS: u8 =
            SSP1CON2_SEN | SSP1CON2_RSEN | SSP1CON2_PEN | SSP1CON2_RCEN | SSP1CON2_ACKEN;
        let commands = value & COMMANDS;
        // ACKSTAT is hardware-owned. Firmware may clear it, but a write must
        // not manufacture a NACK that was never observed on the bus.
        self.registers[SSP1CON2] =
            (value & !SSP1CON2_ACKSTAT) | (self.registers[SSP1CON2] & SSP1CON2_ACKSTAT);
        if !self.i2c_master_enabled() {
            self.registers[SSP1CON2] &= !COMMANDS;
            return;
        }

        // The hardware has no event queue: setting more than one command bit
        // while an operation is being requested is a collision rather than a
        // sequence of operations.
        if commands.count_ones() > 1 {
            self.registers[SSP1CON1] |= SSP1CON1_WCOL;
            self.registers[SSP1CON2] &= !COMMANDS;
            return;
        }

        if self.registers[SSP1STAT] & SSP1STAT_BF != 0 && commands != SSP1CON2_RCEN {
            self.registers[SSP1CON1] |= SSP1CON1_WCOL;
            self.registers[SSP1CON2] &= !COMMANDS;
            return;
        }

        match commands {
            SSP1CON2_SEN => {
                self.registers[SSP1STAT] |= SSP1STAT_S;
                self.registers[SSP1STAT] &= !SSP1STAT_P;
                self.i2c_address = None;
                self.i2c_read = false;
                self.i2c_events.push(Pic16I2cEvent::Start);
                self.registers[PIR3] |= SSP1IF;
            }
            SSP1CON2_RSEN => {
                self.registers[SSP1STAT] |= SSP1STAT_S;
                self.registers[SSP1STAT] &= !SSP1STAT_P;
                self.i2c_address = None;
                self.i2c_read = false;
                self.i2c_events.push(Pic16I2cEvent::RepeatedStart);
                self.registers[PIR3] |= SSP1IF;
            }
            SSP1CON2_PEN => {
                self.registers[SSP1STAT] |= SSP1STAT_P;
                self.registers[SSP1STAT] &= !SSP1STAT_S;
                self.i2c_address = None;
                self.i2c_read = false;
                self.i2c_events.push(Pic16I2cEvent::Stop);
                self.registers[PIR3] |= SSP1IF;
            }
            SSP1CON2_RCEN => {
                if let (Some(address), true) = (self.i2c_address, self.i2c_read) {
                    if self.registers[SSP1STAT] & SSP1STAT_BF != 0
                        && self.registers[SSP1CON3] & SSP1CON3_BOEN == 0
                    {
                        self.registers[SSP1CON1] |= 1 << 6;
                    } else {
                        let value = self
                            .i2c_responses
                            .get_mut(&address)
                            .and_then(VecDeque::pop_front)
                            .unwrap_or(0xff);
                        self.registers[SSP1BUF] = value;
                        self.registers[SSP1STAT] |= SSP1STAT_BF;
                        self.registers[SSP1STAT] &= !(SSP1STAT_DA | SSP1STAT_RW);
                        self.emit_i2c_byte(value, at);
                        self.i2c_events.push(Pic16I2cEvent::Read { address, value });
                        self.registers[PIR3] |= SSP1IF;
                    }
                } else {
                    self.registers[SSP1CON1] |= SSP1CON1_WCOL;
                }
            }
            SSP1CON2_ACKEN => {
                let acknowledge = self.registers[SSP1CON2] & SSP1CON2_ACKDT == 0;
                self.i2c_events.push(Pic16I2cEvent::Ack { acknowledge });
                self.registers[PIR3] |= SSP1IF;
                if !acknowledge {
                    self.i2c_address = None;
                    self.i2c_read = false;
                }
            }
            0 => {}
            _ => unreachable!("MSSP command mask is one-hot"),
        }

        // SEN/RSEN/PEN/RCEN/ACKEN are command strobes. Firmware waits for
        // SSP1IF and observes these bits cleared by the peripheral.
        self.registers[SSP1CON2] &= !COMMANDS;
    }

    fn i2c_acknowledged(&self, address: u8) -> bool {
        self.i2c_acknowledgements
            .get(&address)
            .copied()
            .unwrap_or(true)
    }

    fn set_i2c_ackstat(&mut self, acknowledged: bool) {
        if acknowledged {
            self.registers[SSP1CON2] &= !SSP1CON2_ACKSTAT;
        } else {
            self.registers[SSP1CON2] |= SSP1CON2_ACKSTAT;
        }
    }

    fn i2c_buffer_write(&mut self, value: u8, at: SimTime) {
        if !self.i2c_master_enabled() {
            self.registers[SSP1BUF] = value;
            return;
        }

        if self.registers[SSP1STAT] & SSP1STAT_BF != 0 {
            self.registers[SSP1CON1] |= SSP1CON1_WCOL;
            return;
        }

        if self.i2c_address.is_none() {
            if self.registers[SSP1CON1] & 0x0f == SSP1_I2C_MASTER_10BIT {
                // The functional host slice intentionally accepts only the
                // common 7-bit address form; preserve the documented WCOL
                // diagnostic for a 10-bit transaction.
                self.registers[SSP1CON1] |= SSP1CON1_WCOL;
                return;
            }
            self.i2c_address = Some(value >> 1);
            self.i2c_read = value & 1 != 0;
            self.registers[SSP1BUF] = value;
            self.registers[SSP1STAT] |= SSP1STAT_BF | SSP1STAT_RW;
            self.emit_i2c_byte(value, at);
            let acknowledged = self.i2c_acknowledged(value >> 1);
            self.set_i2c_ackstat(acknowledged);
            self.registers[SSP1STAT] &= !(SSP1STAT_BF | SSP1STAT_DA | SSP1STAT_RW);
            self.registers[PIR3] |= SSP1IF;
            return;
        }

        if self.i2c_read {
            self.registers[SSP1CON1] |= SSP1CON1_WCOL;
            return;
        }
        let address = self.i2c_address.expect("I²C address was checked above");
        self.registers[SSP1BUF] = value;
        self.registers[SSP1STAT] |= SSP1STAT_BF | SSP1STAT_RW;
        self.emit_i2c_byte(value, at);
        self.i2c_events
            .push(Pic16I2cEvent::Write { address, value });
        let acknowledged = self.i2c_acknowledged(address);
        self.set_i2c_ackstat(acknowledged);
        self.registers[SSP1STAT] &= !(SSP1STAT_BF | SSP1STAT_DA | SSP1STAT_RW);
        self.registers[PIR3] |= SSP1IF;
    }

    fn reset_registers(&mut self, at: SimTime) {
        self.registers.fill(0);
        for port in 0..5 {
            self.registers[TRIS_BASE + port] = PORT_MASKS[port];
            self.registers[ANSEL[port]] = PORT_MASKS[port];
        }
        self.registers[SSP1ADD] = 0;
        self.registers[SSP1MSK] = 0xff;
        self.registers[SSP1CON2] = 0;
        self.registers[SSP1CON3] = 0;
        // NCO1INCL's bit zero powers up set on the PIC16F15376.
        self.registers[Pic16NcoRegister::Nco1Incl.offset()] = 1;
        self.registers[PIR3] = TX1IF;
        self.registers[Pic16Timer2Register::T2Pr.index()] = u8::MAX;
        self.registers[TX1STA] = 1 << 1; // TRMT
        self.registers[OSCSTAT] = 1 << 6; // internal HF oscillator ready
        self.registers[Pic16PpsRegister::Ppslock.offset()] = 0;
        self.uart.clear();
        self.spi.clear();
        self.spi_incoming.clear();
        self.i2c_events.clear();
        self.i2c_responses.clear();
        self.i2c_acknowledgements.clear();
        self.i2c_address = None;
        self.i2c_read = false;
        self.timer0_epoch = at.ticks();
        self.timer1_epoch = at.ticks();
        self.timer2_epoch = at.ticks();
        self.timer2_postscale = 0;
        self.nco_epoch = at.ticks();
        self.nco_increment_active = 1;
        self.nco_increment_pending = false;
        self.nco_raw_output = false;
        self.nco_pulse_remaining = 0;
        self.watchdog_epoch = at.ticks();
        self.watchdog_reset = false;
        self.adc_inputs = [0; 64];
        self.adc_started = None;
        self.set_signal(self.uart_strobe_signal, 0, 1, at);
        self.set_signal(self.spi_byte_signal, 0, 8, at);
        self.set_signal(self.spi_strobe_signal, 0, 1, at);
        self.set_signal(self.spi_irq_signal, 0, 1, at);
        self.set_signal(self.i2c_byte_signal, 0, 8, at);
        self.set_signal(self.i2c_strobe_signal, 0, 1, at);
        self.set_signal(self.timer0_irq_signal, 0, 1, at);
        self.set_signal(self.timer1_irq_signal, 0, 1, at);
        self.set_signal(self.timer2_irq_signal, 0, 1, at);
        self.update_dac_signals(at);
        self.set_signal(self.comparator1_output_signal, 0, 1, at);
        self.publish_nco_output(at);
        self.set_signal(self.interrupt_signal, 0, 1, at);
        self.set_signal(self.watchdog_reset_signal, 0, 1, at);
        for port in 0..5 {
            let _ = self.refresh_port(port, at);
        }
    }

    fn interrupt_pending(&self) -> bool {
        let peripheral = self.registers[INTCON] & INTCON_PEIE != 0
            && ((self.registers[PIR0] & self.registers[PIE0] & TMR0IF != 0)
                || (self.registers[Pic16Timer2Register::Pir4.index()]
                    & self.registers[Pic16Timer2Register::Pie4.index()]
                    & TMR1IF
                    != 0)
                || (self.registers[PIR3] & self.registers[PIE3] & (TX1IF | RC1IF) != 0)
                || (self.registers[PIR1] & ADIF != 0 && self.registers[PIE1] & ADIE != 0)
                || (self.registers[Pic16Timer2Register::Pir4.index()]
                    & self.registers[Pic16Timer2Register::Pie4.index()]
                    & TMR2IF
                    != 0)
                || (self.registers[Pic16ComparatorRegister::Pir2.index()]
                    & self.registers[Pic16ComparatorRegister::Pie2.index()]
                    & C1IF
                    != 0)
                || (self.registers[Pic16NcoRegister::Pir7.offset()]
                    & self.registers[Pic16NcoRegister::Pie7.offset()]
                    & NCO1IF
                    != 0)
                || (self.registers[PIR3] & SSP1IF != 0 && self.registers[PIE3] & SSP1IE != 0));
        self.registers[INTCON] & INTCON_GIE != 0 && peripheral
    }

    fn update_interrupt_signals(&self, at: SimTime) {
        self.set_signal(
            self.spi_irq_signal,
            u64::from(self.registers[PIR3] & SSP1IF != 0),
            1,
            at,
        );
        self.set_signal(
            self.interrupt_signal,
            u64::from(self.interrupt_pending()),
            1,
            at,
        );
    }
}

/// Host-facing PIC16F15376 peripheral state.
#[derive(Clone)]
pub struct Pic16PeripheralsHandle(Arc<Mutex<Pic16State>>);

impl Pic16PeripheralsHandle {
    /// Captured EUSART1 transmit bytes.
    pub fn uart_bytes(&self) -> Vec<u8> {
        self.0
            .lock()
            .expect("PIC16 peripheral lock poisoned")
            .uart
            .clone()
    }

    /// Captured MSSP1 MOSI bytes from functional SPI master transfers.
    pub fn spi_bytes(&self) -> Vec<u8> {
        self.0
            .lock()
            .expect("PIC16 peripheral lock poisoned")
            .spi
            .clone()
    }

    /// Queues one MISO byte for the next completed MSSP1 transfer.
    pub fn inject_spi_rx(&self, value: u8, at: SimTime) {
        let mut state = self.0.lock().expect("PIC16 peripheral lock poisoned");
        state.spi_incoming.push_back(value);
        state.update_interrupt_signals(at);
    }

    /// Returns the normalized 5-bit DAC code, or zero while DAC1 is disabled.
    pub fn dac1_code(&self) -> u8 {
        let state = self.0.lock().expect("PIC16 peripheral lock poisoned");
        if state.registers[Pic16DacRegister::Dac1Con0.index()] & DAC1EN != 0 {
            state.registers[Pic16DacRegister::Dac1Con1.index()] & DAC1R_MASK
        } else {
            0
        }
    }

    /// Returns whether DAC1 is enabled.
    pub fn dac1_enabled(&self) -> bool {
        self.0
            .lock()
            .expect("PIC16 peripheral lock poisoned")
            .registers[Pic16DacRegister::Dac1Con0.index()]
            & DAC1EN
            != 0
    }

    /// Returns the current logical C1 comparator output.
    pub fn comparator1_output(&self) -> bool {
        self.0
            .lock()
            .expect("PIC16 peripheral lock poisoned")
            .registers[Pic16ComparatorRegister::Cm1Con0.index()]
            & CM1CON0_OUT
            != 0
    }

    /// Returns the current logical NCO1 output.
    pub fn nco1_output(&self) -> bool {
        let state = self.0.lock().expect("PIC16 peripheral lock poisoned");
        state.nco_output()
    }

    /// Queues deterministic bytes returned by a 7-bit MSSP1 I²C slave.
    ///
    /// The queue is keyed by the 7-bit address used in the address byte. A
    /// missing response returns `0xff`, which keeps firmware runs bounded and
    /// reproducible without pretending to model an electrical bus.
    pub fn queue_i2c_read(&self, address: u8, bytes: impl IntoIterator<Item = u8>) {
        let mut state = self.0.lock().expect("PIC16 peripheral lock poisoned");
        let address = address & 0x7f;
        state.i2c_acknowledgements.insert(address, true);
        state
            .i2c_responses
            .entry(address)
            .or_default()
            .extend(bytes);
    }

    /// Configures whether the deterministic host should observe an ACK for a
    /// seven-bit address. Addresses ACK by default; this hook lets firmware
    /// tests exercise the documented `ACKSTAT` NACK path without electrical
    /// bus timing.
    pub fn set_i2c_ack(&self, address: u8, acknowledge: bool) {
        self.0
            .lock()
            .expect("PIC16 peripheral lock poisoned")
            .i2c_acknowledgements
            .insert(address & 0x7f, acknowledge);
    }

    /// Returns the byte-level MSSP1 I²C host events observed since reset or
    /// [`Self::clear_i2c`].
    pub fn i2c_events(&self) -> Vec<Pic16I2cEvent> {
        self.0
            .lock()
            .expect("PIC16 peripheral lock poisoned")
            .i2c_events
            .clone()
    }

    /// Clears captured I²C events while leaving queued slave responses intact.
    pub fn clear_i2c(&self) {
        self.0
            .lock()
            .expect("PIC16 peripheral lock poisoned")
            .i2c_events
            .clear();
    }

    /// Advances functional timers and returns the combined interrupt request.
    pub fn poll(&self, now: SimTime) -> bool {
        let mut state = self.0.lock().expect("PIC16 peripheral lock poisoned");
        for port in 0..5 {
            let _ = state.refresh_port(port, now);
        }
        state.update_comparator(now);
        if state.registers[T0CON0] & 0x80 != 0 {
            let period = u64::from(state.registers[TMR0H]).saturating_add(1).max(1);
            let elapsed = now.ticks().saturating_sub(state.timer0_epoch);
            state.registers[TMR0L] = (elapsed % period) as u8;
            if elapsed >= period {
                state.timer0_epoch = now.ticks();
                state.registers[PIR0] |= TMR0IF;
                state.set_signal(state.timer0_irq_signal, 1, 1, now);
            }
        }
        if state.registers[T1CON] & 1 != 0 {
            let initial =
                u16::from(state.registers[TMR1L]) | (u16::from(state.registers[TMR1H]) << 8);
            let elapsed = now.ticks().saturating_sub(state.timer1_epoch);
            let total = u64::from(initial).saturating_add(elapsed);
            let value = total as u16;
            state.registers[TMR1L] = value as u8;
            state.registers[TMR1H] = (value >> 8) as u8;
            state.timer1_epoch = now.ticks();
            if total > u64::from(u16::MAX) {
                state.registers[Pic16Timer2Register::Pir4.index()] |= TMR1IF;
                state.set_signal(state.timer1_irq_signal, 1, 1, now);
            }
        }
        state.update_nco(now);
        if state.registers[Pic16Timer2Register::T2Con.index()] & T2ON != 0 {
            let prescaler = 1_u64
                << u32::from(
                    (state.registers[Pic16Timer2Register::T2Con.index()] & T2CKPS_MASK) >> 4,
                );
            let period = u64::from(state.registers[Pic16Timer2Register::T2Pr.index()])
                .saturating_add(1)
                .max(1);
            let elapsed = now.ticks().saturating_sub(state.timer2_epoch);
            let increments = elapsed / prescaler;
            if increments != 0 {
                let total = u64::from(state.registers[Pic16Timer2Register::T2Tmr.index()])
                    .saturating_add(increments);
                let matches = total / period;
                state.registers[Pic16Timer2Register::T2Tmr.index()] = (total % period) as u8;
                state.timer2_epoch = state
                    .timer2_epoch
                    .saturating_add(increments.saturating_mul(prescaler));
                if matches != 0 {
                    let postscaler = u64::from(
                        state.registers[Pic16Timer2Register::T2Con.index()] & T2OUTPS_MASK,
                    ) + 1;
                    let accumulated = u64::from(state.timer2_postscale) + matches;
                    if accumulated >= postscaler {
                        state.registers[Pic16Timer2Register::Pir4.index()] |= TMR2IF;
                        state.set_signal(state.timer2_irq_signal, 1, 1, now);
                    }
                    state.timer2_postscale = (accumulated % postscaler) as u8;
                }
            }
        }
        if state.registers[WDTCON0] & 1 != 0 {
            let exponent = u32::from((state.registers[WDTCON0] >> 1) & 0x1f).min(20);
            let period = 32_u64.checked_shl(exponent).unwrap_or(u64::MAX);
            if now.ticks().saturating_sub(state.watchdog_epoch) >= period {
                state.watchdog_reset = true;
                state.set_signal(state.watchdog_reset_signal, 1, 1, now);
            }
        }
        if let Some((channel, started)) = state.adc_started {
            if now.ticks() > started {
                let sample = state.adc_inputs[usize::from(channel.min(63))] & 0x03ff;
                if state.registers[ADCON1] & (1 << 7) != 0 {
                    state.registers[ADRESL] = sample as u8;
                    state.registers[ADRESH] = (sample >> 8) as u8;
                } else {
                    state.registers[ADRESH] = (sample >> 2) as u8;
                    state.registers[ADRESL] = ((sample & 0x3) << 6) as u8;
                }
                state.registers[ADCON0] &= !ADCON0_GO;
                state.registers[PIR1] |= ADIF;
                state.adc_started = None;
            }
        }
        for port in 0..5 {
            let _ = state.refresh_port(port, now);
        }
        let pending = state.interrupt_pending();
        state.update_interrupt_signals(now);
        pending
    }

    /// Drives a deterministic 10-bit analog value for one ADC channel.
    pub fn set_adc_input(&self, channel: u8, value: u16) {
        let mut state = self.0.lock().expect("PIC16 peripheral lock poisoned");
        state.adc_inputs[usize::from(channel.min(63))] = value & 0x03ff;
    }

    /// Restarts the functional watchdog interval after CLRWDT.
    pub fn clear_watchdog(&self, now: SimTime) {
        self.0
            .lock()
            .expect("PIC16 peripheral lock poisoned")
            .watchdog_epoch = now.ticks();
    }

    /// Consumes a watchdog reset request.
    pub fn take_watchdog_reset(&self) -> bool {
        std::mem::take(
            &mut self
                .0
                .lock()
                .expect("PIC16 peripheral lock poisoned")
                .watchdog_reset,
        )
    }
}

/// PIC16F15376 banked data and peripheral window.
pub struct Pic16Peripherals {
    name: String,
    state: Arc<Mutex<Pic16State>>,
}

impl Pic16Peripherals {
    /// Creates the documented peripheral slice and five package port handles.
    pub fn new(
        name: impl Into<String>,
        hub: SignalHub,
    ) -> Result<(Self, Pic16PeripheralsHandle, [GpioHandle; 5]), remu_signals::SignalError> {
        let (porta, signals_a, handle_a) = vendor_gpio(8, "board.pic16f15376.porta", &hub)?;
        let (portb, signals_b, handle_b) = vendor_gpio(8, "board.pic16f15376.portb", &hub)?;
        let (portc, signals_c, handle_c) = vendor_gpio(8, "board.pic16f15376.portc", &hub)?;
        let (portd, signals_d, handle_d) = vendor_gpio(8, "board.pic16f15376.portd", &hub)?;
        let (porte, signals_e, handle_e) = vendor_gpio(4, "board.pic16f15376.porte", &hub)?;
        let uart_byte_signal = hub.declare(
            "board.pic16f15376.eusart1.tx_byte",
            SignalValue::from_u64(0, 8)?,
            Some("last byte written to EUSART1 TXREG".to_owned()),
        )?;
        let uart_strobe_signal = hub.declare(
            "board.pic16f15376.eusart1.tx_strobe",
            SignalValue::from_u64(0, 1)?,
            Some("toggles for each EUSART1 byte".to_owned()),
        )?;
        let spi_byte_signal = hub.declare(
            "board.pic16f15376.mssp1.tx_byte",
            SignalValue::from_u64(0, 8)?,
            Some("last byte written to MSSP1 SSPBUF".to_owned()),
        )?;
        let spi_strobe_signal = hub.declare(
            "board.pic16f15376.mssp1.tx_strobe",
            SignalValue::from_u64(0, 1)?,
            Some("toggles for each functional MSSP1 transfer".to_owned()),
        )?;
        let spi_irq_signal = hub.declare(
            "board.pic16f15376.mssp1.irq",
            SignalValue::from_u64(0, 1)?,
            Some("MSSP1 transfer-complete interrupt flag".to_owned()),
        )?;
        let i2c_byte_signal = hub.declare(
            "board.pic16f15376.mssp1.i2c_byte",
            SignalValue::from_u64(0, 8)?,
            Some("last byte observed on the functional MSSP1 I²C host".to_owned()),
        )?;
        let i2c_strobe_signal = hub.declare(
            "board.pic16f15376.mssp1.i2c_strobe",
            SignalValue::from_u64(0, 1)?,
            Some("toggles for each functional MSSP1 I²C byte".to_owned()),
        )?;
        let timer0_irq_signal = hub.declare(
            "board.pic16f15376.timer0.irq",
            SignalValue::from_u64(0, 1)?,
            Some("functional Timer0 interrupt flag".to_owned()),
        )?;
        let timer1_irq_signal = hub.declare(
            "board.pic16f15376.timer1.irq",
            SignalValue::from_u64(0, 1)?,
            Some("functional Timer1 interrupt flag".to_owned()),
        )?;
        let timer2_irq_signal = hub.declare(
            "board.pic16f15376.timer2.irq",
            SignalValue::from_u64(0, 1)?,
            Some("functional Timer2 period-match interrupt flag".to_owned()),
        )?;
        let nco1_output_signal = hub.declare(
            "board.pic16f15376.nco1.output",
            SignalValue::from_u64(0, 1)?,
            Some("functional NCO1 output".to_owned()),
        )?;
        let dac1_value_signal = hub.declare(
            "board.pic16f15376.dac1.value",
            SignalValue::from_u64(0, 5)?,
            Some("normalized 5-bit DAC1 code while enabled".to_owned()),
        )?;
        let dac1_active_signal = hub.declare(
            "board.pic16f15376.dac1.active",
            SignalValue::from_u64(0, 1)?,
            Some("DAC1 enable state".to_owned()),
        )?;
        let comparator1_output_signal = hub.declare(
            "board.pic16f15376.comparator1.output",
            SignalValue::from_u64(0, 1)?,
            Some("functional C1 comparator output".to_owned()),
        )?;
        let interrupt_signal = hub.declare(
            "board.pic16f15376.interrupt.request",
            SignalValue::from_u64(0, 1)?,
            Some("combined enabled peripheral interrupt request".to_owned()),
        )?;
        let watchdog_reset_signal = hub.declare(
            "board.pic16f15376.watchdog.reset",
            SignalValue::from_u64(0, 1)?,
            Some("functional watchdog reset request".to_owned()),
        )?;
        let state = Arc::new(Mutex::new(Pic16State {
            registers: vec![0; DATA_BYTES],
            ports: [porta, portb, portc, portd, porte],
            port_signals: [signals_a, signals_b, signals_c, signals_d, signals_e],
            hub,
            uart: Vec::new(),
            spi: Vec::new(),
            spi_incoming: VecDeque::new(),
            i2c_events: Vec::new(),
            i2c_responses: BTreeMap::new(),
            i2c_acknowledgements: BTreeMap::new(),
            i2c_address: None,
            i2c_read: false,
            timer0_epoch: 0,
            timer1_epoch: 0,
            timer2_epoch: 0,
            timer2_postscale: 0,
            nco_epoch: 0,
            nco_increment_active: 0,
            nco_increment_pending: false,
            nco_raw_output: false,
            nco_pulse_remaining: 0,
            watchdog_epoch: 0,
            watchdog_reset: false,
            adc_inputs: [0; 64],
            adc_started: None,
            uart_byte_signal,
            uart_strobe_signal,
            spi_byte_signal,
            spi_strobe_signal,
            spi_irq_signal,
            i2c_byte_signal,
            i2c_strobe_signal,
            timer0_irq_signal,
            timer1_irq_signal,
            timer2_irq_signal,
            nco1_output_signal,
            dac1_value_signal,
            dac1_active_signal,
            comparator1_output_signal,
            interrupt_signal,
            watchdog_reset_signal,
        }));
        state
            .lock()
            .expect("new PIC16 peripheral lock poisoned")
            .reset_registers(SimTime::ZERO);
        Ok((
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Pic16PeripheralsHandle(state),
            [handle_a, handle_b, handle_c, handle_d, handle_e],
        ))
    }

    fn canonical_offset(offset: usize) -> usize {
        if offset & 0x7f >= 0x70 {
            offset & 0x7f
        } else {
            offset
        }
    }

    fn port_for(address: usize, bases: &[usize]) -> Option<usize> {
        bases
            .iter()
            .position(|base| (*base..*base + 5).contains(&address))
            .or_else(|| {
                if (PORT_BASE..PORT_BASE + 5).contains(&address) {
                    Some(address - PORT_BASE)
                } else if (TRIS_BASE..TRIS_BASE + 5).contains(&address) {
                    Some(address - TRIS_BASE)
                } else if (LAT_BASE..LAT_BASE + 5).contains(&address) {
                    Some(address - LAT_BASE)
                } else {
                    None
                }
            })
    }
}

impl Device for Pic16Peripherals {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Byte {
            return Err(DeviceError::new("PIC16 data space requires byte accesses"));
        }
        let raw = usize::try_from(offset).map_err(|_| DeviceError::new("PIC16 offset overflow"))?;
        let address = Self::canonical_offset(raw);
        let mut state = self.state.lock().expect("PIC16 peripheral lock poisoned");
        if (PORT_BASE..PORT_BASE + 5).contains(&address) {
            state.refresh_port(address - PORT_BASE, at)?;
        }
        if Pic16NcoRegister::from_data_address(address).is_some() {
            state.update_nco(at);
        }
        let value = match address {
            OSCSTAT => state.registers[address] | (1 << 6),
            TX1STA => state.registers[address] | (1 << 1),
            address
                if Pic16PpsRegister::from_data_address(address)
                    == Some(Pic16PpsRegister::Ppslock) =>
            {
                state.registers[address] & PPSLOCKED
            }
            address if Pic16PpsRegister::from_data_address(address).is_some() => {
                state.registers[address] & PPS_OUTPUT_MASK
            }
            RC1REG => {
                state.registers[PIR3] &= !RC1IF;
                state.registers[address]
            }
            SSP1BUF => {
                state.registers[SSP1STAT] &= !SSP1STAT_BF;
                state.registers[address]
            }
            _ => *state.registers.get(address).ok_or_else(|| {
                DeviceError::new(format!("PIC16 read outside data space: {raw:#x}"))
            })?,
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
        if width != AccessWidth::Byte {
            return Err(DeviceError::new("PIC16 data space requires byte accesses"));
        }
        let raw = usize::try_from(offset).map_err(|_| DeviceError::new("PIC16 offset overflow"))?;
        let address = Self::canonical_offset(raw);
        let value = value as u8;
        let mut state = self.state.lock().expect("PIC16 peripheral lock poisoned");
        if !(address < DATA_BYTES) {
            return Err(DeviceError::new(format!(
                "PIC16 write outside data space: {raw:#x}"
            )));
        }
        match address {
            PORT_BASE..=0x010 => {
                let port = address - PORT_BASE;
                state.registers[LAT_BASE + port] = value & PORT_MASKS[port];
                state.refresh_port(port, at)?;
            }
            TRIS_BASE..=0x016 => {
                let port = address - TRIS_BASE;
                state.registers[address] = value & PORT_MASKS[port];
                state.refresh_port(port, at)?;
            }
            LAT_BASE..=0x01c => {
                let port = address - LAT_BASE;
                state.registers[address] = value & PORT_MASKS[port];
                state.refresh_port(port, at)?;
            }
            TX1REG => {
                state.registers[address] = value;
                state.registers[PIR3] |= TX1IF;
                if state.registers[RC1STA] & SPEN != 0 && state.registers[TX1STA] & TXEN != 0 {
                    state.uart.push(value);
                    state.set_signal(state.uart_byte_signal, u64::from(value), 8, at);
                    let previous = state.hub.with_registry(|registry| {
                        registry
                            .value(state.uart_strobe_signal)
                            .and_then(|signal| signal.bit(0))
                            .map_or(0, |logic| u64::from(logic == Logic::One))
                    });
                    state.set_signal(state.uart_strobe_signal, previous ^ 1, 1, at);
                }
            }
            SSP1BUF => {
                if state.i2c_master_enabled() {
                    state.i2c_buffer_write(value, at);
                } else {
                    let enabled = state.registers[SSP1CON1] & SSP1CON1_SSPEN != 0;
                    let master_mode = state.registers[SSP1CON1] & 0x0f <= 0x03;
                    if enabled && master_mode {
                        if state.registers[SSP1STAT] & SSP1STAT_BF != 0 {
                            state.registers[SSP1CON1] |= SSP1CON1_WCOL;
                        } else {
                            let received = state.spi_incoming.pop_front().unwrap_or(value);
                            state.registers[address] = received;
                            state.registers[SSP1STAT] |= SSP1STAT_BF;
                            state.registers[PIR3] |= SSP1IF;
                            state.spi.push(value);
                            state.set_signal(state.spi_byte_signal, u64::from(value), 8, at);
                            let previous = state.hub.with_registry(|registry| {
                                registry
                                    .value(state.spi_strobe_signal)
                                    .and_then(|signal| signal.bit(0))
                                    .map_or(0, |logic| u64::from(logic == Logic::One))
                            });
                            state.set_signal(state.spi_strobe_signal, previous ^ 1, 1, at);
                        }
                    } else {
                        state.registers[address] = value;
                    }
                }
            }
            SSP1CON1 => {
                let was_enabled = state.i2c_master_enabled();
                state.registers[address] = value;
                if was_enabled && !state.i2c_master_enabled() {
                    state.i2c_address = None;
                    state.i2c_read = false;
                    state.registers[SSP1STAT] &=
                        !(SSP1STAT_BF | SSP1STAT_RW | SSP1STAT_S | SSP1STAT_P | SSP1STAT_DA);
                    state.registers[SSP1CON2] &= !(SSP1CON2_SEN
                        | SSP1CON2_RSEN
                        | SSP1CON2_PEN
                        | SSP1CON2_RCEN
                        | SSP1CON2_ACKEN);
                }
            }
            SSP1CON2 => state.i2c_command(value, at),
            PIR0 => {
                state.registers[address] = value;
                state.set_signal(
                    state.timer0_irq_signal,
                    u64::from(value & TMR0IF != 0),
                    1,
                    at,
                );
            }
            address if address == Pic16Timer2Register::Pir4.offset() => {
                state.registers[address] = value;
                state.set_signal(
                    state.timer1_irq_signal,
                    u64::from(value & TMR1IF != 0),
                    1,
                    at,
                );
                state.set_signal(
                    state.timer2_irq_signal,
                    u64::from(value & TMR2IF != 0),
                    1,
                    at,
                );
            }
            SSP1STAT => {
                // BF, R/W, D/A, S and P are maintained by the functional
                // transfer models; SMP and CKE are writable mode bits.
                state.registers[address] = (state.registers[address] & 0x3f) | (value & 0xc0);
            }
            T0CON0 => {
                if state.registers[address] & 0x80 == 0 && value & 0x80 != 0 {
                    state.timer0_epoch = at.ticks();
                }
                state.registers[address] = value;
            }
            T1CON => {
                if state.registers[address] & 1 == 0 && value & 1 != 0 {
                    state.timer1_epoch = at.ticks();
                }
                state.registers[address] = value;
            }
            address if address == Pic16Timer2Register::T2Tmr.offset() => {
                state.registers[address] = value;
                state.timer2_epoch = at.ticks();
                state.timer2_postscale = 0;
            }
            address if address == Pic16Timer2Register::T2Con.offset() => {
                state.registers[address] = value;
                state.timer2_epoch = at.ticks();
                state.timer2_postscale = 0;
            }
            address if address == Pic16DacRegister::Dac1Con0.offset() => {
                state.registers[address] = value & DAC1CON0_MASK;
                state.update_dac_signals(at);
            }
            address if address == Pic16DacRegister::Dac1Con1.offset() => {
                state.registers[address] = value & DAC1R_MASK;
                state.update_dac_signals(at);
            }
            address if address == Pic16ComparatorRegister::Pir2.offset() => {
                state.registers[address] = value & C1IF;
            }
            address if address == Pic16ComparatorRegister::Pie2.offset() => {
                state.registers[address] = value & C1IF;
            }
            address if address == Pic16ComparatorRegister::Cmout.offset() => {
                // CMOUT is a read-only mirror of comparator outputs.
            }
            address if address == Pic16ComparatorRegister::Cm1Con0.offset() => {
                state.registers[address] =
                    (state.registers[address] & CM1CON0_OUT) | (value & CM1CON0_WRITE_MASK);
                state.update_comparator(at);
            }
            address if address == Pic16ComparatorRegister::Cm1Con1.offset() => {
                state.registers[address] = value & CM1CON1_MASK;
                state.update_comparator(at);
            }
            address
                if address == Pic16ComparatorRegister::Cm1Nch.offset()
                    || address == Pic16ComparatorRegister::Cm1Pch.offset() =>
            {
                state.registers[address] = value & CM1_CHANNEL_MASK;
                state.update_comparator(at);
            }
            address if Pic16NcoRegister::from_data_address(address).is_some() => {
                let register = Pic16NcoRegister::from_data_address(address)
                    .expect("NCO register guard returned Some");
                state.update_nco(at);
                match register {
                    Pic16NcoRegister::Nco1Accl
                    | Pic16NcoRegister::Nco1Acch
                    | Pic16NcoRegister::Nco1Accu => {
                        state.registers[address] = if register == Pic16NcoRegister::Nco1Accu {
                            value & 0x0f
                        } else {
                            value
                        };
                    }
                    Pic16NcoRegister::Nco1Incl
                    | Pic16NcoRegister::Nco1Inch
                    | Pic16NcoRegister::Nco1Incu => {
                        state.registers[address] = if register == Pic16NcoRegister::Nco1Incu {
                            value & 0x0f
                        } else {
                            value
                        };
                        if state.nco_enabled() {
                            if register == Pic16NcoRegister::Nco1Incl {
                                state.nco_increment_pending = true;
                            }
                        } else {
                            state.nco_increment_active = state.nco_increment_registers();
                            state.nco_increment_pending = false;
                        }
                    }
                    Pic16NcoRegister::Nco1Con => {
                        let was_enabled = state.nco_enabled();
                        let output = state.registers[address] & NCO1OUT;
                        state.registers[address] = (value & (NCO1EN | NCO1POL | NCO1PFM)) | output;
                        if was_enabled && !state.nco_enabled() {
                            state.nco_raw_output = false;
                            state.nco_pulse_remaining = 0;
                        }
                        if !was_enabled && state.nco_enabled() {
                            state.nco_epoch = at.ticks();
                        }
                        state.publish_nco_output(at);
                    }
                    Pic16NcoRegister::Nco1Clk => {
                        state.registers[address] = value & 0xef;
                        state.publish_nco_output(at);
                    }
                    Pic16NcoRegister::Pir7 => {
                        state.registers[address] = value & NCO1IF;
                    }
                    Pic16NcoRegister::Pie7 => {
                        state.registers[address] = value & NCO1IE;
                    }
                }
            }
            WDTCON0 => {
                state.registers[address] = value & 0x3f;
                state.watchdog_epoch = at.ticks();
            }
            ADCON0 => {
                let previous = state.registers[address];
                state.registers[address] = value;
                if value & ADCON0_GO != 0 && value & ADCON0_ADON != 0 && previous & ADCON0_GO == 0 {
                    state.adc_started = Some(((value >> 2) & 0x3f, at.ticks()));
                }
            }
            PIR1 => state.registers[address] = value,
            address if Pic16PpsRegister::from_data_address(address).is_some() => {
                let register = Pic16PpsRegister::from_data_address(address)
                    .expect("PPS address was checked above");
                if register == Pic16PpsRegister::Ppslock {
                    state.registers[address] = value & PPSLOCKED;
                } else if state.registers[Pic16PpsRegister::Ppslock.offset()] & PPSLOCKED == 0 {
                    state.registers[address] = value & PPS_OUTPUT_MASK;
                    if let Some((port, _pin)) = register.port_pin() {
                        state.refresh_port(port, at)?;
                    }
                }
            }
            _ => {
                state.registers[address] = value;
                if let Some(port) = Self::port_for(address, &ANSEL) {
                    state.refresh_port(port, at)?;
                }
            }
        }
        state.update_interrupt_signals(at);
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state
            .lock()
            .expect("PIC16 peripheral lock poisoned")
            .reset_registers(SimTime::ZERO);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timer2_register_ids_are_named_and_native() {
        assert_eq!(Pic16Timer2Register::ALL.len(), 6);
        assert_eq!(Pic16Timer2Register::T2Con.offset(), 0x28e);
        assert_eq!(Pic16Timer2Register::T2Con.index(), 0x28e);
        assert_eq!(Pic16Timer2Register::T2Con.name(), "t2con");
        assert_eq!(
            Pic16Timer2Register::from_data_address(0x71a),
            Some(Pic16Timer2Register::Pie4)
        );
        assert_eq!(Pic16Timer2Register::from_data_address(0x28f), None);
    }

    #[test]
    fn dac_register_ids_are_named_and_stable() {
        assert_eq!(Pic16DacRegister::ALL.len(), 2);
        assert_eq!(Pic16DacRegister::Dac1Con0.offset(), 0x90e);
        assert_eq!(Pic16DacRegister::Dac1Con0.index(), 0x90e);
        assert_eq!(Pic16DacRegister::Dac1Con0.name(), "dac1con0");
        assert_eq!(
            Pic16DacRegister::from_data_address(0x90f),
            Some(Pic16DacRegister::Dac1Con1)
        );
        assert_eq!(Pic16DacRegister::from_data_address(0x90d), None);
    }

    #[test]
    fn comparator_register_ids_are_named_and_stable() {
        assert_eq!(Pic16ComparatorRegister::ALL.len(), 7);
        assert_eq!(Pic16ComparatorRegister::Pir2.offset(), 0x70e);
        assert_eq!(Pic16ComparatorRegister::Cm1Con0.index(), 0x990);
        assert_eq!(Pic16ComparatorRegister::Cm1Con0.name(), "cm1con0");
        assert_eq!(
            Pic16ComparatorRegister::from_data_address(0x993),
            Some(Pic16ComparatorRegister::Cm1Pch)
        );
        assert_eq!(Pic16ComparatorRegister::from_data_address(0x994), None);
    }

    #[test]
    fn nco_registers_are_named_and_match_the_documented_map() {
        assert_eq!(Pic16NcoRegister::ALL.len(), 10);
        for (index, register) in Pic16NcoRegister::ALL.into_iter().enumerate() {
            assert_eq!(register.index(), index);
            assert_eq!(
                Pic16NcoRegister::from_data_address(register.offset()),
                Some(register)
            );
            assert!(!register.name().is_empty());
        }

        let hub = SignalHub::new();
        let (mut device, _handle, _ports) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
        assert_eq!(
            device
                .read(
                    Pic16NcoRegister::Nco1Incl.offset() as u64,
                    AccessWidth::Byte,
                    SimTime::ZERO,
                )
                .unwrap(),
            1
        );
        device
            .write(
                Pic16NcoRegister::Nco1Con.offset() as u64,
                AccessWidth::Byte,
                u64::from(NCO1EN | NCO1POL | NCO1OUT | 0x0e),
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            device
                .read(
                    Pic16NcoRegister::Nco1Con.offset() as u64,
                    AccessWidth::Byte,
                    SimTime::ZERO,
                )
                .unwrap(),
            u64::from(NCO1EN | NCO1OUT | NCO1POL)
        );
    }

    #[test]
    fn mssp1_register_ids_cover_the_native_window() {
        assert_eq!(
            Pic16Mssp1Register::from_offset(0x18c),
            Some(Pic16Mssp1Register::Buffer)
        );
        assert_eq!(
            Pic16Mssp1Register::Control3.offset(),
            0x192,
            "the typed ID must retain the PIC16 native offset"
        );
        assert_eq!(Pic16Mssp1Register::from_offset(0x193), None);
    }

    #[test]
    fn gpio_uart_timer_and_watchdog_slice_is_functional() {
        let hub = SignalHub::new();
        let (mut device, handle, ports) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
        device
            .write(ANSEL[0] as u64, AccessWidth::Byte, 0, SimTime::ZERO)
            .unwrap();
        device
            .write(TRIS_BASE as u64, AccessWidth::Byte, 0xfe, SimTime::ZERO)
            .unwrap();
        device
            .write(LAT_BASE as u64, AccessWidth::Byte, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(ports[0].output() & 1, 1);

        device
            .write(RC1STA as u64, AccessWidth::Byte, SPEN.into(), SimTime::ZERO)
            .unwrap();
        device
            .write(TX1STA as u64, AccessWidth::Byte, TXEN.into(), SimTime::ZERO)
            .unwrap();
        device
            .write(TX1REG as u64, AccessWidth::Byte, b'P'.into(), SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.uart_bytes(), b"P");

        device
            .write(TMR0H as u64, AccessWidth::Byte, 3, SimTime::ZERO)
            .unwrap();
        device
            .write(PIE0 as u64, AccessWidth::Byte, TMR0IF.into(), SimTime::ZERO)
            .unwrap();
        device
            .write(
                INTCON as u64,
                AccessWidth::Byte,
                (INTCON_GIE | INTCON_PEIE).into(),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(T0CON0 as u64, AccessWidth::Byte, 0x80, SimTime::ZERO)
            .unwrap();
        assert!(handle.poll(SimTime::from_ticks(4)));
    }

    #[test]
    fn mssp1_spi_master_transfer_exposes_loopback_and_interrupt_state() {
        let hub = SignalHub::new();
        let (mut device, handle, _ports) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();

        device
            .write(
                SSP1CON1 as u64,
                AccessWidth::Byte,
                SSP1CON1_SSPEN.into(),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(PIE3 as u64, AccessWidth::Byte, SSP1IE.into(), SimTime::ZERO)
            .unwrap();
        device
            .write(
                INTCON as u64,
                AccessWidth::Byte,
                (INTCON_GIE | INTCON_PEIE).into(),
                SimTime::ZERO,
            )
            .unwrap();
        handle.inject_spi_rx(0xa5, SimTime::ZERO);
        device
            .write(SSP1BUF as u64, AccessWidth::Byte, 0x3c, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.spi_bytes(), vec![0x3c]);
        assert_eq!(
            device
                .read(SSP1BUF as u64, AccessWidth::Byte, SimTime::ZERO)
                .unwrap(),
            0xa5
        );
        assert_eq!(
            device
                .read(SSP1STAT as u64, AccessWidth::Byte, SimTime::ZERO)
                .unwrap()
                & u64::from(SSP1STAT_BF),
            0
        );
        assert!(handle.poll(SimTime::from_ticks(1)));

        handle.inject_spi_rx(0x5a, SimTime::from_ticks(1));
        device
            .write(
                SSP1BUF as u64,
                AccessWidth::Byte,
                0xc3,
                SimTime::from_ticks(1),
            )
            .unwrap();
        device
            .write(
                SSP1BUF as u64,
                AccessWidth::Byte,
                0xff,
                SimTime::from_ticks(1),
            )
            .unwrap();
        assert_eq!(
            device
                .read(SSP1CON1 as u64, AccessWidth::Byte, SimTime::from_ticks(1))
                .unwrap()
                & u64::from(SSP1CON1_WCOL),
            u64::from(SSP1CON1_WCOL)
        );
        assert_eq!(handle.spi_bytes(), vec![0x3c, 0xc3]);
    }

    #[test]
    fn adc_conversion_formats_result_and_sets_interrupt() {
        let hub = SignalHub::new();
        let (mut device, handle, _) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
        handle.set_adc_input(3, 0x2a5);
        device
            .write(PIE1 as u64, AccessWidth::Byte, ADIE.into(), SimTime::ZERO)
            .unwrap();
        device
            .write(
                INTCON as u64,
                AccessWidth::Byte,
                (INTCON_GIE | INTCON_PEIE).into(),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(ADCON1 as u64, AccessWidth::Byte, 1 << 7, SimTime::ZERO)
            .unwrap();
        device
            .write(
                ADCON0 as u64,
                AccessWidth::Byte,
                ((3 << 2) | ADCON0_GO | ADCON0_ADON).into(),
                SimTime::ZERO,
            )
            .unwrap();
        assert!(!handle.poll(SimTime::ZERO));
        assert!(handle.poll(SimTime::from_ticks(1)));
        assert_eq!(
            device
                .read(ADRESL as u64, AccessWidth::Byte, SimTime::ZERO)
                .unwrap(),
            0xa5
        );
        assert_eq!(
            device
                .read(ADRESH as u64, AccessWidth::Byte, SimTime::ZERO)
                .unwrap(),
            0x02
        );
        assert_eq!(
            device
                .read(ADCON0 as u64, AccessWidth::Byte, SimTime::ZERO)
                .unwrap()
                & u64::from(ADCON0_GO),
            0
        );
    }

    #[test]
    fn timer2_period_match_honors_prescaler_and_postscaler() {
        let hub = SignalHub::new();
        let (mut device, handle, _) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
        device
            .write(
                Pic16Timer2Register::T2Pr.offset() as u64,
                AccessWidth::Byte,
                2,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Pic16Timer2Register::Pie4.offset() as u64,
                AccessWidth::Byte,
                TMR2IF.into(),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                INTCON as u64,
                AccessWidth::Byte,
                (INTCON_GIE | INTCON_PEIE).into(),
                SimTime::ZERO,
            )
            .unwrap();
        // CKPS=1:2 and OUTPS=1:2. A T2TMR-to-T2PR match occurs every
        // (2 + 1) * 2 ticks, and the interrupt is raised on the second match.
        device
            .write(
                Pic16Timer2Register::T2Con.offset() as u64,
                AccessWidth::Byte,
                (T2ON | (1 << 4) | 1).into(),
                SimTime::ZERO,
            )
            .unwrap();
        assert!(!handle.poll(SimTime::from_ticks(5)));
        assert!(!handle.poll(SimTime::from_ticks(11)));
        assert!(handle.poll(SimTime::from_ticks(12)));
        device
            .write(
                Pic16Timer2Register::Pir4.offset() as u64,
                AccessWidth::Byte,
                0,
                SimTime::from_ticks(12),
            )
            .unwrap();
        assert!(!handle.poll(SimTime::from_ticks(13)));
    }

    #[test]
    fn dac1_exposes_a_masked_code_and_enable_state() {
        let hub = SignalHub::new();
        let (mut device, handle, _) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
        assert!(!handle.dac1_enabled());
        assert_eq!(handle.dac1_code(), 0);
        device
            .write(
                Pic16DacRegister::Dac1Con1.offset() as u64,
                AccessWidth::Byte,
                0xb5,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Pic16DacRegister::Dac1Con0.offset() as u64,
                AccessWidth::Byte,
                u64::from(DAC1EN | (1 << 5) | (1 << 2) | 1),
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            device
                .read(
                    Pic16DacRegister::Dac1Con0.offset() as u64,
                    AccessWidth::Byte,
                    SimTime::ZERO,
                )
                .unwrap(),
            u64::from(DAC1EN | (1 << 5) | (1 << 2))
        );
        assert!(handle.dac1_enabled());
        assert_eq!(handle.dac1_code(), 0x15);
        device
            .write(
                Pic16DacRegister::Dac1Con0.offset() as u64,
                AccessWidth::Byte,
                0,
                SimTime::from_ticks(1),
            )
            .unwrap();
        assert!(!handle.dac1_enabled());
        assert_eq!(handle.dac1_code(), 0);
    }

    #[test]
    fn comparator1_selects_gpio_inputs_and_latches_edge_interrupts() {
        let hub = SignalHub::new();
        let (mut device, handle, ports) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
        ports[0].set_input(0, Logic::Zero, SimTime::ZERO).unwrap(); // C1IN0-
        ports[0].set_input(2, Logic::One, SimTime::ZERO).unwrap(); // C1IN0+
        device
            .write(
                Pic16ComparatorRegister::Pie2.offset() as u64,
                AccessWidth::Byte,
                C1IF.into(),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Pic16ComparatorRegister::Cm1Con1.offset() as u64,
                AccessWidth::Byte,
                0x02,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                INTCON as u64,
                AccessWidth::Byte,
                (INTCON_GIE | INTCON_PEIE).into(),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Pic16ComparatorRegister::Cm1Con0.offset() as u64,
                AccessWidth::Byte,
                C1ON.into(),
                SimTime::ZERO,
            )
            .unwrap();
        assert!(handle.comparator1_output());
        assert!(handle.poll(SimTime::from_ticks(1)));
        assert_eq!(
            device
                .read(
                    Pic16ComparatorRegister::Cm1Con0.offset() as u64,
                    AccessWidth::Byte,
                    SimTime::from_ticks(1),
                )
                .unwrap(),
            u64::from(C1ON | CM1CON0_OUT)
        );
        assert_eq!(
            device
                .read(
                    Pic16ComparatorRegister::Cmout.offset() as u64,
                    AccessWidth::Byte,
                    SimTime::from_ticks(1),
                )
                .unwrap(),
            u64::from(CMOUT_C1OUT)
        );
        device
            .write(
                Pic16ComparatorRegister::Cmout.offset() as u64,
                AccessWidth::Byte,
                u8::MAX.into(),
                SimTime::from_ticks(1),
            )
            .unwrap();
        assert_eq!(
            device
                .read(
                    Pic16ComparatorRegister::Cmout.offset() as u64,
                    AccessWidth::Byte,
                    SimTime::from_ticks(1),
                )
                .unwrap(),
            u64::from(CMOUT_C1OUT)
        );

        device
            .write(
                Pic16ComparatorRegister::Pir2.offset() as u64,
                AccessWidth::Byte,
                0,
                SimTime::from_ticks(1),
            )
            .unwrap();
        ports[0]
            .set_input(2, Logic::Zero, SimTime::from_ticks(2))
            .unwrap();
        assert!(!handle.poll(SimTime::from_ticks(2)));
        assert!(!handle.comparator1_output());
    }

    #[test]
    fn comparator1_stays_low_when_disabled_even_if_polarity_is_inverted() {
        let hub = SignalHub::new();
        let (mut device, handle, ports) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
        ports[0].set_input(0, Logic::Zero, SimTime::ZERO).unwrap();
        ports[0].set_input(2, Logic::One, SimTime::ZERO).unwrap();
        device
            .write(
                Pic16ComparatorRegister::Cm1Con0.offset() as u64,
                AccessWidth::Byte,
                C1POL.into(),
                SimTime::ZERO,
            )
            .unwrap();
        assert!(!handle.comparator1_output());
        device
            .write(
                Pic16ComparatorRegister::Cm1Con0.offset() as u64,
                AccessWidth::Byte,
                u64::from(C1ON | C1POL),
                SimTime::from_ticks(1),
            )
            .unwrap();
        assert!(!handle.comparator1_output());
        device
            .write(
                Pic16ComparatorRegister::Cm1Con0.offset() as u64,
                AccessWidth::Byte,
                C1ON.into(),
                SimTime::from_ticks(2),
            )
            .unwrap();
        assert!(handle.comparator1_output());
    }

    #[test]
    fn pps_routes_timer0_and_eusart_strobes_to_gpio_outputs() {
        let hub = SignalHub::new();
        let (mut device, handle, ports) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
        device
            .write(ANSEL[0] as u64, AccessWidth::Byte, 0, SimTime::ZERO)
            .unwrap();
        device
            .write(TRIS_BASE as u64, AccessWidth::Byte, 0xfc, SimTime::ZERO)
            .unwrap();
        device
            .write(
                Pic16PpsRegister::Ra0Pps.offset() as u64,
                AccessWidth::Byte,
                PPS_OUTPUT_TMR0.into(),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(TMR0H as u64, AccessWidth::Byte, 1, SimTime::ZERO)
            .unwrap();
        device
            .write(T0CON0 as u64, AccessWidth::Byte, 0x80, SimTime::ZERO)
            .unwrap();
        assert_eq!(ports[0].output() & 1, 0);
        handle.poll(SimTime::from_ticks(2));
        assert_eq!(ports[0].output() & 1, 1);

        device
            .write(
                Pic16PpsRegister::Ra0Pps.offset() as u64,
                AccessWidth::Byte,
                PPS_OUTPUT_TX1.into(),
                SimTime::from_ticks(2),
            )
            .unwrap();
        device
            .write(
                RC1STA as u64,
                AccessWidth::Byte,
                SPEN.into(),
                SimTime::from_ticks(2),
            )
            .unwrap();
        device
            .write(
                TX1STA as u64,
                AccessWidth::Byte,
                TXEN.into(),
                SimTime::from_ticks(2),
            )
            .unwrap();
        device
            .write(
                TX1REG as u64,
                AccessWidth::Byte,
                b'P'.into(),
                SimTime::from_ticks(2),
            )
            .unwrap();
        handle.poll(SimTime::from_ticks(2));
        assert_eq!(ports[0].output() & 1, 1);
    }

    #[test]
    fn pps_registers_are_named_cover_all_pins_and_honor_the_lock() {
        assert_eq!(Pic16PpsRegister::ALL.len(), 37);
        for (index, register) in Pic16PpsRegister::ALL.iter().copied().enumerate() {
            assert_eq!(register.index(), index);
            assert_eq!(
                Pic16PpsRegister::from_data_address(register.offset()),
                Some(register)
            );
        }
        assert_eq!(Pic16PpsRegister::Ra7Pps.port_pin(), Some((0, 7)));
        assert_eq!(Pic16PpsRegister::Re3Pps.port_pin(), Some((4, 3)));
        assert_eq!(
            Pic16PpsRegister::output(3, 7),
            Some(Pic16PpsRegister::Rd7Pps)
        );

        let hub = SignalHub::new();
        let (mut device, handle, ports) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
        let at = SimTime::ZERO;
        device
            .write(ANSEL[0] as u64, AccessWidth::Byte, 0, at)
            .unwrap();
        device
            .write(TRIS_BASE as u64, AccessWidth::Byte, 0x7f, at)
            .unwrap();
        device
            .write(
                Pic16PpsRegister::Ra7Pps.offset() as u64,
                AccessWidth::Byte,
                PPS_OUTPUT_TMR0.into(),
                at,
            )
            .unwrap();
        device
            .write(TMR0H as u64, AccessWidth::Byte, 1, at)
            .unwrap();
        device
            .write(T0CON0 as u64, AccessWidth::Byte, 0x80, at)
            .unwrap();
        handle.poll(SimTime::from_ticks(2));
        assert_eq!(ports[0].output() & 0x80, 0x80);

        device
            .write(
                Pic16PpsRegister::Ppslock.offset() as u64,
                AccessWidth::Byte,
                PPSLOCKED.into(),
                at,
            )
            .unwrap();
        device
            .write(
                Pic16PpsRegister::Ra7Pps.offset() as u64,
                AccessWidth::Byte,
                0,
                at,
            )
            .unwrap();
        assert_eq!(
            device.read(
                Pic16PpsRegister::Ra7Pps.offset() as u64,
                AccessWidth::Byte,
                at
            ),
            Ok(u64::from(PPS_OUTPUT_TMR0))
        );
    }

    #[test]
    fn nco1_accumulates_and_routes_overflow_interrupt() {
        let hub = SignalHub::new();
        let (mut device, handle, _ports) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
        device
            .write(
                Pic16NcoRegister::Nco1Incu.offset() as u64,
                AccessWidth::Byte,
                0x0f,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Pic16NcoRegister::Nco1Inch.offset() as u64,
                AccessWidth::Byte,
                0xff,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Pic16NcoRegister::Nco1Incl.offset() as u64,
                AccessWidth::Byte,
                0xff,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Pic16NcoRegister::Pie7.offset() as u64,
                AccessWidth::Byte,
                NCO1IE.into(),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                INTCON as u64,
                AccessWidth::Byte,
                (INTCON_GIE | INTCON_PEIE).into(),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Pic16NcoRegister::Nco1Con.offset() as u64,
                AccessWidth::Byte,
                NCO1EN.into(),
                SimTime::ZERO,
            )
            .unwrap();

        assert!(!handle.nco1_output());
        assert!(handle.poll(SimTime::from_ticks(2)));
        assert!(handle.nco1_output());
        assert_eq!(
            device
                .read(
                    Pic16NcoRegister::Pir7.offset() as u64,
                    AccessWidth::Byte,
                    SimTime::from_ticks(2),
                )
                .unwrap() as u8
                & NCO1IF,
            NCO1IF
        );
    }

    #[test]
    fn nco_fixed_duty_polarity_and_pulse_mode_are_observable() {
        let hub = SignalHub::new();
        let (mut device, handle, _ports) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
        for (register, value) in [
            (Pic16NcoRegister::Nco1Incu, 0x04_u64),
            (Pic16NcoRegister::Nco1Inch, 0),
            (Pic16NcoRegister::Nco1Incl, 0),
        ] {
            device
                .write(
                    register.offset() as u64,
                    AccessWidth::Byte,
                    value,
                    SimTime::ZERO,
                )
                .unwrap();
        }
        device
            .write(
                Pic16NcoRegister::Nco1Con.offset() as u64,
                AccessWidth::Byte,
                NCO1EN.into(),
                SimTime::ZERO,
            )
            .unwrap();
        assert!(!handle.nco1_output());
        assert!(!handle.poll(SimTime::from_ticks(4)));
        assert!(handle.nco1_output());

        device
            .write(
                Pic16NcoRegister::Nco1Con.offset() as u64,
                AccessWidth::Byte,
                u64::from(NCO1EN | NCO1POL),
                SimTime::from_ticks(4),
            )
            .unwrap();
        assert!(!handle.nco1_output());

        // A 1/4-scale increment overflows every four abstract input clocks.
        device
            .write(
                Pic16NcoRegister::Nco1Con.offset() as u64,
                AccessWidth::Byte,
                u64::from(NCO1EN | NCO1PFM),
                SimTime::from_ticks(4),
            )
            .unwrap();
        assert!(!handle.poll(SimTime::from_ticks(8)));
        assert!(handle.nco1_output());
        assert!(!handle.poll(SimTime::from_ticks(9)));
        assert!(!handle.nco1_output());
    }

    #[test]
    fn mssp1_i2c_host_records_write_start_and_stop() {
        let hub = SignalHub::new();
        let (mut device, handle, _) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
        device
            .write(
                SSP1CON1 as u64,
                AccessWidth::Byte,
                u64::from(SSP1CON1_SSPEN | SSP1_I2C_MASTER_7BIT),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(PIE3 as u64, AccessWidth::Byte, SSP1IE.into(), SimTime::ZERO)
            .unwrap();
        device
            .write(
                INTCON as u64,
                AccessWidth::Byte,
                u64::from(INTCON_GIE | INTCON_PEIE),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                SSP1CON2 as u64,
                AccessWidth::Byte,
                SSP1CON2_SEN.into(),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                SSP1BUF as u64,
                AccessWidth::Byte,
                0xa0,
                SimTime::from_ticks(1),
            )
            .unwrap();
        device
            .write(
                SSP1BUF as u64,
                AccessWidth::Byte,
                0x10,
                SimTime::from_ticks(2),
            )
            .unwrap();
        device
            .write(
                SSP1CON2 as u64,
                AccessWidth::Byte,
                SSP1CON2_PEN.into(),
                SimTime::from_ticks(3),
            )
            .unwrap();
        assert_eq!(
            handle.i2c_events(),
            vec![
                Pic16I2cEvent::Start,
                Pic16I2cEvent::Write {
                    address: 0x50,
                    value: 0x10
                },
                Pic16I2cEvent::Stop,
            ]
        );
        assert!(handle.poll(SimTime::from_ticks(3)));
    }

    #[test]
    fn mssp1_i2c_host_reads_queued_response_and_clears_bf() {
        let hub = SignalHub::new();
        let (mut device, handle, _) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
        device
            .write(
                SSP1CON1 as u64,
                AccessWidth::Byte,
                u64::from(SSP1CON1_SSPEN | SSP1_I2C_MASTER_7BIT),
                SimTime::ZERO,
            )
            .unwrap();
        handle.queue_i2c_read(0x50, [0x42]);
        device
            .write(
                SSP1CON2 as u64,
                AccessWidth::Byte,
                SSP1CON2_SEN.into(),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                SSP1BUF as u64,
                AccessWidth::Byte,
                0xa1,
                SimTime::from_ticks(1),
            )
            .unwrap();
        device
            .write(
                SSP1CON2 as u64,
                AccessWidth::Byte,
                SSP1CON2_RCEN.into(),
                SimTime::from_ticks(2),
            )
            .unwrap();
        assert_ne!(
            device
                .read(SSP1STAT as u64, AccessWidth::Byte, SimTime::from_ticks(2))
                .unwrap()
                & u64::from(SSP1STAT_BF),
            0
        );
        assert_eq!(
            device
                .read(SSP1BUF as u64, AccessWidth::Byte, SimTime::from_ticks(3))
                .unwrap(),
            0x42
        );
        assert_eq!(
            device
                .read(SSP1STAT as u64, AccessWidth::Byte, SimTime::from_ticks(3))
                .unwrap()
                & u64::from(SSP1STAT_BF),
            0
        );
        assert_eq!(
            handle.i2c_events(),
            vec![
                Pic16I2cEvent::Start,
                Pic16I2cEvent::Read {
                    address: 0x50,
                    value: 0x42
                }
            ]
        );
    }

    #[test]
    fn mssp1_i2c_master_reports_ackstat_and_ack_sequence() {
        let hub = SignalHub::new();
        let (mut device, handle, _ports) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
        device
            .write(
                SSP1CON1 as u64,
                AccessWidth::Byte,
                u64::from(SSP1CON1_SSPEN | SSP1_I2C_MASTER_7BIT),
                SimTime::ZERO,
            )
            .unwrap();
        handle.set_i2c_ack(0x50, false);
        device
            .write(
                SSP1CON2 as u64,
                AccessWidth::Byte,
                u64::from(SSP1CON2_SEN),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                SSP1BUF as u64,
                AccessWidth::Byte,
                0xa0,
                SimTime::from_ticks(1),
            )
            .unwrap();
        assert_ne!(
            device
                .read(SSP1CON2 as u64, AccessWidth::Byte, SimTime::from_ticks(1))
                .unwrap()
                & u64::from(SSP1CON2_ACKSTAT),
            0,
            "a configured NACK must be visible through ACKSTAT"
        );

        handle.set_i2c_ack(0x50, true);
        handle.queue_i2c_read(0x50, [0x42]);
        device
            .write(
                SSP1CON2 as u64,
                AccessWidth::Byte,
                u64::from(SSP1CON2_RSEN),
                SimTime::from_ticks(2),
            )
            .unwrap();
        device
            .write(
                SSP1BUF as u64,
                AccessWidth::Byte,
                0xa1,
                SimTime::from_ticks(3),
            )
            .unwrap();
        device
            .write(
                SSP1CON2 as u64,
                AccessWidth::Byte,
                u64::from(SSP1CON2_RCEN),
                SimTime::from_ticks(4),
            )
            .unwrap();
        assert_eq!(
            device
                .read(SSP1BUF as u64, AccessWidth::Byte, SimTime::from_ticks(5))
                .unwrap(),
            0x42
        );
        device
            .write(
                SSP1CON2 as u64,
                AccessWidth::Byte,
                u64::from(SSP1CON2_ACKDT | SSP1CON2_ACKEN),
                SimTime::from_ticks(6),
            )
            .unwrap();
        assert_eq!(
            handle.i2c_events().last(),
            Some(&Pic16I2cEvent::Ack { acknowledge: false })
        );
        assert_eq!(
            device
                .read(SSP1CON2 as u64, AccessWidth::Byte, SimTime::from_ticks(6))
                .unwrap()
                & u64::from(SSP1CON2_ACKEN),
            0
        );
    }

    #[test]
    fn mssp1_i2c_master_rejects_queued_commands_and_preserves_receive_buffer() {
        let hub = SignalHub::new();
        let (mut device, handle, _ports) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
        device
            .write(
                SSP1CON1 as u64,
                AccessWidth::Byte,
                u64::from(SSP1CON1_SSPEN | SSP1_I2C_MASTER_7BIT),
                SimTime::ZERO,
            )
            .unwrap();
        handle.queue_i2c_read(0x50, [0x10, 0x20]);
        device
            .write(
                SSP1CON2 as u64,
                AccessWidth::Byte,
                u64::from(SSP1CON2_SEN),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                SSP1BUF as u64,
                AccessWidth::Byte,
                0xa1,
                SimTime::from_ticks(1),
            )
            .unwrap();
        device
            .write(
                SSP1CON2 as u64,
                AccessWidth::Byte,
                u64::from(SSP1CON2_RCEN),
                SimTime::from_ticks(2),
            )
            .unwrap();
        device
            .write(
                SSP1CON2 as u64,
                AccessWidth::Byte,
                u64::from(SSP1CON2_SEN | SSP1CON2_PEN),
                SimTime::from_ticks(3),
            )
            .unwrap();
        assert_ne!(
            device
                .read(SSP1CON1 as u64, AccessWidth::Byte, SimTime::from_ticks(3))
                .unwrap()
                & u64::from(SSP1CON1_WCOL),
            0
        );
        assert_eq!(
            device
                .read(SSP1BUF as u64, AccessWidth::Byte, SimTime::from_ticks(4))
                .unwrap(),
            0x10
        );
    }
}
