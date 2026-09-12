//! Four-state values in VPI's own encoding.
//!
//! A [`LogicVec`] is two bit-planes of 32-bit words, `aval` and `bval`,
//! exactly as `s_vpi_vecval` lays them out, so reads and writes through
//! VPI's `vpiVectorVal` are memcpy-shaped. The encoding per bit is:
//!
//! | aval | bval | value |
//! |------|------|-------|
//! | 0    | 0    | 0     |
//! | 1    | 0    | 1     |
//! | 0    | 1    | Z     |
//! | 1    | 1    | X     |

use std::fmt;

/// One four-state bit.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum Logic {
    Zero,
    One,
    Z,
    X,
}

impl Logic {
    #[inline]
    pub fn from_ab(a: bool, b: bool) -> Logic {
        match (a, b) {
            (false, false) => Logic::Zero,
            (true, false) => Logic::One,
            (false, true) => Logic::Z,
            (true, true) => Logic::X,
        }
    }

    #[inline]
    pub fn to_ab(self) -> (bool, bool) {
        match self {
            Logic::Zero => (false, false),
            Logic::One => (true, false),
            Logic::Z => (false, true),
            Logic::X => (true, true),
        }
    }

    /// `true` for `0` or `1`.
    #[inline]
    pub fn is_resolvable(self) -> bool {
        matches!(self, Logic::Zero | Logic::One)
    }

    pub fn to_char(self) -> char {
        match self {
            Logic::Zero => '0',
            Logic::One => '1',
            Logic::Z => 'z',
            Logic::X => 'x',
        }
    }

    /// Parse a character from the VPI/VHDL alphabet. `U`, `W`, `-` map to
    /// `X`; `L` and `H` map to `0` and `1`.
    pub fn from_char(c: char) -> Option<Logic> {
        Some(match c {
            '0' | 'l' | 'L' => Logic::Zero,
            '1' | 'h' | 'H' => Logic::One,
            'z' | 'Z' => Logic::Z,
            'x' | 'X' | 'u' | 'U' | 'w' | 'W' | '-' => Logic::X,
            _ => return None,
        })
    }
}

impl From<bool> for Logic {
    fn from(b: bool) -> Logic {
        if b {
            Logic::One
        } else {
            Logic::Zero
        }
    }
}

impl fmt::Display for Logic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_char())
    }
}

/// Error returned when converting a value containing `X` or `Z` bits to an
/// integer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unresolved {
    pub value: LogicVec,
}

impl fmt::Display for Unresolved {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "value {} contains X or Z bits", self.value)
    }
}

impl std::error::Error for Unresolved {}

/// A packed four-state vector, bit 0 is the least significant.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct LogicVec {
    width: u32,
    aval: Vec<u32>,
    bval: Vec<u32>,
}

#[inline]
fn words_for(width: u32) -> usize {
    ((width as usize) + 31) / 32
}

impl LogicVec {
    /// All zeros.
    pub fn zeros(width: u32) -> LogicVec {
        let n = words_for(width);
        LogicVec { width, aval: vec![0; n], bval: vec![0; n] }
    }

    /// All `X`.
    pub fn xs(width: u32) -> LogicVec {
        let mut v = LogicVec::zeros(width);
        for w in 0..v.aval.len() {
            v.aval[w] = u32::MAX;
            v.bval[w] = u32::MAX;
        }
        v.mask_top();
        v
    }

    /// All `Z`.
    pub fn zs(width: u32) -> LogicVec {
        let mut v = LogicVec::zeros(width);
        for w in 0..v.bval.len() {
            v.bval[w] = u32::MAX;
        }
        v.mask_top();
        v
    }

    /// Build from raw bit-planes. `aval`/`bval` must have `ceil(width/32)`
    /// words; bits above `width` are cleared.
    pub fn from_planes(width: u32, aval: Vec<u32>, bval: Vec<u32>) -> LogicVec {
        assert_eq!(aval.len(), words_for(width));
        assert_eq!(bval.len(), words_for(width));
        let mut v = LogicVec { width, aval, bval };
        v.mask_top();
        v
    }

    /// Two-state value from an integer, truncated or zero-extended to
    /// `width`.
    pub fn from_u64(width: u32, value: u64) -> LogicVec {
        let mut v = LogicVec::zeros(width);
        if !v.aval.is_empty() {
            v.aval[0] = value as u32;
        }
        if v.aval.len() > 1 {
            v.aval[1] = (value >> 32) as u32;
        }
        v.mask_top();
        v
    }

    /// Two-state value from a 128-bit integer.
    pub fn from_u128(width: u32, value: u128) -> LogicVec {
        let mut v = LogicVec::zeros(width);
        for (i, w) in v.aval.iter_mut().enumerate().take(4) {
            *w = (value >> (32 * i)) as u32;
        }
        v.mask_top();
        v
    }

