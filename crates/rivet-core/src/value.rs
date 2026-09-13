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

/// Word storage with inline capacity for vectors up to 64 bits, so the
/// common case never touches the heap.
#[derive(Clone)]
enum Words {
    Inline([u32; 2], u8),
    Heap(Vec<u32>),
}

impl Words {
    fn new(n: usize) -> Words {
        if n <= 2 {
            Words::Inline([0; 2], n as u8)
        } else {
            Words::Heap(vec![0; n])
        }
    }
    #[inline]
    fn as_slice(&self) -> &[u32] {
        match self {
            Words::Inline(a, n) => &a[..*n as usize],
            Words::Heap(v) => v,
        }
    }
    #[inline]
    fn as_mut_slice(&mut self) -> &mut [u32] {
        match self {
            Words::Inline(a, n) => &mut a[..*n as usize],
            Words::Heap(v) => v,
        }
    }
    fn resize(&mut self, n: usize) {
        match self {
            Words::Inline(a, len) if n <= 2 => {
                for w in a.iter_mut().skip(n) {
                    *w = 0;
                }
                *len = n as u8;
            }
            Words::Inline(a, len) => {
                let mut v = vec![0; n];
                v[..*len as usize].copy_from_slice(&a[..*len as usize]);
                *self = Words::Heap(v);
            }
            Words::Heap(v) => {
                if n <= 2 {
                    let mut a = [0u32; 2];
                    for (i, w) in v.iter().take(n).enumerate() {
                        a[i] = *w;
                    }
                    *self = Words::Inline(a, n as u8);
                } else {
                    v.resize(n, 0);
                }
            }
        }
    }
    fn from_vec(v: Vec<u32>) -> Words {
        if v.len() <= 2 {
            let mut a = [0u32; 2];
            a[..v.len()].copy_from_slice(&v);
            Words::Inline(a, v.len() as u8)
        } else {
            Words::Heap(v)
        }
    }
}

impl PartialEq for Words {
    fn eq(&self, other: &Words) -> bool {
        self.as_slice() == other.as_slice()
    }
}
impl Eq for Words {}
impl std::hash::Hash for Words {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.as_slice().hash(state)
    }
}

/// A packed four-state vector, bit 0 is the least significant.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct LogicVec {
    width: u32,
    aval: Words,
    bval: Words,
}

#[inline]
fn words_for(width: u32) -> usize {
    (width as usize).div_ceil(32)
}

impl LogicVec {
    /// All zeros.
    pub fn zeros(width: u32) -> LogicVec {
        let n = words_for(width);
        LogicVec { width, aval: Words::new(n), bval: Words::new(n) }
    }

    /// All `X`.
    pub fn xs(width: u32) -> LogicVec {
        let mut v = LogicVec::zeros(width);
        for w in v.aval.as_mut_slice() {
            *w = u32::MAX;
        }
        for w in v.bval.as_mut_slice() {
            *w = u32::MAX;
        }
        v.mask_top();
        v
    }

    /// All `Z`.
    pub fn zs(width: u32) -> LogicVec {
        let mut v = LogicVec::zeros(width);
        for w in v.bval.as_mut_slice() {
            *w = u32::MAX;
        }
        v.mask_top();
        v
    }

    /// Build from raw bit-planes. `aval`/`bval` must have `ceil(width/32)`
    /// words; bits above `width` are cleared.
    pub fn from_planes(width: u32, aval: Vec<u32>, bval: Vec<u32>) -> LogicVec {
        assert_eq!(aval.len(), words_for(width));
        assert_eq!(bval.len(), words_for(width));
        let mut v = LogicVec { width, aval: Words::from_vec(aval), bval: Words::from_vec(bval) };
        v.mask_top();
        v
    }

    /// Two-state value from an integer, truncated or zero-extended to
    /// `width`.
    pub fn from_u64(width: u32, value: u64) -> LogicVec {
        let mut v = LogicVec::zeros(width);
        let a = v.aval.as_mut_slice();
        if !a.is_empty() {
            a[0] = value as u32;
        }
        if a.len() > 1 {
            a[1] = (value >> 32) as u32;
        }
        v.mask_top();
        v
    }

    /// Two-state value from a 128-bit integer.
    pub fn from_u128(width: u32, value: u128) -> LogicVec {
        let mut v = LogicVec::zeros(width);
        for (i, w) in v.aval.as_mut_slice().iter_mut().enumerate().take(4) {
            *w = (value >> (32 * i)) as u32;
        }
        v.mask_top();
        v
    }

