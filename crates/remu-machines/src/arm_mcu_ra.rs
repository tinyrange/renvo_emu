use super::*;
use remu_devices::{
    RA4M1_EVENT_ADC0_SCAN_END, RA4M1_EVENT_AGT0_INT, RA4M1_EVENT_AGT1_INT,
    RA4M1_EVENT_GPT1_OVERFLOW, RA4M1_EVENT_GPT2_OVERFLOW, RA4M1_EVENT_GPT3_OVERFLOW,
    RA4M1_EVENT_GPT4_OVERFLOW, RA4M1_EVENT_GPT5_OVERFLOW, RA4M1_EVENT_GPT6_OVERFLOW,
    RA4M1_EVENT_GPT7_OVERFLOW, RA4M1_EVENT_KINT, RA4M1_EVENT_RTC_ALARM, RaAdcHandle, RaAgt,
    RaAgtHandle, RaCac, RaCacHandle, RaCrc, RaCrcHandle, RaDac, RaDacHandle, RaDoc, RaDocHandle,
    RaElc, RaElcHandle, RaGptHandle, RaIic, RaKint, RaKintHandle, RaPoeg, RaPoegHandle, RaRtc,
    RaRtcHandle, RaSpi,
};

pub(super) struct RaMachineState {
    pub(super) agt: Vec<(u16, RaAgtHandle)>,
    pub(super) gpt: Vec<(u16, RaGptHandle)>,
    pub(super) kint: RaKintHandle,
    pub(super) elc: RaElcHandle,
    pub(super) rtc: RaRtcHandle,
    pub(super) dac: RaDacHandle,
    pub(super) crc: RaCrcHandle,
    pub(super) doc: RaDocHandle,
    pub(super) cac: RaCacHandle,
    pub(super) poeg: RaPoegHandle,
    pub(super) adc: RaAdcHandle,
    kint_irq_signal: SignalId,
    elc_event_signal: SignalId,
    elc_strobe_signal: SignalId,
    adc_irq_signal: SignalId,
    elc_strobe: bool,
}

impl ArmMcuMachine {
    pub(super) fn create_ra4m1(
        bus: &mut AddressSpace,
        signals: SignalHub,
    ) -> Result<
        (
            GpioHandle,
            RaSciHandle,
            RaGptHandle,
            RaIcuHandle,
            RaMachineState,
        ),
        ArmMachineError,
    > {
        let kint_irq_signal = signals.declare(
            "board.r7fa4m1ab3cfm.kint.irq",
            SignalValue::from_u64(0, 1)?,
            Some("KINT interrupt request".to_owned()),
        )?;
        let elc_event_signal = signals.declare(
            "board.r7fa4m1ab3cfm.elc.event",
            SignalValue::from_u64(0, 9)?,
            Some("ELC event source".to_owned()),
        )?;
        let elc_strobe_signal = signals.declare(
            "board.r7fa4m1ab3cfm.elc.strobe",
            SignalValue::from_u64(0, 1)?,
            Some("toggles for each ELC software event".to_owned()),
        )?;
        let adc_irq_signal = signals.declare(
            "board.r7fa4m1ab3cfm.adc0.irq",
            SignalValue::from_u64(0, 1)?,
            Some("ADC140 group-A scan-end request".to_owned()),
        )?;

        let mut ports = Vec::new();
        let mut handles = Vec::new();
        for port in 0..15 {
            let (device, handle) = RaIoPort::new(
                format!("r7fa4m1ab3cfm.port{port}"),
                &format!("board.r7fa4m1ab3cfm.port{port}"),
                signals.clone(),
            )?;
            ports.push(device);
            handles.push(handle);
        }
        let pfs = RaPfs::new("r7fa4m1ab3cfm.pfs", &ports);
        let (gpt0_device, timer) = RaGpt::new("r7fa4m1ab3cfm.gpt0");
        let gpt_events = [
            RA4M1_EVENT_GPT1_OVERFLOW,
            RA4M1_EVENT_GPT2_OVERFLOW,
            RA4M1_EVENT_GPT3_OVERFLOW,
            RA4M1_EVENT_GPT4_OVERFLOW,
            RA4M1_EVENT_GPT5_OVERFLOW,
            RA4M1_EVENT_GPT6_OVERFLOW,
            RA4M1_EVENT_GPT7_OVERFLOW,
        ];
        let mut gpt_devices = Vec::new();
        let mut gpt = Vec::new();
        for (offset, event) in gpt_events.into_iter().enumerate() {
            let index = offset + 1;
            let (device, handle) = if index <= 2 {
                RaGpt::new(format!("r7fa4m1ab3cfm.gpt{index}"))
            } else {
                RaGpt::new_16(format!("r7fa4m1ab3cfm.gpt{index}"))
            };
            gpt_devices.push(device);
            gpt.push((event, handle));
        }

        let (sci9_device, uart) = RaSci::new("r7fa4m1ab3cfm.sci9");
        let (spi0_device, _) = RaSpi::new("r7fa4m1ab3cfm.spi0");
        let (spi1_device, _) = RaSpi::new("r7fa4m1ab3cfm.spi1");
        let (icu_device, icu) = RaIcu::new("r7fa4m1ab3cfm.icu");
        let (agt0_device, agt0) = RaAgt::new("r7fa4m1ab3cfm.agt0");
        let (agt1_device, agt1) = RaAgt::new("r7fa4m1ab3cfm.agt1");
        let (iic0_device, _) = RaIic::new("r7fa4m1ab3cfm.iic0");
        let (iic1_device, _) = RaIic::new("r7fa4m1ab3cfm.iic1");
        let (rtc_device, rtc) = RaRtc::new("r7fa4m1ab3cfm.rtc");
        let (dac_device, dac) = RaDac::new(
            "r7fa4m1ab3cfm.dac12",
            "board.r7fa4m1ab3cfm.dac0",
            signals.clone(),
        )?;
        let (crc_device, crc) = RaCrc::new("r7fa4m1ab3cfm.crc");
        let (doc_device, doc) = RaDoc::new("r7fa4m1ab3cfm.doc");
        let (cac_device, cac) = RaCac::new("r7fa4m1ab3cfm.cac");
        let (poeg_device, poeg) = RaPoeg::new("r7fa4m1ab3cfm.poeg");
        let (kint_device, kint) = RaKint::new("r7fa4m1ab3cfm.kint");
        let (elc_device, elc) = RaElc::new("r7fa4m1ab3cfm.elc");
        let (adc_device, adc) = remu_devices::RaAdc::new("r7fa4m1ab3cfm.adc0");
        Self::map_ra4m1(
            bus,
            ports,
            pfs,
            icu_device,
            gpt0_device,
            gpt_devices,
            kint_device,
            elc_device,
            sci9_device,
            agt0_device,
            agt1_device,
            spi0_device,
            spi1_device,
            iic0_device,
            iic1_device,
            rtc_device,
            dac_device,
            crc_device,
            doc_device,
            cac_device,
            poeg_device,
            adc_device,
        )?;
        let gpio = handles.remove(1);
        Ok((
            gpio,
            uart,
            timer,
            icu,
            RaMachineState {
                agt: vec![(RA4M1_EVENT_AGT0_INT, agt0), (RA4M1_EVENT_AGT1_INT, agt1)],
                gpt,
                kint,
                elc,
                rtc,
                dac,
                crc,
                doc,
                cac,
                poeg,
                adc,
                kint_irq_signal,
                elc_event_signal,
                elc_strobe_signal,
                adc_irq_signal,
                elc_strobe: false,
            },
        ))
    }

