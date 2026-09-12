//! A small non-cryptographic hasher (the rustc "Fx" hash) for the runtime's
//! integer-keyed maps. SipHash showed up at 15% of instructions in profiles.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

#[derive(Default, Clone, Copy)]
pub struct FxHasher {
    hash: u64,
}

const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

impl FxHasher {
    #[inline]
    fn add(&mut self, word: u64) {
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(SEED);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut w = [0u8; 8];
            w[..chunk.len()].copy_from_slice(chunk);
            self.add(u64::from_le_bytes(w));
        }
    }
    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.add(i as u64)
    }
    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.add(i as u64)
    }
    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.add(i)
    }
    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.add(i as u64)
    }
    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }
}

pub type FxBuildHasher = BuildHasherDefault<FxHasher>;
pub type FxHashMap<K, V> = HashMap<K, V, FxBuildHasher>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::hash::BuildHasher;

    #[test]
    fn map_works_and_hashes_differ() {
        let mut m: FxHashMap<u64, u32> = FxHashMap::default();
        for i in 0..1000u64 {
            m.insert(i, i as u32 * 2);
        }
        assert_eq!(m[&999], 1998);
        assert_eq!(m.len(), 1000);
        let bh = FxBuildHasher::default();
        let h = |v: u64| bh.hash_one(v);
        assert_ne!(h(1), h(2));
        assert_eq!(h(7), h(7));
        let mut s = FxHasher::default();
        s.write(&[1, 2, 3, 4, 5, 6, 7, 8, 9]);
        assert_ne!(s.finish(), 0);
    }
}
