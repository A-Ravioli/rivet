//! `#[derive(Randomize)]` and per-test seeding through the regression runner.

use rivet_core::random::{self, Rng};
use rivet_core::test::{boxed, TestDesc};
use rivet_core::{inventory, Module, Randomize};
use rivet_macros::Randomize;
use rivet_mock::Design;
use std::cell::RefCell;

#[derive(Randomize, Debug, Clone, PartialEq)]
#[rand(crate = rivet_core)]
#[rand(constraint = |t: &Self| t.lo < t.hi)]
#[rand(constraint = |t: &Self| t.burst != 8 || t.hi > 100)]
struct Txn {
    #[rand(range = 0..200u32)]
    lo: u32,
    #[rand(range = 0..=255u32)]
    hi: u32,
    #[rand(one_of = [1u8, 2, 4, 8])]
    burst: u8,
    #[rand(weighted = [(0u8, 9), (1, 1)])]
    err: u8,
    #[rand(with = |r: &mut Rng| r.gen_range(0..4u32) * 4)]
    aligned: u32,
    #[rand(skip)]
    note: String,
    data: [u8; 4],
    kind: Kind,
    flag: bool,
}

#[derive(Randomize, Debug, Clone, PartialEq)]
#[rand(crate = rivet_core)]
#[allow(dead_code)]
enum Kind {
    #[rand(weight = 3)]
    Read,
    Write {
        #[rand(range = 0..4u8)]
        strb: u8,
    },
    Pair(#[rand(range = 1..=2u8)] u8, bool),
    #[rand(skip)]
    Never,
}

#[derive(Randomize, Debug)]
#[rand(crate = rivet_core, tries = 5)]
#[rand(constraint = |_t: &Self| false)]
struct Impossible {
    #[rand(range = 0..2u8)]
    _x: u8,
}

#[derive(Randomize, Debug, PartialEq)]
#[rand(crate = rivet_core)]
struct Unit;

#[derive(Randomize, Debug)]
#[rand(crate = rivet_core)]
struct Generic<T: rivet_core::Randomize> {
    inner: T,
    #[rand(range = 10..20i64)]
    n: i64,
}

#[test]
fn derive_respects_attributes_and_constraints() {
    let mut rng = Rng::seed_from_u64(1);
    let mut writes = 0;
    let mut reads = 0;
    let mut errs = 0;
    let mut aligned_ok = true;
    for _ in 0..2000 {
        let t = Txn::randomize(&mut rng);
        assert!(t.lo < t.hi, "{t:?}");
        assert!(t.burst != 8 || t.hi > 100);
        assert!([1, 2, 4, 8].contains(&t.burst));
        assert!(t.note.is_empty());
        aligned_ok &= t.aligned % 4 == 0 && t.aligned < 16;
        errs += t.err as u32;
        match t.kind {
            Kind::Read => reads += 1,
            Kind::Write { strb } => {
                assert!(strb < 4);
                writes += 1;
            }
            Kind::Pair(a, _) => assert!((1..=2).contains(&a)),
            Kind::Never => panic!("skipped variant produced"),
        }
    }
    assert!(aligned_ok);
    assert!((100..300).contains(&errs), "weighted err count {errs}");
    // Read has weight 3 of 5.
    assert!(reads > writes * 2, "reads {reads} writes {writes}");
    assert_eq!(Unit::randomize(&mut rng), Unit);
    let g: Generic<Txn> = Randomize::randomize(&mut rng);
    assert!((10..20).contains(&g.n));
    assert!(g.inner.lo < g.inner.hi);
}

#[test]
fn derive_is_reproducible() {
    let a: Vec<Txn> = {
        let mut r = Rng::seed_from_u64(77);
        (0..10).map(|_| Txn::randomize(&mut r)).collect()
    };
    let b: Vec<Txn> = {
        let mut r = Rng::seed_from_u64(77);
        (0..10).map(|_| Txn::randomize(&mut r)).collect()
    };
    assert_eq!(a, b);
}

#[test]
#[should_panic(expected = "Randomize for Impossible: no value satisfied the constraints after 5 tries")]
fn unsatisfiable_constraint_panics() {
    let mut rng = Rng::seed_from_u64(1);
    let _ = Impossible::randomize(&mut rng);
}

// ---------------------------------------------------------------------------
// Seeds through the regression runner

thread_local! {
    static SEEN: RefCell<Vec<(String, u64, u64)>> = const { RefCell::new(Vec::new()) };
}

fn record(name: &str) {
    let v = random::rng().next_u64();
    SEEN.with(|s| s.borrow_mut().push((name.to_string(), random::test_seed(), v)));
}

inventory::submit! {
    TestDesc { name: "seed_a", module: "rand", run: |_dut: Module| boxed(async { record("seed_a"); Ok(()) }), timeout: || None, skip: false, expect_fail: false, stage: 0, wall_timeout: None, param_sets: &[] }
}
inventory::submit! {
    TestDesc { name: "seed_b", module: "rand", run: |_dut: Module| boxed(async { record("seed_b"); Ok(()) }), timeout: || None, skip: false, expect_fail: false, stage: 0, wall_timeout: None, param_sets: &[] }
}

fn design() -> Design {
    let mut d = Design::new("top").precision(-9);
    let x = d.logic("x", 4);
    d.init(x, rivet_core::LogicVec::from_u64(4, 0));
    d
}

#[test]
fn per_test_seeds_survive_filtering_and_land_in_results() {
    random::set_base_seed(0xDEAD_BEEF);
    let all = rivet_mock::run_regression(design(), rivet_core::test::all_tests(), Some("seed_a,seed_b"));
    let first: Vec<(String, u64, u64)> = SEEN.with(|s| std::mem::take(&mut *s.borrow_mut()));
    assert_eq!(first.len(), 2);
    assert_ne!(first[0].1, first[1].1, "different tests get different seeds");
    assert_ne!(first[0].2, first[1].2);
    for r in &all {
        let seen = first.iter().find(|(n, _, _)| *n == r.name).unwrap();
        assert_eq!(r.seed, seen.1, "results carry the test seed");
    }
    // Run only seed_b: it must see exactly the same stream.
    let _ = rivet_mock::run_regression(design(), rivet_core::test::all_tests(), Some("seed_b"));
    let second: Vec<(String, u64, u64)> = SEEN.with(|s| std::mem::take(&mut *s.borrow_mut()));
    assert_eq!(second.len(), 1);
    assert_eq!(second[0], first[1]);
    // A different base seed changes everything.
    random::set_base_seed(1);
    let _ = rivet_mock::run_regression(design(), rivet_core::test::all_tests(), Some("seed_b"));
    let third: Vec<(String, u64, u64)> = SEEN.with(|s| std::mem::take(&mut *s.borrow_mut()));
    assert_ne!(third[0].1, second[0].1);
    // results.xml records the seed.
    let dir = std::env::temp_dir().join(format!("rivet-seed-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("results.xml");
    rivet_core::test::write_results_xml_with(&p, &all, "mock", -9).unwrap();
    let xml = std::fs::read_to_string(&p).unwrap();
    assert!(xml.contains(&format!(r#"<property name="random_seed" value="{}" />"#, first[0].1)));
    std::fs::remove_dir_all(&dir).unwrap();
}
