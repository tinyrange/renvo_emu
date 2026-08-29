use super::*;
use remu_devices::{Stm32H7Dma, Stm32H7DmaHandle};

impl ArmMcuMachine {
    pub(super) fn create_stm32h743(
        bus: &mut AddressSpace,
        signals: &SignalHub,
    ) -> Result<
        (
            GpioHandle,
            Stm32UsartHandle,
            Stm32TimerHandle,
            Stm32H7DmaHandle,
            Stm32H7DmaHandle,
        ),
        ArmMachineError,
    > {
        let (gpioa, gpio) = Self::h7_gpio("a", signals)?;
        let (gpiob, _) = Self::h7_gpio("b", signals)?;
        let (gpioc, _) = Self::h7_gpio("c", signals)?;
        let (gpiod, _) = Self::h7_gpio("d", signals)?;
        let (gpioe, _) = Self::h7_gpio("e", signals)?;
        let (gpiof, _) = Self::h7_gpio("f", signals)?;
        let (gpiog, _) = Self::h7_gpio("g", signals)?;
        let (gpioh, _) = Self::h7_gpio("h", signals)?;
        let (gpioi, _) = Self::h7_gpio("i", signals)?;
        let (gpioj, _) = Self::h7_gpio("j", signals)?;
        let (gpiok, _) = Self::h7_gpio("k", signals)?;
        let (tim2, timer) = Stm32Timer::new("stm32h743zi.tim2");
        let (usart3, uart) = Stm32Usart::new("stm32h743zi.usart3");
        let (dma1, dma1_handle) = Stm32H7Dma::new("stm32h743zi.dma1");
        let (dma2, dma2_handle) = Stm32H7Dma::new("stm32h743zi.dma2");

        Self::map_stm32h743(
            bus,
            [
                gpioa, gpiob, gpioc, gpiod, gpioe, gpiof, gpiog, gpioh, gpioi, gpioj, gpiok,
            ],
            tim2,
            usart3,
            dma1,
            dma2,
        )?;
        Ok((gpio, uart, timer, dma1_handle, dma2_handle))
    }

    fn h7_gpio(
        port: &str,
        signals: &SignalHub,
    ) -> Result<(Stm32Gpio, GpioHandle), ArmMachineError> {
        Ok(Stm32Gpio::new(
            format!("stm32h743zi.gpio{port}"),
            &format!("board.stm32h743zi.gpio{port}"),
            signals.clone(),
        )?)
    }