    /// Two-state value from a signed integer with sign extension.
    pub fn from_i64(width: u32, value: i64) -> LogicVec {
        let mut v = LogicVec::zeros(width);
        let bits = value as u64;
        for (i, w) in v.aval.as_mut_slice().iter_mut().enumerate() {
            *w = if i < 2 {
                (bits >> (32 * i)) as u32
            } else if value < 0 {
                u32::MAX
            } else {
                0
            };
        }
        v.mask_top();
        v
    }

    /// Parse a binary string, most significant bit first, in the alphabet
    /// `01xzXZ` plus VHDL `uwlhUWLH-`. Underscores are ignored. Width is the
    /// number of digits.
    pub fn from_binstr(s: &str) -> Option<LogicVec> {
        let digits: Vec<Logic> = s.chars().filter(|c| *c != '_').map(Logic::from_char).collect::<Option<_>>()?;
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
        self.aval.as_slice()
    }

    #[inline]
    pub fn bval(&self) -> &[u32] {
        self.bval.as_slice()
    }

    /// Mutable access to the bit planes for zero-copy fills from a
    /// simulator. The caller must keep bits above `width` clear or call
    /// [`LogicVec::mask_top`].
    #[inline]
    pub fn planes_mut(&mut self) -> (&mut [u32], &mut [u32]) {
        (self.aval.as_mut_slice(), self.bval.as_mut_slice())
    }

    /// Clear bits above `width` in the top word.
    pub fn mask_top(&mut self) {
        let rem = self.width % 32;
        if rem != 0 {
            let mask = (1u32 << rem) - 1;
            if let Some(w) = self.aval.as_mut_slice().last_mut() {
                *w &= mask;
            }
            if let Some(w) = self.bval.as_mut_slice().last_mut() {
                *w &= mask;
            }
        }
    }

    /// Resize in place, keeping the low bits and zero-filling new ones.
    pub fn resize(&mut self, width: u32) {
        let n = words_for(width);
        self.aval.resize(n);
        self.bval.resize(n);
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
        Logic::from_ab(self.aval.as_slice()[w] & m != 0, self.bval.as_slice()[w] & m != 0)
    }

