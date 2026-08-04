use super::*;

const ESP_RTC_ULP_INT_BIT: u32 = 1 << 5;
const ESP_RTC_ULP_TIMER_ENABLE: u32 = 1 << 31;
const ESP_RTC_ULP_TIMER_PERIOD_MASK: u32 = 0x00ff_ffff;
const ESP_RTC_INT_MASK: u32 = 0x001f_ffff;

/// Native ESP32-S3 RTC control and shared SAR register identifiers.
///
/// The offsets are taken from Espressif's `rtc_cntl_reg.h` and `sens_reg.h`.
/// Keeping the identifiers typed prevents callers from silently using a
/// register from the wrong peripheral or an address in a reserved hole.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u16)]
#[allow(missing_docs)]
pub enum Esp32S3RtcRegister {
    Options0 = 0x000,
    SlpTimer0 = 0x004,
    SlpTimer1 = 0x008,
    TimeUpdate = 0x00c,
    TimeLow0 = 0x010,
    TimeHigh0 = 0x014,
    State0 = 0x018,
    Timer1 = 0x01c,
    Timer2 = 0x020,
    Timer3 = 0x024,
    Timer4 = 0x028,
    Timer5 = 0x02c,
    Timer6 = 0x030,
    AnaConf = 0x034,
    ResetState = 0x038,
    WakeupState = 0x03c,
    IntEna = 0x040,
    IntRaw = 0x044,
    IntSt = 0x048,
    IntClr = 0x04c,
    Store0 = 0x050,
    Store1 = 0x054,
    Store2 = 0x058,
    Store3 = 0x05c,
    ExtXtlConf = 0x060,
    ExtWakeupConf = 0x064,
    SlpRejectConf = 0x068,
    CpuPeriodConf = 0x06c,
    SdioActConf = 0x070,
    ClkConf = 0x074,
    SlowClkConf = 0x078,
    SdioConf = 0x07c,
    BiasConf = 0x080,
    Pwc = 0x088,
    RegulatorDrvCtrl = 0x08c,
    DigPwc = 0x090,
    DigIso = 0x094,
    WdtConfig0 = 0x098,
    WdtConfig1 = 0x09c,
    WdtConfig2 = 0x0a0,
    WdtConfig3 = 0x0a4,
    WdtConfig4 = 0x0a8,
    WdtFeed = 0x0ac,
    WdtWprotect = 0x0b0,
    SwdConf = 0x0b4,
    SwdWprotect = 0x0b8,
    SwCpuStall = 0x0bc,
    Store4 = 0x0c0,
    Store5 = 0x0c4,
    Store6 = 0x0c8,
    Store7 = 0x0cc,
    LowPowerSt = 0x0d0,
    Diag0 = 0x0d4,
    PadHold = 0x0d8,
    DigPadHold = 0x0dc,
    ExtWakeup1 = 0x0e0,
    ExtWakeup1Status = 0x0e4,
    BrownOut = 0x0e8,
    TimeLow1 = 0x0ec,
    TimeHigh1 = 0x0f0,
    Xtal32kClkFactor = 0x0f4,
    Xtal32kConf = 0x0f8,
    UlpCpTimer = 0x0fc,
    UlpCpCtrl = 0x100,
    CoCpuCtrl = 0x104,
    TouchCtrl1 = 0x108,
    TouchCtrl2 = 0x10c,
    TouchScanCtrl = 0x110,
    TouchSlpThres = 0x114,
    TouchApproach = 0x118,
    TouchFilterCtrl = 0x11c,
    UsbConf = 0x120,
    TouchTimeoutCtrl = 0x124,
    SlpRejectCause = 0x128,
    Option1 = 0x12c,
    SlpWakeupCause = 0x130,
    UlpCpTimer1 = 0x134,
    IntEnaW1ts = 0x138,
    IntEnaW1tc = 0x13c,
    RetentionCtrl = 0x140,
    PgCtrl = 0x144,
    FibSel = 0x148,
    TouchDac = 0x14c,
    TouchDac1 = 0x150,
    CoCpuDisable = 0x154,
    Date = 0x1fc,
    /// `SENS_SAR_MEAS1_CTRL2_REG`, visible in the shared RTC/SENS window.
    SarMeas1Ctrl2 = 0x80c,
    /// `SENS_SAR_MEAS2_CTRL2_REG`, visible in the shared RTC/SENS window.
    SarMeas2Ctrl2 = 0x830,
}

