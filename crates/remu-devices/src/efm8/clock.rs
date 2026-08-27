use super::*;

const PAGE10: usize = 0x10;
const PAGE30: usize = 0x30;

/// Clock and power-control registers in the EFM8BB52 SFR map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum Efm8ClockRegister {
    /// System clock source and divider (all SFR pages, `0xA9`).
    ClkSel = 0xa9,
    /// Clock-group source/divider control (pages 0/0x10/0x30, `0xAF`).
    ClkGrp0 = 0xaf,
    /// High-frequency oscillator control (page `0x10`, `0xEF`).
    Hfo0Cn = (PAGE10 << 8) | 0xef,
    /// Low-frequency oscillator control (pages 0/0x10, `0xB1`).
    Lfo0Cn = 0xb1,
    /// CPU idle/stop one-shot controls (all pages, `0x87`).
    Pcon0 = 0x87,
    /// Snooze and pin-retain controls (all pages, `0xCD`).
    Pcon1 = 0xcd,
    /// Regulator stop/shutdown selection (page 0, `0xC9`).
    Reg0Cn = 0xc9,
    /// Power-state status (page `0x10`, `0xAA`).
    Pstat0 = (PAGE10 << 8) | 0xaa,
}

pub(super) const CLKSEL: usize = Efm8ClockRegister::ClkSel as usize;
pub(super) const CLKGRP0: usize = Efm8ClockRegister::ClkGrp0 as usize;
pub(super) const HFO0CN: usize = Efm8ClockRegister::Hfo0Cn as usize;
pub(super) const LFO0CN: usize = Efm8ClockRegister::Lfo0Cn as usize;
pub(super) const PCON0: usize = Efm8ClockRegister::Pcon0 as usize;
pub(super) const PCON1: usize = Efm8ClockRegister::Pcon1 as usize;
pub(super) const REG0CN: usize = Efm8ClockRegister::Reg0Cn as usize;
pub(super) const PSTAT0: usize = Efm8ClockRegister::Pstat0 as usize;

const PCON0_CPUIDLE: u8 = 0x01;
const PCON0_CPUSTOP: u8 = 0x02;
const PCON1_SNOOZE: u8 = 0x80;
const REG0CN_STOPCF: u8 = 0x08;

/// EFM8BB52 system-clock source selected by `CLKSEL.CLKSL`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Efm8ClockSource {
    /// 24.5 MHz HFOSC0 output.
    Hfosc0Clk24p5,
    /// External CMOS clock, whose nominal frequency is supplied by the host.
    External,
    /// 80 kHz LFOSC0 output with `LFO0CN.LFODIV` applied.
    Lfosc,
    /// 49 MHz HFOSC0 output.
    Hfosc0Clk49,
    /// 24.5 MHz HFOSC0 output divided by 1.5.
    Hfosc0Clk24p5Div1p5,
    /// 10 MHz fast-start oscillator output.
    FsrcoClk10,
    /// 2.5 MHz fast-start oscillator output.
    FsrcoClk2p5,
    /// 49 MHz HFOSC0 output divided by 1.5.
    Hfosc0Clk49Div1p5,
}

impl Efm8ClockSource {
    fn from_bits(bits: u8) -> Self {
        match bits & 0x07 {
            0 => Self::Hfosc0Clk24p5,
            1 => Self::External,
            2 => Self::Lfosc,
            3 => Self::Hfosc0Clk49,
            4 => Self::Hfosc0Clk24p5Div1p5,
            5 => Self::FsrcoClk10,
            6 => Self::FsrcoClk2p5,
            _ => Self::Hfosc0Clk49Div1p5,
        }
    }

    fn bits(self) -> u64 {
        self as u64
    }

    fn nominal_hz(self, external_hz: u32, lfo_divider: u32) -> u32 {
        match self {
            Self::Hfosc0Clk24p5 => 24_500_000,
            Self::External => external_hz,
            Self::Lfosc => 80_000 / lfo_divider,
            Self::Hfosc0Clk49 => 49_000_000,
            Self::Hfosc0Clk24p5Div1p5 => 16_333_333,
            Self::FsrcoClk10 => 10_000_000,
            Self::FsrcoClk2p5 => 2_500_000,
            Self::Hfosc0Clk49Div1p5 => 32_666_666,
        }
    }
}

/// Functional EFM8 CPU power state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Efm8PowerMode {
    /// CPU and peripheral clocks run normally.
    Active,
    /// CPU is idle while peripherals may continue running.
    Idle,
    /// CPU and internal clocks are stopped until reset.
    Stop,
    /// High-frequency clocks and the regulator are in snooze state.
    Snooze,
    /// Regulator and retained state are shut down until reset.
    Shutdown,
}

