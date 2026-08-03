use super::{GpioHandle, GpioState, SignalHub, refresh_gpio, vendor_gpio};
use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use remu_signals::{Logic, SignalId, SignalValue};
use std::sync::{Arc, Mutex};

const SFR_BYTES: usize = 0x1_0000;
const PAGE3: usize = 0x20;
const P0: usize = 0x80;
const TCON: usize = 0x88;
const TMOD: usize = 0x89;
const TL0: usize = 0x8a;
const TH0: usize = 0x8c;
const TL1: usize = 0x8b;
const TH1: usize = 0x8d;
const P1: usize = 0x90;
const WDTCN: usize = 0x97;
const SCON0: usize = 0x98;
const SBUF0: usize = 0x99;
const SPI0CFG: usize = 0xa1;
const SPI0CKR: usize = 0xa2;
const SPI0CN0: usize = 0xf8;
const SPI0DAT: usize = 0xa3;
const P3MDOUT: usize = (PAGE3 << 8) | 0x9c;
const P2: usize = 0xa0;
const P0MDOUT: usize = 0xa4;
const P1MDOUT: usize = 0xa5;
const P2MDOUT: usize = 0xa6;
const IE: usize = 0xa8;
const CLKSEL: usize = 0xa9;
const P3: usize = 0xb0;
const IP: usize = 0xb8;
const TMR2CN0: usize = 0xc8;
const TMR2RLL: usize = 0xca;
const TMR2RLH: usize = 0xcb;
const TMR2L: usize = 0xce;
const TMR2H: usize = 0xcf;
const CRC0IN: usize = (PAGE3 << 8) | 0xca;
const CRC0DAT: usize = (PAGE3 << 8) | 0xcb;
const CRC0CN0: usize = (PAGE3 << 8) | 0xce;
const CRC0FLIP: usize = (PAGE3 << 8) | 0xcf;
const CRC0CN0_MASK: u8 = 0x05;
const XBR0: usize = 0xe1;
const XBR2: usize = 0xe3;
const RSTSRC: usize = 0xef;
const P0MDIN: usize = 0xf1;
const P1MDIN: usize = 0xf2;
const P2MDIN: usize = 0xf3;
const P3MDIN: usize = (PAGE3 << 8) | 0xf4;

/// Named EFM8 PCA and interrupt-control register identifier.
///
/// The EFM8 exposes most PCA registers on SFR pages 0 and 0x10.  The
/// identifier stores the canonical page-0 address (or the explicit extended
/// address for the priority registers), so device code and callers do not
/// have to pass unlabelled integer register IDs around.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[repr(u16)]
pub enum Efm8PcaRegister {
    /// PCA control and status flags (PCA0CN0, address 0xd8).
    Pca0Cn = 0xd8,
    /// PCA clock source and overflow interrupt enable (PCA0MD, 0xd9).
    Pca0Md = 0xd9,
    /// PCA channel 0 mode (PCA0CPM0, 0xda).
    Pca0Cpm0 = 0xda,
    /// PCA channel 1 mode (PCA0CPM1, 0xdb).
    Pca0Cpm1 = 0xdb,
    /// PCA channel 2 mode (PCA0CPM2, 0xdc).
    Pca0Cpm2 = 0xdc,
    /// PCA channel output polarity (PCA0POL, 0x96).
    Pca0Pol = 0x96,
    /// PCA PWM cycle length and overflow flags (PCA0PWM, 0xf7).
    Pca0Pwm = 0xf7,
    /// PCA edge/center selection (PCA0CENT, 0xf8).
    Pca0Cent = 0xf8,
    /// PCA counter low byte (PCA0L, 0xf9).
    Pca0L = 0xf9,
    /// PCA counter high byte (PCA0H, 0xfa).
    Pca0H = 0xfa,
    /// PCA channel 0 compare low byte (PCA0CPL0, 0xfb).
    Pca0Cpl0 = 0xfb,
    /// PCA channel 0 compare high byte (PCA0CPH0, 0xfc).
    Pca0Cph0 = 0xfc,
    /// PCA channel 1 compare low byte (PCA0CPL1, 0xe9).
    Pca0Cpl1 = 0xe9,
    /// PCA channel 1 compare high byte (PCA0CPH1, 0xea).
    Pca0Cph1 = 0xea,
    /// PCA channel 2 compare low byte (PCA0CPL2, 0xeb).
    Pca0Cpl2 = 0xeb,
    /// PCA channel 2 compare high byte (PCA0CPH2, 0xec).
    Pca0Cph2 = 0xec,
    /// PCA interrupt enable (EIE1, 0xe6).
    Eie1 = 0xe6,
    /// PCA interrupt priority (EIP1, extended page address 0x10bb).
    Eip1 = 0x10bb,
    /// PCA high-priority interrupt priority (EIP1H, extended page address 0x10ee).
    Eip1h = 0x10ee,
}

impl Efm8PcaRegister {
    /// Stable list of modeled PCA register IDs.
    pub const ALL: [Self; 19] = [
        Self::Pca0Cn,
        Self::Pca0Md,
        Self::Pca0Cpm0,
        Self::Pca0Cpm1,
        Self::Pca0Cpm2,
        Self::Pca0Pol,
        Self::Pca0Pwm,
        Self::Pca0Cent,
        Self::Pca0L,
        Self::Pca0H,
        Self::Pca0Cpl0,
        Self::Pca0Cph0,
        Self::Pca0Cpl1,
        Self::Pca0Cph1,
        Self::Pca0Cpl2,
        Self::Pca0Cph2,
        Self::Eie1,
        Self::Eip1,
        Self::Eip1h,
    ];

    /// Returns the canonical register address used by the device bus.
    pub const fn address(self) -> usize {
        self as usize
    }

    /// Returns the stable debugger/script-facing register name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Pca0Cn => "pca0cn",
            Self::Pca0Md => "pca0md",
            Self::Pca0Cpm0 => "pca0cpm0",
            Self::Pca0Cpm1 => "pca0cpm1",
            Self::Pca0Cpm2 => "pca0cpm2",
            Self::Pca0Pol => "pca0pol",
            Self::Pca0Pwm => "pca0pwm",
            Self::Pca0Cent => "pca0cent",
            Self::Pca0L => "pca0l",
            Self::Pca0H => "pca0h",
            Self::Pca0Cpl0 => "pca0cpl0",
            Self::Pca0Cph0 => "pca0cph0",
            Self::Pca0Cpl1 => "pca0cpl1",
            Self::Pca0Cph1 => "pca0cph1",
            Self::Pca0Cpl2 => "pca0cpl2",
            Self::Pca0Cph2 => "pca0cph2",
            Self::Eie1 => "eie1",
            Self::Eip1 => "eip1",
            Self::Eip1h => "eip1h",
        }
    }

    /// Resolves a raw SFR address to a named register.
    ///
    /// PCA registers are mirrored on SFR pages 0 and 0x10.  The device's
    /// canonicalisation already resolves those aliases; accepting either
    /// form here makes the public helper useful before canonicalisation too.
    pub fn from_address(address: usize) -> Option<Self> {
        match address {
            0x10bb => Some(Self::Eip1),
            0x10ee => Some(Self::Eip1h),
            _ => match address & 0xff {
                0xd8 => Some(Self::Pca0Cn),
                0xd9 => Some(Self::Pca0Md),
                0xda => Some(Self::Pca0Cpm0),
                0xdb => Some(Self::Pca0Cpm1),
                0xdc => Some(Self::Pca0Cpm2),
                0x96 => Some(Self::Pca0Pol),
                0xf7 => Some(Self::Pca0Pwm),
                0xf8 => Some(Self::Pca0Cent),
                0xf9 => Some(Self::Pca0L),
                0xfa => Some(Self::Pca0H),
                0xfb => Some(Self::Pca0Cpl0),
                0xfc => Some(Self::Pca0Cph0),
                0xe9 => Some(Self::Pca0Cpl1),
                0xea => Some(Self::Pca0Cph1),
                0xeb => Some(Self::Pca0Cpl2),
                0xec => Some(Self::Pca0Cph2),
                0xe6 => Some(Self::Eie1),
                _ => None,
            },
        }
    }
}