impl Esp32S3RtcRegister {
    /// Returns the native byte offset in the mapped RTC/SENS window.
    pub const fn offset(self) -> u64 {
        self as u64
    }

    /// Resolves a native byte offset. Reserved holes return `None`.
    pub const fn from_offset(offset: u64) -> Option<Self> {
        Some(match offset {
            0x000 => Self::Options0,
            0x004 => Self::SlpTimer0,
            0x008 => Self::SlpTimer1,
            0x00c => Self::TimeUpdate,
            0x010 => Self::TimeLow0,
            0x014 => Self::TimeHigh0,
            0x018 => Self::State0,
            0x01c => Self::Timer1,
            0x020 => Self::Timer2,
            0x024 => Self::Timer3,
            0x028 => Self::Timer4,
            0x02c => Self::Timer5,
            0x030 => Self::Timer6,
            0x034 => Self::AnaConf,
            0x038 => Self::ResetState,
            0x03c => Self::WakeupState,
            0x040 => Self::IntEna,
            0x044 => Self::IntRaw,
            0x048 => Self::IntSt,
            0x04c => Self::IntClr,
            0x050 => Self::Store0,
            0x054 => Self::Store1,
            0x058 => Self::Store2,
            0x05c => Self::Store3,
            0x060 => Self::ExtXtlConf,
            0x064 => Self::ExtWakeupConf,
            0x068 => Self::SlpRejectConf,
            0x06c => Self::CpuPeriodConf,
            0x070 => Self::SdioActConf,
            0x074 => Self::ClkConf,
            0x078 => Self::SlowClkConf,
            0x07c => Self::SdioConf,
            0x080 => Self::BiasConf,
            0x088 => Self::Pwc,
            0x08c => Self::RegulatorDrvCtrl,
            0x090 => Self::DigPwc,
            0x094 => Self::DigIso,
            0x098 => Self::WdtConfig0,
            0x09c => Self::WdtConfig1,
            0x0a0 => Self::WdtConfig2,
            0x0a4 => Self::WdtConfig3,
            0x0a8 => Self::WdtConfig4,
            0x0ac => Self::WdtFeed,
            0x0b0 => Self::WdtWprotect,
            0x0b4 => Self::SwdConf,
            0x0b8 => Self::SwdWprotect,
            0x0bc => Self::SwCpuStall,
            0x0c0 => Self::Store4,
            0x0c4 => Self::Store5,
            0x0c8 => Self::Store6,
            0x0cc => Self::Store7,
            0x0d0 => Self::LowPowerSt,
            0x0d4 => Self::Diag0,
            0x0d8 => Self::PadHold,
            0x0dc => Self::DigPadHold,
            0x0e0 => Self::ExtWakeup1,
            0x0e4 => Self::ExtWakeup1Status,
            0x0e8 => Self::BrownOut,
            0x0ec => Self::TimeLow1,
            0x0f0 => Self::TimeHigh1,
            0x0f4 => Self::Xtal32kClkFactor,
            0x0f8 => Self::Xtal32kConf,
            0x0fc => Self::UlpCpTimer,
            0x100 => Self::UlpCpCtrl,
            0x104 => Self::CoCpuCtrl,
            0x108 => Self::TouchCtrl1,
            0x10c => Self::TouchCtrl2,
            0x110 => Self::TouchScanCtrl,
            0x114 => Self::TouchSlpThres,
            0x118 => Self::TouchApproach,
            0x11c => Self::TouchFilterCtrl,
            0x120 => Self::UsbConf,
            0x124 => Self::TouchTimeoutCtrl,
            0x128 => Self::SlpRejectCause,
            0x12c => Self::Option1,
            0x130 => Self::SlpWakeupCause,
            0x134 => Self::UlpCpTimer1,
            0x138 => Self::IntEnaW1ts,
            0x13c => Self::IntEnaW1tc,
            0x140 => Self::RetentionCtrl,
            0x144 => Self::PgCtrl,
            0x148 => Self::FibSel,
            0x14c => Self::TouchDac,
            0x150 => Self::TouchDac1,
            0x154 => Self::CoCpuDisable,
            0x1fc => Self::Date,
            0x80c => Self::SarMeas1Ctrl2,
            0x830 => Self::SarMeas2Ctrl2,
            _ => return None,
        })
    }