    /// Two-state value from a signed integer with sign extension.
    pub fn from_i64(width: u32, value: i64) -> LogicVec {
        let mut v = LogicVec::zeros(width);
        let bits = value as u64;
        for (i, w) in v.aval.iter_mut().enumerate() {
            *w = if i < 2 { (bits >> (32 * i)) as u32 } else if value < 0 { u32::MAX } else { 0 };
        }
        v.mask_top();
        v
    }

    /// Parse a binary string, most significant bit first, in the alphabet
    /// `01xzXZ` plus VHDL `uwlhUWLH-`. Underscores are ignored. Width is the
    /// number of digits.
    pub fn from_binstr(s: &str) -> Option<LogicVec> {
        let digits: Vec<Logic> =
            s.chars().filter(|c| *c != '_').map(Logic::from_char).collect::<Option<_>>()?;
        let mut v = LogicVec::zeros(digits.len() as u32);
        for (i, d) in digits.iter().rev().enumerate() {
            v.set_bit(i as u32, *d);
        }
        Some(v)
    }

    /// Parse a Verilog-style sized literal: `8'b1010_xxzz`, `16'hdead`,
    /// `4'd9`. Unsized digit strings are treated as binary.
    pub fn parse(s: &str) -> Option<LogicVec> {
        let s = s.trim();
        let Some((w, rest)) = s.split_once('\'') else {
            return LogicVec::from_binstr(s);
        };
        let width: u32 = w.trim().parse().ok()?;
        let mut chars = rest.chars();
        let base = chars.next()?.to_ascii_lowercase();
        let digits: String = chars.filter(|c| *c != '_').collect();
        let mut v = LogicVec::zeros(width);
        match base {
            'b' => {
                let parsed = LogicVec::from_binstr(&digits)?;
                v.assign_low(&parsed);
            }
            'h' => {
                let mut bit = 0u32;
                for c in digits.chars().rev() {
                    let nibble: [Logic; 4] = match c.to_ascii_lowercase() {
                        'x' => [Logic::X; 4],
                        'z' | '?' => [Logic::Z; 4],
                        d => {
                            let n = d.to_digit(16)?;
                            [
                                Logic::from(n & 1 != 0),
                                Logic::from(n & 2 != 0),
                                Logic::from(n & 4 != 0),
                                Logic::from(n & 8 != 0),
                            ]
                        }
                    };
                    for (i, l) in nibble.iter().enumerate() {
                        if bit + (i as u32) < width {
                            v.set_bit(bit + i as u32, *l);
                        }
                    }
                    bit += 4;
                }
            }
            'd' => {
                let n: u128 = digits.parse().ok()?;
                v = LogicVec::from_u128(width, n);
            }
            _ => return None,
        }
        Some(v)
    }

    #[inline]
    pub fn width(&self) -> u32 {
        self.width
    }

    #[inline]
    pub fn aval(&self) -> &[u32] {
        &self.aval
    }

    #[inline]
    pub fn bval(&self) -> &[u32] {
        &self.bval
    }

    /// Mutable access to the bit planes for zero-copy fills from a
    /// simulator. The caller must keep bits above `width` clear or call
    /// [`LogicVec::mask_top`].
    #[inline]
    pub fn planes_mut(&mut self) -> (&mut [u32], &mut [u32]) {
        (&mut self.aval, &mut self.bval)
    }

    /// Clear bits above `width` in the top word.
    pub fn mask_top(&mut self) {
        let rem = self.width % 32;
        if rem != 0 {
            let mask = (1u32 << rem) - 1;
            if let Some(w) = self.aval.last_mut() {
                *w &= mask;
            }
            if let Some(w) = self.bval.last_mut() {
                *w &= mask;
            }
        }
    }

    /// Resize in place, keeping the low bits and zero-filling new ones.
    pub fn resize(&mut self, width: u32) {
        let n = words_for(width);
        self.aval.resize(n, 0);
        self.bval.resize(n, 0);
        self.width = width;
        self.mask_top();
    }

    fn assign_low(&mut self, other: &LogicVec) {
        for i in 0..self.width.min(other.width) {
            self.set_bit(i, other.bit(i));
        }
    }

    #[inline]
    pub fn bit(&self, i: u32) -> Logic {
        assert!(i < self.width, "bit index {i} out of range for width {}", self.width);
        let w = (i / 32) as usize;
        let m = 1u32 << (i % 32);
        Logic::from_ab(self.aval[w] & m != 0, self.bval[w] & m != 0)
    }

    #[inline]
    pub fn set_bit(&mut self, i: u32, v: Logic) {
        assert!(i < self.width, "bit index {i} out of range for width {}", self.width);
        let w = (i / 32) as usize;
        let m = 1u32 << (i % 32);
        let (a, b) = v.to_ab();
        if a {
            self.aval[w] |= m;
        } else {
            self.aval[w] &= !m;
        }
        if b {
            self.bval[w] |= m;
        } else {
            self.bval[w] &= !m;
        }
    }