const PCA0CN: usize = Efm8PcaRegister::Pca0Cn.address();
const PCA0MD: usize = Efm8PcaRegister::Pca0Md.address();
const PCA0CPM0: usize = Efm8PcaRegister::Pca0Cpm0.address();
const PCA0CPM1: usize = Efm8PcaRegister::Pca0Cpm1.address();
const PCA0CPM2: usize = Efm8PcaRegister::Pca0Cpm2.address();
const EIE1: usize = Efm8PcaRegister::Eie1.address();
const EIP1: usize = Efm8PcaRegister::Eip1.address();
const EIP1H: usize = Efm8PcaRegister::Eip1h.address();
const PCA0POL: usize = Efm8PcaRegister::Pca0Pol.address();
const PCA0PWM: usize = Efm8PcaRegister::Pca0Pwm.address();
const PCA0CENT: usize = Efm8PcaRegister::Pca0Cent.address();
const PCA0L: usize = Efm8PcaRegister::Pca0L.address();
const PCA0H: usize = Efm8PcaRegister::Pca0H.address();
const PCA0CPL0: usize = Efm8PcaRegister::Pca0Cpl0.address();
const PCA0CPH0: usize = Efm8PcaRegister::Pca0Cph0.address();
const PCA0CPL1: usize = Efm8PcaRegister::Pca0Cpl1.address();
const PCA0CPH1: usize = Efm8PcaRegister::Pca0Cph1.address();
const PCA0CPL2: usize = Efm8PcaRegister::Pca0Cpl2.address();
const PCA0CPH2: usize = Efm8PcaRegister::Pca0Cph2.address();

const PORTS: [usize; 4] = [P0, P1, P2, P3];
const PORT_WIDTHS: [u8; 4] = [8, 8, 8, 5];
const PORT_MASKS: [u8; 4] = [0xff, 0xff, 0xff, 0x1f];
const PORT_MDOUT: [usize; 4] = [P0MDOUT, P1MDOUT, P2MDOUT, P3MDOUT];
const PORT_MDIN: [usize; 4] = [P0MDIN, P1MDIN, P2MDIN, P3MDIN];
const PCA0_CPM: [usize; 3] = [PCA0CPM0, PCA0CPM1, PCA0CPM2];
const PCA0_CPL: [usize; 3] = [PCA0CPL0, PCA0CPL1, PCA0CPL2];
const PCA0_CPH: [usize; 3] = [PCA0CPH0, PCA0CPH1, PCA0CPH2];
const PCA0_CCF: [u8; 3] = [PCA0CN_CCF0, PCA0CN_CCF1, PCA0CN_CCF2];

const IE_EA: u8 = 0x80;
const IE_ET0: u8 = 0x02;
const IE_ET1: u8 = 0x08;
const IE_ES0: u8 = 0x10;
const IE_ET2: u8 = 0x20;
const IE_ESPI0: u8 = 0x40;
const EIE1_EPCA0: u8 = 0x10;
const EIP1_PPCA0: u8 = 0x10;
const EIP1H_PHPCA0: u8 = 0x10;
const TCON_TR0: u8 = 0x10;
const TCON_TF0: u8 = 0x20;
const TCON_TR1: u8 = 0x40;
const TCON_TF1: u8 = 0x80;
const TMR2_TR2: u8 = 0x04;
const TMR2_TF2H: u8 = 0x80;
const SCON0_RI: u8 = 0x01;
const SCON0_TI: u8 = 0x02;
const SPI0_SPIF: u8 = 0x80;
const SPI0_TXNF: u8 = 0x02;
const SPI0_SPIEN: u8 = 0x01;
const XBR0_URT0E: u8 = 0x01;
const XBR2_XBARE: u8 = 0x40;
const PCA0CN_CF: u8 = 0x80;
const PCA0CN_CR: u8 = 0x40;
const PCA0CN_CCF0: u8 = 0x01;
const PCA0CN_CCF1: u8 = 0x02;
const PCA0CN_CCF2: u8 = 0x04;
const PCA0MD_ECF: u8 = 0x01;
const PCA0PWM_ECOV: u8 = 0x40;
const PCA0PWM_COVF: u8 = 0x20;
const PCA0PWM_CLSEL_MASK: u8 = 0x07;
const PCA0CPM_PWM16: u8 = 0x80;
const PCA0CPM_ECOM: u8 = 0x40;
const PCA0CPM_CAPP: u8 = 0x20;
const PCA0CPM_CAPN: u8 = 0x10;
const PCA0CPM_MAT: u8 = 0x08;
const PCA0CPM_TOG: u8 = 0x04;
const PCA0CPM_PWM: u8 = 0x02;
const PCA0CPM_ECCF: u8 = 0x01;

fn crc16_ccitt(mut crc: u16, input: u8) -> u16 {
    crc ^= u16::from(input) << 8;
    for _ in 0..8 {
        crc = if crc & 0x8000 != 0 {
            (crc << 1) ^ 0x1021
        } else {
            crc << 1
        };
    }
    crc
}

fn reverse_bits(value: u8) -> u8 {
    value.reverse_bits()
}

struct Efm8State {
    registers: Box<[u8]>,
    ports: [Arc<Mutex<GpioState>>; 4],
    port_signals: [Vec<SignalId>; 4],
    hub: SignalHub,
    uart: Vec<u8>,
    timer0_epoch: u64,
    timer1_epoch: u64,
    timer2_epoch: u64,
    crc_result: u16,
    watchdog_epoch: u64,
    watchdog_key: u8,
    watchdog_enabled: bool,
    watchdog_reset: bool,
    spi_tx: Vec<u8>,
    spi_rx: Vec<u8>,
    uart_byte_signal: SignalId,
    uart_strobe_signal: SignalId,
    timer0_irq_signal: SignalId,
    timer1_irq_signal: SignalId,
    timer2_irq_signal: SignalId,
    interrupt_signal: SignalId,
    watchdog_reset_signal: SignalId,
    pca_epoch: u64,
    pca_outputs: [Logic; 3],
    pca_inputs: [Logic; 3],
    pca_output_signals: [SignalId; 3],
    pca_interrupt_signal: SignalId,
}

impl Efm8State {
    fn set_signal(&self, signal: SignalId, value: u64, width: u16, at: SimTime) {
        self.hub
            .set(
                signal,
                SignalValue::from_u64(value, width).expect("fixed EFM8 signal width is valid"),
                at,
            )
            .expect("EFM8 signal identity is fixed at construction");
    }

    fn resolved_port(&self, port: usize) -> u8 {
        self.ports[port]
            .lock()
            .expect("EFM8 GPIO lock poisoned")
            .nets
            .iter()
            .enumerate()
            .fold(0_u8, |value, (pin, net)| {
                value | (u8::from(net.resolved() == Logic::One) << pin)
            })
            & PORT_MASKS[port]
    }

    fn refresh_port(&mut self, port: usize, at: SimTime) -> Result<(), DeviceError> {
        let latch = self.registers[PORTS[port]] & PORT_MASKS[port];
        let push_pull = self.registers[PORT_MDOUT[port]] & PORT_MASKS[port];
        let direction = push_pull | ((!latch) & PORT_MASKS[port]);
        {
            let mut gpio = self.ports[port].lock().expect("EFM8 GPIO lock poisoned");
            gpio.direction = u32::from(direction);
            gpio.output = u32::from(latch);
        }
        refresh_gpio(
            &self.ports[port],
            &self.port_signals[port],
            &self.hub,
            PORT_WIDTHS[port],
            at,
        )
    }

