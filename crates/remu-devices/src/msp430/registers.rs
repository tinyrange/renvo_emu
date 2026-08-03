/// One functional eUSCI_B0 I²C host transaction observed by the test harness.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Msp430I2cEvent {
    /// START condition on the virtual bus.
    Start,
    /// Repeated START without releasing the virtual bus.
    RepeatedStart,
    /// A transmitted address or data byte.
    Write {
        /// Seven-bit target address selected in UCB0I2CSA.
        address: u16,
        /// Byte placed in UCB0TXBUF.
        value: u8,
    },
    /// A received byte supplied by the host fixture.
    Read {
        /// Seven-bit target address selected in UCB0I2CSA.
        address: u16,
        /// Byte supplied by the host fixture.
        value: u8,
    },
    /// A target did not acknowledge its address.
    Nack {
        /// Seven-bit target address selected in UCB0I2CSA.
        address: u16,
    },
    /// STOP condition on the virtual bus.
    Stop,
}

/// FR2433 eUSCI_B0 register identities from the TI device table.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(usize)]
pub enum Msp430EusciB0Register {
    /// Control word 0.
    Ctlw0 = 0x0540,
    /// Control word 1.
    Ctlw1 = 0x0542,
    /// Bit-rate control word (UCB0BR0/UCB0BR1).
    Brw = 0x0546,
    /// Read-only status word.
    Statw = 0x0548,
    /// Automatic-stop byte-counter threshold.
    TbCnt = 0x054a,
    /// Receive buffer.
    RxBuf = 0x054c,
    /// Transmit buffer.
    TxBuf = 0x054e,
    /// Own address 0.
    I2cOa0 = 0x0554,
    /// Own address 1.
    I2cOa1 = 0x0556,
    /// Own address 2.
    I2cOa2 = 0x0558,
    /// Own address 3.
    I2cOa3 = 0x055a,
    /// Received address.
    AddrX = 0x055c,
    /// Address mask.
    AddMask = 0x055e,
    /// Slave address used by master transactions.
    I2cSa = 0x0560,
    /// Interrupt enables.
    Ie = 0x056a,
    /// Interrupt flags.
    Ifg = 0x056c,
    /// Interrupt vector.
    Iv = 0x056e,
}

impl Msp430EusciB0Register {
    /// Returns the unified FR2433 peripheral-window address.
    pub const fn address(self) -> usize {
        self as usize
    }

    /// Resolves an exact register address to its named identity.
    pub const fn from_address(address: usize) -> Option<Self> {
        match address {
            0x0540 => Some(Self::Ctlw0),
            0x0542 => Some(Self::Ctlw1),
            0x0546 => Some(Self::Brw),
            0x0548 => Some(Self::Statw),
            0x054a => Some(Self::TbCnt),
            0x054c => Some(Self::RxBuf),
            0x054e => Some(Self::TxBuf),
            0x0554 => Some(Self::I2cOa0),
            0x0556 => Some(Self::I2cOa1),
            0x0558 => Some(Self::I2cOa2),
            0x055a => Some(Self::I2cOa3),
            0x055c => Some(Self::AddrX),
            0x055e => Some(Self::AddMask),
            0x0560 => Some(Self::I2cSa),
            0x056a => Some(Self::Ie),
            0x056c => Some(Self::Ifg),
            0x056e => Some(Self::Iv),
            _ => None,
        }
    }
}

pub(super) const UCB0CTLW0: usize = Msp430EusciB0Register::Ctlw0.address();
pub(super) const UCB0CTLW1: usize = Msp430EusciB0Register::Ctlw1.address();
pub(super) const UCB0BRW: usize = Msp430EusciB0Register::Brw.address();
pub(super) const UCB0STATW: usize = Msp430EusciB0Register::Statw.address();
pub(super) const UCB0TBCNT: usize = Msp430EusciB0Register::TbCnt.address();
pub(super) const UCB0RXBUF: usize = Msp430EusciB0Register::RxBuf.address();
pub(super) const UCB0TXBUF: usize = Msp430EusciB0Register::TxBuf.address();
pub(super) const UCB0I2COA0: usize = Msp430EusciB0Register::I2cOa0.address();
pub(super) const UCB0I2COA1: usize = Msp430EusciB0Register::I2cOa1.address();
pub(super) const UCB0I2COA2: usize = Msp430EusciB0Register::I2cOa2.address();
pub(super) const UCB0I2COA3: usize = Msp430EusciB0Register::I2cOa3.address();
pub(super) const UCB0ADDRX: usize = Msp430EusciB0Register::AddrX.address();
pub(super) const UCB0ADDMASK: usize = Msp430EusciB0Register::AddMask.address();
pub(super) const UCB0I2CSA: usize = Msp430EusciB0Register::I2cSa.address();
pub(super) const UCB0IE: usize = Msp430EusciB0Register::Ie.address();
pub(super) const UCB0IFG: usize = Msp430EusciB0Register::Ifg.address();
pub(super) const UCB0IV: usize = Msp430EusciB0Register::Iv.address();

pub(super) const UCSWRST: u16 = 0x0001;
pub(super) const UCSYNC: u16 = 0x0100;
pub(super) const UCMODE_MASK: u16 = 0x0600;
pub(super) const UCMST: u16 = 0x0800;
pub(super) const UCTR: u16 = 1 << 4;
pub(super) const UCTXSTT: u16 = 1 << 1;
pub(super) const UCTXSTP: u16 = 1 << 2;
pub(super) const UCTXNACK: u16 = 1 << 3;
pub(super) const UCMODE_I2C: u16 = 0x0600;
pub(super) const UCSSEL_MASK: u16 = 0x00c0;
pub(super) const UCA10: u16 = 1 << 15;
pub(super) const UCSLA10: u16 = 1 << 14;
pub(super) const UCMM: u16 = 1 << 13;
pub(super) const UCTXACK: u16 = 1 << 5;
pub(super) const UCB0_CONFIG_MASK: u16 =
    UCA10 | UCSLA10 | UCMM | UCMST | UCMODE_MASK | UCSYNC | UCSSEL_MASK;
pub(super) const UCB0_RUNTIME_CONTROL_MASK: u16 =
    UCTXACK | UCTR | UCTXNACK | UCTXSTP | UCTXSTT | UCSWRST;
pub(super) const UCB0_IFG_MASK: u16 = 0x7fff;
pub(super) const UCB0_IE_MASK: u16 = 0x7fff;
pub(super) const UCBBUSY: u16 = 1 << 4;
pub(super) const UCASTP_MASK: u16 = 0x000c;
pub(super) const UCASTP_STOP: u16 = 0x0008;
pub(super) const UCBCNTIFG: u16 = 1 << 6;
pub(super) const UCCLTOIFG: u16 = 1 << 7;
pub(super) const UCSTTIFG: u16 = 1 << 2;
pub(super) const UCSTPIFG: u16 = 1 << 3;
pub(super) const UCNACKIFG: u16 = 1 << 5;
pub(super) const UCALIFG: u16 = 1 << 4;
pub(super) const UCBIT9IFG: u16 = 1 << 14;
pub(super) const UCRXIFG: u16 = 0x0001;
pub(super) const UCTXIFG: u16 = 0x0002;
pub(super) const UCB0_INTERRUPT_FLAGS: u16 = UCRXIFG
    | UCTXIFG
    | UCSTTIFG
    | UCSTPIFG
    | UCNACKIFG
    | UCALIFG
    | UCBCNTIFG
    | UCCLTOIFG
    | UCBIT9IFG;
