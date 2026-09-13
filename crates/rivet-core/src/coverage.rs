//! Functional coverage: covergroups, coverpoints with named bins, and
//! crosses, recorded as plain data.
//!
//! ```ignore
//! let cg = Covergroup::new("axi_writes");
//! let len = cg.point("len", Bins::new().bin("single", 1..=1).bin("short", 2..=4).bin("long", 5..=16));
//! let size = cg.point("size", Bins::new().values("bytes", [1]).values("words", [4, 8]));
//! let cross = cg.cross("len_x_size", &[len, size]);
//! // ... in a monitor:
//! len.sample(txn.len);
//! size.sample(txn.size);   // the cross counts once both points were sampled
//! ```
//!
//! Every covergroup registers itself in a per-process registry. The runner
//! writes the registry to `RIVET_COVERAGE_FILE` (JSON) at the end of the
//! regression; `rivet cov report` merges the files from every shard and
//! parameter set and applies a threshold. Tests can also check coverage
//! in-process with [`Covergroup::percent`] or [`percent`].

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::rc::Rc;

/// A named bin: a set of inclusive integer ranges.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bin {
    pub name: String,
    pub ranges: Vec<(i128, i128)>,
}

impl Bin {
    fn contains(&self, v: i128) -> bool {
        self.ranges.iter().any(|(lo, hi)| *lo <= v && v <= *hi)
    }
}

/// Bin specification for a coverpoint.
#[derive(Clone, Debug, Default)]
pub struct Bins {
    bins: Vec<Bin>,
    ignore: Vec<(i128, i128)>,
    illegal: Vec<(i128, i128)>,
}

fn range_bounds<R: std::ops::RangeBounds<T>, T: Copy + Into<i128>>(r: R) -> (i128, i128) {
    use std::ops::Bound::*;
    let lo = match r.start_bound() {
        Included(v) => (*v).into(),
        Excluded(v) => (*v).into() + 1,
        Unbounded => i128::MIN,
    };
    let hi = match r.end_bound() {
        Included(v) => (*v).into(),
        Excluded(v) => (*v).into() - 1,
        Unbounded => i128::MAX,
    };
    (lo, hi)
}

impl Bins {
    pub fn new() -> Bins {
        Bins::default()
    }

    /// One bin covering a range: `bin("short", 1..=4)`.
    pub fn bin<T: Copy + Into<i128>>(mut self, name: &str, range: impl std::ops::RangeBounds<T>) -> Bins {
        self.bins.push(Bin { name: name.into(), ranges: vec![range_bounds(range)] });
        self
    }

    /// One bin covering listed values: `values("pow2", [1, 2, 4, 8])`.
    pub fn values<T: Copy + Into<i128>>(mut self, name: &str, values: impl IntoIterator<Item = T>) -> Bins {
        let ranges = values.into_iter().map(|v| (v.into(), v.into())).collect();
        self.bins.push(Bin { name: name.into(), ranges });
        self
    }

    /// One bin per value in the range, named `<prefix><value>`.
    pub fn auto<T: Copy + Into<i128>>(mut self, prefix: &str, range: impl std::ops::RangeBounds<T>) -> Bins {
        let (lo, hi) = range_bounds(range);
        assert!(hi - lo < 1 << 20, "auto bins: range too large ({lo}..={hi})");
        for v in lo..=hi {
            self.bins.push(Bin { name: format!("{prefix}{v}"), ranges: vec![(v, v)] });
        }
        self
    }

    /// Split a range into `n` equal bins named `<prefix>0..n`.
    pub fn split<T: Copy + Into<i128>>(mut self, prefix: &str, range: impl std::ops::RangeBounds<T>, n: u32) -> Bins {
        let (lo, hi) = range_bounds(range);
        assert!(n > 0 && hi >= lo, "split: empty range or zero bins");
        let span = hi - lo + 1;
        for i in 0..n as i128 {
            let a = lo + span * i / n as i128;
            let b = lo + span * (i + 1) / n as i128 - 1;
            if b >= a {
                self.bins.push(Bin { name: format!("{prefix}{i}"), ranges: vec![(a, b)] });
            }
        }
        self
    }

    /// Values that are neither counted nor reported as out of range.
    pub fn ignore<T: Copy + Into<i128>>(mut self, range: impl std::ops::RangeBounds<T>) -> Bins {
        self.ignore.push(range_bounds(range));
        self
    }

    /// Values that fail the running test when sampled.
    pub fn illegal<T: Copy + Into<i128>>(mut self, range: impl std::ops::RangeBounds<T>) -> Bins {
        self.illegal.push(range_bounds(range));
        self
    }

    pub fn len(&self) -> usize {
        self.bins.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bins.is_empty()
    }
}