    fn port_read(&self, port: usize) -> u8 {
        let latch = self.registers[PORTS[port]];
        let push_pull = self.registers[PORT_MDOUT[port]];
        let input = self.resolved_port(port) & self.registers[PORT_MDIN[port]];
        ((latch & push_pull) | (input & !push_pull)) & PORT_MASKS[port]
    }

    fn reset_registers(&mut self, at: SimTime, kind: ResetKind) {
        self.registers.fill(0);
        for port in 0..4 {
            self.registers[PORTS[port]] = PORT_MASKS[port];
            self.registers[PORT_MDIN[port]] = PORT_MASKS[port];
        }
        self.registers[CLKSEL] = 0x80;
        self.registers[RSTSRC] = match kind {
            ResetKind::PowerOn => 0x02,
            ResetKind::External => 0x01,
            ResetKind::Software => 0x10,
            ResetKind::Watchdog => 0x08,
        };
        self.uart.clear();
        self.timer0_epoch = at.ticks();
        self.timer1_epoch = at.ticks();
        self.timer2_epoch = at.ticks();
        self.crc_result = 0;
        self.watchdog_epoch = at.ticks();
        self.watchdog_key = 0;
        self.watchdog_enabled = true;
        self.watchdog_reset = false;
        self.spi_tx.clear();
        self.spi_rx.clear();
        self.registers[SPI0CN0] = SPI0_TXNF;
        self.pca_epoch = at.ticks();
        self.pca_outputs = [Logic::Zero; 3];
        self.pca_inputs = [Logic::Zero; 3];
        for signal in [
            self.uart_strobe_signal,
            self.timer0_irq_signal,
            self.timer1_irq_signal,
            self.timer2_irq_signal,
            self.interrupt_signal,
            self.watchdog_reset_signal,
            self.pca_output_signals[0],
            self.pca_output_signals[1],
            self.pca_output_signals[2],
            self.pca_interrupt_signal,
        ] {
            self.set_signal(signal, 0, 1, at);
        }
        for port in 0..4 {
            let _ = self.refresh_port(port, at);
        }
    }

    fn canonical(raw: usize) -> usize {
        let page = raw >> 8;
        let address = raw & 0xff;
        if page == PAGE3 && matches!(address, 0x86 | 0x9c | 0xca..=0xcf | 0xd2..=0xd3 | 0xf4) {
            return raw;
        }
        match address {
            0x80
            | 0x88..=0x8e
            | 0x90
            | SPI0CFG
            | 0x97..=0x99
            | 0xa0
            | SPI0DAT
            | 0xa4..=0xa6
            | 0xa8..=0xa9
            | SPI0CKR
            | 0xb0
            | 0xb8
            | 0xc8
            | 0xca..=0xcf
            | 0xd8..=0xdc
            | 0xd4..=0xd5
            | 0xe1..=0xe3
            | 0xe6
            | 0xe9..=0xec
            | 0xef
            | 0xf1..=0xf3
            | 0xf7..=0xfc => address,
            0x9c | 0xf4 if page == PAGE3 => (PAGE3 << 8) | address,
            _ => raw,
        }
    }

    fn interrupt_levels(&self) -> [bool; 10] {
        let enabled = self.registers[IE];
        if enabled & IE_EA == 0 {
            return [false; 10];
        }
        let active = [
            enabled & IE_ET0 != 0 && self.registers[TCON] & TCON_TF0 != 0,
            enabled & IE_ES0 != 0 && self.registers[SCON0] & (SCON0_RI | SCON0_TI) != 0,
            enabled & IE_ET2 != 0 && self.registers[TMR2CN0] & TMR2_TF2H != 0,
            enabled & IE_ESPI0 != 0 && self.registers[SPI0CN0] & SPI0_SPIF != 0,
            enabled & IE_ET1 != 0 && self.registers[TCON] & TCON_TF1 != 0,
        ];
        let priorities = [IE_ET0, IE_ES0, IE_ET2, IE_ESPI0, IE_ET1];
        const LOW_LINES: [usize; 5] = [0, 1, 2, 6, 8];
        const HIGH_LINES: [usize; 5] = [3, 4, 5, 7, 9];
        let mut levels = [false; 10];
        for source in 0..active.len() {
            if active[source] {
                let high = self.registers[IP] & priorities[source] != 0;
                let line = if high {
                    HIGH_LINES[source]
                } else {
                    LOW_LINES[source]
                };
                levels[line] = true;
            }
        }
        if self.pca_interrupt_pending() {
            levels[if self.pca_high_priority() { 7 } else { 6 }] = true;
        }
        levels
    }

    fn update_interrupt_signals(&self, at: SimTime) {
        self.set_signal(
            self.timer0_irq_signal,
            u64::from(self.registers[TCON] & TCON_TF0 != 0),
            1,
            at,
        );
        self.set_signal(
            self.timer1_irq_signal,
            u64::from(self.registers[TCON] & TCON_TF1 != 0),
            1,
            at,
        );
        self.set_signal(
            self.timer2_irq_signal,
            u64::from(self.registers[TMR2CN0] & TMR2_TF2H != 0),
            1,
            at,
        );
        self.set_signal(
            self.interrupt_signal,
            u64::from(self.interrupt_levels().iter().any(|level| *level)),
            1,
            at,
        );
        self.set_signal(
            self.pca_interrupt_signal,
            u64::from(self.pca_interrupt_pending()),
            1,
            at,
        );
    }

    fn pca_counter(&self) -> u16 {
        u16::from_le_bytes([self.registers[PCA0L], self.registers[PCA0H]])
    }

    fn set_pca_counter(&mut self, value: u16) {
        let [low, high] = value.to_le_bytes();
        self.registers[PCA0L] = low;
        self.registers[PCA0H] = high;
    }

    fn pca_divider(&self) -> u64 {
        // PCA0MD.CPS is bits 3:1.  Timer0 overflow and ECI are external
        // event sources that are represented by one functional simulation
        // tick here; SYSCLK is the unscaled abstract tick.  The EFM8 manual
        // defines the oscillator sources as divided by eight.
        match (self.registers[PCA0MD] >> 1) & 0x07 {
            0 => 12,
            1 => 4,
            2..=4 => 1,
            5 | 6 => 8,
            _ => 1,
        }
    }

    fn pca_cycle_bits(&self) -> u8 {
        // CLSEL values 4..7 are reserved.  Treat an invalid value as the
        // reset 8-bit mode instead of silently widening the cycle to 11 bits.
        match self.registers[PCA0PWM] & PCA0PWM_CLSEL_MASK {
            0 => 8,
            1 => 9,
            2 => 10,
            3 => 11,
            _ => 8,
        }
    }

    fn pca_crossed(start: u16, ticks: u64, target: u16, modulus: u64) -> bool {
        if ticks == 0 {
            return false;
        }
        if ticks >= modulus {
            return true;
        }
        let end = u64::from(start) + ticks;
        if end < modulus {
            u64::from(target) > u64::from(start) && u64::from(target) <= end
        } else {
            let wrapped = end % modulus;
            u64::from(target) > u64::from(start) || u64::from(target) <= wrapped
        }
    }

    fn pca_width(&self, channel: usize, mode: u8) -> u8 {
        if mode & PCA0CPM_PWM16 != 0 {
            16
        } else {
            self.pca_cycle_bits()
        }
        .min(if channel < 3 { 16 } else { 8 })
    }

