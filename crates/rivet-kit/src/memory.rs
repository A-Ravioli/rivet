//! A sparse byte-addressed memory model, plus `$readmemh` loading.
//!
//! `Memory` is shared (cloning gives another handle to the same bytes), so
//! a bus responder can own one while the test inspects it.

use rivet_core::error::{Error, Result};
use rivet_core::handle::Signal;
use rivet_core::value::{Logic, LogicVec};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::Path;
use std::rc::Rc;

const PAGE_SHIFT: u32 = 12;
const PAGE: usize = 1 << PAGE_SHIFT;

struct Inner {
    pages: BTreeMap<u64, Box<[u8; PAGE]>>,
    fill: u8,
}

/// Sparse memory: only pages that were written take space.
#[derive(Clone)]
pub struct Memory {
    inner: Rc<RefCell<Inner>>,
}

impl Default for Memory {
    fn default() -> Memory {
        Memory::new()
    }
}

impl Memory {
    /// Unwritten bytes read as zero.
    pub fn new() -> Memory {
        Memory::with_fill(0)
    }

    /// Unwritten bytes read as `fill`.
    pub fn with_fill(fill: u8) -> Memory {
        Memory { inner: Rc::new(RefCell::new(Inner { pages: BTreeMap::new(), fill })) }
    }

    pub fn read_u8(&self, addr: u64) -> u8 {
        let g = self.inner.borrow();
        match g.pages.get(&(addr >> PAGE_SHIFT)) {
            Some(p) => p[(addr as usize) & (PAGE - 1)],
            None => g.fill,
        }
    }

    pub fn write_u8(&self, addr: u64, v: u8) {
        let mut g = self.inner.borrow_mut();
        let fill = g.fill;
        let page = g.pages.entry(addr >> PAGE_SHIFT).or_insert_with(|| Box::new([fill; PAGE]));
        page[(addr as usize) & (PAGE - 1)] = v;
    }

    pub fn read_bytes(&self, addr: u64, out: &mut [u8]) {
        for (i, b) in out.iter_mut().enumerate() {
            *b = self.read_u8(addr + i as u64);
        }
    }

    pub fn read_vec(&self, addr: u64, n: usize) -> Vec<u8> {
        let mut v = vec![0; n];
        self.read_bytes(addr, &mut v);
        v
    }

    pub fn write_bytes(&self, addr: u64, data: &[u8]) {
        for (i, b) in data.iter().enumerate() {
            self.write_u8(addr + i as u64, *b);
        }
    }

    /// Little-endian word of `bytes` bytes (1..=8).
    pub fn read_word(&self, addr: u64, bytes: u32) -> u64 {
        let mut v = 0u64;
        for i in 0..bytes.min(8) {
            v |= (self.read_u8(addr + i as u64) as u64) << (8 * i);
        }
        v
    }

    /// Write a little-endian word; only bytes whose `strobe` bit is set.
    pub fn write_word(&self, addr: u64, bytes: u32, value: u64, strobe: u64) {
        for i in 0..bytes.min(8) {
            if strobe >> i & 1 == 1 {
                self.write_u8(addr + i as u64, (value >> (8 * i)) as u8);
            }
        }
    }

    pub fn read_u16(&self, addr: u64) -> u16 {
        self.read_word(addr, 2) as u16
    }
    pub fn read_u32(&self, addr: u64) -> u32 {
        self.read_word(addr, 4) as u32
    }
    pub fn read_u64(&self, addr: u64) -> u64 {
        self.read_word(addr, 8)
    }
    pub fn write_u16(&self, addr: u64, v: u16) {
        self.write_word(addr, 2, v as u64, 0xff)
    }
    pub fn write_u32(&self, addr: u64, v: u32) {
        self.write_word(addr, 4, v as u64, 0xff)
    }
    pub fn write_u64(&self, addr: u64, v: u64) {
        self.write_word(addr, 8, v, 0xff)
    }

    /// Number of pages allocated (each 4 KiB).
    pub fn pages(&self) -> usize {
        self.inner.borrow().pages.len()
    }