    pub(super) fn poll_ra4m1(&mut self) -> Result<bool, ArmMachineError> {
        let Some(ra) = &self.ra else {
            return Ok(false);
        };
        let kint_inputs = (0..8).fold(0_u8, |value, pin| {
            let pin = u8::try_from(pin).expect("KINT pin index fits u8");
            value | (u8::from(self.gpio.resolved(pin) == Ok(Logic::One)) << pin)
        });
        let kint_pending = ra.kint.poll(kint_inputs);
        let rtc_pending = ra.rtc.poll(self.now);
        let adc_pending = ra.adc.poll(self.now);
        let mut pending = kint_pending || rtc_pending || adc_pending;

        self.signals.set(
            ra.kint_irq_signal,
            SignalValue::from_u64(u64::from(kint_pending), 1)?,
            self.now,
        )?;
        self.signals.set(
            ra.adc_irq_signal,
            SignalValue::from_u64(u64::from(adc_pending), 1)?,
            self.now,
        )?;
        let Some(icu) = &self.ra_icu else {
            return Ok(pending);
        };
        for (event_pending, event) in [
            (kint_pending, RA4M1_EVENT_KINT),
            (rtc_pending, RA4M1_EVENT_RTC_ALARM),
            (adc_pending, RA4M1_EVENT_ADC0_SCAN_END),
        ] {
            if event_pending {
                for line in icu.route_event(event) {
                    self.cpu
                        .set_interrupt(line, self.ppb.interrupt_enabled(line))?;
                }
            }
        }
        for (event, timer) in &ra.gpt {
            if timer.poll(self.now) {
                pending = true;
                for line in icu.route_event(*event) {
                    self.cpu
                        .set_interrupt(line, self.ppb.interrupt_enabled(line))?;
                }
            }
        }
        for (event, timer) in &ra.agt {
            if timer.poll(self.now) {
                pending = true;
                for line in icu.route_event(*event) {
                    self.cpu
                        .set_interrupt(line, self.ppb.interrupt_enabled(line))?;
                }
            }
        }
        Ok(pending)
    }

    pub(super) fn trace_ra4m1_events(&mut self) -> Result<(), ArmMachineError> {
        let Some(ra) = &mut self.ra else {
            return Ok(());
        };
        for event in ra.elc.take_software_events() {
            self.signals.set(
                ra.elc_event_signal,
                SignalValue::from_u64(u64::from(event), 9)?,
                self.now,
            )?;
            ra.elc_strobe = !ra.elc_strobe;
            self.signals.set(
                ra.elc_strobe_signal,
                SignalValue::from_u64(u64::from(ra.elc_strobe), 1)?,
                self.now,
            )?;
        }
        Ok(())
    }

    /// Current host-visible RA4M1 DAC12 channel 0 sample, when present.
    pub fn dac_value(&self) -> Option<u16> {
        self.ra.as_ref().map(|ra| ra.dac.value())
    }

    /// Current host-visible RA4M1 CRC result, when present.
    pub fn crc_value(&self) -> Option<u32> {
        self.ra.as_ref().map(|ra| ra.crc.value())
    }

    /// Current host-visible RA4M1 DOC result, when present.
    pub fn doc_result(&self) -> Option<u16> {
        self.ra.as_ref().map(|ra| ra.doc.result())
    }

    /// Returns the host-facing RA4M1 CAC measurement state.
    pub fn cac(&self) -> Option<RaCacHandle> {
        self.ra.as_ref().map(|ra| ra.cac.clone())
    }

    /// Returns the host-facing RA4M1 POEG groups.
    pub fn poeg(&self) -> Option<RaPoegHandle> {
        self.ra.as_ref().map(|ra| ra.poeg.clone())
    }
}