    fn pca_interrupt_pending(&self) -> bool {
        if self.registers[EIE1] & EIE1_EPCA0 == 0 {
            return false;
        }
        let cn = self.registers[PCA0CN];
        let pwm = self.registers[PCA0PWM];
        (cn & PCA0CN_CF != 0 && self.registers[PCA0MD] & PCA0MD_ECF != 0)
            || (pwm & PCA0PWM_COVF != 0 && pwm & PCA0PWM_ECOV != 0)
            || (0..3).any(|channel| {
                cn & PCA0_CCF[channel] != 0 && self.registers[PCA0_CPM[channel]] & PCA0CPM_ECCF != 0
            })
    }

    fn pca_high_priority(&self) -> bool {
        self.registers[EIP1H] & EIP1H_PHPCA0 != 0 || self.registers[EIP1] & EIP1_PPCA0 != 0
    }

    fn update_pca_output(&mut self, channel: usize, value: Logic, at: SimTime) {
        if self.pca_outputs[channel] == value {
            return;
        }
        self.pca_outputs[channel] = value;
        self.set_signal(
            self.pca_output_signals[channel],
            u64::from(value == Logic::One),
            1,
            at,
        );
    }

    fn advance_pca(&mut self, now: SimTime) -> Result<(), DeviceError> {
        let elapsed = now.ticks().saturating_sub(self.pca_epoch);
        if self.registers[PCA0CN] & PCA0CN_CR == 0 {
            self.pca_epoch = now.ticks();
            self.update_interrupt_signals(now);
            return Ok(());
        }
        let divider = self.pca_divider();
        let ticks = elapsed / divider;
        if ticks == 0 {
            self.update_interrupt_signals(now);
            return Ok(());
        }
        let start = self.pca_counter();
        let end = start.wrapping_add(ticks as u16);
        let overflow = u64::from(start) + ticks >= 0x1_0000;
        if overflow {
            self.registers[PCA0CN] |= PCA0CN_CF;
        }
        let cycle_bits = self.pca_cycle_bits();
        let cycle_modulus = 1_u64 << cycle_bits;
        if u64::from(start) + ticks >= cycle_modulus {
            self.registers[PCA0PWM] |= PCA0PWM_COVF;
        }
        for channel in 0..3 {
            let mode = self.registers[PCA0_CPM[channel]];
            let compare = u16::from_le_bytes([
                self.registers[PCA0_CPL[channel]],
                self.registers[PCA0_CPH[channel]],
            ]);
            let matched =
                mode & PCA0CPM_ECOM != 0 && Self::pca_crossed(start, ticks, compare, 0x1_0000);
            if matched && mode & PCA0CPM_MAT != 0 {
                self.registers[PCA0CN] |= PCA0_CCF[channel];
            }
            if matched && mode & PCA0CPM_TOG != 0 {
                let value = if self.pca_outputs[channel] == Logic::One {
                    Logic::Zero
                } else {
                    Logic::One
                };
                self.update_pca_output(channel, value, now);
            } else if mode & PCA0CPM_PWM != 0 && mode & PCA0CPM_TOG == 0 {
                let width = self.pca_width(channel, mode);
                let mask = (1_u32 << width) - 1;
                let duty = u32::from(compare) & mask;
                let count = u32::from(end) & mask;
                let mut high = count >= duty;
                if self.registers[PCA0POL] & (1 << channel) != 0 {
                    high = !high;
                }
                self.update_pca_output(channel, if high { Logic::One } else { Logic::Zero }, now);
            } else if mode & PCA0CPM_PWM == 0 && mode & PCA0CPM_TOG == 0 {
                self.update_pca_output(channel, Logic::Zero, now);
            }
        }
        self.set_pca_counter(end);
        self.pca_epoch = now.ticks().saturating_sub(elapsed % divider);
        self.update_interrupt_signals(now);
        Ok(())
    }

    fn capture_pca_input(
        &mut self,
        channel: usize,
        value: Logic,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        let Some(cpm_address) = PCA0_CPM.get(channel).copied() else {
            return Err(DeviceError::new(format!(
                "EFM8 PCA channel {channel} is outside 0..2"
            )));
        };
        self.advance_pca(at)?;
        let previous = self.pca_inputs[channel];
        self.pca_inputs[channel] = value;
        let rising = previous != Logic::One && value == Logic::One;
        let falling = previous != Logic::Zero && value == Logic::Zero;
        let mode = self.registers[cpm_address];
        if (rising && mode & PCA0CPM_CAPP != 0) || (falling && mode & PCA0CPM_CAPN != 0) {
            let counter = self.pca_counter();
            let [low, high] = counter.to_le_bytes();
            self.registers[PCA0_CPL[channel]] = low;
            self.registers[PCA0_CPH[channel]] = high;
            self.registers[PCA0CN] |= PCA0_CCF[channel];
        }
        self.update_interrupt_signals(at);
        Ok(())
    }
}

/// Machine-facing EFM8BB52F32G peripheral state.
#[derive(Clone)]
pub struct Efm8PeripheralsHandle(Arc<Mutex<Efm8State>>);

impl Efm8PeripheralsHandle {
    /// Captured UART0 transmit bytes.
    pub fn uart_bytes(&self) -> Vec<u8> {
        self.0.lock().expect("EFM8 lock poisoned").uart.clone()
    }

    /// Returns the resolved PCA CEX output for a channel.
    pub fn pca_output(&self, channel: usize) -> Logic {
        self.0
            .lock()
            .expect("EFM8 lock poisoned")
            .pca_outputs
            .get(channel)
            .copied()
            .unwrap_or(Logic::X)
    }

    /// Returns the current 16-bit PCA counter.
    pub fn pca_counter(&self) -> u16 {
        self.0.lock().expect("EFM8 lock poisoned").pca_counter()
    }

    /// Supplies a sampled CEX input edge for a capture channel.
    pub fn set_pca_input(
        &self,
        channel: usize,
        value: Logic,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        self.0
            .lock()
            .expect("EFM8 lock poisoned")
            .capture_pca_input(channel, value, at)
    }

    /// Returns the currently asserted PCA interrupt request.
    pub fn pca_interrupt_pending(&self) -> bool {
        self.0
            .lock()
            .expect("EFM8 lock poisoned")
            .pca_interrupt_pending()
    }

    /// Supplies one received UART0 byte and raises RI.
    pub fn inject_uart_rx(&self, value: u8, at: SimTime) {
        let mut state = self.0.lock().expect("EFM8 lock poisoned");
        state.registers[SBUF0] = value;
        state.registers[SCON0] |= SCON0_RI;
        state.update_interrupt_signals(at);
    }

    /// Supplies the next byte returned by a functional SPI0 master transfer.
    pub fn inject_spi_rx(&self, value: u8) {
        self.0
            .lock()
            .expect("EFM8 lock poisoned")
            .spi_rx
            .push(value);
    }

    /// Captured bytes written to SPI0DAT.
    pub fn spi_bytes(&self) -> Vec<u8> {
        self.0.lock().expect("EFM8 lock poisoned").spi_tx.clone()
    }

    /// Applies the native Timer1 side effect of vectoring to its interrupt.
    ///
    /// EFM8 hardware clears TF1 when the core acknowledges the Timer1
    /// interrupt. The machine calls this only after the MCS-51 core has
    /// actually selected the Timer1 vector, so a masked flag remains visible
    /// until it is serviced or explicitly cleared by firmware.
    pub fn acknowledge_timer1_interrupt(&self, at: SimTime) {
        let mut state = self.0.lock().expect("EFM8 lock poisoned");
        state.registers[TCON] &= !TCON_TF1;
        state.update_interrupt_signals(at);
    }

