use super::{GpioHandle, GpioState, SignalHub, refresh_gpio, vendor_gpio};
mod adc;
mod dac;
mod registers;
use adc::*;
use dac::*;
pub use registers::{Efm8PcaRegister, Efm8SmbusRegister};
use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use remu_signals::{Logic, SignalId, SignalValue};
use std::collections::VecDeque;
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
const UART1_PAGE: usize = 0x20 << 8;
const SCON1: usize = UART1_PAGE | 0xc8;
const SBUF1: usize = UART1_PAGE | 0x92;
const SBCON1: usize = UART1_PAGE | 0x94;
const UART1FCN0: usize = UART1_PAGE | 0x9d;
const UART1FCN1: usize = UART1_PAGE | 0xd8;
const UART1FCT: usize = UART1_PAGE | 0xfa;
const EIE2: usize = 0xf3;
const EIE2_PAGE10: usize = (0x10 << 8) | 0xf3;
const EIP2: usize = (0x10 << 8) | 0xed;
const EIP2H: usize = (0x10 << 8) | 0xf6;
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
const TMR3CN0: usize = 0x91;
const TMR3RLL: usize = 0x92;
const TMR3RLH: usize = 0x93;
const TMR3L: usize = 0x94;
const TMR3H: usize = 0x95;
const TMR3CN1: usize = (0x10 << 8) | 0xfe;
const TMR4RLL: usize = (0x10 << 8) | 0xa2;
const TMR4RLH: usize = (0x10 << 8) | 0xa3;
const TMR4L: usize = (0x10 << 8) | 0xa4;
const TMR4H: usize = (0x10 << 8) | 0xa5;
const TMR4CN0: usize = (0x10 << 8) | 0x98;
const TMR4CN1: usize = (0x10 << 8) | 0xff;
const TMR5RLL: usize = (0x10 << 8) | 0xd2;
const TMR5RLH: usize = (0x10 << 8) | 0xd3;
const TMR5L: usize = (0x10 << 8) | 0xd4;
const TMR5H: usize = (0x10 << 8) | 0xd5;
const TMR5CN0: usize = (0x10 << 8) | 0xc0;
const TMR5CN1: usize = (0x10 << 8) | 0xf1;
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
const TMR3_TR3: u8 = 0x04;
const TMR3_TF3L: u8 = 0x40;
const TMR3_TF3H: u8 = 0x80;
const TMR3_TF3LEN: u8 = 0x20;
const TMR3_TF3CEN: u8 = 0x10;
const TMR4_TR4: u8 = 0x04;
const TMR4_TF4L: u8 = 0x40;
const TMR4_TF4H: u8 = 0x80;
const TMR4_TF4LEN: u8 = 0x20;
const TMR4_TF4CEN: u8 = 0x10;
const TMR5_TR5: u8 = 0x04;
const TMR5_TF5L: u8 = 0x40;
const TMR5_TF5H: u8 = 0x80;
const TMR5_TF5LEN: u8 = 0x20;
const TMR5_TF5CEN: u8 = 0x10;
const EIE1_ET3: u8 = 0x80;
const EIE2_ET4: u8 = 0x04;
const EIE2_ET5: u8 = 0x08;
const SCON0_RI: u8 = 0x01;
const SCON0_TI: u8 = 0x02;
const SCON1_RI: u8 = 0x01;
const SCON1_TI: u8 = 0x02;
const SCON1_REN: u8 = 0x10;
const SBCON1_BREN: u8 = 0x40;
const UART1FCN0_TFLSH: u8 = 0x40;
const UART1FCN0_RFLSH: u8 = 0x04;
const UART1FCN1_TFRQ: u8 = 0x80;
const UART1FCN1_TXNF: u8 = 0x40;
const UART1FCN1_RFRQ: u8 = 0x08;
const SPI0_SPIF: u8 = 0x80;
const SPI0_TXNF: u8 = 0x02;
const SPI0_SPIEN: u8 = 0x01;
const SMB0CN0_MASTER: u8 = 1 << 7;
const SMB0CN0_TXMODE: u8 = 1 << 6;
const SMB0CN0_STA: u8 = 1 << 5;
const SMB0CN0_STO: u8 = 1 << 4;
const SMB0CN0_ACKRQ: u8 = 1 << 3;
const SMB0CN0_ARBLOST: u8 = 1 << 2;
const SMB0CN0_ACK: u8 = 1 << 1;
const SMB0CN0_SI: u8 = 1;
const SMB0CF_ENSMB: u8 = 1 << 7;
const SMB0CF_INH: u8 = 1 << 6;
const SMB0CF_BUSY: u8 = 1 << 5;
const EIE1_ESMB0: u8 = 1;
const EIP1_PSMB0: u8 = 1;
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
    uart1: Vec<u8>,
    uart1_rx: VecDeque<u8>,
    uart1_last_rx: u8,
    adc_inputs: [u16; 32],
    smbus0_tx: Vec<u8>,
    smbus0_tx_fifo: VecDeque<u8>,
    smbus0_rx: VecDeque<u8>,
    timer0_epoch: u64,
    timer1_epoch: u64,
    timer2_epoch: u64,
    timer3_epoch: u64,
    timer4_epoch: u64,
    timer5_epoch: u64,
    crc_result: u16,
    watchdog_epoch: u64,
    watchdog_key: u8,
    watchdog_enabled: bool,
    watchdog_reset: bool,
    dac_output: u16,
    dac_update_inhibited: bool,
    spi_tx: Vec<u8>,
    spi_rx: Vec<u8>,
    uart_byte_signal: SignalId,
    uart_strobe_signal: SignalId,
    uart1_byte_signal: SignalId,
    uart1_strobe_signal: SignalId,
    adc_result_signal: SignalId,
    adc_eoc_signal: SignalId,
    adc_window_signal: SignalId,
    smbus0_tx_byte_signal: SignalId,
    smbus0_tx_strobe_signal: SignalId,
    smbus0_busy_signal: SignalId,
    smbus0_interrupt_signal: SignalId,
    timer0_irq_signal: SignalId,
    timer1_irq_signal: SignalId,
    timer2_irq_signal: SignalId,
    timer3_irq_signal: SignalId,
    timer4_irq_signal: SignalId,
    timer5_irq_signal: SignalId,
    interrupt_signal: SignalId,
    watchdog_reset_signal: SignalId,
    dac_output_signal: SignalId,
    dac_enabled_signal: SignalId,
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

    fn update_smbus0_signals(&self, at: SimTime) {
        let enabled = self.registers[Efm8SmbusRegister::Smb0Cf.offset()] & SMB0CF_ENSMB != 0;
        let busy = enabled && self.registers[Efm8SmbusRegister::Smb0Cf.offset()] & SMB0CF_BUSY != 0;
        let master = self.registers[Efm8SmbusRegister::Smb0Cn0.offset()] & SMB0CN0_MASTER != 0;
        let interrupt = enabled
            && self.registers[Efm8SmbusRegister::Eie1.offset()] & EIE1_ESMB0 != 0
            && self.registers[Efm8SmbusRegister::Smb0Cn0.offset()] & SMB0CN0_SI != 0
            && (master || self.registers[Efm8SmbusRegister::Smb0Cf.offset()] & SMB0CF_INH == 0);
        self.set_signal(self.smbus0_busy_signal, u64::from(busy), 1, at);
        self.set_signal(self.smbus0_interrupt_signal, u64::from(interrupt), 1, at);
    }

    fn smbus0_start(&mut self) {
        self.registers[Efm8SmbusRegister::Smb0Cf.offset()] |= SMB0CF_BUSY;
        self.registers[Efm8SmbusRegister::Smb0Cn0.offset()] |=
            SMB0CN0_MASTER | SMB0CN0_STA | SMB0CN0_SI;
        self.registers[Efm8SmbusRegister::Smb0Cn0.offset()] &= !SMB0CN0_STO;
    }

    fn smbus0_stop(&mut self) {
        self.registers[Efm8SmbusRegister::Smb0Cf.offset()] &= !SMB0CF_BUSY;
        self.registers[Efm8SmbusRegister::Smb0Cn0.offset()] &=
            !(SMB0CN0_MASTER | SMB0CN0_TXMODE | SMB0CN0_STA | SMB0CN0_STO | SMB0CN0_ACKRQ);
    }

    fn smbus0_fcn1(&self) -> u8 {
        let control = self.registers[Efm8SmbusRegister::Smb0Fcn0.offset()];
        let tx_threshold = (control >> 4) & 0x03;
        let tfrq = self.smbus0_tx_fifo.len() <= usize::from(tx_threshold);
        let txnf = true;
        let rx_threshold = control & 0x03;
        let rfrq = self.smbus0_rx.len() > usize::from(rx_threshold);
        let rxe = self.smbus0_rx.is_empty();
        (u8::from(tfrq) << 7) | (u8::from(txnf) << 6) | (u8::from(rfrq) << 3) | (u8::from(rxe) << 2)
    }

    fn smbus0_fct(&self) -> u8 {
        (u8::from(!self.smbus0_tx_fifo.is_empty()) << 4) | u8::from(!self.smbus0_rx.is_empty())
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
        self.uart1.clear();
        self.uart1_rx.clear();
        self.uart1_last_rx = 0;
        self.adc_inputs.fill(0);
        self.registers[ADC0MX] = 0x1f;
        self.registers[ADC0CF2] = 0x1f;
        self.registers[ADC0GTH] = 0xff;
        self.registers[ADC0GTL] = 0xff;
        self.registers[UART1FCN1] = UART1FCN1_TFRQ | UART1FCN1_TXNF | 0x10 | 0x01;
        self.smbus0_tx.clear();
        self.smbus0_tx_fifo.clear();
        self.smbus0_rx.clear();
        self.registers[Efm8SmbusRegister::Smb0Adm.offset()] = 0x7f;
        self.timer0_epoch = at.ticks();
        self.timer1_epoch = at.ticks();
        self.timer2_epoch = at.ticks();
        self.timer3_epoch = at.ticks();
        self.timer4_epoch = at.ticks();
        self.timer5_epoch = at.ticks();
        self.crc_result = 0;
        self.watchdog_epoch = at.ticks();
        self.watchdog_key = 0;
        self.watchdog_enabled = true;
        self.watchdog_reset = false;
        self.dac_output = 0;
        self.dac_update_inhibited = false;
        self.spi_tx.clear();
        self.spi_rx.clear();
        self.registers[SPI0CN0] = SPI0_TXNF;
        self.pca_epoch = at.ticks();
        self.pca_outputs = [Logic::Zero; 3];
        self.pca_inputs = [Logic::Zero; 3];
        for signal in [
            self.uart_strobe_signal,
            self.uart1_strobe_signal,
            self.adc_eoc_signal,
            self.adc_window_signal,
            self.smbus0_tx_strobe_signal,
            self.timer0_irq_signal,
            self.timer1_irq_signal,
            self.timer2_irq_signal,
            self.timer3_irq_signal,
            self.timer4_irq_signal,
            self.timer5_irq_signal,
            self.interrupt_signal,
            self.watchdog_reset_signal,
            self.dac_enabled_signal,
            self.pca_output_signals[0],
            self.pca_output_signals[1],
            self.pca_output_signals[2],
            self.pca_interrupt_signal,
        ] {
            self.set_signal(signal, 0, 1, at);
        }
        self.set_signal(self.smbus0_tx_byte_signal, 0, 8, at);
        self.set_signal(self.adc_result_signal, 0, 16, at);
        self.set_signal(self.dac_output_signal, 0, 10, at);
        self.update_smbus0_signals(at);
        for port in 0..4 {
            let _ = self.refresh_port(port, at);
        }
    }

    fn canonical(raw: usize) -> usize {
        if let Some(register) = Efm8SmbusRegister::from_data_address(raw) {
            return register.offset();
        }
        let page = raw >> 8;
        let address = raw & 0xff;
        if matches!(raw, DAC0L | DAC0H | DAC0ALT | DAC0CF0 | DAC0CF1) {
            return raw;
        }
        if page == 0x10 {
            if (0x91..=0x95).contains(&address) {
                return address;
            }
            if matches!(
                raw,
                TMR3CN1
                    | TMR4CN0
                    | TMR4RLL
                    | TMR4RLH
                    | TMR4L
                    | TMR4H
                    | TMR4CN1
                    | TMR5RLL
                    | TMR5RLH
                    | TMR5L
                    | TMR5H
                    | TMR5CN0
                    | TMR5CN1
            ) {
                return raw;
            }
        }
        if page == 0x30
            && matches!(
                address,
                ADC0CN1
                    | ADC0CN2
                    | ADC0CF1
                    | ADC0MX
                    | ADC0L..=ADC0H
                    | ADC0GTL..=ADC0LTH
                    | ADC0CF2
                    | ADC0CN0
            )
        {
            return address;
        }
        if page == 0
            && matches!(
                address,
                ADC0CN1
                    | ADC0CN2
                    | ADC0CF1
                    | ADC0MX
                    | ADC0L..=ADC0H
                    | ADC0GTL..=ADC0LTH
                    | ADC0CF2
                    | ADC0CN0
            )
        {
            return address;
        }
        if page == (UART1_PAGE >> 8) && matches!(address, 0x92 | 0x94 | 0x9d | 0xc8 | 0xd8 | 0xfa) {
            return raw;
        }
        if page == PAGE3
            && matches!(
                address,
                0x86 | 0x9c | 0xca..=0xcf | 0xd2..=0xd3 | 0xef | 0xf4
            )
        {
            return raw;
        }
        match address {
            0x80
            | 0x88..=0x8e
            | 0x90
            | 0x91..=0x95
            | SPI0CFG
            | 0x97..=0x99
            | 0xa0
            | SPI0DAT
            | 0xa4..=0xa6
            | 0xa8..=0xa9
            | SPI0CKR
            | 0xac
            | 0xb0
            | 0xb8
            | 0xc0..=0xc2
            | 0xc8
            | 0xca..=0xcf
            | 0xd4..=0xdc
            | 0xe1..=0xe3
            | 0xe6
            | 0xe9..=0xec
            | 0xef
            | 0xf1..=0xf3
            | 0xf7..=0xfc => address,
            0x9c | 0xc3..=0xc5 | 0xf4 if page == PAGE3 => (PAGE3 << 8) | address,
            _ => raw,
        }
    }

    fn interrupt_levels(&self) -> [bool; 24] {
        let enabled = self.registers[IE];
        if enabled & IE_EA == 0 {
            return [false; 24];
        }
        let active = [
            enabled & IE_ET0 != 0 && self.registers[TCON] & TCON_TF0 != 0,
            enabled & IE_ES0 != 0 && self.registers[SCON0] & (SCON0_RI | SCON0_TI) != 0,
            enabled & IE_ET2 != 0 && self.registers[TMR2CN0] & TMR2_TF2H != 0,
            enabled & IE_ESPI0 != 0 && self.registers[SPI0CN0] & SPI0_SPIF != 0,
            enabled & IE_ET1 != 0 && self.registers[TCON] & TCON_TF1 != 0,
            self.registers[Efm8SmbusRegister::Eie1.offset()] & EIE1_ESMB0 != 0
                && self.registers[Efm8SmbusRegister::Smb0Cn0.offset()] & SMB0CN0_SI != 0
                && (self.registers[Efm8SmbusRegister::Smb0Cn0.offset()] & SMB0CN0_MASTER != 0
                    || self.registers[Efm8SmbusRegister::Smb0Cf.offset()] & SMB0CF_INH == 0),
        ];
        let priorities = [
            self.registers[IP] & IE_ET0 != 0,
            self.registers[IP] & IE_ES0 != 0,
            self.registers[IP] & IE_ET2 != 0,
            self.registers[IP] & IE_ESPI0 != 0,
            self.registers[IP] & IE_ET1 != 0,
            self.registers[Efm8SmbusRegister::Eip1.offset()] & EIP1_PSMB0 != 0,
        ];
        const LOW_LINES: [usize; 6] = [0, 1, 2, 6, 8, 10];
        const HIGH_LINES: [usize; 6] = [3, 4, 5, 7, 9, 11];
        let mut levels = [false; 24];
        for source in 0..active.len() {
            if active[source] {
                levels[if priorities[source] {
                    HIGH_LINES[source]
                } else {
                    LOW_LINES[source]
                }] = true;
            }
        }
        if self.pca_interrupt_pending() {
            levels[if self.pca_high_priority() { 7 } else { 6 }] = true;
        }
        let uart1_pending = self.registers[SCON1] & (SCON1_RI | SCON1_TI) != 0;
        let uart1_enabled = self.registers[EIE2] & 1 != 0 || self.registers[EIE2_PAGE10] & 1 != 0;
        if uart1_pending && uart1_enabled {
            let high = self.registers[EIP2] & 1 != 0 || self.registers[EIP2H] & 1 != 0;
            levels[12 + usize::from(high)] = true;
        }
        let timer3 = self.registers[EIE1] & EIE1_ET3 != 0
            && ((self.registers[TMR3CN0] & TMR3_TF3H != 0
                && self.registers[TMR3CN0] & TMR3_TF3CEN != 0)
                || (self.registers[TMR3CN0] & TMR3_TF3L != 0
                    && self.registers[TMR3CN0] & TMR3_TF3LEN != 0));
        let timer4 = self.registers[EIE2] & EIE2_ET4 != 0
            && ((self.registers[TMR4CN0] & TMR4_TF4H != 0
                && self.registers[TMR4CN0] & TMR4_TF4CEN != 0)
                || (self.registers[TMR4CN0] & TMR4_TF4L != 0
                    && self.registers[TMR4CN0] & TMR4_TF4LEN != 0));
        let timer5 = self.registers[EIE2] & EIE2_ET5 != 0
            && ((self.registers[TMR5CN0] & TMR5_TF5H != 0
                && self.registers[TMR5CN0] & TMR5_TF5CEN != 0)
                || (self.registers[TMR5CN0] & TMR5_TF5L != 0
                    && self.registers[TMR5CN0] & TMR5_TF5LEN != 0));
        let timer3_high = self.registers[EIP1] & 0x80 != 0 || self.registers[EIP1H] & 0x80 != 0;
        let timer4_high = self.registers[EIP2] & 0x04 != 0 || self.registers[EIP2H] & 0x04 != 0;
        let timer5_high = self.registers[EIP2] & 0x08 != 0 || self.registers[EIP2H] & 0x08 != 0;
        if timer3 {
            levels[14 + usize::from(timer3_high)] = true;
        }
        if timer4 {
            levels[16 + usize::from(timer4_high)] = true;
        }
        if timer5 {
            levels[18 + usize::from(timer5_high)] = true;
        }
        let adc_window =
            self.registers[EIE1] & ADC0_EWADC0 != 0 && self.registers[ADC0CN0] & ADC0_ADWINT != 0;
        let adc_complete =
            self.registers[EIE1] & ADC0_EADC0 != 0 && self.registers[ADC0CN0] & ADC0_ADINT != 0;
        let adc_window_high = self.registers[EIP1] & 0x04 != 0 || self.registers[EIP1H] & 0x04 != 0;
        let adc_complete_high =
            self.registers[EIP1] & 0x08 != 0 || self.registers[EIP1H] & 0x08 != 0;
        if adc_window {
            levels[20 + usize::from(adc_window_high)] = true;
        }
        if adc_complete {
            levels[22 + usize::from(adc_complete_high)] = true;
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
            self.timer3_irq_signal,
            u64::from(self.registers[TMR3CN0] & (TMR3_TF3L | TMR3_TF3H) != 0),
            1,
            at,
        );
        self.set_signal(
            self.timer4_irq_signal,
            u64::from(self.registers[TMR4CN0] & (TMR4_TF4L | TMR4_TF4H) != 0),
            1,
            at,
        );
        self.set_signal(
            self.timer5_irq_signal,
            u64::from(self.registers[TMR5CN0] & (TMR5_TF5L | TMR5_TF5H) != 0),
            1,
            at,
        );
        self.set_signal(
            self.adc_eoc_signal,
            u64::from(self.registers[ADC0CN0] & ADC0_ADINT != 0),
            1,
            at,
        );
        self.set_signal(
            self.adc_window_signal,
            u64::from(self.registers[ADC0CN0] & ADC0_ADWINT != 0),
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

mod handle;

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
        let uart1_byte_signal = hub.declare(
            "board.efm8bb52f32g.uart1.tx_byte",
            SignalValue::from_u64(0, 8)?,
            Some("last UART1 transmit byte".to_owned()),
        )?;
        let uart1_strobe_signal = hub.declare(
            "board.efm8bb52f32g.uart1.tx_strobe",
            SignalValue::from_u64(0, 1)?,
            Some("toggles for every UART1 transmit byte".to_owned()),
        )?;
        let smbus0_tx_byte_signal = hub.declare(
            "board.efm8bb52f32g.smb0.tx_byte",
            SignalValue::from_u64(0, 8)?,
            Some("last SMBus 0 byte written by the guest".to_owned()),
        )?;
        let smbus0_tx_strobe_signal = hub.declare(
            "board.efm8bb52f32g.smb0.tx_strobe",
            SignalValue::from_u64(0, 1)?,
            Some("toggles for every SMBus 0 transmit byte".to_owned()),
        )?;
        let smbus0_busy_signal = hub.declare(
            "board.efm8bb52f32g.smb0.busy",
            SignalValue::from_u64(0, 1)?,
            Some("functional SMBus 0 bus-busy state".to_owned()),
        )?;
        let smbus0_interrupt_signal = hub.declare(
            "board.efm8bb52f32g.smb0.interrupt",
            SignalValue::from_u64(0, 1)?,
            Some("enabled SMBus 0 service request".to_owned()),
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
        let timer3_irq_signal = hub.declare(
            "board.efm8bb52f32g.timer3.irq",
            SignalValue::from_u64(0, 1)?,
            Some("Timer3 overflow request".to_owned()),
        )?;
        let timer4_irq_signal = hub.declare(
            "board.efm8bb52f32g.timer4.irq",
            SignalValue::from_u64(0, 1)?,
            Some("Timer4 overflow request".to_owned()),
        )?;
        let timer5_irq_signal = hub.declare(
            "board.efm8bb52f32g.timer5.irq",
            SignalValue::from_u64(0, 1)?,
            Some("Timer5 overflow request".to_owned()),
        )?;
        let adc_result_signal = hub.declare(
            "board.efm8bb52f32g.adc0.result",
            SignalValue::from_u64(0, 16)?,
            Some("last ADC0 conversion result".to_owned()),
        )?;
        let adc_eoc_signal = hub.declare(
            "board.efm8bb52f32g.adc0.end_of_conversion",
            SignalValue::from_u64(0, 1)?,
            Some("ADC0 conversion-complete flag".to_owned()),
        )?;
        let adc_window_signal = hub.declare(
            "board.efm8bb52f32g.adc0.window",
            SignalValue::from_u64(0, 1)?,
            Some("ADC0 window-comparison flag".to_owned()),
        )?;
        let dac_output_signal = hub.declare(
            "board.efm8bb52f32g.dac0.output",
            SignalValue::from_u64(0, 10)?,
            Some("last DAC0 digital output code".to_owned()),
        )?;
        let dac_enabled_signal = hub.declare(
            "board.efm8bb52f32g.dac0.enabled",
            SignalValue::from_u64(0, 1)?,
            Some("DAC0 output buffer enable".to_owned()),
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
            uart1: Vec::new(),
            uart1_rx: VecDeque::new(),
            uart1_last_rx: 0,
            adc_inputs: [0; 32],
            smbus0_tx: Vec::new(),
            smbus0_tx_fifo: VecDeque::new(),
            smbus0_rx: VecDeque::new(),
            timer0_epoch: 0,
            timer1_epoch: 0,
            timer2_epoch: 0,
            timer3_epoch: 0,
            timer4_epoch: 0,
            timer5_epoch: 0,
            crc_result: 0,
            watchdog_epoch: 0,
            watchdog_key: 0,
            watchdog_enabled: true,
            watchdog_reset: false,
            dac_output: 0,
            dac_update_inhibited: false,
            spi_tx: Vec::new(),
            spi_rx: Vec::new(),
            uart_byte_signal,
            uart_strobe_signal,
            uart1_byte_signal,
            uart1_strobe_signal,
            adc_result_signal,
            adc_eoc_signal,
            adc_window_signal,
            smbus0_tx_byte_signal,
            smbus0_tx_strobe_signal,
            smbus0_busy_signal,
            smbus0_interrupt_signal,
            timer0_irq_signal,
            timer1_irq_signal,
            timer2_irq_signal,
            timer3_irq_signal,
            timer4_irq_signal,
            timer5_irq_signal,
            interrupt_signal,
            watchdog_reset_signal,
            dac_output_signal,
            dac_enabled_signal,
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
        if address == SBUF1 {
            let value = state.uart1_rx.pop_front().unwrap_or(state.uart1_last_rx);
            state.uart1_last_rx = value;
            if state.uart1_rx.is_empty() {
                state.registers[SCON1] &= !SCON1_RI;
            }
            state.update_interrupt_signals(at);
            return Ok(u64::from(value));
        }
        if address == UART1FCN1 {
            let mut value = state.registers[UART1FCN1] & 0x37;
            value |= UART1FCN1_TFRQ | UART1FCN1_TXNF;
            if !state.uart1_rx.is_empty() {
                value |= UART1FCN1_RFRQ;
            }
            return Ok(u64::from(value));
        }
        if address == UART1FCT {
            let rx_count = u8::try_from(state.uart1_rx.len().min(7))
                .expect("bounded UART1 receive FIFO count fits in three bits");
            return Ok(u64::from(rx_count));
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
            match Efm8SmbusRegister::from_data_address(address) {
                Some(Efm8SmbusRegister::Smb0Dat) => {
                    let value = state.registers[Efm8SmbusRegister::Smb0Dat.offset()];
                    state.smbus0_rx.pop_front();
                    if let Some(&next) = state.smbus0_rx.front() {
                        state.registers[Efm8SmbusRegister::Smb0Dat.offset()] = next;
                    } else {
                        state.registers[Efm8SmbusRegister::Smb0Cn0.offset()] &= !SMB0CN0_ACKRQ;
                    }
                    state.update_smbus0_signals(at);
                    value
                }
                Some(Efm8SmbusRegister::Smb0Fcn1) => state.smbus0_fcn1(),
                Some(Efm8SmbusRegister::Smb0Fct) => state.smbus0_fct(),
                _ => *state.registers.get(address).ok_or_else(|| {
                    DeviceError::new(format!("EFM8 read outside SFR space: {raw:#x}"))
                })?,
            }
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
        let smbus_register = Efm8SmbusRegister::from_data_address(address);
        match smbus_register {
            Some(Efm8SmbusRegister::Smb0Cf) => {
                let busy = state.registers[Efm8SmbusRegister::Smb0Cf.offset()] & SMB0CF_BUSY;
                state.registers[Efm8SmbusRegister::Smb0Cf.offset()] = (value & !SMB0CF_BUSY) | busy;
                if value & SMB0CF_ENSMB == 0 {
                    state.smbus0_stop();
                    state.registers[Efm8SmbusRegister::Smb0Cn0.offset()] &= !SMB0CN0_SI;
                }
            }
            Some(Efm8SmbusRegister::Smb0Tc) => {
                state.registers[Efm8SmbusRegister::Smb0Tc.offset()] = value & 0x93;
            }
            Some(Efm8SmbusRegister::Smb0Adr) => {
                state.registers[Efm8SmbusRegister::Smb0Adr.offset()] = value;
            }
            Some(Efm8SmbusRegister::Smb0Adm) => {
                state.registers[Efm8SmbusRegister::Smb0Adm.offset()] = value;
            }
            Some(Efm8SmbusRegister::Smb0Fcn0) => {
                let flush_tx = value & (1 << 6) != 0;
                let flush_rx = value & (1 << 2) != 0;
                state.registers[Efm8SmbusRegister::Smb0Fcn0.offset()] = value & !(1 << 6 | 1 << 2);
                if flush_tx {
                    state.smbus0_tx_fifo.clear();
                }
                if flush_rx {
                    state.smbus0_rx.clear();
                    state.registers[Efm8SmbusRegister::Smb0Cn0.offset()] &= !SMB0CN0_ACKRQ;
                }
            }
            Some(Efm8SmbusRegister::Smb0Fcn1) | Some(Efm8SmbusRegister::Smb0Fct) => {
                // Both registers are read-only status surfaces on this device.
            }
            Some(Efm8SmbusRegister::Smb0Rxln) => {
                state.registers[Efm8SmbusRegister::Smb0Rxln.offset()] = value;
            }
            Some(Efm8SmbusRegister::Eie1) => {
                state.registers[Efm8SmbusRegister::Eie1.offset()] = value;
            }
            Some(Efm8SmbusRegister::Eip1) => {
                state.registers[Efm8SmbusRegister::Eip1.offset()] = value;
            }
            Some(Efm8SmbusRegister::Eip1h) => {
                state.registers[Efm8SmbusRegister::Eip1h.offset()] = value;
            }
            Some(Efm8SmbusRegister::Smb0Cn0) => {
                let request_start = value & SMB0CN0_STA != 0;
                let request_stop = value & SMB0CN0_STO != 0;
                let old_hardware = state.registers[Efm8SmbusRegister::Smb0Cn0.offset()]
                    & (SMB0CN0_MASTER | SMB0CN0_TXMODE | SMB0CN0_ACKRQ | SMB0CN0_ARBLOST);
                let software = value & (SMB0CN0_STA | SMB0CN0_STO | SMB0CN0_ACK | SMB0CN0_SI);
                state.registers[Efm8SmbusRegister::Smb0Cn0.offset()] = old_hardware | software;
                if value & SMB0CN0_SI == 0 {
                    state.registers[Efm8SmbusRegister::Smb0Cn0.offset()] &= !SMB0CN0_ARBLOST;
                }
                if request_stop {
                    state.smbus0_stop();
                }
                if request_start {
                    state.smbus0_start();
                }
            }
            Some(Efm8SmbusRegister::Smb0Dat) => {
                state.registers[Efm8SmbusRegister::Smb0Dat.offset()] = value;
                if state.registers[Efm8SmbusRegister::Smb0Cf.offset()] & SMB0CF_ENSMB != 0 {
                    state.smbus0_tx.push(value);
                    state.smbus0_tx_fifo.push_back(value);
                    state.registers[Efm8SmbusRegister::Smb0Cf.offset()] |= SMB0CF_BUSY;
                    state.registers[Efm8SmbusRegister::Smb0Cn0.offset()] |=
                        SMB0CN0_MASTER | SMB0CN0_TXMODE | SMB0CN0_SI;
                    state.registers[Efm8SmbusRegister::Smb0Cn0.offset()] &= !SMB0CN0_ACKRQ;
                    state.set_signal(state.smbus0_tx_byte_signal, u64::from(value), 8, at);
                    let previous = state.hub.with_registry(|registry| {
                        registry
                            .value(state.smbus0_tx_strobe_signal)
                            .and_then(|signal| signal.bit(0))
                            .map_or(0, |logic| u64::from(logic == Logic::One))
                    });
                    state.set_signal(state.smbus0_tx_strobe_signal, previous ^ 1, 1, at);
                }
            }
            None if address == CLKSEL => {
                state.registers[address] = value;
            }
            None => {
                state.registers[address] = value;
            }
        }
        if smbus_register.is_none() {
            if address == SBUF1 {
                if state.registers[SBCON1] & SBCON1_BREN != 0 {
                    state.uart1.push(value);
                    state.set_signal(state.uart1_byte_signal, u64::from(value), 8, at);
                    let previous = state.hub.with_registry(|registry| {
                        registry
                            .value(state.uart1_strobe_signal)
                            .and_then(|signal| signal.bit(0))
                            .map_or(0, |logic| u64::from(logic == Logic::One))
                    });
                    state.set_signal(state.uart1_strobe_signal, previous ^ 1, 1, at);
                    state.registers[SCON1] |= SCON1_TI;
                }
            } else if matches!(address, DAC0L | DAC0H | DAC0ALT | DAC0CF0 | DAC0CF1) {
                state.write_dac_register(address, value, at);
            } else if address == ADC0CN0 && value & ADC0_ADBUSY != 0 {
                if value & ADC0_ADEN != 0 && state.registers[ADC0CN2] & 0x0f == 0 {
                    state.complete_adc_conversion(at);
                } else {
                    state.registers[ADC0CN0] &= !ADC0_ADBUSY;
                }
            } else if address == SCON1 {
                state.registers[address] = value & !SCON1_RI;
                if !state.uart1_rx.is_empty() {
                    state.registers[address] |= SCON1_RI;
                }
            } else if address == UART1FCN0 {
                if value & UART1FCN0_TFLSH != 0 {
                    state.uart1.clear();
                    state.registers[SCON1] &= !SCON1_TI;
                }
                if value & UART1FCN0_RFLSH != 0 {
                    state.uart1_rx.clear();
                    state.registers[SCON1] &= !SCON1_RI;
                }
                state.registers[address] = value & !0x44;
            } else if address == UART1FCN1 {
                state.registers[address] = value & 0x37;
            } else if address == CRC0CN0 {
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
                if state.registers[XBR0] & XBR0_URT0E != 0
                    && state.registers[XBR2] & XBR2_XBARE != 0
                {
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
            } else if address == TMR3CN0 && value & TMR3_TR3 != 0 {
                state.registers[address] = value;
                state.timer3_epoch = at.ticks();
            } else if address == TMR4CN0 && value & TMR4_TR4 != 0 {
                state.registers[address] = value;
                state.timer4_epoch = at.ticks();
            } else if address == TMR5CN0 && value & TMR5_TR5 != 0 {
                state.registers[address] = value;
                state.timer5_epoch = at.ticks();
            } else {
                state.registers[address] = value;
            }
        }
        state.update_smbus0_signals(at);
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
mod tests;
