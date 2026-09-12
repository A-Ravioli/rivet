//! Reproducible random stimulus.
//!
//! Every run has a base seed (`RIVET_SEED`, or one drawn from the clock and
//! logged). Each test gets its own stream derived from the base seed and
//! the test's full name, so filtering or reordering tests does not change
//! the values a test sees. Replay a failure with `rivet run --seed <n>`.
//!
//! ```ignore
//! let mut rng = rivet::rng();               // a fresh stream for this task
//! let addr = rng.gen_range(0..256u32);
//! let txn = Txn::randomize(&mut rng);       // #[derive(Randomize)]
//! ```
//!
//! The generator is xoshiro256** seeded through splitmix64: small, fast,
//! and identical on every platform.

use crate::value::{Logic, LogicVec};
use std::cell::{Cell, RefCell};
use std::ops::{Range, RangeInclusive};

/// A deterministic pseudo-random generator (xoshiro256**).
#[derive(Clone, Debug)]
pub struct Rng {
    s: [u64; 4],
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl Rng {
    /// A generator whose sequence is fixed by `seed`.
    pub fn seed_from_u64(seed: u64) -> Rng {
        let mut st = seed;
        let s = [splitmix64(&mut st), splitmix64(&mut st), splitmix64(&mut st), splitmix64(&mut st)];
        Rng { s }
    }

    pub fn next_u64(&mut self) -> u64 {
        let s = &mut self.s;
        let result = s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = s[1] << 17;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        result
    }

    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// An independent stream derived from this one.
    pub fn fork(&mut self) -> Rng {
        Rng::seed_from_u64(self.next_u64())
    }

    /// Uniform in `[0, n)`; `n == 0` returns 0.
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            return 0;
        }
        // Rejection sampling on the top of the range removes modulo bias.
        let zone = u64::MAX - (u64::MAX % n);
        loop {
            let v = self.next_u64();
            if v < zone {
                return v % n;
            }
        }
    }

    /// Uniform over a range: `0..16`, `1..=6`, `-8..8i32`.
    pub fn gen_range<T: SampleRange>(&mut self, range: T) -> T::Output {
        range.sample(self)
    }

    /// `true` with probability `p`.
    pub fn gen_bool(&mut self, p: f64) -> bool {
        self.gen::<f64>() < p
    }

    /// A random value of any [`Random`] type.
    pub fn gen<T: Random>(&mut self) -> T {
        T::random(self)
    }

    /// A random element, or `None` if the slice is empty.
    pub fn choose<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        if items.is_empty() {
            None
        } else {
            Some(&items[self.below(items.len() as u64) as usize])
        }
    }

    /// A random element by weight; `None` if the weights sum to zero.
    pub fn choose_weighted<'a, T>(&mut self, items: &'a [(T, u32)]) -> Option<&'a T> {
        let total: u64 = items.iter().map(|(_, w)| *w as u64).sum();
        if total == 0 {
            return None;
        }
        let mut pick = self.below(total);
        for (item, w) in items {
            if pick < *w as u64 {
                return Some(item);
            }
            pick -= *w as u64;
        }
        None
    }

    /// Fisher-Yates shuffle.
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.below(i as u64 + 1) as usize;
            items.swap(i, j);
        }
    }

    pub fn fill_bytes(&mut self, out: &mut [u8]) {
        for chunk in out.chunks_mut(8) {
            let v = self.next_u64().to_le_bytes();
            chunk.copy_from_slice(&v[..chunk.len()]);
        }
    }

    /// A fully resolved (no X/Z) vector of `width` bits.
    pub fn logic_vec(&mut self, width: u32) -> LogicVec {
        let mut v = LogicVec::zeros(width);
        for i in 0..width {
            if self.next_u64() & 1 == 1 {
                v.set_bit(i, Logic::One);
            }
        }
        v
    }

    /// A vector where each bit is X with probability `p_x` (for X-robustness tests).
    pub fn logic_vec_with_x(&mut self, width: u32, p_x: f64) -> LogicVec {
        let mut v = self.logic_vec(width);
        for i in 0..width {
            if self.gen_bool(p_x) {
                v.set_bit(i, Logic::X);
            }
        }
        v
    }
}