    /// Advances functional timers/watchdog and returns low/high CPU interrupt inputs.
    pub fn poll(&self, now: SimTime) -> [bool; 10] {
        let mut state = self.0.lock().expect("EFM8 lock poisoned");
        for port in 0..4 {
            let _ = state.refresh_port(port, now);
        }
        if state.registers[TCON] & TCON_TR0 != 0 {
            let initial = u16::from_be_bytes([state.registers[TH0], state.registers[TL0]]);
            let elapsed = now.ticks().saturating_sub(state.timer0_epoch);
            let total = u64::from(initial).saturating_add(elapsed);
            let mode = state.registers[TMOD] & 3;
            if mode == 2 {
                let reload = state.registers[TH0];
                let period = u64::from(256_u16 - u16::from(reload)).max(1);
                state.registers[TL0] = reload.wrapping_add((elapsed % period).to_le_bytes()[0]);
                if elapsed >= period {
                    state.registers[TCON] |= TCON_TF0;
                    state.timer0_epoch = now.ticks();
                }
            } else {
                let bytes = total.to_le_bytes();
                state.registers[TL0] = bytes[0];
                state.registers[TH0] = bytes[1];
                state.timer0_epoch = now.ticks();
                if total > u64::from(u16::MAX) {
                    state.registers[TCON] |= TCON_TF0;
                }
            }
        }
        if state.registers[TCON] & TCON_TR1 != 0 {
            let mode = (state.registers[TMOD] >> 4) & 3;
            let elapsed = now.ticks().saturating_sub(state.timer1_epoch);
            match mode {
                1 => {
                    let initial = u16::from_be_bytes([state.registers[TH1], state.registers[TL1]]);
                    let total = u64::from(initial).saturating_add(elapsed);
                    let [low, high] = (total as u16).to_le_bytes();
                    state.registers[TL1] = low;
                    state.registers[TH1] = high;
                    if total > u64::from(u16::MAX) {
                        state.registers[TCON] |= TCON_TF1;
                    }
                    state.timer1_epoch = now.ticks();
                }
                2 => {
                    // In auto-reload mode the first overflow depends on the
                    // current TL1 value. Subsequent overflows reload TH1.
                    let initial = u64::from(state.registers[TL1]);
                    let total = initial.saturating_add(elapsed);
                    let reload = state.registers[TH1];
                    let period = u64::from(256_u16 - u16::from(reload)).max(1);
                    if total >= 256 {
                        let after_first = total - 256;
                        state.registers[TL1] = reload.wrapping_add((after_first % period) as u8);
                        state.registers[TCON] |= TCON_TF1;
                    } else {
                        state.registers[TL1] = total as u8;
                    }
                    state.timer1_epoch = now.ticks();
                }
                // Mode 0 is the legacy 13-bit form and mode 3 leaves Timer1
                // inactive on the EFM8. Neither mode is part of this
                // functional slice; rebase time so changing modes while the
                // timer is running cannot count the unsupported interval.
                _ => state.timer1_epoch = now.ticks(),
            }
        }
        if state.registers[TMR2CN0] & TMR2_TR2 != 0 {
            let initial = u16::from_le_bytes([state.registers[TMR2L], state.registers[TMR2H]]);
            let elapsed = now.ticks().saturating_sub(state.timer2_epoch);
            let until_overflow = u64::from(u16::MAX - initial) + 1;
            if elapsed >= until_overflow {
                state.registers[TMR2CN0] |= TMR2_TF2H;
                state.registers[TMR2L] = state.registers[TMR2RLL];
                state.registers[TMR2H] = state.registers[TMR2RLH];
                state.timer2_epoch = now.ticks();
            } else {
                let elapsed = u16::try_from(elapsed)
                    .expect("non-overflowing Timer2 elapsed value fits in 16 bits");
                let value = initial.wrapping_add(elapsed);
                let [low, high] = value.to_le_bytes();
                state.registers[TMR2L] = low;
                state.registers[TMR2H] = high;
                state.timer2_epoch = now.ticks();
            }
        }
        if state.watchdog_enabled && now.ticks().saturating_sub(state.watchdog_epoch) >= 65_536 {
            state.watchdog_reset = true;
            state.set_signal(state.watchdog_reset_signal, 1, 1, now);
        }
        let _ = state.advance_pca(now);
        state.update_interrupt_signals(now);
        state.interrupt_levels()
    }

    /// Consumes a watchdog reset request.
    pub fn take_watchdog_reset(&self) -> bool {
        std::mem::take(&mut self.0.lock().expect("EFM8 lock poisoned").watchdog_reset)
    }
}

/// EFM8BB52F32G paged SFR peripheral window.
pub struct Efm8Peripherals {
    name: String,
    state: Arc<Mutex<Efm8State>>,
}

impl Efm8Peripherals {
    /// Creates the named functional slice and all 29 package GPIO handles.
    pub fn new(
        name: impl Into<String>,
        hub: SignalHub,
    ) -> Result<(Self, Efm8PeripheralsHandle, [GpioHandle; 4]), remu_signals::SignalError> {
        let (port0, signals0, handle0) = vendor_gpio(8, "board.efm8bb52f32g.port0", &hub)?;
        let (port1, signals1, handle1) = vendor_gpio(8, "board.efm8bb52f32g.port1", &hub)?;
        let (port2, signals2, handle2) = vendor_gpio(8, "board.efm8bb52f32g.port2", &hub)?;
        let (port3, signals3, handle3) = vendor_gpio(5, "board.efm8bb52f32g.port3", &hub)?;
        let uart_byte_signal = hub.declare(
            "board.efm8bb52f32g.uart0.tx_byte",
            SignalValue::from_u64(0, 8)?,
            Some("last UART0 transmit byte".to_owned()),
        )?;
        let uart_strobe_signal = hub.declare(
            "board.efm8bb52f32g.uart0.tx_strobe",
            SignalValue::from_u64(0, 1)?,
            Some("toggles for every UART0 transmit byte".to_owned()),
        )?;
        let timer0_irq_signal = hub.declare(
            "board.efm8bb52f32g.timer0.irq",
            SignalValue::from_u64(0, 1)?,
            Some("Timer0 overflow request".to_owned()),
        )?;
        let timer1_irq_signal = hub.declare(
            "board.efm8bb52f32g.timer1.irq",
            SignalValue::from_u64(0, 1)?,
            Some("Timer1 overflow request".to_owned()),
        )?;
        let timer2_irq_signal = hub.declare(
            "board.efm8bb52f32g.timer2.irq",
            SignalValue::from_u64(0, 1)?,
            Some("Timer2 high-byte overflow request".to_owned()),
        )?;
        let interrupt_signal = hub.declare(
            "board.efm8bb52f32g.interrupt.request",
            SignalValue::from_u64(0, 1)?,
            Some("combined enabled EFM8 interrupt request".to_owned()),
        )?;
        let watchdog_reset_signal = hub.declare(
            "board.efm8bb52f32g.watchdog.reset",
            SignalValue::from_u64(0, 1)?,
            Some("functional watchdog reset request".to_owned()),
        )?;
        let pca_output_signals = [
            hub.declare(
                "board.efm8bb52f32g.pca0.cex0",
                SignalValue::from_u64(0, 1)?,
                Some("PCA channel 0 CEX output".to_owned()),
            )?,
            hub.declare(
                "board.efm8bb52f32g.pca0.cex1",
                SignalValue::from_u64(0, 1)?,
                Some("PCA channel 1 CEX output".to_owned()),
            )?,
            hub.declare(
                "board.efm8bb52f32g.pca0.cex2",
                SignalValue::from_u64(0, 1)?,
                Some("PCA channel 2 CEX output".to_owned()),
            )?,
        ];
        let pca_interrupt_signal = hub.declare(
            "board.efm8bb52f32g.pca0.interrupt",
            SignalValue::from_u64(0, 1)?,
            Some("PCA capture/compare interrupt request".to_owned()),
        )?;
        let state = Arc::new(Mutex::new(Efm8State {
            registers: vec![0; SFR_BYTES].into_boxed_slice(),
            ports: [port0, port1, port2, port3],
            port_signals: [signals0, signals1, signals2, signals3],
            hub,
            uart: Vec::new(),
            timer0_epoch: 0,
            timer1_epoch: 0,
            timer2_epoch: 0,
            crc_result: 0,
            watchdog_epoch: 0,
            watchdog_key: 0,
            watchdog_enabled: true,
            watchdog_reset: false,
            spi_tx: Vec::new(),
            spi_rx: Vec::new(),
            uart_byte_signal,
            uart_strobe_signal,
            timer0_irq_signal,
            timer1_irq_signal,
            timer2_irq_signal,
            interrupt_signal,
            watchdog_reset_signal,
            pca_epoch: 0,
            pca_outputs: [Logic::Zero; 3],
            pca_inputs: [Logic::Zero; 3],
            pca_output_signals,
            pca_interrupt_signal,
        }));
        state
            .lock()
            .expect("new EFM8 lock poisoned")
            .reset_registers(SimTime::ZERO, ResetKind::PowerOn);
        Ok((
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Efm8PeripheralsHandle(state),
            [handle0, handle1, handle2, handle3],
        ))
    }

