//! Simulation time.
//!
//! Time is carried as an integer count of simulator precision units
//! (`SimTime`), plus a unit-aware [`Duration`] users write in `ns()` and
//! friends. Conversion from a `Duration` to steps needs the simulator's
//! precision and happens inside the runtime.

use std::fmt;

/// Absolute simulation time in simulator precision steps.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct SimTime(pub u64);

/// A time unit as a power of ten of seconds.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Unit {
    Step,
    Fs,
    Ps,
    Ns,
    Us,
    Ms,
    Sec,
}

impl Unit {
    /// Exponent such that one unit is `10^exp` seconds. `None` for `Step`.
    pub fn exponent(self) -> Option<i32> {
        Some(match self {
            Unit::Step => return None,
            Unit::Fs => -15,
            Unit::Ps => -12,
            Unit::Ns => -9,
            Unit::Us => -6,
            Unit::Ms => -3,
            Unit::Sec => 0,
        })
    }

    pub fn suffix(self) -> &'static str {
        match self {
            Unit::Step => "step",
            Unit::Fs => "fs",
            Unit::Ps => "ps",
            Unit::Ns => "ns",
            Unit::Us => "us",
            Unit::Ms => "ms",
            Unit::Sec => "s",
        }
    }
}

/// A duration in some unit. Fractional values are allowed and are rounded
/// to the nearest precision step when converted, erroring if they are not
/// representable (matching cocotb's default `round_mode="error"`).
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct Duration {
    pub value: f64,
    pub unit: Unit,
}

/// How to handle a duration that is not a whole number of precision steps.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum RoundMode {
    /// Panic if the duration is not exactly representable.
    #[default]
    Error,
    Round,
    Ceil,
    Floor,
}

impl Duration {
    pub const fn new(value: f64, unit: Unit) -> Duration {
        Duration { value, unit }
    }

    pub const fn steps(n: u64) -> Duration {
        Duration { value: n as f64, unit: Unit::Step }
    }

    /// Convert to precision steps given the simulator precision exponent
    /// (for example `-12` for picoseconds).
    pub fn to_steps(self, precision: i32) -> u64 {
        self.to_steps_with(precision, RoundMode::Error)
    }

    pub fn to_steps_with(self, precision: i32, mode: RoundMode) -> u64 {
        let Some(exp) = self.unit.exponent() else {
            assert!(self.value >= 0.0 && self.value.fract() == 0.0, "step count must be a non-negative integer");
            return self.value as u64;
        };
        assert!(self.value >= 0.0, "duration must be non-negative");
        let scale = exp - precision;
        let raw = if scale >= 0 { self.value * 10f64.powi(scale) } else { self.value / 10f64.powi(-scale) };
        // Clean up floating point noise before checking representability.
        let rounded = raw.round();
        let exact = (raw - rounded).abs() < 1e-6 * rounded.abs().max(1.0);
        match mode {
            RoundMode::Error => {
                assert!(
                    exact,
                    "duration {} is not a whole number of simulator steps (precision 1e{})",
                    self, precision
                );
                rounded as u64
            }
            RoundMode::Round => rounded as u64,
            RoundMode::Ceil => {
                if exact {
                    rounded as u64
                } else {
                    raw.ceil() as u64
                }
            }
            RoundMode::Floor => {
                if exact {
                    rounded as u64
                } else {
                    raw.floor() as u64
                }
            }
        }
    }
}

impl fmt::Display for Duration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.value, self.unit.suffix())
    }
}

/// Convenience constructors: `10.ns()`, `2.5.us()`, `100.ps()`.
pub trait TimeExt {
    fn fs(self) -> Duration;
    fn ps(self) -> Duration;
    fn ns(self) -> Duration;
    fn us(self) -> Duration;
    fn ms(self) -> Duration;
    fn sec(self) -> Duration;
    fn steps(self) -> Duration;
}

macro_rules! impl_time_ext {
    ($($t:ty),*) => {$(
        impl TimeExt for $t {
            fn fs(self) -> Duration { Duration::new(self as f64, Unit::Fs) }
            fn ps(self) -> Duration { Duration::new(self as f64, Unit::Ps) }
            fn ns(self) -> Duration { Duration::new(self as f64, Unit::Ns) }
            fn us(self) -> Duration { Duration::new(self as f64, Unit::Us) }
            fn ms(self) -> Duration { Duration::new(self as f64, Unit::Ms) }
            fn sec(self) -> Duration { Duration::new(self as f64, Unit::Sec) }
            fn steps(self) -> Duration { Duration::new(self as f64, Unit::Step) }
        }
    )*};
}
impl_time_ext!(u8, u16, u32, u64, i32, i64, usize, f32, f64);

/// Format a step count in a unit given the precision exponent.
pub fn format_time(steps: u64, precision: i32, unit: Unit) -> String {
    let Some(exp) = unit.exponent() else {
        return format!("{steps} steps");
    };
    let scale = precision - exp;
    let v = if scale >= 0 { steps as f64 * 10f64.powi(scale) } else { steps as f64 / 10f64.powi(-scale) };
    format!("{v}{}", unit.suffix())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convert() {
        assert_eq!(10.ns().to_steps(-12), 10_000);
        assert_eq!(2.5.us().to_steps(-9), 2_500);
        assert_eq!(7.steps().to_steps(-9), 7);
        assert_eq!(1.5.ns().to_steps_with(-9, RoundMode::Round), 2);
        assert_eq!(1.5.ns().to_steps_with(-9, RoundMode::Floor), 1);
        assert_eq!(1.5.ns().to_steps_with(-9, RoundMode::Ceil), 2);
    }

    #[test]
    #[should_panic]
    fn error_on_inexact() {
        let _ = 1.5.ns().to_steps(-9);
    }

    #[test]
    fn format() {
        assert_eq!(format_time(2_500, -12, Unit::Ns), "2.5ns");
    }

    #[test]
    fn units_and_display() {
        assert_eq!(Unit::Step.exponent(), None);
        assert_eq!(Unit::Fs.exponent(), Some(-15));
        assert_eq!(Unit::Sec.suffix(), "s");
        assert_eq!(format!("{}", 2.5.us()), "2.5us");
        assert_eq!(format!("{}", 3.steps()), "3step");
        assert_eq!(1.fs().to_steps(-15), 1);
        assert_eq!(1.ms().to_steps(-9), 1_000_000);
        assert_eq!(1.sec().to_steps(-12), 1_000_000_000_000);
        assert_eq!(Duration::steps(9).to_steps(-3), 9);
        assert_eq!(format_time(7, -9, Unit::Step), "7 steps");
        assert_eq!(format_time(1_500_000, -12, Unit::Us), "1.5us");
        assert_eq!(format_time(0, -12, Unit::Ns), "0ns");
        assert_eq!(RoundMode::default(), RoundMode::Error);
        assert_eq!(0.3.ns().to_steps_with(-9, RoundMode::Ceil), 1);
        assert_eq!(0.3.ns().to_steps_with(-9, RoundMode::Floor), 0);
        assert_eq!(0.3.ns().to_steps_with(-9, RoundMode::Round), 0);
        // Precision coarser than the unit still converts.
        assert_eq!(1000.ps().to_steps(-9), 1);
        // Floating-point noise below 1e-6 relative is tolerated.
        assert_eq!((0.1 + 0.2).ns().to_steps(-12), 300);
    }

    #[test]
    #[should_panic]
    fn negative_duration_panics() {
        let _ = (-1).ns().to_steps(-9);
    }

    #[test]
    #[should_panic]
    fn fractional_steps_panic() {
        let _ = 1.5.steps().to_steps(-9);
    }
}
