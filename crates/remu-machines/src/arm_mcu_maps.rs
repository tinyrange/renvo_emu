use super::*;
use remu_devices::{
    RaAdc, RaAgt, RaCac, RaCrc, RaDac, RaDoc, RaElc, RaIic, RaKint, RaPoeg, RaRtc, RaSpi,
};

impl ArmMcuMachine {
    pub(super) fn map_samd21(
        bus: &mut AddressSpace,
        port: Samd21Port,
        eic: Samd21Eic,
        watchdog: Samd21Wdt,
        timers: [Samd21Tc; 3],
        tccs: [Samd21Tcc; 3],
        rtc: Samd21Rtc,
        sercoms: [Samd21Usart; 4],
        evsys: Samd21Evsys,
        usb: Samd21UsbDevice,
        dmac: Samd21Dmac,
        i2s: Samd21I2s,
        adc: Samd21Adc,
        ac: Samd21Ac,
        dac: Samd21Dac,
    ) -> Result<(), remu_bus::MapError> {
        bus.map_device(
            "atsamd21e18.pm",
            0x4000_0400,
            0x100,
            Box::new(Samd21RegisterBlock::new("atsamd21e18.pm", 0x100, [])),
        )?;
        // PCLKSR.OSC8MRDY and DFLLRDY are asserted in the functional clock model.
        bus.map_device(
            "atsamd21e18.sysctrl",
            0x4000_0800,
            0x100,
            Box::new(Samd21RegisterBlock::new(
                "atsamd21e18.sysctrl",
                0x100,
                [(0x0c, 0x18)],
            )),
        )?;
        bus.map_device(
            "atsamd21e18.gclk",
            0x4000_0c00,
            0x100,
            Box::new(Samd21RegisterBlock::new("atsamd21e18.gclk", 0x100, [])),
        )?;
        bus.map_device("atsamd21e18.wdt", 0x4000_1000, 0x100, Box::new(watchdog))?;
        bus.map_device("atsamd21e18.rtc", 0x4000_1400, 0x20, Box::new(rtc))?;
        bus.map_device("atsamd21e18.eic", 0x4000_1800, 0x100, Box::new(eic))?;
        bus.map_device("atsamd21e18.evsys", 0x4200_0400, 0x20, Box::new(evsys))?;
        bus.map_device("atsamd21e18.usb", 0x4100_5000, 0x200, Box::new(usb))?;
        bus.map_device("atsamd21e18.ac", 0x4200_4400, 0x100, Box::new(ac))?;
        for (index, sercom) in sercoms.into_iter().enumerate() {
            bus.map_device(
                format!("atsamd21e18.sercom{index}"),
                0x4200_0800 + u64::try_from(index).expect("SERCOM index fits u64") * 0x400,
                0x40,
                Box::new(sercom),
            )?;
        }
        for (index, timer) in timers.into_iter().enumerate() {
            let instance = index + 3;
            bus.map_device(
                format!("atsamd21e18.tc{instance}"),
                0x4200_2c00 + u64::try_from(index).expect("TC index fits u64") * 0x400,
                0x40,
                Box::new(timer),
            )?;
        }
        for (index, tcc) in tccs.into_iter().enumerate() {
            bus.map_device(
                format!("atsamd21e18.tcc{index}"),
                0x4200_2000 + u64::try_from(index).expect("TCC index fits u64") * 0x400,
                0x80,
                Box::new(tcc),
            )?;
        }
        bus.map_device("atsamd21e18.dmac", 0x4100_4800, 0x100, Box::new(dmac))?;
        bus.map_device("atsamd21e18.i2s", 0x4200_5000, 0x100, Box::new(i2s))?;
        bus.map_device("atsamd21e18.adc", 0x4200_4000, 0x100, Box::new(adc))?;
        bus.map_device("atsamd21e18.dac", 0x4200_4800, 0x20, Box::new(dac))?;
        // NVMCTRL.INTFLAG.READY is set after reset.
        bus.map_device(
            "atsamd21e18.nvmctrl",
            0x4100_4000,
            0x400,
            Box::new(Samd21RegisterBlock::new(
                "atsamd21e18.nvmctrl",
                0x400,
                [(0x14, 1)],
            )),
        )?;
        bus.map_device("atsamd21e18.port", 0x4100_4400, 0x100, Box::new(port))?;
        Ok(())
    }