    /// Bits `[hi:lo]` inclusive as a new vector.
    pub fn slice(&self, hi: u32, lo: u32) -> LogicVec {
        assert!(hi >= lo && hi < self.width);
        let mut v = LogicVec::zeros(hi - lo + 1);
        for i in lo..=hi {
            v.set_bit(i - lo, self.bit(i));
        }
        v
    }

    /// `true` if no bit is `X` or `Z`.
    #[inline]
    pub fn is_resolvable(&self) -> bool {
        self.bval.iter().all(|w| *w == 0)
    }

    /// `true` if any bit is `X`.
    pub fn has_x(&self) -> bool {
        self.aval.iter().zip(&self.bval).any(|(a, b)| a & b != 0)
    }

    /// `true` if any bit is `Z`.
    pub fn has_z(&self) -> bool {
        self.aval.iter().zip(&self.bval).any(|(a, b)| !a & b != 0)
    }

    /// Zero-extended integer value; `Err` if any bit is `X`/`Z` or the
    /// value does not fit in 64 bits.
    pub fn to_u64(&self) -> Result<u64, Unresolved> {
        if !self.is_resolvable() {
            return Err(Unresolved { value: self.clone() });
        }
        if self.aval.iter().skip(2).any(|w| *w != 0) {
            return Err(Unresolved { value: self.clone() });
        }
        let lo = *self.aval.first().unwrap_or(&0) as u64;
        let hi = *self.aval.get(1).unwrap_or(&0) as u64;
        Ok(lo | (hi << 32))
    }

    /// Zero-extended integer value up to 128 bits.
    pub fn to_u128(&self) -> Result<u128, Unresolved> {
        if !self.is_resolvable() || self.aval.iter().skip(4).any(|w| *w != 0) {
            return Err(Unresolved { value: self.clone() });
        }
        let mut v = 0u128;
        for (i, w) in self.aval.iter().enumerate().take(4) {
            v |= (*w as u128) << (32 * i);
        }
        Ok(v)
    }

    /// Sign-extended integer value.
    pub fn to_i64(&self) -> Result<i64, Unresolved> {
        let u = self.to_u64()?;
        if self.width == 0 || self.width >= 64 {
            return Ok(u as i64);
        }
        let sign = 1u64 << (self.width - 1);
        Ok(if u & sign != 0 { (u | !(sign | (sign - 1))) as i64 } else { u as i64 })
    }

    /// `to_u64` with `X`/`Z` bits resolved to zero.
    pub fn to_u64_lossy(&self) -> u64 {
        let lo = (self.aval.first().copied().unwrap_or(0) & !self.bval.first().copied().unwrap_or(0)) as u64;
        let hi = (self.aval.get(1).copied().unwrap_or(0) & !self.bval.get(1).copied().unwrap_or(0)) as u64;
        lo | (hi << 32)
    }

    /// Binary string, most significant bit first.
    pub fn to_binstr(&self) -> String {
        (0..self.width).rev().map(|i| self.bit(i).to_char()).collect()
    }

    /// Hex string, most significant nibble first. A nibble with any `X` is
    /// `x`; with any `Z` (and no `X`) is `z`.
    pub fn to_hexstr(&self) -> String {
        let nibbles = (self.width + 3) / 4;
        let mut s = String::with_capacity(nibbles as usize);
        for n in (0..nibbles).rev() {
            let lo = n * 4;
            let hi = (lo + 3).min(self.width - 1);
            let part = self.slice(hi, lo);
            if part.has_x() {
                s.push('x');
            } else if part.has_z() {
                s.push('z');
            } else {
                s.push(std::char::from_digit(part.to_u64().unwrap() as u32, 16).unwrap());
            }
        }
        s
    }

    pub fn iter(&self) -> impl Iterator<Item = Logic> + '_ {
        (0..self.width).map(move |i| self.bit(i))
    }
}

impl fmt::Debug for LogicVec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}'b{}", self.width, self.to_binstr())
    }
}

impl fmt::Display for LogicVec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if f.alternate() {
            write!(f, "{}'h{}", self.width, self.to_hexstr())
        } else {
            write!(f, "{}'b{}", self.width, self.to_binstr())
        }
    }
}

impl fmt::LowerHex for LogicVec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hexstr())
    }
}

impl fmt::Binary for LogicVec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_binstr())
    }
}

impl PartialEq<u64> for LogicVec {
    fn eq(&self, other: &u64) -> bool {
        self.to_u64().map(|v| v == *other).unwrap_or(false)
    }
}