    /// Forget everything.
    pub fn clear(&self) {
        self.inner.borrow_mut().pages.clear();
    }

    /// Load a `$readmemh`-format file: hex words separated by whitespace,
    /// `@addr` to set the word address, `//` and `/* */` comments. Each
    /// word occupies `word_bytes` bytes starting at `base + index * word_bytes`.
    /// Words with X or Z digits are skipped (left as they were).
    pub fn load_hex(&self, path: impl AsRef<Path>, base: u64, word_bytes: u32) -> Result<usize> {
        let text = std::fs::read_to_string(path.as_ref())
            .map_err(|e| Error::Msg(format!("cannot read {}: {e}", path.as_ref().display())))?;
        let mut n = 0;
        for (index, word) in parse_readmemh(&text)? {
            if let Ok(v) = word.to_u64() {
                self.write_word(base + index * word_bytes as u64, word_bytes, v, u64::MAX);
                n += 1;
            }
        }
        Ok(n)
    }

    /// Load raw bytes from a file at `base`.
    pub fn load_bin(&self, path: impl AsRef<Path>, base: u64) -> Result<usize> {
        let data = std::fs::read(path.as_ref())
            .map_err(|e| Error::Msg(format!("cannot read {}: {e}", path.as_ref().display())))?;
        self.write_bytes(base, &data);
        Ok(data.len())
    }

    /// Write `words` words of `word_bytes` from `base` in `$readmemh` format.
    pub fn dump_hex(&self, path: impl AsRef<Path>, base: u64, words: u64, word_bytes: u32) -> Result<()> {
        use std::fmt::Write as _;
        let mut s = String::new();
        for i in 0..words {
            let v = self.read_word(base + i * word_bytes as u64, word_bytes);
            let _ = writeln!(s, "{:0width$x}", v, width = 2 * word_bytes as usize);
        }
        std::fs::write(path.as_ref(), s)
            .map_err(|e| Error::Msg(format!("cannot write {}: {e}", path.as_ref().display())))
    }
}

/// Parse `$readmemh` text into `(word index, value)` pairs. Values keep X
/// and Z digits.
pub fn parse_readmemh(text: &str) -> Result<Vec<(u64, LogicVec)>> {
    let mut out = Vec::new();
    let mut index = 0u64;
    let stripped = strip_comments(text);
    for tok in stripped.split_whitespace() {
        if let Some(a) = tok.strip_prefix('@') {
            index = u64::from_str_radix(a, 16).map_err(|_| Error::Msg(format!("bad address {tok:?}")))?;
            continue;
        }
        let v = hex_to_logic(tok).ok_or_else(|| Error::Msg(format!("bad hex word {tok:?}")))?;
        out.push((index, v));
        index += 1;
    }
    Ok(out)
}

fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '/' && chars.peek() == Some(&'/') {
            for c in chars.by_ref() {
                if c == '\n' {
                    out.push('\n');
                    break;
                }
            }
        } else if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            let mut prev = ' ';
            for c in chars.by_ref() {
                if prev == '*' && c == '/' {
                    break;
                }
                prev = c;
            }
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out
}

/// Hex digits (with `x`/`z`, underscores ignored) to a vector of 4 bits per
/// digit.
pub fn hex_to_logic(s: &str) -> Option<LogicVec> {
    let digits: Vec<char> = s.chars().filter(|c| *c != '_').collect();
    if digits.is_empty() {
        return None;
    }
    let width = 4 * digits.len() as u32;
    let mut v = LogicVec::zeros(width);
    for (i, d) in digits.iter().rev().enumerate() {
        let base = 4 * i as u32;
        match d.to_ascii_lowercase() {
            'x' => (0..4).for_each(|b| v.set_bit(base + b, Logic::X)),
            'z' | '?' => (0..4).for_each(|b| v.set_bit(base + b, Logic::Z)),
            c => {
                let n = c.to_digit(16)?;
                for b in 0..4 {
                    if n >> b & 1 == 1 {
                        v.set_bit(base + b, Logic::One);
                    }
                }
            }
        }
    }
    Some(v)
}