struct PointInner {
    name: String,
    bins: Bins,
    hits: Vec<u64>,
    /// Bin index of the most recent sample (for crosses).
    last_bin: Option<usize>,
    /// Incremented on every counted sample.
    gen: u64,
    out_of_range: u64,
    ignored: u64,
}

struct CrossInner {
    name: String,
    points: Vec<usize>,
    /// Generation of each point at the last cross hit.
    seen: Vec<u64>,
    hits: BTreeMap<Vec<usize>, u64>,
}

struct GroupInner {
    name: String,
    points: Vec<PointInner>,
    crosses: Vec<CrossInner>,
}

/// A group of coverpoints and crosses. Cheap to clone (shared).
#[derive(Clone)]
pub struct Covergroup {
    inner: Rc<RefCell<GroupInner>>,
}

/// A coverpoint handle (`Copy`; borrow-free to sample).
#[derive(Clone)]
pub struct CoverPoint {
    group: Covergroup,
    idx: usize,
}

/// A cross handle.
#[derive(Clone)]
pub struct Cross {
    group: Covergroup,
    idx: usize,
}

thread_local! {
    static REGISTRY: RefCell<Vec<Covergroup>> = const { RefCell::new(Vec::new()) };
}

impl Covergroup {
    /// Create and register a covergroup. Names should be unique; two groups
    /// with the same name are merged in reports.
    pub fn new(name: &str) -> Covergroup {
        let cg = Covergroup {
            inner: Rc::new(RefCell::new(GroupInner { name: name.into(), points: Vec::new(), crosses: Vec::new() })),
        };
        REGISTRY.with(|r| r.borrow_mut().push(cg.clone()));
        cg
    }

    pub fn name(&self) -> String {
        self.inner.borrow().name.clone()
    }

    pub fn point(&self, name: &str, bins: Bins) -> CoverPoint {
        assert!(!bins.is_empty(), "coverpoint {name} has no bins");
        let mut g = self.inner.borrow_mut();
        let hits = vec![0; bins.len()];
        g.points.push(PointInner {
            name: name.into(),
            bins,
            hits,
            last_bin: None,
            gen: 0,
            out_of_range: 0,
            ignored: 0,
        });
        CoverPoint { group: self.clone(), idx: g.points.len() - 1 }
    }

    /// A cross of two or more points of this group. It counts a tuple each
    /// time every constituent point has been sampled since the last count.
    pub fn cross(&self, name: &str, points: &[&CoverPoint]) -> Cross {
        assert!(points.len() >= 2, "cross {name} needs at least two points");
        for p in points {
            assert!(Rc::ptr_eq(&p.group.inner, &self.inner), "cross {name}: point from another covergroup");
        }
        let mut g = self.inner.borrow_mut();
        g.crosses.push(CrossInner {
            name: name.into(),
            points: points.iter().map(|p| p.idx).collect(),
            seen: vec![0; points.len()],
            hits: BTreeMap::new(),
        });
        Cross { group: self.clone(), idx: g.crosses.len() - 1 }
    }

    /// Covered bins over total bins, points and crosses together.
    pub fn percent(&self) -> f64 {
        let (c, t) = self.counts();
        if t == 0 {
            100.0
        } else {
            100.0 * c as f64 / t as f64
        }
    }

    /// `(covered, total)` bins.
    pub fn counts(&self) -> (usize, usize) {
        let g = self.inner.borrow();
        let mut covered = 0;
        let mut total = 0;
        for p in &g.points {
            total += p.hits.len();
            covered += p.hits.iter().filter(|h| **h > 0).count();
        }
        for c in &g.crosses {
            let all: usize = c.points.iter().map(|i| g.points[*i].hits.len()).product();
            total += all;
            covered += c.hits.values().filter(|h| **h > 0).count();
        }
        (covered, total)
    }

    fn after_sample(g: &mut GroupInner, point: usize) {
        for c in g.crosses.iter_mut() {
            if !c.points.contains(&point) {
                continue;
            }
            let ready = c.points.iter().zip(&c.seen).all(|(p, seen)| g.points[*p].gen > *seen);
            if ready {
                let key: Option<Vec<usize>> = c.points.iter().map(|p| g.points[*p].last_bin).collect();
                if let Some(key) = key {
                    *c.hits.entry(key).or_insert(0) += 1;
                }
                for (i, p) in c.points.iter().enumerate() {
                    c.seen[i] = g.points[*p].gen;
                }
            }
        }
    }
}

impl CoverPoint {
    pub fn name(&self) -> String {
        self.group.inner.borrow().points[self.idx].name.clone()
    }

