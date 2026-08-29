use super::*;

impl ArmMcuMachine {
    pub(super) fn create_stm32f103(
        bus: &mut AddressSpace,
        signals: &SignalHub,
    ) -> Result<
        (
            GpioHandle,
            Stm32F1UsartHandle,
            Stm32TimerHandle,
            Stm32ExtiHandle,
        ),
        ArmMachineError,
    > {
        let (gpioa_device, gpio) = Stm32F1Gpio::new(
            "stm32f103c8.gpioa",
            "board.stm32f103c8.gpioa",
            signals.clone(),
        )?;
        let (gpiob_device, _) = Stm32F1Gpio::new(
            "stm32f103c8.gpiob",
            "board.stm32f103c8.gpiob",
            signals.clone(),
        )?;
        let (gpioc_device, _) = Stm32F1Gpio::new(
            "stm32f103c8.gpioc",
            "board.stm32f103c8.gpioc",
            signals.clone(),
        )?;
        let (gpiod_device, _) = Stm32F1Gpio::new(
            "stm32f103c8.gpiod",
            "board.stm32f103c8.gpiod",
            signals.clone(),
        )?;
        let (tim2_device, timer) = Stm32Timer::new("stm32f103c8.tim2");
        let (usart1_device, usart1) = Stm32F1Usart::new("stm32f103c8.usart1");
        let (exti_device, exti) = Stm32Exti::new("board.stm32f103c8.exti", signals.clone())?;
        Self::map_stm32f103(
            bus,
            [gpioa_device, gpiob_device, gpioc_device, gpiod_device],
            tim2_device,
            usart1_device,
            exti_device,
        )?;
        Ok((gpio, usart1, timer, exti))
    }

    pub(super) fn create_samd51(
        bus: &mut AddressSpace,
        signals: &SignalHub,
    ) -> Result<(GpioHandle, Samd21UsartHandle, Samd51TcHandle), ArmMachineError> {
        let (porta_device, gpio) = Samd21Port::new(
            "atsamd51j19a.porta",
            32,
            "board.atsamd51j19a.porta",
            signals.clone(),
        )?;
        let (portb_device, _) = Samd21Port::new(
            "atsamd51j19a.portb",
            19,
            "board.atsamd51j19a.portb",
            signals.clone(),
        )?;
        let (sercom0_device, sercom0) = Samd21Usart::new("atsamd51j19a.sercom0");
        let (tc0_device, tc0) = Samd51Tc::new("atsamd51j19a.tc0");
        Self::map_samd51(
            bus,
            [porta_device, portb_device],
            sercom0_device,
            tc0_device,
        )?;
        Ok((gpio, sercom0, tc0))
    }

    pub(super) fn create_stm32f411(
        bus: &mut AddressSpace,
        signals: &SignalHub,
    ) -> Result<(GpioHandle, UartHandle, Stm32TimerHandle, Stm32ExtiHandle), ArmMachineError> {
        let (gpioa_device, gpio) = Stm32Gpio::new(
            "stm32f411re.gpioa",
            "board.stm32f411re.gpioa",
            signals.clone(),
        )?;
        let (gpiob_device, _) = Stm32Gpio::new(
            "stm32f411re.gpiob",
            "board.stm32f411re.gpiob",
            signals.clone(),
        )?;
        let (gpioc_device, _) = Stm32Gpio::new(
            "stm32f411re.gpioc",
            "board.stm32f411re.gpioc",
            signals.clone(),
        )?;
        let (gpioh_device, _) = Stm32Gpio::new(
            "stm32f411re.gpioh",
            "board.stm32f411re.gpioh",
            signals.clone(),
        )?;
        let (tim2_device, timer) = Stm32Timer::new("stm32f411re.tim2");
        let (usart2_device, usart2) =
            FunctionalUart::new_lenient("stm32f411re.usart2", 0x04, 0x00, 1 << 7);
        let usart2_device = usart2_device.with_rx_count_field(1 << 5, 5);
        let (exti_device, exti) = Stm32Exti::new("board.stm32f411re.exti", signals.clone())?;
        Self::map_stm32f411(
            bus,
            [gpioa_device, gpiob_device, gpioc_device, gpioh_device],
            tim2_device,
            usart2_device,
            exti_device,
        )?;
        Ok((gpio, usart2, timer, exti))
    }

    pub(super) fn create_nrf52840(
        bus: &mut AddressSpace,
        signals: &SignalHub,
    ) -> Result<(GpioHandle, UartHandle, Nrf52840TimerHandle), ArmMachineError> {
        let (gpio_device, gpio) =
            Nrf52840Gpio::new("nrf52840.gpio", "board.nrf52840.gpio", signals.clone())?;
        let (timer_device, timer) = Nrf52840Timer::new("nrf52840.timer0");
        let (uart_device, uart) = Nrf52840Uart::new("nrf52840.uart0");
        Self::map_nrf52840(bus, gpio_device, timer_device, uart_device)?;
        Ok((gpio, uart, timer))
    }
}