/// Load a `$readmemh` file into an unpacked HDL array through the testbench
/// (`mem[i] <= word`), for designs without their own `$readmemh`. Returns
/// the number of elements written. X/Z digits are written as such.
pub fn load_hex_into(array: &Signal, path: impl AsRef<Path>) -> Result<usize> {
    let text = std::fs::read_to_string(path.as_ref())
        .map_err(|e| Error::Msg(format!("cannot read {}: {e}", path.as_ref().display())))?;
    let count = array.width() as u64;
    let mut n = 0;
    for (index, word) in parse_readmemh(&text)? {
        if index >= count {
            return Err(Error::Msg(format!("{}: index {index} outside the array ({count} elements)", array.path())));
        }
        let elem = array.index(index as i64)?;
        let mut w = word;
        w.resize(elem.width());
        elem.set(w);
        n += 1;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_and_pages() {
        let m = Memory::with_fill(0xff);
        assert_eq!(m.read_u8(12345), 0xff);
        m.write_u32(0x1000, 0xdead_beef);
        assert_eq!(m.read_u8(0x1000), 0xef);
        assert_eq!(m.read_u16(0x1002), 0xdead);
        assert_eq!(m.read_u32(0x1000), 0xdead_beef);
        assert_eq!(m.read_u64(0x1000), 0xffff_ffff_dead_beef);
        m.write_word(0x1000, 4, 0x1122_3344, 0b0101);
        assert_eq!(m.read_u32(0x1000), 0xde22_be44);
        m.write_u64(u64::MAX - 7, 0x0102_0304_0506_0708);
        assert_eq!(m.read_u8(u64::MAX), 0x01);
        assert_eq!(m.pages(), 2);
        let m2 = m.clone();
        m2.write_u8(0, 7);
        assert_eq!(m.read_u8(0), 7, "clones share storage");
        assert_eq!(m.read_vec(0x1000, 2), vec![0x44, 0xbe]);
        m.clear();
        assert_eq!(m.pages(), 0);
    }

    #[test]
    fn readmemh_parsing() {
        let text = "// header\n00 ff /* two */ 1_2\n@10 abcd\nxz 0x\n";
        let words = parse_readmemh(text).unwrap();
        let idx: Vec<u64> = words.iter().map(|w| w.0).collect();
        assert_eq!(idx, [0, 1, 2, 0x10, 0x11, 0x12]);
        assert_eq!(words[1].1.to_u64().unwrap(), 0xff);
        assert_eq!(words[2].1.to_u64().unwrap(), 0x12);
        assert_eq!(words[3].1.width(), 16);
        assert_eq!(words[3].1.to_u64().unwrap(), 0xabcd);
        assert!(words[4].1.has_x() && words[4].1.has_z());
        assert!(words[5].1.has_x());
        assert!(parse_readmemh("@zz").is_err());
        assert!(parse_readmemh("g1").is_err());
        assert!(hex_to_logic("").is_none());
    }

    #[test]
    fn load_and_dump_hex_files() {
        let dir = std::env::temp_dir().join(format!("rivet-mem-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("prog.hex");
        std::fs::write(&f, "deadbeef\n@4 00000001\nxxxxxxxx\n").unwrap();
        let m = Memory::new();
        assert_eq!(m.load_hex(&f, 0x100, 4).unwrap(), 2, "the X word is skipped");
        assert_eq!(m.read_u32(0x100), 0xdead_beef);
        assert_eq!(m.read_u32(0x110), 1);
        assert_eq!(m.read_u32(0x114), 0);
        let out = dir.join("out.hex");
        m.dump_hex(&out, 0x100, 2, 4).unwrap();
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "deadbeef\n00000000\n");
        let bin = dir.join("x.bin");
        std::fs::write(&bin, [1u8, 2, 3]).unwrap();
        assert_eq!(m.load_bin(&bin, 0x200).unwrap(), 3);
        assert_eq!(m.read_u16(0x201), 0x0302);
        assert!(m.load_hex(dir.join("missing"), 0, 4).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
