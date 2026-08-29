use super::*;

impl ArmMcuMachine {
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