    /// Bits returned by a native read of this register.
    pub const fn read_mask(self) -> u32 {
        match self {
            Self::TimeUpdate => 0x3800_0000,
            Self::TimeLow0 | Self::TimeLow1 => u32::MAX,
            Self::TimeHigh0 | Self::TimeHigh1 => 0x0000_ffff,
            Self::ResetState
            | Self::WakeupState
            | Self::IntRaw
            | Self::IntSt
            | Self::ExtWakeup1Status
            | Self::SlpRejectCause
            | Self::SlpWakeupCause => 0x001f_ffff,
            Self::IntEna => ESP_RTC_INT_MASK,
            Self::IntClr | Self::IntEnaW1ts | Self::IntEnaW1tc | Self::WdtFeed => 0,
            Self::UlpCpTimer => 0xa000_07ff,
            Self::UlpCpCtrl => 0xf03f_ffff,
            Self::UlpCpTimer1 => 0xffffff00,
            Self::Date => 0x0fff_ffff,
            Self::SarMeas1Ctrl2 | Self::SarMeas2Ctrl2 => u32::MAX,
            _ => u32::MAX,
        }
    }

    /// Bits accepted by a native write of this register.
    pub const fn write_mask(self) -> u32 {
        match self {
            Self::TimeUpdate => 0xb800_0000,
            Self::TimeLow0
            | Self::TimeHigh0
            | Self::TimeLow1
            | Self::TimeHigh1
            | Self::ResetState
            | Self::WakeupState
            | Self::IntRaw
            | Self::IntSt
            | Self::ExtWakeup1Status
            | Self::SlpRejectCause
            | Self::SlpWakeupCause => 0,
            Self::IntEna => ESP_RTC_INT_MASK,
            Self::IntClr | Self::IntEnaW1ts | Self::IntEnaW1tc => ESP_RTC_INT_MASK,
            Self::UlpCpTimer => 0xe000_07ff,
            Self::UlpCpCtrl => 0xf07f_ffff,
            Self::UlpCpTimer1 => 0xffffff00,
            Self::Date => 0x0fff_ffff,
            Self::SarMeas1Ctrl2 | Self::SarMeas2Ctrl2 => 0xfffe_0000,
            _ => u32::MAX,
        }
    }
}

struct EspRtcControlState {
    registers: BTreeMap<Esp32S3RtcRegister, u32>,
    ulp_started: bool,
    ulp_last_tick: u64,
    ulp_wakeups: u64,
    ulp_signal: Option<(SignalHub, SignalId)>,
}

impl EspRtcControlState {
    fn new(ulp_signal: Option<(SignalHub, SignalId)>) -> Self {
        let mut state = Self {
            registers: BTreeMap::new(),
            ulp_started: false,
            ulp_last_tick: 0,
            ulp_wakeups: 0,
            ulp_signal,
        };
        state.reset();
        state
    }

    fn register(&self, register: Esp32S3RtcRegister) -> u32 {
        self.registers.get(&register).copied().unwrap_or_default()
    }

    fn set_register(&mut self, register: Esp32S3RtcRegister, value: u32) {
        self.registers.insert(register, value);
    }