    pub(super) fn map_stm32l432(
        bus: &mut AddressSpace,
        gpio: [Stm32Gpio; 4],
        tim1: Stm32AdvancedTimer,
        tim2: Stm32Timer,
        usart1: Stm32Usart,
        usart2: Stm32Usart,
        lpuart1: Stm32Usart,
        spi1: Stm32Spi,
        spi3: Stm32Spi,
        i2c1: Stm32I2c,
        i2c3: Stm32I2c,
        watchdog: Stm32Watchdog,
        adc: Stm32Adc,
        crc: Stm32Crc,
        rtc: Stm32Rtc,
        rng: Stm32Rng,
        dac1: Stm32Dac,
        exti: Stm32Exti,
        wwdg: Stm32Wwdg,
        tim6: Stm32BasicTimer,
        tim7: Stm32Tim7,
        tim15: Stm32Tim15,
        tim16: Stm32Tim16,
        lptim1: Stm32Lptim1,
        lptim2: Stm32Lptim2,
        usb: Stm32UsbFs,
        usb_pma: Stm32UsbPma,
        sai1: Stm32Sai1,
        qspi: Stm32QuadSpi,
        qspi_flash: SharedMemory,
        swpmi: Stm32Swpmi,
        dma1: Stm32Dma,
        dma2: Stm32Dma,
        tsc: Stm32Tsc,
        comparators: Stm32Comparators,
        opamp: Stm32Opamp,
        flash_controller: Stm32FlashController,
    ) -> Result<(), remu_bus::MapError> {
        bus.map_device(
            "stm32l432kc.rcc",
            0x4002_1000,
            0x400,
            Box::new(RegisterBank::new(
                "stm32l432kc.rcc",
                [
                    (0x00, 0x0000_0063, u32::MAX),
                    (0x08, 0x0000_1000, u32::MAX),
                    (0x4c, 0, u32::MAX),
                    (0x58, 0, u32::MAX),
                    (0x5c, 0, u32::MAX),
                    (0x88, 0, u32::MAX),
                ],
            )),
        )?;
        bus.map_device("stm32l432kc.rng", 0x5006_0800, 0x400, Box::new(rng))?;
        bus.map_device(
            "stm32l432kc.pwr",
            0x4000_7000,
            0x400,
            Box::new(RegisterBank::new(
                "stm32l432kc.pwr",
                [(0x00, 0x0000_0200, u32::MAX), (0x14, 0, u32::MAX)],
            )),
        )?;
        bus.map_device(
            "stm32l432kc.flash-control",
            0x4002_2000,
            0x400,
            Box::new(flash_controller),
        )?;
        bus.map_device(
            "stm32l432kc.syscfg",
            0x4001_0000,
            0x200,
            Box::new(RegisterBank::new(
                "stm32l432kc.syscfg",
                [
                    (0x00, 0, u32::MAX),
                    (0x08, 0, u32::MAX),
                    (0x0c, 0, u32::MAX),
                ],
            )),
        )?;
        bus.map_device("stm32l432kc.exti", 0x4001_0400, 0x400, Box::new(exti))?;
        bus.map_device("stm32l432kc.wwdg", 0x4000_2c00, 0x400, Box::new(wwdg))?;
        bus.map_device("stm32l432kc.tim6", 0x4000_1000, 0x400, Box::new(tim6))?;
        bus.map_device("stm32l432kc.tim7", 0x4000_1400, 0x400, Box::new(tim7))?;
        bus.map_device("stm32l432kc.tim15", 0x4001_4000, 0x400, Box::new(tim15))?;
        bus.map_device("stm32l432kc.tim16", 0x4001_4400, 0x400, Box::new(tim16))?;
        bus.map_device("stm32l432kc.lptim1", 0x4000_7c00, 0x400, Box::new(lptim1))?;
        bus.map_device("stm32l432kc.lptim2", 0x4000_9400, 0x400, Box::new(lptim2))?;
        bus.map_device("stm32l432kc.usb", 0x4000_6800, 0x400, Box::new(usb))?;
        bus.map_device("stm32l432kc.usb-pma", 0x4000_6c00, 0x400, Box::new(usb_pma))?;
        bus.map_device("stm32l432kc.sai1", 0x4001_5400, 0x400, Box::new(sai1))?;
        bus.map_device("stm32l432kc.quadspi", 0xa000_1000, 0x400, Box::new(qspi))?;
        bus.map_shared(
            "stm32l432kc.quadspi.flash",
            0x9000_0000,
            qspi_flash.len(),
            Permissions::RX,
            qspi_flash,
            0,
        )?;
        bus.map_device("stm32l432kc.swpmi", 0x4000_8800, 0x400, Box::new(swpmi))?;
        bus.map_device("stm32l432kc.dma1", 0x4002_0000, 0x100, Box::new(dma1))?;
        bus.map_device("stm32l432kc.dma2", 0x4002_0400, 0x100, Box::new(dma2))?;
        bus.map_device("stm32l432kc.tsc", 0x4002_4000, 0x100, Box::new(tsc))?;
        bus.map_device(
            "stm32l432kc.comp",
            0x4001_0200,
            0x100,
            Box::new(comparators),
        )?;
        bus.map_device("stm32l432kc.opamp", 0x4000_7800, 0x100, Box::new(opamp))?;
        bus.map_device("stm32l432kc.tim1", 0x4001_2c00, 0x400, Box::new(tim1))?;
        bus.map_device("stm32l432kc.tim2", 0x4000_0000, 0x400, Box::new(tim2))?;
        bus.map_device("stm32l432kc.usart1", 0x4001_3800, 0x400, Box::new(usart1))?;
        bus.map_device("stm32l432kc.usart2", 0x4000_4400, 0x400, Box::new(usart2))?;
        bus.map_device("stm32l432kc.lpuart1", 0x4000_8000, 0x400, Box::new(lpuart1))?;
        bus.map_device("stm32l432kc.spi3", 0x4000_3c00, 0x400, Box::new(spi3))?;
        bus.map_device("stm32l432kc.spi1", 0x4001_3000, 0x400, Box::new(spi1))?;
        bus.map_device("stm32l432kc.i2c1", 0x4000_5400, 0x400, Box::new(i2c1))?;
        bus.map_device("stm32l432kc.i2c3", 0x4000_5c00, 0x400, Box::new(i2c3))?;
        bus.map_device("stm32l432kc.iwdg", 0x4000_3000, 0x10, Box::new(watchdog))?;
        // STM32L432 places ADC1 in the AHB2 peripheral window at 0x5004_0000
        // (the 0x5000_0000 base is used by other MCU families).
        bus.map_device("stm32l432kc.adc1", 0x5004_0000, 0x400, Box::new(adc))?;
        bus.map_device("stm32l432kc.crc", 0x4002_3000, 0x400, Box::new(crc))?;
        bus.map_device("stm32l432kc.rtc", 0x4000_2800, 0x400, Box::new(rtc))?;
        bus.map_device("stm32l432kc.dac1", 0x4000_7400, 0x400, Box::new(dac1))?;
        bus.map_device(
            "stm32l432kc.can1",
            0x4000_6400,
            0x400,
            Box::new(Stm32Can::new("stm32l432kc.can1")),
        )?;
        let [gpioa, gpiob, gpioc, gpioh] = gpio;
        bus.map_device("stm32l432kc.gpioa", 0x4800_0000, 0x400, Box::new(gpioa))?;
        bus.map_device("stm32l432kc.gpiob", 0x4800_0400, 0x400, Box::new(gpiob))?;
        bus.map_device("stm32l432kc.gpioc", 0x4800_0800, 0x400, Box::new(gpioc))?;
        bus.map_device("stm32l432kc.gpioh", 0x4800_1c00, 0x400, Box::new(gpioh))?;
        Ok(())
    }

