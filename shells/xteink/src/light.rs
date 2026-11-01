//! The X4 Pro's front light: cool LEDs on GPIO 8 and warm ones on GPIO 9,
//! each dimmed by PWM (the LEDC unit: 10 kHz, 12 bits, the most its
//! 80 MHz clock allows at that rate). The settings `x4.frontlight` (how
//! bright, in percent) and `x4.warmth` (how much of it warm, see
//! `device`) are shown. The eye sees light about as its cube root, and
//! is most sensitive where it is dim, so the percentages follow the
//! lightness the eye sees (CIE L*): each step looks about as large as the
//! next, the lowest ones too, and 1 % is still very dim.

use alloc::boxed::Box;

use esp_hal::gpio::DriveMode;
use esp_hal::ledc::channel::{self, ChannelHW, ChannelIFace};
use esp_hal::ledc::timer::{self, TimerIFace};
use esp_hal::ledc::{LSGlobalClkSource, Ledc, LowSpeed};
use esp_hal::peripherals::{GPIO8, GPIO9, LEDC};
use esp_hal::time::Rate;

/// The PWM's resolution: duty from 0 to 4095.
const MAX_DUTY: u32 = 4095;

/// The duty for each percent (see [`duty`]).
const DUTIES: [u16; 101] = duties();

// Every percent brighter than the one before, the last full.
const _: () = {
    let mut level = 1;
    while level <= 100 {
        assert!(DUTIES[level] > DUTIES[level - 1]);
        level += 1;
    }
    assert!(DUTIES[0] == 0 && DUTIES[100] as u32 == MAX_DUTY);
};

pub struct Light {
    cool: channel::Channel<'static, LowSpeed>,
    warm: channel::Channel<'static, LowSpeed>,
    /// The brightness and warmth shown, in percent.
    shown: Option<(u8, u8)>,
}

impl Light {
    /// The light, off; `None` if the PWM unit cannot be set up.
    pub fn new(ledc: LEDC<'static>, cool: GPIO8<'static>, warm: GPIO9<'static>) -> Option<Self> {
        let mut ledc = Ledc::new(ledc);
        ledc.set_global_slow_clock(LSGlobalClkSource::APBClk);
        let ledc: &'static Ledc<'static> = Box::leak(Box::new(ledc));
        let mut pwm = ledc.timer::<LowSpeed>(timer::Number::Timer0);
        pwm.configure(timer::config::Config {
            duty: timer::config::Duty::Duty12Bit,
            clock_source: timer::LSClockSource::APBClk,
            frequency: Rate::from_khz(10),
        })
        .ok()?;
        let pwm: &'static timer::Timer<'static, LowSpeed> = Box::leak(Box::new(pwm));
        let channel = |number, pin: esp_hal::gpio::AnyPin<'static>| {
            let mut c = ledc.channel(number, pin);
            c.configure(channel::config::Config {
                timer: pwm,
                duty_pct: 0,
                drive_mode: DriveMode::PushPull,
            })
            .ok()
            .map(|()| c)
        };
        Some(Self {
            cool: channel(channel::Number::Channel0, cool.into())?,
            warm: channel(channel::Number::Channel1, warm.into())?,
            shown: None,
        })
    }

    /// Shows `level` percent of brightness, `warmth` percent of it warm.
    pub fn show(&mut self, level: i64, warmth: i64) {
        let level = level.clamp(0, 100) as u8;
        let warmth = warmth.clamp(0, 100) as u8;
        if self.shown == Some((level, warmth)) {
            return;
        }
        self.shown = Some((level, warmth));
        let total = duty(level);
        let warm = total * u32::from(warmth) / 100;
        self.warm.set_duty_hw(warm);
        self.cool.set_duty_hw(total - warm);
    }

    /// Off, e.g. asleep or for want of input (`light_timer`);
    /// [`Light::show`] brings it back.
    pub fn off(&mut self) {
        if self.shown.is_none() {
            return;
        }
        self.shown = None;
        self.cool.set_duty_hw(0);
        self.warm.set_duty_hw(0);
    }
}

/// The duty for `level` percent (at most 100).
fn duty(level: u8) -> u32 {
    u32::from(DUTIES[usize::from(level.min(100))])
}

/// The duty for each percent `L` taken as the lightness the eye sees
/// (CIE L*): the share of the light `((L + 16) / 116)^3`, and `L / 903.3`
/// below 8, where the curve is straight; rounded, and at least 1 when
/// on.
const fn duties() -> [u16; 101] {
    let max = MAX_DUTY as u64;
    let mut duties = [0u16; 101];
    let mut level = 1;
    while level <= 100 {
        let l = level as u64;
        let duty = if l <= 8 {
            (l * max * 10_000 + 9_033_000 / 2) / 9_033_000
        } else {
            let base = l + 16;
            let cube = 116 * 116 * 116;
            (base * base * base * max + cube / 2) / cube
        };
        duties[level] = if duty == 0 { 1 } else { duty as u16 };
        level += 1;
    }
    duties
}