    fn signal(&self, value: bool, at: SimTime) {
        if let Some((hub, signal)) = &self.ulp_signal {
            hub.set(
                *signal,
                SignalValue::from_u64(u64::from(value), 1)
                    .expect("ULP interrupt signal is one bit wide"),
                at,
            )
            .expect("ULP interrupt signal remains declared");
        }
    }

    fn refresh_interrupt_status(&mut self) {
        let status =
            self.register(Esp32S3RtcRegister::IntRaw) & self.register(Esp32S3RtcRegister::IntEna);
        self.set_register(Esp32S3RtcRegister::IntSt, status & ESP_RTC_INT_MASK);
    }

    fn refresh_ulp(&mut self, now: SimTime) {
        let timer = self.register(Esp32S3RtcRegister::UlpCpTimer);
        let period = (self.register(Esp32S3RtcRegister::UlpCpTimer1) >> 8
            & ESP_RTC_ULP_TIMER_PERIOD_MASK)
            .max(1);
        if timer & ESP_RTC_ULP_TIMER_ENABLE == 0 || !self.ulp_started {
            return;
        }
        let elapsed = now.ticks().saturating_sub(self.ulp_last_tick);
        let periods = elapsed / u64::from(period);
        if periods == 0 {
            return;
        }
        self.ulp_last_tick = self
            .ulp_last_tick
            .saturating_add(periods.saturating_mul(u64::from(period)));
        self.ulp_wakeups = self.ulp_wakeups.saturating_add(periods);
        self.set_register(
            Esp32S3RtcRegister::IntRaw,
            self.register(Esp32S3RtcRegister::IntRaw) | ESP_RTC_ULP_INT_BIT,
        );
        self.refresh_interrupt_status();
        self.signal(true, now);
    }

    fn clear_interrupts(&mut self, mask: u32, at: SimTime) {
        self.set_register(
            Esp32S3RtcRegister::IntRaw,
            self.register(Esp32S3RtcRegister::IntRaw) & !mask,
        );
        self.refresh_interrupt_status();
        if mask & ESP_RTC_ULP_INT_BIT != 0 {
            self.signal(false, at);
        }
    }

    fn reset(&mut self) {
        self.registers.clear();
        self.set_register(Esp32S3RtcRegister::UlpCpTimer1, 200 << 8);
        self.set_register(Esp32S3RtcRegister::UlpCpCtrl, (512 << 11) | 512);
        self.set_register(Esp32S3RtcRegister::Xtal32kConf, 0x0ff0_0000);
        self.set_register(Esp32S3RtcRegister::Date, 0x0210_1271);
        self.ulp_started = false;
        self.ulp_last_tick = 0;
        self.ulp_wakeups = 0;
        self.signal(false, SimTime::ZERO);
    }
}

/// Host-side view of the ESP32-S3 ULP/RTC wakeup path.
#[derive(Clone)]
pub struct EspRtcControlHandle {
    state: Rc<RefCell<EspRtcControlState>>,
}

impl EspRtcControlHandle {
    /// Returns true when the ULP interrupt is both raw and enabled.
    pub fn ulp_pending(&self, now: SimTime) -> bool {
        let mut state = self.state.borrow_mut();
        state.refresh_ulp(now);
        state.register(Esp32S3RtcRegister::IntRaw)
            & state.register(Esp32S3RtcRegister::IntEna)
            & ESP_RTC_ULP_INT_BIT
            != 0
    }

    /// Returns the number of deterministic ULP timer wakeups observed.
    pub fn ulp_wakeups(&self) -> u64 {
        self.state.borrow().ulp_wakeups
    }

    /// Reports whether RTC-domain writes to a pad are held at their prior value.
    pub fn pad_held(&self, pin: u8) -> bool {
        pin < 22 && self.state.borrow().register(Esp32S3RtcRegister::PadHold) & (1 << pin) != 0
    }
}

