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
// PIC16F15376 data-sheet register summary, bank 17 (DS40001866A §4.3).
const CLKRCON: usize = 0x895;
const CLKRCLK: usize = 0x896;
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
const CLKRCON_ENABLE: u8 = 1 << 7;
const CLKRCON_WRITABLE_MASK: u8 = 0x9f;
const CLKRCLK_WRITABLE_MASK: u8 = 0x0f;
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