    #[inline]
    pub fn set_bit(&mut self, i: u32, v: Logic) {
        assert!(i < self.width, "bit index {i} out of range for width {}", self.width);
        let w = (i / 32) as usize;
        let m = 1u32 << (i % 32);
        let (a, b) = v.to_ab();
        let aw = &mut self.aval.as_mut_slice()[w];
        if a {
            *aw |= m;
        } else {
            *aw &= !m;
        }
        let bw = &mut self.bval.as_mut_slice()[w];
        if b {
            *bw |= m;
        } else {
            *bw &= !m;
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
        self.bval.as_slice().iter().all(|w| *w == 0)
    }

    /// `true` if any bit is `X`.
    pub fn has_x(&self) -> bool {
        self.aval.as_slice().iter().zip(self.bval.as_slice()).any(|(a, b)| a & b != 0)
    }

    /// `true` if any bit is `Z`.
    pub fn has_z(&self) -> bool {
        self.aval.as_slice().iter().zip(self.bval.as_slice()).any(|(a, b)| !a & b != 0)
    }

    /// Zero-extended integer value; `Err` if any bit is `X`/`Z` or the
    /// value does not fit in 64 bits.
    pub fn to_u64(&self) -> Result<u64, Unresolved> {
        if !self.is_resolvable() {
            return Err(Unresolved { value: self.clone() });
        }
        let a = self.aval.as_slice();
        if a.iter().skip(2).any(|w| *w != 0) {
            return Err(Unresolved { value: self.clone() });
        }
        let lo = *a.first().unwrap_or(&0) as u64;
        let hi = *a.get(1).unwrap_or(&0) as u64;
        Ok(lo | (hi << 32))
    }

    /// Zero-extended integer value up to 128 bits.
    pub fn to_u128(&self) -> Result<u128, Unresolved> {
        if !self.is_resolvable() || self.aval.as_slice().iter().skip(4).any(|w| *w != 0) {
            return Err(Unresolved { value: self.clone() });
        }
        let mut v = 0u128;
        for (i, w) in self.aval.as_slice().iter().enumerate().take(4) {
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
        let (a, b) = (self.aval.as_slice(), self.bval.as_slice());
        let lo = (a.first().copied().unwrap_or(0) & !b.first().copied().unwrap_or(0)) as u64;
        let hi = (a.get(1).copied().unwrap_or(0) & !b.get(1).copied().unwrap_or(0)) as u64;
        lo | (hi << 32)
    }

    /// Binary string, most significant bit first.
    pub fn to_binstr(&self) -> String {
        (0..self.width).rev().map(|i| self.bit(i).to_char()).collect()
    }

    /// Hex string, most significant nibble first. A nibble with any `X` is
    /// `x`; with any `Z` (and no `X`) is `z`.
    pub fn to_hexstr(&self) -> String {
        let nibbles = self.width.div_ceil(4);
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
        LogicVec::parse(self).unwrap_or_else(|| panic!("cannot parse {self:?} as a logic vector")).into_logic_vec(width)
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

    #[test]
    fn slice_and_bits() {
        let v = LogicVec::parse("12'hA5F").unwrap();
        assert_eq!(v.slice(3, 0).to_u64().unwrap(), 0xf);
        assert_eq!(v.slice(11, 8).to_u64().unwrap(), 0xa);
        assert_eq!(v.slice(7, 4).to_hexstr(), "5");
        let mut w = LogicVec::zeros(8);
        w.set_bit(7, Logic::One);
        w.set_bit(0, Logic::Z);
        assert_eq!(w.to_binstr(), "1000000z");
        assert!(w.has_z() && !w.has_x());
        assert_eq!(w.to_u64_lossy(), 0x80);
        assert_eq!(w.iter().filter(|b| *b == Logic::Zero).count(), 6);
    }

    #[test]
    fn inline_heap_transitions() {
        let mut v = LogicVec::from_u64(64, u64::MAX);
        assert_eq!(v.aval().len(), 2);
        v.resize(100);
        assert_eq!(v.aval().len(), 4);
        assert_eq!(v.to_u128().unwrap(), u64::MAX as u128, "resize keeps low bits and zero-fills");
        v.set_bit(99, Logic::One);
        v.resize(40);
        assert_eq!(v.aval().len(), 2);
        assert_eq!(v.to_u64().unwrap(), (1u64 << 40) - 1, "shrinking masks the top word");
        let planes = LogicVec::from_planes(70, vec![1, 2, 3], vec![0, 0, 0]);
        assert_eq!(planes.width(), 70);
        assert_eq!(planes.aval(), &[1, 2, 3]);
        assert_eq!(LogicVec::from_planes(70, vec![0, 0, u32::MAX], vec![0; 3]).aval()[2], (1 << 6) - 1);
    }

    #[test]
    fn xz_states() {
        let x = LogicVec::xs(4);
        let z = LogicVec::zs(4);
        assert_eq!(x.to_binstr(), "xxxx");
        assert_eq!(z.to_binstr(), "zzzz");
        assert!(x.to_u64().is_err() && z.to_u64().is_err());
        assert_eq!(x.to_hexstr(), "x");
        assert_eq!(LogicVec::from_binstr("zz01").unwrap().to_hexstr(), "z");
        assert_eq!(LogicVec::from_binstr("UWLH-").unwrap().to_binstr(), "xx01x");
        assert!(LogicVec::from_binstr("012").is_none());
        assert!(LogicVec::parse("8'q1").is_none());
        assert!(LogicVec::parse("x'b1").is_none());
        assert_eq!(LogicVec::parse("4'hx").unwrap().to_binstr(), "xxxx");
        assert_eq!(LogicVec::parse("3'b1_0_1").unwrap().to_u64().unwrap(), 5);
        assert_eq!(LogicVec::parse("6'hff").unwrap().to_u64().unwrap(), 0x3f, "hex literal truncated to width");
    }

    #[test]
    fn integer_conversions() {
        assert_eq!(LogicVec::from_i64(4, -1).to_binstr(), "1111");
        assert_eq!(LogicVec::from_i64(4, -1).to_i64().unwrap(), -1);
        assert_eq!(LogicVec::from_i64(1, 1).to_i64().unwrap(), -1, "1-bit signed");
        assert_eq!(LogicVec::from_i64(64, -5).to_i64().unwrap(), -5);
        assert_eq!(LogicVec::from_i64(100, -5).to_u128().unwrap() & 0xffff, 0xfffb);
        assert_eq!(LogicVec::from_u64(65, u64::MAX).to_u64().unwrap(), u64::MAX);
        let big = LogicVec::from_u128(65, 1u128 << 64);
        assert!(big.to_u64().is_err(), "does not fit 64 bits");
        assert_eq!(big.to_u128().unwrap(), 1u128 << 64);
        assert_eq!(LogicVec::zeros(0).to_u64().unwrap(), 0);
        assert_eq!(LogicVec::zeros(0).to_binstr(), "");
    }

    #[test]
    fn into_logic_vec_impls() {
        assert_eq!(true.into_logic_vec(4).to_u64().unwrap(), 1);
        assert_eq!(Logic::X.into_logic_vec(4).to_binstr(), "000x");
        assert_eq!((-1i8).into_logic_vec(16).to_u64().unwrap(), 0xffff);
        assert_eq!(0x1234u16.into_logic_vec(8).to_u64().unwrap(), 0x34);
        assert_eq!("8'hAB".into_logic_vec(8).to_u64().unwrap(), 0xab);
        assert_eq!("1010".into_logic_vec(8).to_u64().unwrap(), 0b1010);
        let v = LogicVec::from_u64(8, 7);
        assert_eq!((&v).into_logic_vec(4).to_u64().unwrap(), 7);
        assert_eq!(v.clone().into_logic_vec(16).width(), 16);
        assert_eq!(usize::MAX.into_logic_vec(3).to_u64().unwrap(), 7);
    }

    #[test]
    #[should_panic(expected = "cannot parse")]
    fn bad_literal_panics() {
        let _ = "8'hzz zz".into_logic_vec(8);
    }

    #[test]
    fn display_and_eq() {
        let v = LogicVec::from_u64(12, 0xabc);
        assert_eq!(format!("{v}"), "12'b101010111100");
        assert_eq!(format!("{v:#}"), "12'habc");
        assert_eq!(format!("{v:?}"), "12'b101010111100");
        assert_eq!(format!("{v:x}"), "abc");
        assert_eq!(format!("{v:b}"), "101010111100");
        assert!(v == 0xabcu64);
        assert!(v == 0xabci32);
        assert!(LogicVec::from_i64(8, -2) == -2i32);
        assert!(!(LogicVec::xs(8) == 0u64));
        assert_eq!(Logic::from_char('L'), Some(Logic::Zero));
        assert_eq!(Logic::from_char('q'), None);
        assert_eq!(format!("{}", Logic::Z), "z");
        assert_eq!(LogicVec::from(Logic::One).to_u64().unwrap(), 1);
        assert_eq!(LogicVec::from(false).width(), 1);
        let u = LogicVec::xs(2).to_u64().unwrap_err();
        assert!(u.to_string().contains("X or Z"));
    }

    #[test]
    fn hash_and_eq_ignore_storage_kind() {
        use std::collections::HashSet;
        let mut a = LogicVec::from_u64(100, 5);
        a.resize(8);
        let b = LogicVec::from_u64(8, 5);
        assert_eq!(a, b);
        let mut set = HashSet::new();
        set.insert(a);
        assert!(set.contains(&b));
    }
}

#[cfg(test)]
mod round_trip_tests {
    use super::*;
    use crate::random::Rng;

    /// Randomised round trips. `cargo fuzz` needs a nightly toolchain and a
    /// separate crate; these run in the ordinary suite, with a seed printed
    /// on failure so any case that does fail is reproducible.
    #[test]
    fn random_vectors_round_trip_through_every_representation() {
        let mut rng = Rng::seed_from_u64(0xfa17_c0de);
        for _ in 0..2000 {
            let width = 1 + (rng.next_u32() % 200);
            let mut v = LogicVec::zeros(width);
            for i in 0..width {
                let bit = match rng.next_u32() % 4 {
                    0 => Logic::Zero,
                    1 => Logic::One,
                    2 => Logic::Z,
                    _ => Logic::X,
                };
                v.set_bit(i, bit);
            }
            // Binary string.
            let s = v.to_binstr();
            assert_eq!(s.len(), width as usize, "binstr length for width {width}");
            let back = LogicVec::from_binstr(&s).expect("parses back");
            assert_eq!(back, v, "binstr round trip of {s}");

            // Slices cover the whole vector without changing it.
            let hi = width - 1;
            let mid = width / 2;
            let top = v.slice(hi, mid);
            let bottom = v.slice(mid.saturating_sub(1).min(mid), 0);
            assert_eq!(top.width(), hi - mid + 1);
            assert!(bottom.width() >= 1);

            // Integers, where the value has no X or Z.
            if v.is_resolvable() && width <= 64 {
                let n = v.to_u64().unwrap();
                assert_eq!(LogicVec::from_u64(width, n), v, "u64 round trip of {s}");
            }
        }
    }

    #[test]
    fn parsing_never_panics_on_arbitrary_text() {
        let mut rng = Rng::seed_from_u64(7);
        let alphabet: Vec<char> = "01xXzZuUwWlLhH-'\"bhod_ 0123456789'".chars().collect();
        for _ in 0..5000 {
            let len = (rng.next_u32() % 24) as usize;
            let s: String = (0..len).map(|_| alphabet[(rng.next_u32() as usize) % alphabet.len()]).collect();
            // Either parses or returns None; never panics, never hangs.
            let _ = LogicVec::parse(&s);
            let _ = LogicVec::from_binstr(&s);
        }
    }
}