    /// Record a value. Returns `false` if it matched no bin (out of range).
    pub fn sample<T: Into<i128>>(&self, value: T) -> bool {
        let v = value.into();
        let mut g = self.group.inner.borrow_mut();
        let gname = g.name.clone();
        let p = &mut g.points[self.idx];
        if p.bins.illegal.iter().any(|(lo, hi)| *lo <= v && v <= *hi) {
            let msg = format!("coverpoint {gname}.{}: illegal value {v}", p.name);
            drop(g);
            log::error!("{msg}");
            crate::runtime::report_failure(msg);
            return false;
        }
        if p.bins.ignore.iter().any(|(lo, hi)| *lo <= v && v <= *hi) {
            p.ignored += 1;
            return true;
        }
        match p.bins.bins.iter().position(|b| b.contains(v)) {
            Some(i) => {
                p.hits[i] += 1;
                p.last_bin = Some(i);
                p.gen += 1;
                let idx = self.idx;
                Covergroup::after_sample(&mut g, idx);
                true
            }
            None => {
                p.out_of_range += 1;
                false
            }
        }
    }

    /// Hits per bin, in definition order.
    pub fn hits(&self) -> Vec<(String, u64)> {
        let g = self.group.inner.borrow();
        let p = &g.points[self.idx];
        p.bins.bins.iter().zip(&p.hits).map(|(b, h)| (b.name.clone(), *h)).collect()
    }

    pub fn percent(&self) -> f64 {
        let g = self.group.inner.borrow();
        let p = &g.points[self.idx];
        100.0 * p.hits.iter().filter(|h| **h > 0).count() as f64 / p.hits.len() as f64
    }
}

impl Cross {
    pub fn name(&self) -> String {
        self.group.inner.borrow().crosses[self.idx].name.clone()
    }

    /// Hits per bin tuple (bin names joined with " x ").
    pub fn hits(&self) -> Vec<(String, u64)> {
        let g = self.group.inner.borrow();
        let c = &g.crosses[self.idx];
        c.hits
            .iter()
            .map(|(k, h)| {
                let name: Vec<String> =
                    k.iter().zip(&c.points).map(|(b, p)| g.points[*p].bins.bins[*b].name.clone()).collect();
                (name.join(" x "), *h)
            })
            .collect()
    }

    pub fn percent(&self) -> f64 {
        let g = self.group.inner.borrow();
        let c = &g.crosses[self.idx];
        let all: usize = c.points.iter().map(|i| g.points[*i].hits.len()).product();
        100.0 * c.hits.values().filter(|h| **h > 0).count() as f64 / all as f64
    }
}

/// Every registered covergroup.
pub fn groups() -> Vec<Covergroup> {
    REGISTRY.with(|r| r.borrow().clone())
}

/// Forget every registered covergroup (tests).
pub fn clear() {
    REGISTRY.with(|r| r.borrow_mut().clear());
}

/// Overall coverage across all groups, in percent (100 if none).
pub fn percent() -> f64 {
    let (c, t) = groups().iter().map(|g| g.counts()).fold((0, 0), |(a, b), (c, d)| (a + c, b + d));
    if t == 0 {
        100.0
    } else {
        100.0 * c as f64 / t as f64
    }
}

fn json_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// The registry as JSON (the `coverage.json` format; see `rivet cov`).
pub fn to_json() -> String {
    let mut s = String::from("{\"groups\":[");
    for (gi, cg) in groups().iter().enumerate() {
        let g = cg.inner.borrow();
        if gi > 0 {
            s.push(',');
        }
        s.push_str("{\"name\":");
        json_str(&g.name, &mut s);
        s.push_str(",\"points\":[");
        for (pi, p) in g.points.iter().enumerate() {
            if pi > 0 {
                s.push(',');
            }
            s.push_str("{\"name\":");
            json_str(&p.name, &mut s);
            let _ = write!(s, ",\"out_of_range\":{},\"ignored\":{},\"bins\":[", p.out_of_range, p.ignored);
            for (bi, (b, h)) in p.bins.bins.iter().zip(&p.hits).enumerate() {
                if bi > 0 {
                    s.push(',');
                }
                s.push_str("{\"name\":");
                json_str(&b.name, &mut s);
                let _ = write!(s, ",\"hits\":{h}}}");
            }
            s.push_str("]}");
        }
        s.push_str("],\"crosses\":[");
        for (ci, c) in g.crosses.iter().enumerate() {
            if ci > 0 {
                s.push(',');
            }
            s.push_str("{\"name\":");
            json_str(&c.name, &mut s);
            s.push_str(",\"points\":[");
            for (i, p) in c.points.iter().enumerate() {
                if i > 0 {
                    s.push(',');
                }
                json_str(&g.points[*p].name, &mut s);
            }
            let total: usize = c.points.iter().map(|i| g.points[*i].hits.len()).product();
            let _ = write!(s, "],\"total\":{total},\"bins\":[");
            for (bi, (k, h)) in c.hits.iter().enumerate() {
                if bi > 0 {
                    s.push(',');
                }
                let name: Vec<String> =
                    k.iter().zip(&c.points).map(|(b, p)| g.points[*p].bins.bins[*b].name.clone()).collect();
                s.push_str("{\"name\":");
                json_str(&name.join(" x "), &mut s);
                let _ = write!(s, ",\"hits\":{h}}}");
            }
            s.push_str("]}");
        }
        s.push_str("]}");
    }
    s.push_str("]}");
    s
}