    fn port_index(address: usize) -> Option<usize> {
        PORTS.iter().position(|candidate| *candidate == address)
    }
}

impl Device for Efm8Peripherals {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Byte {
            return Err(DeviceError::new("EFM8 SFR space requires byte accesses"));
        }
        let raw = usize::try_from(offset).map_err(|_| DeviceError::new("EFM8 offset overflow"))?;
        let address = Efm8State::canonical(raw);
        let mut state = self.state.lock().expect("EFM8 lock poisoned");
        if matches!(
            address,
            PCA0CN
                | PCA0MD
                | PCA0CPM0
                | PCA0CPM1
                | PCA0CPM2
                | PCA0PWM
                | PCA0CENT
                | PCA0L
                | PCA0H
                | PCA0CPL0
                | PCA0CPH0
                | PCA0CPL1
                | PCA0CPH1
                | PCA0CPL2
                | PCA0CPH2
                | PCA0POL
        ) {
            state.advance_pca(at)?;
        }
        if let Some(port) = Self::port_index(address) {
            state.refresh_port(port, at)?;
            return Ok(u64::from(state.port_read(port)));
        }
        let mut value = if address == CRC0DAT {
            let value = if state.registers[CRC0CN0] & 1 == 0 {
                state.crc_result.to_le_bytes()[0]
            } else {
                state.crc_result.to_be_bytes()[0]
            };
            state.registers[CRC0CN0] ^= 1;
            value
        } else {
            *state
                .registers
                .get(address)
                .ok_or_else(|| DeviceError::new(format!("EFM8 read outside SFR space: {raw:#x}")))?
        };
        if address == CRC0CN0 {
            value &= CRC0CN0_MASK;
        }
        if address == CLKSEL {
            Ok(u64::from(value | 0x80))
        } else {
            Ok(u64::from(value))
        }
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Byte {
            return Err(DeviceError::new("EFM8 SFR space requires byte accesses"));
        }
        let raw = usize::try_from(offset).map_err(|_| DeviceError::new("EFM8 offset overflow"))?;
        let address = Efm8State::canonical(raw);
        let value = value.to_le_bytes()[0];
        let mut state = self.state.lock().expect("EFM8 lock poisoned");
        if address >= SFR_BYTES {
            return Err(DeviceError::new(format!(
                "EFM8 write outside SFR space: {raw:#x}"
            )));
        }
        let previous = state.registers[address];
        let pca_register = matches!(
            address,
            PCA0CN
                | PCA0MD
                | PCA0CPM0
                | PCA0CPM1
                | PCA0CPM2
                | PCA0PWM
                | PCA0CENT
                | PCA0L
                | PCA0H
                | PCA0CPL0
                | PCA0CPH0
                | PCA0CPL1
                | PCA0CPH1
                | PCA0CPL2
                | PCA0CPH2
                | PCA0POL
        );
        if pca_register {
            state.advance_pca(at)?;
        }
        if address == CRC0CN0 {
            state.registers[address] = value & CRC0CN0_MASK;
            if value & 0x08 != 0 {
                state.crc_result = if value & 0x04 != 0 { u16::MAX } else { 0 };
            }
        } else if address == CRC0IN {
            state.registers[address] = value;
            state.crc_result = crc16_ccitt(state.crc_result, value);
        } else if address == CRC0DAT {
            let [low, high] = state.crc_result.to_le_bytes();
            state.crc_result = if state.registers[CRC0CN0] & 1 == 0 {
                u16::from_le_bytes([value, high])
            } else {
                u16::from_le_bytes([low, value])
            };
            state.registers[CRC0CN0] ^= 1;
        } else if address == CRC0FLIP {
            state.registers[address] = reverse_bits(value);
        } else if let Some(port) = Self::port_index(address) {
            state.registers[address] = value;
            state.registers[address] &= PORT_MASKS[port];
            state.refresh_port(port, at)?;
        } else if let Some(port) = PORT_MDOUT.iter().position(|item| *item == address) {
            state.registers[address] = value;
            state.registers[address] &= PORT_MASKS[port];
            state.refresh_port(port, at)?;
        } else if address == PCA0CN {
            state.registers[address] = value;
            state.registers[address] &=
                PCA0CN_CF | PCA0CN_CR | PCA0CN_CCF0 | PCA0CN_CCF1 | PCA0CN_CCF2;
            state.pca_epoch = at.ticks();
        } else if address == PCA0L || address == PCA0H {
            state.registers[address] = value;
            state.pca_epoch = at.ticks();
        } else if let Some(channel) = PCA0_CPL.iter().position(|item| *item == address) {
            state.registers[address] = value;
            state.registers[PCA0_CPM[channel]] &= !PCA0CPM_ECOM;
        } else if let Some(channel) = PCA0_CPH.iter().position(|item| *item == address) {
            state.registers[address] = value;
            state.registers[PCA0_CPM[channel]] |= PCA0CPM_ECOM;
        } else if address == SBUF0 {
            state.registers[address] = value;
            if state.registers[XBR0] & XBR0_URT0E != 0 && state.registers[XBR2] & XBR2_XBARE != 0 {
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
            state.registers[SCON0] |= SCON0_TI;
        } else if address == SPI0CN0 {
            let tx_not_full = previous & SPI0_TXNF;
            state.registers[SPI0CN0] = (value & !SPI0_TXNF) | tx_not_full;
        } else if address == SPI0DAT {
            if state.registers[SPI0CN0] & SPI0_SPIEN != 0 {
                let received = if state.spi_rx.is_empty() {
                    value
                } else {
                    state.spi_rx.remove(0)
                };
                state.spi_tx.push(value);
                state.registers[SPI0DAT] = received;
                state.registers[SPI0CN0] |= SPI0_SPIF | SPI0_TXNF;
            }
        } else if address == WDTCN {
            state.registers[address] = value;
            if state.watchdog_key == 0xde && value == 0xad {
                state.watchdog_enabled = false;
            }
            state.watchdog_key = value;
            state.watchdog_epoch = at.ticks();
        } else if address == TCON {
            state.registers[address] = value;
            if value & TCON_TR0 != 0 {
                state.timer0_epoch = at.ticks();
            }
            if value & TCON_TR1 != 0 {
                state.timer1_epoch = at.ticks();
            }
        } else if address == TMOD {
            state.registers[address] = value;
            if state.registers[TCON] & TCON_TR0 != 0 {
                state.timer0_epoch = at.ticks();
            }
            if state.registers[TCON] & TCON_TR1 != 0 {
                state.timer1_epoch = at.ticks();
            }
        } else if (address == TL1 || address == TH1) && state.registers[TCON] & TCON_TR1 != 0 {
            state.registers[address] = value;
            state.timer1_epoch = at.ticks();
        } else if address == TMR2CN0 && value & TMR2_TR2 != 0 {
            state.registers[address] = value;
            state.timer2_epoch = at.ticks();
        } else {
            state.registers[address] = value;
        }
        state.update_interrupt_signals(at);
        Ok(())
    }