pub(super) fn canonical_clock_register(raw: usize) -> Option<usize> {
    let page = raw >> 8;
    let address = raw & 0xff;
    match (page, address) {
        (_, 0x87 | 0xa9 | 0xcd) => Some(address),
        (0 | PAGE10 | PAGE30, 0xaf) => Some(CLKGRP0),
        (0 | PAGE10, 0xb1) => Some(LFO0CN),
        (PAGE10, 0xef) => Some(HFO0CN),
        (0, 0xc9) => Some(REG0CN),
        (PAGE10, 0xaa) => Some(PSTAT0),
        _ => None,
    }
}

impl Efm8State {
    pub(super) fn reset_clock(&mut self, at: SimTime) {
        self.registers[CLKSEL] = 0xb0;
        self.registers[CLKGRP0] = 0;
        self.registers[HFO0CN] = 0;
        self.registers[LFO0CN] = 0x43;
        self.registers[PCON0] = 0;
        self.registers[PCON1] = 0;
        self.registers[REG0CN] = 0;
        self.registers[PSTAT0] = 0;
        self.set_power_mode(Efm8PowerMode::Active, at);
        self.refresh_clock(at);
    }

    pub(super) fn clock_source(&self) -> Efm8ClockSource {
        Efm8ClockSource::from_bits(self.registers[CLKSEL])
    }

    pub(super) fn clock_divider(&self) -> u32 {
        1_u32 << u32::from((self.registers[CLKSEL] >> 4) & 0x07)
    }

    fn lfo_divider(&self) -> u32 {
        match self.registers[LFO0CN] & 0x03 {
            0 => 8,
            1 => 4,
            2 => 2,
            _ => 1,
        }
    }

    pub(super) fn system_clock_hz(&self) -> u32 {
        self.clock_source()
            .nominal_hz(self.external_clock_hz, self.lfo_divider())
            / self.clock_divider()
    }

    pub(super) fn refresh_clock(&self, at: SimTime) {
        self.set_signal(self.clock_source_signal, self.clock_source().bits(), 3, at);
        self.set_signal(
            self.clock_divider_signal,
            u64::from(self.clock_divider()),
            8,
            at,
        );
        self.set_signal(
            self.sysclk_hz_signal,
            u64::from(self.system_clock_hz()),
            32,
            at,
        );
    }

    pub(super) fn set_power_mode(&mut self, mode: Efm8PowerMode, at: SimTime) {
        self.power_mode = mode;
        self.set_signal(self.power_mode_signal, mode as u64, 3, at);
    }

    pub(super) fn read_clock_register(&self, address: usize) -> Option<u8> {
        let value = match address {
            CLKSEL => self.registers[CLKSEL] & 0xf7,
            CLKGRP0 => self.registers[CLKGRP0] & 0x3f,
            HFO0CN => self.registers[HFO0CN] & 0x8c,
            LFO0CN => 0x40 | (self.registers[LFO0CN] & 0xbf),
            PCON0 => self.registers[PCON0] & 0xfc,
            PCON1 => self.registers[PCON1] & 0x81,
            REG0CN => self.registers[REG0CN] & REG0CN_STOPCF,
            PSTAT0 => self.registers[PSTAT0],
            _ => return None,
        };
        Some(value)
    }

    pub(super) fn write_clock_register(&mut self, address: usize, value: u8, at: SimTime) -> bool {
        match address {
            CLKSEL => self.registers[CLKSEL] = 0x80 | (value & 0x77),
            CLKGRP0 => self.registers[CLKGRP0] = value & 0x3f,
            HFO0CN => self.registers[HFO0CN] = value & 0x8c,
            LFO0CN => self.registers[LFO0CN] = 0x40 | (value & 0xbf),
            PCON0 => {
                self.registers[PCON0] = value & 0xfc;
                let mode = if value & PCON0_CPUSTOP != 0 {
                    if self.registers[REG0CN] & REG0CN_STOPCF != 0 {
                        Efm8PowerMode::Shutdown
                    } else {
                        Efm8PowerMode::Stop
                    }
                } else if value & PCON0_CPUIDLE != 0 {
                    Efm8PowerMode::Idle
                } else {
                    Efm8PowerMode::Active
                };
                self.set_power_mode(mode, at);
            }
            PCON1 => {
                self.registers[PCON1] = value & 0x81;
                if value & PCON1_SNOOZE != 0 {
                    self.set_power_mode(Efm8PowerMode::Snooze, at);
                } else if self.power_mode == Efm8PowerMode::Snooze {
                    self.set_power_mode(Efm8PowerMode::Active, at);
                }
            }
            REG0CN => self.registers[REG0CN] = value & REG0CN_STOPCF,
            PSTAT0 => return true,
            _ => return false,
        }
        if matches!(address, CLKSEL | CLKGRP0 | HFO0CN | LFO0CN) {
            self.refresh_clock(at);
        }
        true
    }
}
