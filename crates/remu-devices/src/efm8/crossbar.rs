use super::*;

pub(super) const P2SKIP: usize = 0xcc;
pub(super) const P0SKIP: usize = 0xd4;
pub(super) const P1SKIP: usize = 0xd5;
pub(super) const XBR1: usize = 0xe2;

/// Crossbar functions that can be assigned to a QFN32 port pin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Efm8CrossbarFunction {
    /// UART0 transmit output (fixed P0.4).
    Uart0Tx,
    /// UART0 receive input (fixed P0.5).
    Uart0Rx,
    /// SPI0 serial clock.
    Spi0Sck,
    /// SPI0 master-in/slave-out.
    Spi0Miso,
    /// SPI0 master-out/slave-in.
    Spi0Mosi,
    /// SPI0 four-wire slave-select.
    Spi0Nss,
    /// SMBus0 data.
    Smb0Sda,
    /// SMBus0 clock.
    Smb0Scl,
    /// Comparator 0 synchronous output.
    Cmp0,
    /// Comparator 0 asynchronous output.
    Cmp0a,
    /// Comparator 1 synchronous output.
    Cmp1,
    /// Comparator 1 asynchronous output.
    Cmp1a,
    /// SYSCLK output.
    Sysclk,
    /// PCA CEX0 output.
    PcaCex0,
    /// PCA CEX1 output.
    PcaCex1,
    /// PCA CEX2 output.
    PcaCex2,
    /// PCA external counter input.
    PcaEci,
    /// Timer 0 external input.
    Timer0,
    /// Timer 1 external input.
    Timer1,
    /// Timer 2/3/4/5 external input.
    Timer2345,
    /// SMBus1 data.
    Smb1Sda,
    /// SMBus1 clock.
    Smb1Scl,
    /// UART1 transmit output.
    Uart1Tx,
    /// UART1 receive input.
    Uart1Rx,
    /// UART1 RTS output.
    Uart1Rts,
    /// UART1 CTS input.
    Uart1Cts,
    /// PWM channel 0 output.
    Pwm0,
    /// PWM channel 1 output.
    Pwm1,
    /// PWM channel 2 output.
    Pwm2,
}

impl Efm8CrossbarFunction {
    pub(super) const COUNT: usize = 29;

    pub(super) const fn index(self) -> usize {
        self as usize
    }
}

/// A physical QFN32 pin selected by the EFM8 priority crossbar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Efm8CrossbarPin {
    /// Port number (0, 1, or 2).
    pub port: u8,
    /// Bit number within the port.
    pub pin: u8,
}

impl Efm8State {
    fn crossbar_pin(index: usize) -> Efm8CrossbarPin {
        Efm8CrossbarPin {
            port: u8::try_from(index / 8).expect("EFM8 crossbar port fits in u8"),
            pin: u8::try_from(index % 8).expect("EFM8 crossbar pin fits in u8"),
        }
    }

    fn crossbar_skip_mask(&self) -> u32 {
        u32::from(self.registers[P0SKIP])
            | (u32::from(self.registers[P1SKIP]) << 8)
            | (u32::from(self.registers[P2SKIP]) << 16)
    }

    fn assign_crossbar_pin(&mut self, function: Efm8CrossbarFunction, occupied: &mut u32) {
        let unavailable = self.crossbar_skip_mask() | *occupied;
        for index in 0..24 {
            let bit = 1_u32 << index;
            if unavailable & bit == 0 {
                self.crossbar_routes[function.index()] = Some(Self::crossbar_pin(index));
                *occupied |= bit;
                return;
            }
        }
        self.crossbar_routes[function.index()] = None;
    }

    fn assign_crossbar_fixed(
        &mut self,
        function: Efm8CrossbarFunction,
        occupied: &mut u32,
        index: usize,
    ) {
        self.crossbar_routes[function.index()] = Some(Self::crossbar_pin(index));
        *occupied |= 1_u32 << index;
    }