    fn reset(&mut self, kind: ResetKind) {
        self.state
            .lock()
            .expect("EFM8 lock poisoned")
            .reset_registers(SimTime::ZERO, kind);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AccessWidth, CRC0CN0, CRC0DAT, CRC0FLIP, CRC0IN, EIE1_EPCA0, Efm8PcaRegister,
        Efm8Peripherals, IE, IE_EA, IE_ESPI0, IE_ET0, IE_ET1, P0, P0MDOUT, PCA0CN_CR, SBUF0,
        SPI0_SPIEN, SPI0_TXNF, SPI0CN0, SPI0DAT, SimTime, TCON, TCON_TF1, TCON_TR0, TCON_TR1, TH1,
        TL1, TMOD, XBR0, XBR0_URT0E, XBR2, XBR2_XBARE,
    };
    use remu_bus::Device;
    use remu_signals::Logic;

    #[test]
    fn pca_register_ids_are_named_and_page_aliased() {
        assert_eq!(Efm8PcaRegister::Pca0Cn.address(), 0xd8);
        assert_eq!(Efm8PcaRegister::Pca0Cn.name(), "pca0cn");
        assert_eq!(
            Efm8PcaRegister::from_address(0x10d8),
            Some(Efm8PcaRegister::Pca0Cn)
        );
        assert_eq!(
            Efm8PcaRegister::from_address(Efm8PcaRegister::Eip1.address()),
            Some(Efm8PcaRegister::Eip1)
        );
        assert_eq!(Efm8PcaRegister::ALL.len(), 19);
    }