/// Write the registry as JSON.
pub fn write_json(path: &std::path::Path) -> std::io::Result<()> {
    std::fs::write(path, to_json())
}

/// A text table of every group, point, cross, and bin.
pub fn render_table() -> String {
    let mut s = String::new();
    for cg in groups() {
        let g = cg.inner.borrow();
        let (c, t) = cg.counts();
        let _ = writeln!(s, "covergroup {} : {c}/{t} bins ({:.1}%)", g.name, cg.percent());
        for p in &g.points {
            let hit = p.hits.iter().filter(|h| **h > 0).count();
            let _ = writeln!(s, "  point {:<24} {hit}/{}", p.name, p.hits.len());
            for (b, h) in p.bins.bins.iter().zip(&p.hits) {
                let _ = writeln!(s, "    {:<26} {h:>8}{}", b.name, if *h == 0 { "  <-- uncovered" } else { "" });
            }
            if p.out_of_range > 0 {
                let _ = writeln!(s, "    (out of range)             {:>8}", p.out_of_range);
            }
        }
        for cr in &g.crosses {
            let total: usize = cr.points.iter().map(|i| g.points[*i].hits.len()).product();
            let hit = cr.hits.values().filter(|h| **h > 0).count();
            let _ = writeln!(s, "  cross {:<24} {hit}/{total}", cr.name);
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bins_points_and_crosses() {
        clear();
        let cg = Covergroup::new("g");
        let len =
            cg.point("len", Bins::new().bin("one", 1..=1).bin("few", 2..5).values("big", [8u8, 16]).ignore(0..=0));
        let size = cg.point("size", Bins::new().auto("s", 0..2u8));
        let cross = cg.cross("len_x_size", &[&len, &size]);
        assert_eq!(len.hits().len(), 3);
        assert_eq!(size.hits().len(), 2);
        assert!(len.sample(1u8));
        assert!(len.sample(4u8));
        assert!(!len.sample(7u8), "7 is in no bin");
        assert!(len.sample(0u8), "ignored values are not out of range");
        assert_eq!(len.hits(), vec![("one".into(), 1), ("few".into(), 1), ("big".into(), 0)]);
        assert_eq!(cross.hits().len(), 0, "size never sampled");
        size.sample(1u8);
        assert_eq!(cross.hits(), vec![("few x s1".into(), 1)], "counted once both points sampled");
        size.sample(0u8);
        assert_eq!(cross.hits().len(), 1, "len not re-sampled since the last cross hit");
        len.sample(16u8);
        assert_eq!(cross.hits().len(), 2);
        assert_eq!(cg.counts(), (3 + 2 + 2, 3 + 2 + 6));
        assert!((cg.percent() - 100.0 * 7.0 / 11.0).abs() < 1e-9);
        assert!((len.percent() - 100.0).abs() < 1e-9);
        assert!((cross.percent() - 200.0 / 6.0).abs() < 1e-9);
        let json = to_json();
        assert!(
            json.contains(
                r#"{"name":"g","points":[{"name":"len","out_of_range":1,"ignored":1,"bins":[{"name":"one","hits":1}"#
            ),
            "{json}"
        );
        assert!(json.contains(r#""crosses":[{"name":"len_x_size","points":["len","size"],"total":6,"bins":[{"name":"few x s1","hits":1},{"name":"big x s0","hits":1}]}]"#), "{json}");
        let table = render_table();
        assert!(table.contains("covergroup g : 7/11 bins"));
        assert!(!table.contains("<-- uncovered"));
        assert_eq!(groups().len(), 1);
        assert!((percent() - cg.percent()).abs() < 1e-9);
        clear();
        assert_eq!(percent(), 100.0);
    }

    #[test]
    fn split_bins() {
        let b = Bins::new().split("q", 0..=99u32, 4);
        assert_eq!(b.bins.iter().map(|b| b.ranges[0]).collect::<Vec<_>>(), [(0, 24), (25, 49), (50, 74), (75, 99)]);
        let b = Bins::new().split("q", 0..3u8, 4);
        assert_eq!(b.len(), 3, "empty bins are dropped");
        let b = Bins::new().bin("neg", -4..=-1i8).bin("open", 10u8..);
        assert!(b.bins[0].contains(-2));
        assert!(b.bins[1].contains(1 << 40));
    }
}