    pub(super) fn map_stm32f411(
        bus: &mut AddressSpace,
        gpio: [Stm32Gpio; 4],
        tim2: Stm32Timer,
        usart2: FunctionalUart,
        exti: Stm32Exti,
    ) -> Result<(), remu_bus::MapError> {
        bus.map_device(
            "stm32f411re.rcc",
            0x4002_3800,
            0x400,
            Box::new(RegisterBank::new(
                "stm32f411re.rcc",
                [
                    (0x00, 0x0000_0083, u32::MAX),
                    (0x04, 0x2400_3010, u32::MAX),
                    (0x08, 0, u32::MAX),
                    (0x0c, 0, u32::MAX),
                    (0x30, 0, u32::MAX),
                    (0x40, 0x0010_0000, u32::MAX),
                    (0x44, 0, u32::MAX),
                    (0x70, 0x0e00_0000, u32::MAX),
                    (0x74, 0, u32::MAX),
                ],
            )),
        )?;
        bus.map_device(
            "stm32f411re.flash-control",
            0x4002_3c00,
            0x400,
            Box::new(RegisterBank::new(
                "stm32f411re.flash-control",
                [
                    (0x00, 0, 0x0000_070f),
                    (0x04, 0, u32::MAX),
                    (0x08, 0, u32::MAX),
                    (0x0c, 0, 0x0000_01f3),
                    (0x10, 0x8000_0000, 0x8301_03fb),
                    (0x14, 0x0fff_aaed, 0x0fff_ffff),
                ],
            )),
        )?;
        bus.map_device(
            "stm32f411re.syscfg",
            0x4001_3800,
            0x400,
            Box::new(RegisterBank::new(
                "stm32f411re.syscfg",
                [
                    (0x00, 0, 3),
                    (0x04, 0, 0x00ff_00ff),
                    (0x08, 0, 0x0000_ffff),
                    (0x0c, 0, 0x0000_ffff),
                    (0x10, 0, 0x0000_ffff),
                    (0x14, 0, 0x0000_ffff),
                ],
            )),
        )?;
        bus.map_device("stm32f411re.exti", 0x4001_3c00, 0x400, Box::new(exti))?;
        bus.map_device("stm32f411re.tim2", 0x4000_0000, 0x400, Box::new(tim2))?;
        bus.map_device("stm32f411re.usart2", 0x4000_4400, 0x400, Box::new(usart2))?;
        let [gpioa, gpiob, gpioc, gpioh] = gpio;
        bus.map_device("stm32f411re.gpioa", 0x4002_0000, 0x400, Box::new(gpioa))?;
        bus.map_device("stm32f411re.gpiob", 0x4002_0400, 0x400, Box::new(gpiob))?;
        bus.map_device("stm32f411re.gpioc", 0x4002_0800, 0x400, Box::new(gpioc))?;
        bus.map_device("stm32f411re.gpioh", 0x4002_1c00, 0x400, Box::new(gpioh))?;
        Ok(())
    }