/// Functional ESP32-S3 RTC control and ULP timer block.
pub struct EspRtcControl {
    name: String,
    state: Rc<RefCell<EspRtcControlState>>,
}

impl EspRtcControl {
    /// Creates the RTC control page in its power-on state.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            state: Rc::new(RefCell::new(EspRtcControlState::new(None))),
        }
    }

    /// Creates the RTC page, host handle, and traceable ULP interrupt signal.
    pub fn new_with_signals(
        name: impl Into<String>,
        hub: SignalHub,
    ) -> Result<(Self, EspRtcControlHandle), SignalError> {
        let signal = hub.declare(
            "board.esp32s3.ulp.interrupt",
            SignalValue::from_u64(0, 1)?,
            Some("ESP32-S3 ULP timer interrupt".to_string()),
        )?;
        let state = Rc::new(RefCell::new(EspRtcControlState::new(Some((hub, signal)))));
        Ok((
            Self {
                name: name.into(),
                state: state.clone(),
            },
            EspRtcControlHandle { state },
        ))
    }
}

impl Device for EspRtcControl {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP RTC control requires aligned word access",
            ));
        }
        let register = Esp32S3RtcRegister::from_offset(offset).ok_or_else(|| {
            DeviceError::new(format!(
                "unsupported ESP32-S3 RTC register offset {offset:#x}"
            ))
        })?;
        let mut state = self.state.borrow_mut();
        state.refresh_ulp(at);
        Ok(u64::from(state.register(register) & register.read_mask()))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP RTC control requires aligned word access",
            ));
        }
        let register = Esp32S3RtcRegister::from_offset(offset).ok_or_else(|| {
            DeviceError::new(format!(
                "unsupported ESP32-S3 RTC register offset {offset:#x}"
            ))
        })?;
        let value = u32::try_from(value).map_err(|_| {
            DeviceError::new(format!(
                "ESP32-S3 RTC word write exceeds 32 bits: {value:#x}"
            ))
        })?;
        let mut state = self.state.borrow_mut();
        state.refresh_ulp(at);
        match register {
            Esp32S3RtcRegister::TimeUpdate => {
                state.set_register(register, value & register.write_mask() & !(1 << 31));
                if value & (1 << 31) != 0 {
                    let counter = at.ticks();
                    state.set_register(Esp32S3RtcRegister::TimeLow0, counter as u32);
                    state.set_register(Esp32S3RtcRegister::TimeHigh0, (counter >> 32) as u32);
                }
            }
            Esp32S3RtcRegister::IntClr => {
                state.set_register(register, 0);
                state.clear_interrupts(value & register.write_mask(), at);
            }
            Esp32S3RtcRegister::IntEnaW1ts => {
                state.set_register(register, 0);
                let enabled =
                    state.register(Esp32S3RtcRegister::IntEna) | (value & register.write_mask());
                state.set_register(Esp32S3RtcRegister::IntEna, enabled & ESP_RTC_INT_MASK);
                state.refresh_interrupt_status();
            }
            Esp32S3RtcRegister::IntEnaW1tc => {
                state.set_register(register, 0);
                let enabled =
                    state.register(Esp32S3RtcRegister::IntEna) & !(value & register.write_mask());
                state.set_register(Esp32S3RtcRegister::IntEna, enabled & ESP_RTC_INT_MASK);
                state.refresh_interrupt_status();
            }
            Esp32S3RtcRegister::UlpCpTimer => {
                // GPIO wake-clear is a write-only strobe; GPIO wake semantics
                // are outside this functional ULP timer slice.
                state.set_register(register, value & register.write_mask() & !(1 << 30));
            }
            Esp32S3RtcRegister::UlpCpCtrl => {
                state.set_register(register, value & register.write_mask());
                if value & ((1 << 31) | (1 << 30)) != 0 {
                    state.ulp_started = true;
                    state.ulp_last_tick = at.ticks();
                }
                if value & (1 << 29) != 0 {
                    state.ulp_started = false;
                    state.clear_interrupts(ESP_RTC_ULP_INT_BIT, at);
                }
            }
            Esp32S3RtcRegister::SarMeas1Ctrl2 | Esp32S3RtcRegister::SarMeas2Ctrl2 => {
                let mut stored = value & register.write_mask();
                if value & (1 << 17) != 0 {
                    stored = (stored & !(1 << 17)) | (1 << 16);
                }
                state.set_register(register, stored);
            }
            _ if register.write_mask() == 0 => {}
            _ => {
                let old = state.register(register);
                state.set_register(
                    register,
                    (old & !register.write_mask()) | (value & register.write_mask()),
                );
                if register == Esp32S3RtcRegister::IntEna {
                    state.refresh_interrupt_status();
                }
            }
        }
        state.refresh_ulp(at);
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state.borrow_mut().reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ulp_timer_sets_and_clears_the_native_rtc_interrupt() {
        let hub = SignalHub::new();
        let (mut device, handle) = EspRtcControl::new_with_signals("rtc", hub).unwrap();
        device
            .write(
                Esp32S3RtcRegister::UlpCpTimer1.offset(),
                AccessWidth::Word,
                4_u64 << 8,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Esp32S3RtcRegister::IntEna.offset(),
                AccessWidth::Word,
                u64::from(ESP_RTC_ULP_INT_BIT),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Esp32S3RtcRegister::UlpCpCtrl.offset(),
                AccessWidth::Word,
                1_u64 << 31,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Esp32S3RtcRegister::UlpCpTimer.offset(),
                AccessWidth::Word,
                u64::from(ESP_RTC_ULP_TIMER_ENABLE),
                SimTime::ZERO,
            )
            .unwrap();

        assert!(!handle.ulp_pending(SimTime::from_ticks(3)));
        assert!(handle.ulp_pending(SimTime::from_ticks(4)));
        assert_eq!(handle.ulp_wakeups(), 1);
        assert_eq!(
            device
                .read(
                    Esp32S3RtcRegister::IntRaw.offset(),
                    AccessWidth::Word,
                    SimTime::ZERO,
                )
                .unwrap(),
            1 << 5
        );
        assert_eq!(
            device
                .read(
                    Esp32S3RtcRegister::IntSt.offset(),
                    AccessWidth::Word,
                    SimTime::ZERO,
                )
                .unwrap(),
            1 << 5
        );

        device
            .write(
                Esp32S3RtcRegister::IntClr.offset(),
                AccessWidth::Word,
                u64::from(ESP_RTC_ULP_INT_BIT),
                SimTime::ZERO,
            )
            .unwrap();
        assert!(!handle.ulp_pending(SimTime::from_ticks(4)));
    }

    #[test]
    fn interrupt_status_is_raw_source_gated_by_enable() {
        let mut device = EspRtcControl::new("rtc");
        device
            .write(
                Esp32S3RtcRegister::UlpCpTimer1.offset(),
                AccessWidth::Word,
                4_u64 << 8,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Esp32S3RtcRegister::UlpCpCtrl.offset(),
                AccessWidth::Word,
                1_u64 << 31,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Esp32S3RtcRegister::UlpCpTimer.offset(),
                AccessWidth::Word,
                u64::from(ESP_RTC_ULP_TIMER_ENABLE),
                SimTime::ZERO,
            )
            .unwrap();

        assert_eq!(
            device
                .read(
                    Esp32S3RtcRegister::IntRaw.offset(),
                    AccessWidth::Word,
                    SimTime::from_ticks(4),
                )
                .unwrap(),
            u64::from(ESP_RTC_ULP_INT_BIT)
        );
        assert_eq!(
            device
                .read(
                    Esp32S3RtcRegister::IntSt.offset(),
                    AccessWidth::Word,
                    SimTime::ZERO,
                )
                .unwrap(),
            0
        );

        device
            .write(
                Esp32S3RtcRegister::IntEna.offset(),
                AccessWidth::Word,
                u64::from(ESP_RTC_ULP_INT_BIT),
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            device
                .read(
                    Esp32S3RtcRegister::IntSt.offset(),
                    AccessWidth::Word,
                    SimTime::ZERO,
                )
                .unwrap(),
            u64::from(ESP_RTC_ULP_INT_BIT)
        );
    }

    #[test]
    fn register_enum_matches_native_map_and_rejects_reserved_holes() {
        assert_eq!(Esp32S3RtcRegister::Options0.offset(), 0x000);
        assert_eq!(Esp32S3RtcRegister::UlpCpTimer1.offset(), 0x134);
        assert_eq!(Esp32S3RtcRegister::Date.offset(), 0x1fc);
        assert_eq!(Esp32S3RtcRegister::SarMeas2Ctrl2.offset(), 0x830);
        assert_eq!(Esp32S3RtcRegister::from_offset(0x084), None);
        assert_eq!(Esp32S3RtcRegister::from_offset(0x158), None);
        assert_eq!(Esp32S3RtcRegister::IntEna.write_mask(), 0x001f_ffff);
        assert_eq!(Esp32S3RtcRegister::IntRaw.write_mask(), 0);
        assert_eq!(Esp32S3RtcRegister::UlpCpTimer1.read_mask(), 0xffffff00);
    }

    #[test]
    fn register_masks_preserve_ro_fields_and_self_clear_strobes() {
        let mut device = EspRtcControl::new("rtc");
        device
            .write(
                Esp32S3RtcRegister::IntEna.offset(),
                AccessWidth::Word,
                u64::from(u32::MAX),
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            device
                .read(
                    Esp32S3RtcRegister::IntEna.offset(),
                    AccessWidth::Word,
                    SimTime::ZERO,
                )
                .unwrap(),
            0x001f_ffff
        );

        device
            .write(
                Esp32S3RtcRegister::IntEnaW1tc.offset(),
                AccessWidth::Word,
                u64::from(u32::MAX),
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            device
                .read(
                    Esp32S3RtcRegister::IntEnaW1tc.offset(),
                    AccessWidth::Word,
                    SimTime::ZERO,
                )
                .unwrap(),
            0
        );

        device
            .write(
                Esp32S3RtcRegister::TimeLow0.offset(),
                AccessWidth::Word,
                u64::from(u32::MAX),
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            device
                .read(
                    Esp32S3RtcRegister::TimeLow0.offset(),
                    AccessWidth::Word,
                    SimTime::ZERO,
                )
                .unwrap(),
            0
        );

        device
            .write(
                Esp32S3RtcRegister::TimeUpdate.offset(),
                AccessWidth::Word,
                u64::from(u32::MAX),
                SimTime::from_ticks(0x1234_5678_9abc_def0),
            )
            .unwrap();
        assert_eq!(
            device
                .read(
                    Esp32S3RtcRegister::TimeUpdate.offset(),
                    AccessWidth::Word,
                    SimTime::ZERO,
                )
                .unwrap(),
            0x3800_0000
        );
        assert_eq!(
            device
                .read(
                    Esp32S3RtcRegister::TimeLow0.offset(),
                    AccessWidth::Word,
                    SimTime::ZERO,
                )
                .unwrap(),
            0x9abc_def0
        );
        assert_eq!(
            device
                .read(
                    Esp32S3RtcRegister::TimeHigh0.offset(),
                    AccessWidth::Word,
                    SimTime::ZERO,
                )
                .unwrap(),
            0x5678
        );
    }

    #[test]
    fn register_access_rejects_reserved_offsets_and_wide_words() {
        let mut device = EspRtcControl::new("rtc");
        assert!(
            device
                .read(0x084, AccessWidth::Word, SimTime::ZERO)
                .is_err()
        );
        assert!(
            device
                .write(
                    Esp32S3RtcRegister::Options0.offset(),
                    AccessWidth::Word,
                    1_u64 << 32,
                    SimTime::ZERO,
                )
                .is_err()
        );
    }
}