    #[test]
    fn gpio_timer_uart_and_interrupt_slice_is_functional() {
        let hub = super::SignalHub::new();
        let (mut device, handle, ports) = Efm8Peripherals::new("efm8.sfr", hub).unwrap();
        device
            .write(P0MDOUT as u64, AccessWidth::Byte, 1, SimTime::ZERO)
            .unwrap();
        device
            .write(P0 as u64, AccessWidth::Byte, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(ports[0].output() & 1, 1);

        device
            .write(
                XBR0 as u64,
                AccessWidth::Byte,
                XBR0_URT0E.into(),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                XBR2 as u64,
                AccessWidth::Byte,
                XBR2_XBARE.into(),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(SBUF0 as u64, AccessWidth::Byte, b'E'.into(), SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.uart_bytes(), b"E");

        device
            .write(TMOD as u64, AccessWidth::Byte, 2, SimTime::ZERO)
            .unwrap();
        device
            .write(super::TH0 as u64, AccessWidth::Byte, 0xfc, SimTime::ZERO)
            .unwrap();
        device
            .write(
                IE as u64,
                AccessWidth::Byte,
                (IE_EA | IE_ET0).into(),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                TCON as u64,
                AccessWidth::Byte,
                TCON_TR0.into(),
                SimTime::ZERO,
            )
            .unwrap();
        assert!(handle.poll(SimTime::from_ticks(4))[0]);
    }

    #[test]
    fn spi0_master_transfer_exposes_injected_miso_and_interrupt() {
        let hub = super::SignalHub::new();
        let (mut device, handle, _) = Efm8Peripherals::new("efm8.sfr", hub).unwrap();
        handle.inject_spi_rx(0x3c);
        device
            .write(
                SPI0CN0 as u64,
                AccessWidth::Byte,
                SPI0_SPIEN.into(),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                IE as u64,
                AccessWidth::Byte,
                (IE_EA | IE_ESPI0).into(),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(SPI0DAT as u64, AccessWidth::Byte, 0xa5, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.spi_bytes(), [0xa5]);
        assert!(handle.poll(SimTime::from_ticks(1))[6]);
        assert_eq!(
            device
                .read(SPI0DAT as u64, AccessWidth::Byte, SimTime::ZERO)
                .unwrap(),
            0x3c
        );
        assert_eq!(
            device
                .read((0x20_00 | SPI0DAT) as u64, AccessWidth::Byte, SimTime::ZERO)
                .unwrap(),
            0x3c
        );
        assert!(handle.poll(SimTime::from_ticks(1))[6]);
        device
            .write(
                SPI0CN0 as u64,
                AccessWidth::Byte,
                SPI0_SPIEN.into(),
                SimTime::ZERO,
            )
            .unwrap();
        assert!(!handle.poll(SimTime::from_ticks(1))[6]);
        assert_eq!(
            device
                .read(SPI0CN0 as u64, AccessWidth::Byte, SimTime::ZERO)
                .unwrap()
                & u64::from(SPI0_TXNF),
            u64::from(SPI0_TXNF)
        );
    }

    #[test]
    fn timer1_mode2_sets_its_dedicated_interrupt_line() {
        let hub = super::SignalHub::new();
        let (mut device, handle, _) = Efm8Peripherals::new("efm8.sfr", hub).unwrap();
        device
            .write(TMOD as u64, AccessWidth::Byte, 0x20, SimTime::ZERO)
            .unwrap();
        // The first overflow is measured from TL1; TH1 is only the reload
        // value after that overflow.
        device
            .write(TH1 as u64, AccessWidth::Byte, 0xfc, SimTime::ZERO)
            .unwrap();
        device
            .write(TL1 as u64, AccessWidth::Byte, 0xfc, SimTime::ZERO)
            .unwrap();
        device
            .write(
                IE as u64,
                AccessWidth::Byte,
                (IE_EA | IE_ET1).into(),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                TCON as u64,
                AccessWidth::Byte,
                TCON_TR1.into(),
                SimTime::ZERO,
            )
            .unwrap();

        let interrupts = handle.poll(SimTime::from_ticks(4));
        assert!(interrupts[8]);
        assert_eq!(
            device
                .read(TCON as u64, AccessWidth::Byte, SimTime::ZERO)
                .unwrap()
                & 0x80,
            0x80
        );
        assert_eq!(
            device
                .read(TL1 as u64, AccessWidth::Byte, SimTime::ZERO)
                .unwrap(),
            0xfc
        );

        handle.acknowledge_timer1_interrupt(SimTime::from_ticks(4));
        assert!(!handle.poll(SimTime::from_ticks(5))[8]);
        assert_eq!(
            device
                .read(TCON as u64, AccessWidth::Byte, SimTime::ZERO)
                .unwrap() as u8
                & TCON_TF1,
            0
        );
    }

    #[test]
    fn timer1_mode1_overflows_from_the_programmed_16_bit_value() {
        let hub = super::SignalHub::new();
        let (mut device, handle, _) = Efm8Peripherals::new("efm8.sfr", hub).unwrap();
        device
            .write(TMOD as u64, AccessWidth::Byte, 0x10, SimTime::ZERO)
            .unwrap();
        device
            .write(TH1 as u64, AccessWidth::Byte, 0xff, SimTime::ZERO)
            .unwrap();
        device
            .write(TL1 as u64, AccessWidth::Byte, 0xfe, SimTime::ZERO)
            .unwrap();
        device
            .write(
                IE as u64,
                AccessWidth::Byte,
                (IE_EA | IE_ET1).into(),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                TCON as u64,
                AccessWidth::Byte,
                TCON_TR1.into(),
                SimTime::ZERO,
            )
            .unwrap();

        assert!(!handle.poll(SimTime::from_ticks(1))[8]);
        assert!(handle.poll(SimTime::from_ticks(2))[8]);
        assert_eq!(
            device
                .read(TCON as u64, AccessWidth::Byte, SimTime::ZERO)
                .unwrap() as u8
                & TCON_TF1,
            TCON_TF1
        );
        assert_eq!(
            device
                .read(TL1 as u64, AccessWidth::Byte, SimTime::ZERO)
                .unwrap(),
            0
        );
        assert_eq!(
            device
                .read(TH1 as u64, AccessWidth::Byte, SimTime::ZERO)
                .unwrap(),
            0
        );
    }

    #[test]
    fn timer1_mode3_remains_inactive() {
        let hub = super::SignalHub::new();
        let (mut device, handle, _) = Efm8Peripherals::new("efm8.sfr", hub).unwrap();
        device
            .write(TMOD as u64, AccessWidth::Byte, 0x30, SimTime::ZERO)
            .unwrap();
        device
            .write(TH1 as u64, AccessWidth::Byte, 0xff, SimTime::ZERO)
            .unwrap();
        device
            .write(TL1 as u64, AccessWidth::Byte, 0xff, SimTime::ZERO)
            .unwrap();
        device
            .write(
                IE as u64,
                AccessWidth::Byte,
                (IE_EA | IE_ET1).into(),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                TCON as u64,
                AccessWidth::Byte,
                TCON_TR1.into(),
                SimTime::ZERO,
            )
            .unwrap();

        assert!(!handle.poll(SimTime::from_ticks(100_000))[8]);
        assert_eq!(
            device
                .read(TL1 as u64, AccessWidth::Byte, SimTime::ZERO)
                .unwrap(),
            0xff
        );
        assert_eq!(
            device
                .read(TH1 as u64, AccessWidth::Byte, SimTime::ZERO)
                .unwrap(),
            0xff
        );
        assert_eq!(
            device
                .read(TCON as u64, AccessWidth::Byte, SimTime::ZERO)
                .unwrap() as u8
                & TCON_TF1,
            0
        );
    }

    #[test]
    fn crc16_stream_and_bit_reverse_follow_efm8_register_contract() {
        let hub = super::SignalHub::new();
        let (mut device, _, _) = Efm8Peripherals::new("efm8.sfr", hub).unwrap();
        device
            .write(CRC0CN0 as u64, AccessWidth::Byte, 0x0c, SimTime::ZERO)
            .unwrap();
        for byte in [0xaa, 0xbb, 0xcc] {
            device
                .write(CRC0IN as u64, AccessWidth::Byte, byte, SimTime::ZERO)
                .unwrap();
        }
        assert_eq!(
            device
                .read(CRC0DAT as u64, AccessWidth::Byte, SimTime::ZERO)
                .unwrap(),
            0xf6
        );
        assert_eq!(
            device
                .read(CRC0DAT as u64, AccessWidth::Byte, SimTime::ZERO)
                .unwrap(),
            0x6c
        );
        device
            .write(CRC0FLIP as u64, AccessWidth::Byte, 0xc0, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            device
                .read(CRC0FLIP as u64, AccessWidth::Byte, SimTime::ZERO)
                .unwrap(),
            0x03
        );
    }

    #[test]
    fn crc0_control_masks_reserved_bits_and_supports_result_writes() {
        let hub = super::SignalHub::new();
        let (mut device, _, _) = Efm8Peripherals::new("efm8.sfr", hub).unwrap();
        device
            .write(CRC0CN0 as u64, AccessWidth::Byte, 0xff, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            device
                .read(CRC0CN0 as u64, AccessWidth::Byte, SimTime::ZERO)
                .unwrap(),
            0x05
        );

        device
            .write(CRC0CN0 as u64, AccessWidth::Byte, 0x0c, SimTime::ZERO)
            .unwrap();
        device
            .write(CRC0DAT as u64, AccessWidth::Byte, 0x34, SimTime::ZERO)
            .unwrap();
        device
            .write(CRC0DAT as u64, AccessWidth::Byte, 0x12, SimTime::ZERO)
            .unwrap();
        device
            .write(CRC0CN0 as u64, AccessWidth::Byte, 0x00, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            device
                .read(CRC0DAT as u64, AccessWidth::Byte, SimTime::ZERO)
                .unwrap(),
            0x34
        );
        assert_eq!(
            device
                .read(CRC0DAT as u64, AccessWidth::Byte, SimTime::ZERO)
                .unwrap(),
            0x12
        );
    }

    #[test]
    fn pca_pwm_capture_and_interrupt_slice_is_functional() {
        let hub = super::SignalHub::new();
        let (mut device, handle, _) = Efm8Peripherals::new("efm8.sfr", hub).unwrap();
        // Select SYSCLK as the abstract PCA timebase and configure an 8-bit PWM.
        device
            .write(
                Efm8PcaRegister::Pca0Md.address() as u64,
                AccessWidth::Byte,
                0x08,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Efm8PcaRegister::Pca0Pwm.address() as u64,
                AccessWidth::Byte,
                0,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Efm8PcaRegister::Pca0Cpm0.address() as u64,
                AccessWidth::Byte,
                0x02,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Efm8PcaRegister::Pca0Cpl0.address() as u64,
                AccessWidth::Byte,
                0x40,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Efm8PcaRegister::Pca0Cph0.address() as u64,
                AccessWidth::Byte,
                0,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Efm8PcaRegister::Pca0Cn.address() as u64,
                AccessWidth::Byte,
                PCA0CN_CR.into(),
                SimTime::ZERO,
            )
            .unwrap();
        assert!(!handle.poll(SimTime::from_ticks(0))[0]);
        assert!(!handle.poll(SimTime::from_ticks(0x40))[0]);
        assert_eq!(handle.pca_output(0), Logic::One);
        assert_eq!(handle.pca_counter(), 0x40);
        assert!(!handle.poll(SimTime::from_ticks(0x100))[0]);
        assert_eq!(handle.pca_output(0), Logic::Zero);

        // A channel compare and an input capture share the PCA request line.
        device
            .write(
                Efm8PcaRegister::Pca0Cpm0.address() as u64,
                AccessWidth::Byte,
                0x49,
                SimTime::from_ticks(0x100),
            )
            .unwrap();
        device
            .write(
                Efm8PcaRegister::Pca0Cpl0.address() as u64,
                AccessWidth::Byte,
                2,
                SimTime::from_ticks(0x100),
            )
            .unwrap();
        device
            .write(
                Efm8PcaRegister::Pca0Cph0.address() as u64,
                AccessWidth::Byte,
                1,
                SimTime::from_ticks(0x100),
            )
            .unwrap();
        device
            .write(
                Efm8PcaRegister::Eie1.address() as u64,
                AccessWidth::Byte,
                EIE1_EPCA0.into(),
                SimTime::from_ticks(0x100),
            )
            .unwrap();
        device
            .write(
                IE as u64,
                AccessWidth::Byte,
                IE_EA.into(),
                SimTime::from_ticks(0x100),
            )
            .unwrap();
        let levels = handle.poll(SimTime::from_ticks(0x104));
        assert!(levels[6]);
        assert!(handle.pca_interrupt_pending());

        device
            .write(
                Efm8PcaRegister::Pca0Cpm1.address() as u64,
                AccessWidth::Byte,
                0x21,
                SimTime::from_ticks(0x104),
            )
            .unwrap();
        handle
            .set_pca_input(1, Logic::One, SimTime::from_ticks(0x108))
            .unwrap();
        assert_eq!(
            device
                .read(
                    Efm8PcaRegister::Pca0Cpl1.address() as u64,
                    AccessWidth::Byte,
                    SimTime::from_ticks(0x108)
                )
                .unwrap(),
            0x08
        );
        assert_eq!(
            device
                .read(
                    Efm8PcaRegister::Pca0Cph1.address() as u64,
                    AccessWidth::Byte,
                    SimTime::from_ticks(0x108)
                )
                .unwrap(),
            1
        );
    }
}