impl PartialEq<i32> for LogicVec {
    fn eq(&self, other: &i32) -> bool {
        if *other < 0 {
            self.to_i64().map(|v| v == *other as i64).unwrap_or(false)
        } else {
            self.to_u64().map(|v| v == *other as u64).unwrap_or(false)
        }
    }
}

impl From<Logic> for LogicVec {
    fn from(l: Logic) -> LogicVec {
        let mut v = LogicVec::zeros(1);
        v.set_bit(0, l);
        v
    }
}

impl From<bool> for LogicVec {
    fn from(b: bool) -> LogicVec {
        LogicVec::from(Logic::from(b))
    }
}

/// Things that can be written to a signal of a given width. Integers are
/// zero-extended or truncated to the target width.
pub trait IntoLogicVec {
    fn into_logic_vec(self, width: u32) -> LogicVec;
}

impl IntoLogicVec for LogicVec {
    fn into_logic_vec(self, width: u32) -> LogicVec {
        if self.width == width {
            self
        } else {
            let mut v = self;
            v.resize(width);
            v
        }
    }
}

impl IntoLogicVec for &LogicVec {
    fn into_logic_vec(self, width: u32) -> LogicVec {
        self.clone().into_logic_vec(width)
    }
}

impl IntoLogicVec for Logic {
    fn into_logic_vec(self, width: u32) -> LogicVec {
        let mut v = LogicVec::zeros(width);
        if width > 0 {
            v.set_bit(0, self);
        }
        v
    }
}

impl IntoLogicVec for bool {
    fn into_logic_vec(self, width: u32) -> LogicVec {
        LogicVec::from_u64(width, self as u64)
    }
}

impl IntoLogicVec for &str {
    fn into_logic_vec(self, width: u32) -> LogicVec {
        LogicVec::parse(self)
            .unwrap_or_else(|| panic!("cannot parse {self:?} as a logic vector"))
            .into_logic_vec(width)
    }
}

macro_rules! impl_into_unsigned {
    ($($t:ty),*) => {$(
        impl IntoLogicVec for $t {
            fn into_logic_vec(self, width: u32) -> LogicVec {
                LogicVec::from_u128(width, self as u128)
            }
        }
    )*};
}
macro_rules! impl_into_signed {
    ($($t:ty),*) => {$(
        impl IntoLogicVec for $t {
            fn into_logic_vec(self, width: u32) -> LogicVec {
                LogicVec::from_i64(width, self as i64)
            }
        }
    )*};
}
impl_into_unsigned!(u8, u16, u32, u64, u128, usize);
impl_into_signed!(i8, i16, i32, i64, isize);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_u64() {
        let v = LogicVec::from_u64(40, 0x12_3456_789a);
        assert_eq!(v.to_u64().unwrap(), 0x12_3456_789a);
        assert_eq!(v.width(), 40);
        assert_eq!(format!("{:#}", v), "40'h123456789a");
    }

    #[test]
    fn truncates_to_width() {
        let v = LogicVec::from_u64(4, 0xff);
        assert_eq!(v.to_u64().unwrap(), 0xf);
        let w = 300u32.into_logic_vec(8);
        assert_eq!(w.to_u64().unwrap(), 300 & 0xff);
    }

    #[test]
    fn binstr_and_xz() {
        let v = LogicVec::from_binstr("10xz").unwrap();
        assert_eq!(v.bit(3), Logic::One);
        assert_eq!(v.bit(2), Logic::Zero);
        assert_eq!(v.bit(1), Logic::X);
        assert_eq!(v.bit(0), Logic::Z);
        assert!(v.to_u64().is_err());
        assert!(!v.is_resolvable());
        assert_eq!(v.to_binstr(), "10xz");
        assert_eq!(v.to_hexstr(), "x");
    }

    #[test]
    fn parse_literals() {
        assert_eq!(LogicVec::parse("8'hA5").unwrap().to_u64().unwrap(), 0xa5);
        assert_eq!(LogicVec::parse("12'd4095").unwrap().to_u64().unwrap(), 4095);
        let v = LogicVec::parse("8'b1010_xxzz").unwrap();
        assert_eq!(v.to_binstr(), "1010xxzz");
        assert_eq!(LogicVec::parse("4'hz").unwrap().to_binstr(), "zzzz");
    }

    #[test]
    fn signed() {
        let v = LogicVec::from_i64(8, -3);
        assert_eq!(v.to_u64().unwrap(), 0xfd);
        assert_eq!(v.to_i64().unwrap(), -3);
        assert!(v == -3i32);
    }

    #[test]
    fn wide() {
        let v = LogicVec::from_u128(100, u128::MAX);
        assert_eq!(v.to_u128().unwrap(), (1u128 << 100) - 1);
        assert!(v.to_u64().is_err());
        let x = LogicVec::xs(70);
        assert!(x.has_x() && !x.has_z());
        assert_eq!(x.to_binstr().len(), 70);
    }
}