    pub(super) fn refresh_crossbar(&mut self, at: SimTime) {
        self.crossbar_routes.fill(None);
        let mut occupied = 0_u32;
        if self.registers[XBR0] & XBR0_URT0E != 0 {
            self.assign_crossbar_fixed(Efm8CrossbarFunction::Uart0Tx, &mut occupied, 4);
            self.assign_crossbar_fixed(Efm8CrossbarFunction::Uart0Rx, &mut occupied, 5);
        }
        if self.registers[XBR0] & 0x02 != 0 {
            for function in [
                Efm8CrossbarFunction::Spi0Sck,
                Efm8CrossbarFunction::Spi0Miso,
                Efm8CrossbarFunction::Spi0Mosi,
                Efm8CrossbarFunction::Spi0Nss,
            ] {
                self.assign_crossbar_pin(function, &mut occupied);
            }
        }
        if self.registers[XBR0] & 0x04 != 0 {
            self.assign_crossbar_pin(Efm8CrossbarFunction::Smb0Sda, &mut occupied);
            self.assign_crossbar_pin(Efm8CrossbarFunction::Smb0Scl, &mut occupied);
        }
        for (function, mask) in [
            (Efm8CrossbarFunction::Cmp0, 0x08),
            (Efm8CrossbarFunction::Cmp0a, 0x10),
            (Efm8CrossbarFunction::Cmp1, 0x20),
            (Efm8CrossbarFunction::Cmp1a, 0x40),
            (Efm8CrossbarFunction::Sysclk, 0x80),
        ] {
            if self.registers[XBR0] & mask != 0 {
                self.assign_crossbar_pin(function, &mut occupied);
            }
        }
        let xbr1 = self.registers[XBR1];
        for function in [
            Efm8CrossbarFunction::PcaCex0,
            Efm8CrossbarFunction::PcaCex1,
            Efm8CrossbarFunction::PcaCex2,
        ]
        .into_iter()
        .take(usize::from(xbr1 & 0x03))
        {
            self.assign_crossbar_pin(function, &mut occupied);
        }
        for (function, mask) in [
            (Efm8CrossbarFunction::PcaEci, 0x08),
            (Efm8CrossbarFunction::Timer0, 0x10),
            (Efm8CrossbarFunction::Timer1, 0x20),
            (Efm8CrossbarFunction::Timer2345, 0x40),
        ] {
            if xbr1 & mask != 0 {
                self.assign_crossbar_pin(function, &mut occupied);
            }
        }
        if xbr1 & 0x80 != 0 {
            self.assign_crossbar_pin(Efm8CrossbarFunction::Smb1Sda, &mut occupied);
            self.assign_crossbar_pin(Efm8CrossbarFunction::Smb1Scl, &mut occupied);
        }
        let xbr2 = self.registers[XBR2];
        if xbr2 & 0x01 != 0 {
            self.assign_crossbar_pin(Efm8CrossbarFunction::Uart1Tx, &mut occupied);
            self.assign_crossbar_pin(Efm8CrossbarFunction::Uart1Rx, &mut occupied);
        }
        if xbr2 & 0x02 != 0 {
            self.assign_crossbar_pin(Efm8CrossbarFunction::Uart1Rts, &mut occupied);
        }
        if xbr2 & 0x04 != 0 {
            self.assign_crossbar_pin(Efm8CrossbarFunction::Uart1Cts, &mut occupied);
        }
        for function in [
            Efm8CrossbarFunction::Pwm0,
            Efm8CrossbarFunction::Pwm1,
            Efm8CrossbarFunction::Pwm2,
        ]
        .into_iter()
        .take(usize::from((xbr2 >> 3) & 0x03))
        {
            self.assign_crossbar_pin(function, &mut occupied);
        }
        self.set_signal(
            self.crossbar_enabled_signal,
            u64::from(xbr2 & XBR2_XBARE != 0),
            1,
            at,
        );
        self.set_signal(
            self.crossbar_assigned_signal,
            self.crossbar_routes.iter().flatten().count() as u64,
            8,
            at,
        );
        for (signal, function) in [
            (self.crossbar_uart0_tx_signal, Efm8CrossbarFunction::Uart0Tx),
            (self.crossbar_uart0_rx_signal, Efm8CrossbarFunction::Uart0Rx),
        ] {
            let value = self.crossbar_routes[function.index()]
                .map_or(0xff, |pin| u64::from(pin.port * 8 + pin.pin));
            self.set_signal(signal, value, 8, at);
        }
    }
}