/// Types with a natural uniform distribution.
pub trait Random: Sized {
    fn random(rng: &mut Rng) -> Self;
}

macro_rules! impl_random_int {
    ($($t:ty),*) => {$(
        impl Random for $t {
            fn random(rng: &mut Rng) -> $t { rng.next_u64() as $t }
        }
    )*};
}
impl_random_int!(u8, u16, u32, u64, usize, i8, i16, i32, i64, isize);

impl Random for u128 {
    fn random(rng: &mut Rng) -> u128 {
        ((rng.next_u64() as u128) << 64) | rng.next_u64() as u128
    }
}
impl Random for i128 {
    fn random(rng: &mut Rng) -> i128 {
        u128::random(rng) as i128
    }
}
impl Random for bool {
    fn random(rng: &mut Rng) -> bool {
        rng.next_u64() & 1 == 1
    }
}
/// Uniform in `[0, 1)`.
impl Random for f64 {
    fn random(rng: &mut Rng) -> f64 {
        (rng.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }
}
impl Random for f32 {
    fn random(rng: &mut Rng) -> f32 {
        (rng.next_u32() >> 8) as f32 * (1.0 / (1u32 << 24) as f32)
    }
}
impl<T: Random, const N: usize> Random for [T; N] {
    fn random(rng: &mut Rng) -> [T; N] {
        std::array::from_fn(|_| T::random(rng))
    }
}
impl<A: Random, B: Random> Random for (A, B) {
    fn random(rng: &mut Rng) -> (A, B) {
        (A::random(rng), B::random(rng))
    }
}
impl<A: Random, B: Random, C: Random> Random for (A, B, C) {
    fn random(rng: &mut Rng) -> (A, B, C) {
        (A::random(rng), B::random(rng), C::random(rng))
    }
}
impl Random for () {
    fn random(_: &mut Rng) {}
}

/// Ranges that can be sampled uniformly.
pub trait SampleRange {
    type Output;
    fn sample(self, rng: &mut Rng) -> Self::Output;
}

macro_rules! impl_sample_range {
    ($($t:ty),*) => {$(
        impl SampleRange for Range<$t> {
            type Output = $t;
            fn sample(self, rng: &mut Rng) -> $t {
                assert!(self.start < self.end, "gen_range: empty range {:?}", self);
                let span = (self.end as i128 - self.start as i128) as u64;
                (self.start as i128 + rng.below(span) as i128) as $t
            }
        }
        impl SampleRange for RangeInclusive<$t> {
            type Output = $t;
            fn sample(self, rng: &mut Rng) -> $t {
                let (lo, hi) = (*self.start(), *self.end());
                assert!(lo <= hi, "gen_range: empty range {:?}", self);
                let span = (hi as i128 - lo as i128 + 1) as u128;
                if span > u64::MAX as u128 {
                    return rng.next_u64() as $t;
                }
                (lo as i128 + rng.below(span as u64) as i128) as $t
            }
        }
    )*};
}
impl_sample_range!(u8, u16, u32, u64, usize, i8, i16, i32, i64, isize);

impl SampleRange for Range<f64> {
    type Output = f64;
    fn sample(self, rng: &mut Rng) -> f64 {
        self.start + (self.end - self.start) * rng.gen::<f64>()
    }
}

/// Types that can build a random instance of themselves; implemented by
/// `#[derive(Randomize)]` and, through [`Random`], by the primitives.
pub trait Randomize: Sized {
    fn randomize(rng: &mut Rng) -> Self;
}

impl<T: Random> Randomize for T {
    fn randomize(rng: &mut Rng) -> T {
        T::random(rng)
    }
}

impl<T: Randomize> Randomize for Option<T> {
    fn randomize(rng: &mut Rng) -> Option<T> {
        if bool::random(rng) {
            Some(T::randomize(rng))
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Per-run and per-test seeding

thread_local! {
    static BASE_SEED: Cell<Option<u64>> = const { Cell::new(None) };
    static MASTER: RefCell<Option<Rng>> = const { RefCell::new(None) };
    static TEST_SEED: Cell<u64> = const { Cell::new(0) };
}

/// Parse `RIVET_SEED` (decimal or `0x` hex).
pub fn parse_seed(s: &str) -> Option<u64> {
    let s = s.trim();
    if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u64::from_str_radix(h, 16).ok()
    } else {
        s.parse().ok()
    }
}

/// The run's base seed: from `RIVET_SEED` if set, otherwise from the clock.
/// Computed once and logged.
pub fn base_seed() -> u64 {
    BASE_SEED.with(|c| {
        if let Some(s) = c.get() {
            return s;
        }
        let seed = match std::env::var("RIVET_SEED") {
            Ok(v) => parse_seed(&v).unwrap_or_else(|| panic!("RIVET_SEED={v:?} is not an integer")),
            Err(_) => {
                let t = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos() as u64)
                    .unwrap_or(1);
                let mut st = t ^ (std::process::id() as u64) << 32;
                splitmix64(&mut st) >> 1
            }
        };
        c.set(Some(seed));
        seed
    })
}

/// Override the base seed (tests and embedding harnesses).
pub fn set_base_seed(seed: u64) {
    BASE_SEED.with(|c| c.set(Some(seed)));
}

/// The seed a test with this full name gets under the current base seed.
pub fn seed_for_test(base: u64, full_name: &str) -> u64 {
    let mut h = crate::fxhash::FxHasher::default();
    std::hash::Hasher::write(&mut h, full_name.as_bytes());
    let mut st = base ^ std::hash::Hasher::finish(&h).rotate_left(17);
    splitmix64(&mut st)
}

/// Start a test's stream. Returns the test seed (recorded in results.xml).
pub fn begin_test(full_name: &str) -> u64 {
    let seed = seed_for_test(base_seed(), full_name);
    MASTER.with(|m| *m.borrow_mut() = Some(Rng::seed_from_u64(seed)));
    TEST_SEED.with(|c| c.set(seed));
    seed
}

/// The current test's seed.
pub fn test_seed() -> u64 {
    TEST_SEED.with(|c| c.get())
}

/// A fresh stream forked from the current test's master stream. Each call
/// returns a different, reproducible stream, so tasks started in the same
/// order see the same values run after run.
pub fn rng() -> Rng {
    MASTER.with(|m| {
        let mut g = m.borrow_mut();
        let master = g.get_or_insert_with(|| Rng::seed_from_u64(seed_for_test(base_seed(), "")));
        master.fork()
    })
}

/// Run `f` with the test's master stream (no fork).
pub fn with_master<R>(f: impl FnOnce(&mut Rng) -> R) -> R {
    MASTER.with(|m| {
        let mut g = m.borrow_mut();
        let master = g.get_or_insert_with(|| Rng::seed_from_u64(seed_for_test(base_seed(), "")));
        f(master)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PINNED_SEED_42: u64 = 1546998764402558742;

    #[test]
    fn deterministic_and_seed_sensitive() {
        let mut a = Rng::seed_from_u64(42);
        let mut b = Rng::seed_from_u64(42);
        let mut c = Rng::seed_from_u64(43);
        let xa: Vec<u64> = (0..8).map(|_| a.next_u64()).collect();
        let xb: Vec<u64> = (0..8).map(|_| b.next_u64()).collect();
        let xc: Vec<u64> = (0..8).map(|_| c.next_u64()).collect();
        assert_eq!(xa, xb);
        assert_ne!(xa, xc);
        // Pinned so that seeds recorded in results.xml stay replayable
        // across Rivet versions.
        assert_eq!(xa[0], PINNED_SEED_42);
    }

    #[test]
    fn ranges_stay_in_bounds() {
        let mut r = Rng::seed_from_u64(1);
        for _ in 0..10_000 {
            let v = r.gen_range(3..7u8);
            assert!((3..7).contains(&v));
            let v = r.gen_range(-5..=5i32);
            assert!((-5..=5).contains(&v));
            let v = r.gen_range(0..1u64);
            assert_eq!(v, 0);
            let f = r.gen::<f64>();
            assert!((0.0..1.0).contains(&f));
            let f = r.gen_range(2.0..3.0);
            assert!((2.0..3.0).contains(&f));
        }
        let v = r.gen_range(0..=u64::MAX);
        let _ = v;
        assert_eq!(r.gen_range(9..=9u32), 9);
    }

    #[test]
    fn distribution_is_roughly_uniform() {
        let mut r = Rng::seed_from_u64(7);
        let mut counts = [0u32; 10];
        for _ in 0..100_000 {
            counts[r.below(10) as usize] += 1;
        }
        for c in counts {
            assert!((9_000..11_000).contains(&c), "bucket count {c} far from 10000");
        }
        let mut heads = 0;
        for _ in 0..10_000 {
            if r.gen_bool(0.25) {
                heads += 1;
            }
        }
        assert!((2_000..3_000).contains(&heads), "{heads}");
    }

    #[test]
    fn choose_shuffle_weighted() {
        let mut r = Rng::seed_from_u64(3);
        assert_eq!(r.choose::<u8>(&[]), None);
        assert_eq!(r.choose(&[5]), Some(&5));
        let mut v: Vec<u32> = (0..20).collect();
        let orig = v.clone();
        r.shuffle(&mut v);
        assert_ne!(v, orig);
        let mut sorted = v.clone();
        sorted.sort();
        assert_eq!(sorted, orig);
        let w = [("a", 0u32), ("b", 3), ("c", 1)];
        let mut b = 0;
        for _ in 0..4000 {
            match *r.choose_weighted(&w).unwrap() {
                "a" => panic!("zero weight chosen"),
                "b" => b += 1,
                _ => {}
            }
        }
        assert!((2_700..3_300).contains(&b), "{b}");
        assert!(r.choose_weighted(&[("z", 0u32)]).is_none());
    }

    #[test]
    fn vectors_bytes_and_x() {
        let mut r = Rng::seed_from_u64(9);
        let v = r.logic_vec(100);
        assert_eq!(v.width(), 100);
        assert!(v.is_resolvable());
        let mut any_one = false;
        for i in 0..100 {
            any_one |= v.bit(i) == Logic::One;
        }
        assert!(any_one);
        let vx = r.logic_vec_with_x(64, 1.0);
        assert!(!vx.is_resolvable());
        let mut bytes = [0u8; 13];
        r.fill_bytes(&mut bytes);
        assert!(bytes.iter().any(|&b| b != 0));
        let arr: [u8; 4] = r.gen();
        let _ = arr;
        let pair: (u16, bool) = r.gen();
        let _ = pair;
    }

    #[test]
    fn per_test_streams() {
        set_base_seed(1234);
        assert_eq!(base_seed(), 1234);
        assert_eq!(parse_seed("0x10"), Some(16));
        assert_eq!(parse_seed(" 99 "), Some(99));
        assert_eq!(parse_seed("x"), None);
        let s1 = begin_test("m::a");
        let a1: Vec<u64> = (0..4).map(|_| rng().next_u64()).collect();
        let s2 = begin_test("m::b");
        let b1: Vec<u64> = (0..4).map(|_| rng().next_u64()).collect();
        assert_ne!(s1, s2);
        assert_ne!(a1, b1);
        // Same test again: identical stream regardless of what ran before.
        assert_eq!(begin_test("m::a"), s1);
        assert_eq!(test_seed(), s1);
        let a2: Vec<u64> = (0..4).map(|_| rng().next_u64()).collect();
        assert_eq!(a1, a2);
        // Each rng() call is a distinct fork.
        assert_ne!(rng().next_u64(), rng().next_u64());
        with_master(|m| {
            let _ = m.next_u64();
        });
        assert_eq!(seed_for_test(1, "x"), seed_for_test(1, "x"));
        assert_ne!(seed_for_test(1, "x"), seed_for_test(2, "x"));
    }

    #[test]
    fn option_randomize() {
        let mut r = Rng::seed_from_u64(5);
        let mut somes = 0;
        for _ in 0..1000 {
            if Option::<u8>::randomize(&mut r).is_some() {
                somes += 1;
            }
        }
        assert!((400..600).contains(&somes));
    }
}