    pub(super) fn map_nrf52840(
        bus: &mut AddressSpace,
        gpio: Nrf52840Gpio,
        timer0: Nrf52840Timer,
        uart0: Nrf52840Uart,
    ) -> Result<(), remu_bus::MapError> {
        bus.map_device(
            "nrf52840.clock-power",
            0x4000_0000,
            0x1000,
            Box::new(RegisterBank::new(
                "nrf52840.clock-power",
                [
                    (0x100, 0, 1),
                    (0x104, 0, 1),
                    (0x108, 0, 1),
                    (0x40c, 0, 1),
                    (0x418, 0, 1),
                    (0x518, 0, 3),
                    (0x51c, 1, 3),
                ],
            )),
        )?;
        bus.map_device("nrf52840.uart0", 0x4000_2000, 0x1000, Box::new(uart0))?;
        bus.map_device("nrf52840.timer0", 0x4000_8000, 0x1000, Box::new(timer0))?;
        bus.map_device("nrf52840.gpio", 0x5000_0000, 0x1000, Box::new(gpio))?;
        Ok(())
    }

    pub(super) fn map_ra4m1(
        bus: &mut AddressSpace,
        ports: Vec<RaIoPort>,
        pfs: RaPfs,
        icu: RaIcu,
        gpt0: RaGpt,
        gpt: Vec<RaGpt>,
        kint: RaKint,
        elc: RaElc,
        sci9: RaSci,
        agt0: RaAgt,
        agt1: RaAgt,
        spi0: RaSpi,
        spi1: RaSpi,
        iic0: RaIic,
        iic1: RaIic,
        rtc: RaRtc,
        dac: RaDac,
        crc: RaCrc,
        doc: RaDoc,
        cac: RaCac,
        poeg: RaPoeg,
        adc: RaAdc,
    ) -> Result<(), remu_bus::MapError> {
        // Functional clock/reset surface. OSCSF reports the reset-selected HOCO stable.
        bus.map_device(
            "r7fa4m1ab3cfm.system",
            0x4001_e000,
            0x1000,
            Box::new(Samd21RegisterBlock::new(
                "r7fa4m1ab3cfm.system",
                0x1000,
                [(0x3c, 1)],
            )),
        )?;
        bus.map_device(
            "r7fa4m1ab3cfm.mstp",
            0x4004_6ffc,
            0x20,
            Box::new(Samd21RegisterBlock::new("r7fa4m1ab3cfm.mstp", 0x20, [])),
        )?;
        bus.map_device("r7fa4m1ab3cfm.icu", 0x4000_6000, 0x480, Box::new(icu))?;
        bus.map_device("r7fa4m1ab3cfm.gpt0", 0x4007_8000, 0x100, Box::new(gpt0))?;
        for (offset, device) in gpt.into_iter().enumerate() {
            let index = offset + 1;
            bus.map_device(
                format!("r7fa4m1ab3cfm.gpt{index}"),
                0x4007_8000 + u64::try_from(index).expect("GPT index fits u64") * 0x100,
                0x100,
                Box::new(device),
            )?;
        }
        bus.map_device("r7fa4m1ab3cfm.kint", 0x4008_0000, 0x10, Box::new(kint))?;
        bus.map_device("r7fa4m1ab3cfm.elc", 0x4004_1000, 0x80, Box::new(elc))?;
        bus.map_device("r7fa4m1ab3cfm.sci9", 0x4007_0120, 0x20, Box::new(sci9))?;
        bus.map_device("r7fa4m1ab3cfm.agt0", 0x4008_4000, 0x100, Box::new(agt0))?;
        bus.map_device("r7fa4m1ab3cfm.agt1", 0x4008_4100, 0x100, Box::new(agt1))?;
        bus.map_device("r7fa4m1ab3cfm.spi0", 0x4007_2000, 0x20, Box::new(spi0))?;
        bus.map_device("r7fa4m1ab3cfm.spi1", 0x4007_2100, 0x20, Box::new(spi1))?;
        bus.map_device("r7fa4m1ab3cfm.iic0", 0x4005_3000, 0x20, Box::new(iic0))?;
        bus.map_device("r7fa4m1ab3cfm.iic1", 0x4005_3100, 0x20, Box::new(iic1))?;
        bus.map_device("r7fa4m1ab3cfm.rtc", 0x4004_4000, 0x100, Box::new(rtc))?;
        bus.map_device("r7fa4m1ab3cfm.dac12", 0x4005_e000, 0x100, Box::new(dac))?;
        bus.map_device("r7fa4m1ab3cfm.crc", 0x4007_4000, 0x100, Box::new(crc))?;
        bus.map_device("r7fa4m1ab3cfm.doc", 0x4005_4100, 0x10, Box::new(doc))?;
        bus.map_device("r7fa4m1ab3cfm.cac", 0x4004_4600, 0x10, Box::new(cac))?;
        bus.map_device("r7fa4m1ab3cfm.poeg", 0x4004_2000, 0x400, Box::new(poeg))?;
        bus.map_device("r7fa4m1ab3cfm.adc0", 0x4005_c000, 0x200, Box::new(adc))?;
        bus.map_device("r7fa4m1ab3cfm.pfs", 0x4004_0800, 0x3c0, Box::new(pfs))?;
        bus.map_device(
            "r7fa4m1ab3cfm.pmisc",
            0x4004_0d00,
            0x100,
            Box::new(Samd21RegisterBlock::new(
                "r7fa4m1ab3cfm.pmisc",
                0x100,
                [(3, 0x80)],
            )),
        )?;
        for (port, device) in ports.into_iter().enumerate() {
            bus.map_device(
                format!("r7fa4m1ab3cfm.port{port}"),
                0x4004_0000 + u64::try_from(port).expect("port index fits u64") * 0x20,
                0x10,
                Box::new(device),
            )?;
        }
        // Functional WDT startup surface; refresh/timeout fidelity is deferred to the target report.
        bus.map_device(
            "r7fa4m1ab3cfm.wdt",
            0x4004_4200,
            0x10,
            Box::new(Samd21RegisterBlock::new("r7fa4m1ab3cfm.wdt", 0x10, [])),
        )?;
        Ok(())
    }
}
