use remu_bus::{AddressSpace, DeviceError};
use remu_core::SimTime;
use remu_devices::{
    Esp32c6I2c, EspAes, EspAnalogI2c, EspC6BleBaseband, EspC6BleBasebandHandle, EspC6BleControl,
    EspC6BleControlHandle, EspC6BleModem, EspC6BleModemHandle, EspC6ControlBlock, EspC6Ecc,
    EspC6Efuse, EspC6Gdma, EspC6GdmaHandle, EspC6Hmac, EspC6InterruptMatrix,
    EspC6InterruptMatrixHandle, EspC6InterruptPriority, EspC6IoMux, EspC6LpAon, EspC6LpAonHandle,
    EspC6LpClkRst, EspC6LpClkRstHandle, EspC6LpTimer, EspC6LpTimerHandle, EspC6ModemControl,
    EspC6ModemHandle, EspC6PhyRegisters, EspC6Pmu, EspC6PmuHandle, EspC6PowerDetector, EspC6Twai,
    EspC6TwaiHandle, EspC6Uhci, EspC6WifiMacHandle, EspC6WifiMacRegisters, EspDigitalSignature,
    EspEtm, EspEtmHandle, EspI2s, EspI2sHandle, EspIeee802154, EspIeee802154Handle, EspLedc,
    EspLedcHandle, EspLpI2c, EspLpI2cHandle, EspLpUart, EspLpUartHandle, EspLpWatchdog,
    EspLpWatchdogHandle, EspMcpwm, EspParlio, EspParlioHandle, EspPcnt, EspPcntHandle, EspRmt,
    EspRmtHandle, EspRsa, EspSarAdc, EspSdioSlaveHandle, EspSha, EspSpi, EspSpiHandle, EspSystimer,
    EspSystimerHandle, EspUsbSerialJtag, EspUsbSerialJtagHandle, FunctionalUart, SignalHub,
    UartHandle, new_esp_sdio_slave,
};
use remu_signals::Logic;

use super::MachineError;

/// Host-side handles for the functional ESP32-C6 peripheral graph.
pub(super) struct Esp32c6PeripheralHandles {
    pub(super) modem: EspC6ModemHandle,
    pub(super) ble_modem: EspC6BleModemHandle,
    pub(super) ble_baseband: EspC6BleBasebandHandle,
    pub(super) ble_control: EspC6BleControlHandle,
    pub(super) ieee802154: EspIeee802154Handle,
    pub(super) ledc: EspLedcHandle,
    pub(super) rmt: EspRmtHandle,
    pub(super) pcnt: EspPcntHandle,
    pub(super) spi2: EspSpiHandle,
    pub(super) i2s: EspI2sHandle,
    pub(super) twai: [EspC6TwaiHandle; 2],
    pub(super) etm: EspEtmHandle,
    pub(super) parlio: EspParlioHandle,
    pub(super) gdma: EspC6GdmaHandle,
    pub(super) lp_uart: EspLpUartHandle,
    pub(super) lp_i2c: EspLpI2cHandle,
    pub(super) lp_watchdog: EspLpWatchdogHandle,
    pub(super) pmu: EspC6PmuHandle,
    pub(super) lp_clkrst: EspC6LpClkRstHandle,
    pub(super) lp_aon: EspC6LpAonHandle,
    pub(super) lp_timer: EspC6LpTimerHandle,
    pub(super) interrupt_matrix: EspC6InterruptMatrixHandle,
    pub(super) systimer: EspSystimerHandle,
    pub(super) sdio: EspSdioSlaveHandle,
    pub(super) wifi_mac: EspC6WifiMacHandle,
}

impl Esp32c6PeripheralHandles {
    fn clear_host_queues(&self) {
        let _ = self.spi2.take_tx();
        let _ = self.i2s.take_tx_words();
        for twai in &self.twai {
            let _ = twai.take_tx_frames();
        }
        let _ = self.etm.take_tasks();
        let _ = self.parlio.take_tx_words();
        let _ = self.gdma.take_output_words();
        let _ = self.lp_i2c.take_tx();
        for function in 0..8 {
            let _ = self.sdio.take_tx(function);
        }
    }

    pub(super) fn poll_outputs(&self, at: SimTime) -> Result<u64, DeviceError> {
        Ok(u64::from(self.ledc.poll(at)?) + u64::from(self.rmt.poll(at)?))
    }