    fn map_stm32h743(
        bus: &mut AddressSpace,
        gpio: [Stm32Gpio; 11],
        tim2: Stm32Timer,
        usart3: Stm32Usart,
        dma1: Stm32H7Dma,
        dma2: Stm32H7Dma,
    ) -> Result<(), remu_bus::MapError> {
        bus.map_device(
            "stm32h743zi.rcc",
            0x5802_4400,
            0x400,
            Box::new(RegisterBank::new(
                "stm32h743zi.rcc",
                [
                    (0x00, 0x0000_0083, u32::MAX),
                    (0x04, 0x4000_0000, u32::MAX),
                    (0x08, 0x0000_0000, u32::MAX),
                    (0x0c, 0x2000_0000, u32::MAX),
                    (0x10, 0x0000_0000, u32::MAX),
                    (0x18, 0x0000_0000, u32::MAX),
                    (0x1c, 0x0000_0000, u32::MAX),
                    (0x20, 0x0000_0000, u32::MAX),
                    (0x28, 0x0202_0200, u32::MAX),
                    (0x2c, 0x01ff_0000, u32::MAX),
                    (0x30, 0x0101_0200, u32::MAX),
                    (0x38, 0x0101_0200, u32::MAX),
                    (0x40, 0x0101_0200, u32::MAX),
                    (0x60, 0, u32::MAX),
                    (0x64, 0, 0),
                    (0x68, 0, u32::MAX),
                    (0x70, 0, u32::MAX),
                    (0x74, 0, u32::MAX),
                    (0x7c, 0, u32::MAX),
                    (0x80, 0, u32::MAX),
                    (0x84, 0, u32::MAX),
                    (0x88, 0, u32::MAX),
                    (0x8c, 0, u32::MAX),
                    (0x90, 0, u32::MAX),
                    (0x94, 0, u32::MAX),
                    (0x98, 0, u32::MAX),
                    (0x9c, 0, u32::MAX),
                    (0xa0, 0, u32::MAX),
                    (0xd0, 0x0400_0000, u32::MAX),
                    (0xd4, 0, u32::MAX),
                    (0xd8, 0, u32::MAX),
                    (0xdc, 0, u32::MAX),
                    (0xe0, 0, u32::MAX),
                    (0xe4, 0, u32::MAX),
                    (0xe8, 0, u32::MAX),
                    (0xec, 0, u32::MAX),
                    (0xf0, 0, u32::MAX),
                    (0xf4, 0, u32::MAX),
                    (0xfc, 0, u32::MAX),
                    (0x100, 0, u32::MAX),
                    (0x104, 0, u32::MAX),
                    (0x108, 0, u32::MAX),
                    (0x10c, 0, u32::MAX),
                    (0x110, 0, u32::MAX),
                    (0x114, 0, u32::MAX),
                    (0x118, 0, u32::MAX),
                    (0x11c, 0, u32::MAX),
                ],
            )),
        )?;
        bus.map_device(
            "stm32h743zi.flash-control",
            0x5200_2000,
            0x400,
            Box::new(RegisterBank::new(
                "stm32h743zi.flash-control",
                [
                    (0x00, 0, 0x0000_003f),
                    (0x04, 0, u32::MAX),
                    (0x08, 0, u32::MAX),
                    (0x0c, 0, u32::MAX),
                    (0x10, 0, u32::MAX),
                    (0x20, 0, u32::MAX),
                    (0x24, 0, u32::MAX),
                    (0x28, 0, u32::MAX),
                    (0x2c, 0, u32::MAX),
                    (0x30, 0, u32::MAX),
                    (0x40, 0, u32::MAX),
                    (0x44, 0, u32::MAX),
                    (0x48, 0, u32::MAX),
                    (0x4c, 0, u32::MAX),
                    (0x50, 0, u32::MAX),
                ],
            )),
        )?;
        bus.map_device(
            "stm32h743zi.pwr",
            0x5802_4800,
            0x400,
            Box::new(RegisterBank::new(
                "stm32h743zi.pwr",
                (0..=0x38_u64)
                    .step_by(4)
                    .map(|offset| (offset, 0, u32::MAX)),
            )),
        )?;
        bus.map_device(
            "stm32h743zi.syscfg",
            0x5800_0400,
            0x400,
            Box::new(RegisterBank::new(
                "stm32h743zi.syscfg",
                (0..=0x40_u64)
                    .step_by(4)
                    .map(|offset| (offset, 0, u32::MAX)),
            )),
        )?;
        bus.map_device("stm32h743zi.tim2", 0x4000_0000, 0x400, Box::new(tim2))?;
        bus.map_device("stm32h743zi.usart3", 0x4000_4800, 0x400, Box::new(usart3))?;
        bus.map_device("stm32h743zi.dma1", 0x4002_0000, 0x400, Box::new(dma1))?;
        bus.map_device("stm32h743zi.dma2", 0x4002_0400, 0x400, Box::new(dma2))?;
        bus.map_device(
            "stm32h743zi.dmamux1",
            0x4002_0800,
            0x400,
            Box::new(RegisterBank::new(
                "stm32h743zi.dmamux1",
                (0..=0x13c_u64)
                    .step_by(4)
                    .map(|offset| (offset, 0, u32::MAX)),
            )),
        )?;
        for (index, device) in gpio.into_iter().enumerate() {
            let port = char::from(b'a' + u8::try_from(index).expect("GPIO index fits u8"));
            bus.map_device(
                format!("stm32h743zi.gpio{port}"),
                0x5802_0000 + u64::try_from(index).expect("GPIO index fits u64") * 0x400,
                0x400,
                Box::new(device),
            )?;
        }
        Ok(())
    }
}