    pub(super) fn observe_pin(
        &self,
        pin: u8,
        value: Logic,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        self.pcnt.observe_input(pin, value, at)
    }
}

/// Maps the complete ESP32-C6 non-radio peripheral inventory.
pub(super) fn map_esp32c6_peripherals(
    bus: &mut AddressSpace,
    signals: &SignalHub,
    chip_uarts: &mut Vec<UartHandle>,
) -> Result<(Esp32c6PeripheralHandles, EspUsbSerialJtagHandle), MachineError> {
    let (modem_syscon, modem_lpcon, modem) =
        EspC6ModemControl::new_pair("esp32c6.modem-syscon", "esp32c6.modem-lpcon");
    bus.map_device(
        "esp32c6.modem-syscon",
        0x600a_9800,
        0x800,
        Box::new(modem_syscon),
    )?;
    bus.map_device(
        "esp32c6.modem-lpcon",
        0x600a_f000,
        0x800,
        Box::new(modem_lpcon),
    )?;
    // PHY calibration accesses this radio front-end register page directly.
    // Its analog effects are represented by the functional PHY, while the
    // word state remains coherent for read/modify/write sequences.
    bus.map_device(
        "esp32c6.phy-registers",
        0x600a_d000,
        0x1000,
        Box::new(EspC6PhyRegisters::new("esp32c6.phy-registers")),
    )?;
    let (ble_modem_device, ble_modem) = EspC6BleModem::new("esp32c6.ble-modem-registers");
    bus.map_device(
        "esp32c6.ble-modem-registers",
        0x600a_e000,
        0x1000,
        Box::new(ble_modem_device),
    )?;
    bus.map_device(
        "esp32c6.phy-baseband-registers",
        0x600a_7000,
        0x1000,
        Box::new(EspC6ControlBlock::new(
            "esp32c6.phy-baseband-registers",
            0x1000,
            None,
            0,
        )),
    )?;
    bus.map_device(
        "esp32c6.phy-mac-registers",
        0x600a_2000,
        0x1000,
        Box::new(EspC6ControlBlock::new(
            "esp32c6.phy-mac-registers",
            0x1000,
            None,
            0,
        )),
    )?;
    let (ble_baseband_device, ble_baseband) =
        EspC6BleBaseband::new("esp32c6.ble-baseband-registers");
    bus.map_device(
        "esp32c6.ble-baseband-registers",
        0x600a_1000,
        0x1000,
        Box::new(ble_baseband_device),
    )?;
    bus.map_device(
        "esp32c6.phy-front-end-registers",
        0x600a_8000,
        0x1000,
        Box::new(EspC6ControlBlock::new(
            "esp32c6.phy-front-end-registers",
            0x1000,
            None,
            0,
        )),
    )?;
    let (ble_control_device, ble_control) = EspC6BleControl::new("esp32c6.ble-control-registers");
    bus.map_device(
        "esp32c6.ble-control-registers",
        0x600a_9000,
        0x800,
        Box::new(ble_control_device),
    )?;
    let wifi_mac_device = EspC6WifiMacRegisters::new("esp32c6.wifi-mac-registers");
    let wifi_mac = wifi_mac_device.handle();
    bus.map_device(
        "esp32c6.wifi-mac-registers",
        0x600a_4000,
        0x3000,
        Box::new(wifi_mac_device),
    )?;
    bus.map_device(
        "esp32c6.i2c-ana-mst",
        0x600a_f800,
        0x100,
        Box::new(EspAnalogI2c::new("esp32c6.i2c-ana-mst")),
    )?;
    // The closed PHY writes its generated analog-I2C command program into
    // this dedicated word-addressed SRAM before starting RF calibration.
    // Retain the words exactly: later PHY operations patch and read the same
    // program, while the analog side effects remain owned by EspAnalogI2c.
    bus.map_device(
        "esp32c6.phy-i2c-command-memory",
        0x600a_fc00,
        0x400,
        Box::new(EspC6ControlBlock::new(
            "esp32c6.phy-i2c-command-memory",
            0x400,
            None,
            0,
        )),
    )?;
    let (ieee802154_device, ieee802154) = EspIeee802154::new("esp32c6.ieee802154");
    bus.map_device(
        "esp32c6.ieee802154",
        0x600a_3000,
        0x188,
        Box::new(ieee802154_device),
    )?;

    let (uart0, uart0_handle) = FunctionalUart::new_lenient("esp32c6.uart0", 0x00, 0x1c, 0);
    bus.map_device("esp32c6.uart0", 0x6000_0000, 0x1000, Box::new(uart0))?;
    chip_uarts.push(uart0_handle.clone());
    let (uart1, uart1_handle) = FunctionalUart::new_lenient("esp32c6.uart1", 0x00, 0x1c, 0);
    bus.map_device("esp32c6.uart1", 0x6000_1000, 0x1000, Box::new(uart1))?;
    chip_uarts.push(uart1_handle.clone());

    let i2c = Esp32c6I2c::new("esp32c6.i2c0", signals.clone())?;
    bus.map_device("esp32c6.i2c0", 0x6000_4000, 0x1000, Box::new(i2c))?;

    let (spi2, spi2_handle) = EspSpi::new("esp32c6.spi2");
    bus.map_device("esp32c6.spi2", 0x6008_1000, 0x1000, Box::new(spi2))?;

    let (ledc, ledc_handle) = EspLedc::new("esp32c6.ledc", "board.esp32c6.ledc", signals.clone())?;
    bus.map_device("esp32c6.ledc", 0x6000_7000, 0x1000, Box::new(ledc))?;

    let (rmt, rmt_handle) = EspRmt::new("esp32c6.rmt", "board.esp32c6.rmt", signals.clone())?;
    bus.map_device("esp32c6.rmt", 0x6000_6000, 0x1000, Box::new(rmt))?;

    let (pcnt, pcnt_handle) = EspPcnt::new("esp32c6.pcnt", signals.clone())?;
    bus.map_device("esp32c6.pcnt", 0x6001_2000, 0x1000, Box::new(pcnt))?;

    bus.map_device(
        "esp32c6.mcpwm",
        0x6001_4000,
        0x1000,
        Box::new(EspMcpwm::new("esp32c6.mcpwm", signals.clone())?),
    )?;
    bus.map_device(
        "esp32c6.saradc",
        0x6000_e000,
        0x1000,
        Box::new(EspSarAdc::new("esp32c6.saradc", signals.clone())?),
    )?;

    let (twai0, twai0_handle) =
        EspC6Twai::new("esp32c6.twai0", "board.esp32c6.twai0", signals.clone())?;
    bus.map_device("esp32c6.twai0", 0x6000_b000, 0x1000, Box::new(twai0))?;
    let (twai1, twai1_handle) =
        EspC6Twai::new("esp32c6.twai1", "board.esp32c6.twai1", signals.clone())?;
    bus.map_device("esp32c6.twai1", 0x6000_d000, 0x1000, Box::new(twai1))?;

    let (i2s, i2s_handle) = EspI2s::new("esp32c6.i2s", "board.esp32c6.i2s", signals.clone())?;
    bus.map_device("esp32c6.i2s", 0x6000_c000, 0x1000, Box::new(i2s))?;
    let (etm, etm_handle) = EspEtm::new("esp32c6.etm", "board.esp32c6.etm", signals.clone())?;
    bus.map_device("esp32c6.etm", 0x6001_3000, 0x1000, Box::new(etm))?;
    let (parlio, parlio_handle) =
        EspParlio::new("esp32c6.parlio", "board.esp32c6.parlio", signals.clone())?;
    bus.map_device("esp32c6.parlio", 0x6001_5000, 0x1000, Box::new(parlio))?;

    let (gdma, gdma_handle) =
        EspC6Gdma::new("esp32c6.gdma", "board.esp32c6.gdma", signals.clone())?;
    bus.map_device("esp32c6.gdma", 0x6008_0000, 0x2b0, Box::new(gdma))?;

    let (lp_uart, lp_uart_output, lp_uart_handle) =
        EspLpUart::new("esp32c6.lp-uart", "board.esp32c6.lp_uart", signals.clone())?;
    bus.map_device("esp32c6.lp-uart", 0x600b_1400, 0x400, Box::new(lp_uart))?;
    chip_uarts.push(lp_uart_output.clone());
    let (lp_i2c, lp_i2c_handle) = EspLpI2c::new("esp32c6.lp-i2c");
    bus.map_device("esp32c6.lp-i2c", 0x600b_1800, 0x400, Box::new(lp_i2c))?;
    // The LP watchdog counts the nominal 136 kHz RTC slow clock while one
    // machine tick represents one guest instruction at a 40 MHz reset clock.
    // Use the conservative integer ratio; firmware raises the CPU frequency
    // later, which only increases the number of instructions per WDT count.
    let (lp_watchdog, lp_watchdog_handle) =
        EspLpWatchdog::new_with_tick_scale("esp32c6.lp-watchdog", 40_000_000 / 136_000);
    bus.map_device(
        "esp32c6.lp-watchdog",
        0x600b_1c00,
        0x400,
        Box::new(lp_watchdog),
    )?;
    let (pmu, pmu_handle) = EspC6Pmu::new("esp32c6.pmu");
    bus.map_device("esp32c6.pmu", 0x600b_0000, 0x400, Box::new(pmu))?;
    let (lp_clkrst, lp_clkrst_handle) = EspC6LpClkRst::new("esp32c6.lp-clkrst");
    bus.map_device("esp32c6.lp-clkrst", 0x600b_0400, 0x400, Box::new(lp_clkrst))?;
    let (lp_aon, lp_aon_handle) = EspC6LpAon::new("esp32c6.lp-aon");
    bus.map_device("esp32c6.lp-aon", 0x600b_1000, 0x400, Box::new(lp_aon))?;
    let (lp_timer, lp_timer_handle) = EspC6LpTimer::new("esp32c6.lp-timer");
    bus.map_device("esp32c6.lp-timer", 0x600b_0c00, 0x400, Box::new(lp_timer))?;

    let (hinf, slc, sdio_handle) = new_esp_sdio_slave("esp32c6.sdio");
    bus.map_device("esp32c6.hinf", 0x6001_6000, 0x1000, Box::new(hinf))?;
    bus.map_device("esp32c6.slc", 0x6001_7000, 0x1000, Box::new(slc))?;

    bus.map_device(
        "esp32c6.sha",
        0x6008_9000,
        0x1000,
        Box::new(EspSha::new("esp32c6.sha")),
    )?;
    bus.map_device(
        "esp32c6.aes",
        0x6008_8000,
        0x1000,
        Box::new(EspAes::new("esp32c6.aes")),
    )?;
    bus.map_device(
        "esp32c6.hmac",
        0x6008_d000,
        0x1000,
        Box::new(EspC6Hmac::new("esp32c6.hmac")),
    )?;
    bus.map_device(
        "esp32c6.rsa",
        0x6008_a000,
        0x1000,
        Box::new(EspRsa::new_esp32c6("esp32c6.rsa")),
    )?;
    bus.map_device(
        "esp32c6.ecc",
        0x6008_b000,
        0x1000,
        Box::new(EspC6Ecc::new("esp32c6.ecc")),
    )?;
    bus.map_device(
        "esp32c6.digital-signature",
        0x6008_c000,
        0x1000,
        Box::new(EspDigitalSignature::new_esp32c6(
            "esp32c6.digital-signature",
        )),
    )?;
    bus.map_device(
        "esp32c6.efuse",
        0x600b_0800,
        0x400,
        Box::new(EspC6Efuse::new("esp32c6.efuse")),
    )?;

    let (systimer, systimer_handle) = EspSystimer::new_esp32c6("esp32c6.systimer");
    bus.map_device("esp32c6.systimer", 0x6000_a000, 0x1000, Box::new(systimer))?;

    let (interrupt_matrix, interrupt_matrix_handle) =
        EspC6InterruptMatrix::new("esp32c6.interrupt-matrix");
    bus.map_device(
        "esp32c6.interrupt-matrix",
        0x6001_0000,
        0x800,
        Box::new(interrupt_matrix),
    )?;
    bus.map_device(
        "esp32c6.interrupt-priority",
        0x600c_5000,
        0x400,
        Box::new(EspC6InterruptPriority::new("esp32c6.interrupt-priority")),
    )?;
    bus.map_device(
        "esp32c6.io-mux",
        0x6009_0000,
        0x1000,
        Box::new(EspC6IoMux::new("esp32c6.io-mux")),
    )?;
    let (uhci, _uhci_handle) = EspC6Uhci::new(
        "esp32c6.uhci0",
        [uart0_handle, uart1_handle, lp_uart_output],
    );
    bus.map_device("esp32c6.uhci0", 0x6000_5000, 0x1000, Box::new(uhci))?;

    for (name, base, size, date_offset, date) in [
        ("esp32c6.atomic", 0x6001_1000, 0x1000, None, 0),
        ("esp32c6.slchost", 0x6001_8000, 0x1000, None, 0),
        ("esp32c6.pvt-monitor", 0x6001_9000, 0x1000, None, 0),
        (
            "esp32c6.mem-monitor",
            0x6009_2000,
            0x1000,
            Some(0x3fc),
            35_656_192,
        ),
        ("esp32c6.pau", 0x6009_3000, 0x1000, Some(0x3fc), 35_656_192),
        (
            "esp32c6.hp-system",
            0x6009_5000,
            0x1000,
            Some(0x3fc),
            35_656_192,
        ),
        ("esp32c6.tee", 0x6009_8000, 0x1000, Some(0x3fc), 35_656_192),
        (
            "esp32c6.hp-apm",
            0x6009_9000,
            0x800,
            Some(0x3fc),
            35_656_192,
        ),
        (
            "esp32c6.lp-apm0",
            0x6009_9800,
            0x800,
            Some(0x3fc),
            35_656_192,
        ),
        ("esp32c6.misc", 0x6009_f000, 0x1000, None, 0),
        (
            "esp32c6.trace",
            0x600c_0000,
            0x1000,
            Some(0x3fc),
            35_656_192,
        ),
    ] {
        bus.map_device(
            name,
            base,
            size,
            Box::new(EspC6ControlBlock::new(name, size, date_offset, date)),
        )?;
    }

    bus.map_device(
        "esp32c6.power-detector",
        0x600a_0000,
        0x1000,
        Box::new(EspC6PowerDetector::new("esp32c6.power-detector")),
    )?;

    // ESP-IDF consults these documented PCR reset fields during its direct
    // application handoff. The XTAL path starts at 40 MHz and the unused
    // high-speed MSPI divider still resets to its hardware value of three.
    let pcr = EspC6ControlBlock::new("esp32c6.pcr", 0x1000, Some(0xffc), 35_656_192)
        .with_reset_word(0x1c, 3 << 8)
        .with_reset_word(0x110, (2 << 8) | (40 << 24));
    bus.map_device("esp32c6.pcr", 0x6009_6000, 0x1000, Box::new(pcr))?;

    for (name, base) in [
        ("esp32c6.lp-io", 0x600b_2000),
        ("esp32c6.lp-i2c-analog", 0x600b_2400),
        ("esp32c6.lp-peripheral", 0x600b_2800),
        ("esp32c6.lp-analog", 0x600b_2c00),
        ("esp32c6.lp-tee", 0x600b_3400),
        ("esp32c6.lp-apm", 0x600b_3800),
        ("esp32c6.otp-debug", 0x600b_3c00),
    ] {
        bus.map_device(
            name,
            base,
            0x400,
            Box::new(EspC6ControlBlock::new(name, 0x400, Some(0xfc), 35_656_192)),
        )?;
    }

    let (usb_serial_jtag, usb_serial_jtag_handle) =
        EspUsbSerialJtag::new("esp32c6.usb-serial-jtag");
    bus.map_device(
        "esp32c6.usb-serial-jtag",
        0x6000_f000,
        0x1000,
        Box::new(usb_serial_jtag),
    )?;

    let peripherals = Esp32c6PeripheralHandles {
        modem,
        ble_modem,
        ble_baseband,
        ble_control,
        ieee802154,
        ledc: ledc_handle,
        rmt: rmt_handle,
        pcnt: pcnt_handle,
        spi2: spi2_handle,
        i2s: i2s_handle,
        twai: [twai0_handle, twai1_handle],
        etm: etm_handle,
        parlio: parlio_handle,
        gdma: gdma_handle,
        lp_uart: lp_uart_handle,
        lp_i2c: lp_i2c_handle,
        lp_watchdog: lp_watchdog_handle,
        pmu: pmu_handle,
        lp_clkrst: lp_clkrst_handle,
        lp_aon: lp_aon_handle,
        lp_timer: lp_timer_handle,
        interrupt_matrix: interrupt_matrix_handle,
        systimer: systimer_handle,
        sdio: sdio_handle,
        wifi_mac,
    };
    peripherals.clear_host_queues();
    Ok((peripherals, usb_serial_jtag_handle))
}
