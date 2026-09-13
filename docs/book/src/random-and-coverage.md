# Random stimulus and coverage

Constrained-random stimulus and functional coverage are part of the core, not
a separate package. Both are deterministic: the same seed, sources and
simulator produce the same run.

## Seeds

Every run has a base seed. It comes from `RIVET_SEED` (which `rivet run
--seed` sets) or from the clock. `rivet run` prints it before anything else:

```text
rivet: seed 7318782485113623909 (replay with --seed 7318782485113623909)
```

Each test gets its own stream, derived from the base seed and the test's full
`module::name`. That matters: filtering, reordering or sharding a run does not
change the values a test sees, so a failure found in a 400-test regression
reproduces on its own. The per-test seed is logged when the test starts and
recorded in `results.xml` as the `random_seed` property.

```text
running example_bus::axil_regfile_random (2/3, seed 908804911688909664)
```

`rivet run -j N` and `[design.param_sets]` share one base seed across every
process of the run, so `--seed N` replays a sharded, multi-set run as a whole.

Seeds parse as decimal or `0x` hex.

## `rivet::rng()`

`rivet::rng()` returns a fresh stream forked from the test's master stream.
Each call returns a different, reproducible stream, so tasks started in the
same order see the same values run after run.

```rust
let mut rng = rivet::rng();
let addr = rng.gen_range(0..256u32);
```

The generator is xoshiro256** seeded through splitmix64: identical on every
platform.

| Method | Meaning |
|---|---|
| `Rng::seed_from_u64(seed)` | a stream fixed by an explicit seed |
| `rng.next_u64()`, `rng.next_u32()` | raw words |
| `rng.gen::<T>()` | a random value of a `Random` type |
| `rng.gen_range(0..16u64)`, `rng.gen_range(0..=255u64)` | half-open and inclusive ranges |
| `rng.gen_bool(p)` | `true` with probability `p` |
| `rng.below(n)` | uniform in `0..n` |
| `rng.choose(&items)` | a reference to a random element |
| `rng.choose_weighted(&[(item, weight), ...])` | weighted choice |
| `rng.shuffle(&mut items)` | in-place shuffle |
| `rng.fill_bytes(&mut buf)` | random bytes |
| `rng.logic_vec(width)` | a random four-state vector |
| `rng.logic_vec_with_x(width, p_x)` | the same with X bits at probability `p_x` |
| `rng.fork()` | an independent stream derived from this one |

## `#[derive(Randomize)]`

`Randomize` builds a random instance of a type. Every field is randomised
with its own rule; the whole value is then checked against the type's
constraints.

```rust
#[derive(Randomize, Debug)]
#[rand(constraint = |t: &Self| t.lo < t.hi)]
struct Txn {
    #[rand(range = 0..256u32)]           lo: u32,
    #[rand(range = 0..=255u32)]          hi: u32,
    #[rand(one_of = [1, 2, 4, 8])]       burst: u8,
    #[rand(weighted = [(0, 9), (1, 1)])] err: u8,
    #[rand(with = |r| r.gen_range(0..4) * 4)] aligned: u32,
    #[rand(skip)]                        note: String,   // Default
    data: [u8; 4],                                       // Random
    kind: Kind,                                          // Randomize
}

#[derive(Randomize)]
enum Kind { #[rand(weight = 3)] Read, Write { #[rand(range = 0..4)] strb: u8 }, #[rand(skip)] Never }
```

| Attribute | Where | Meaning |
|---|---|---|
| `#[rand(range = a..b)]` | field | uniform over a range, half-open or inclusive |
| `#[rand(one_of = [..])]` | field | one of the listed values |
| `#[rand(weighted = [(v, w), ..])]` | field | weighted choice among values |
| `#[rand(with = \|r\| ...)]` | field | a closure taking `&mut Rng` |
| `#[rand(skip)]` | field | not randomised; `Default::default()` |
| `#[rand(weight = n)]` | enum variant | relative weight when picking a variant |
| `#[rand(skip)]` | enum variant | never produced |
| `#[rand(constraint = \|t: &Self\| ...)]` | type | a predicate the whole value must satisfy |
| `#[rand(tries = n)]` | type | rejection-sampling attempts, default 1000 |

Constraints are satisfied by rejection sampling. Exceeding `tries` panics
with the type name, so an unsatisfiable constraint fails the test loudly
rather than quietly biasing the stimulus. A field with no attribute is
randomised through `Random` (primitives, arrays) or `Randomize` (nested
types); `Option<T>` is randomly `None` or `Some`.

From `examples/bus`:

```rust
#[derive(Randomize, Debug, Clone)]
#[rand(constraint = |t: &Self| t.addr.is_multiple_of(4))]
struct AxilTxn {
    #[rand(with = |r: &mut Rng| r.gen_range(0..16u64) * 4)]
    addr: u64,
    data: u32,
    #[rand(weighted = [(0xfu64, 6), (0x3, 1), (0xc, 1), (0x1, 1), (0x0, 1)])]
    strb: u64,
    #[rand(weighted = [(true, 1), (false, 1)])]
    write: bool,
}

let t = AxilTxn::randomize(&mut rng);
```

## Functional coverage

A `Covergroup` holds coverpoints and crosses. A coverpoint is a set of named
bins; sampling a value increments the bin it falls in. Coverage is the
fraction of bins that have been hit at least once, counting points and
crosses together.

```rust
let cg = Covergroup::new("axil");
let addr_pt = cg.point("addr", Bins::new().auto("reg", 0..8u64).bin("out_of_range", 8..=15u64));
let strb_pt = cg.point(
    "strb",
    Bins::new()
        .values("full", [0xfu64])
        .values("half", [0x3u64, 0xc])
        .values("byte", [0x1u64])
        .values("none", [0x0u64]),
);
let kind_pt = cg.point("kind", Bins::new().values("read", [0u64]).values("write", [1u64]));
let _cross = cg.cross("kind_x_addr", &[&kind_pt, &addr_pt]);

kind_pt.sample(t.write as u64);
addr_pt.sample(t.addr / 4);

ensure!(cg.percent() > 95.0, "coverage {:.1}% too low for 400 transactions", cg.percent());
```

### Bins

| Builder | Meaning |
|---|---|
| `Bins::new()` | an empty specification |
| `.bin("short", 1..=4)` | one bin covering a range |
| `.values("pow2", [1, 2, 4, 8])` | one bin covering the listed values |
| `.auto("reg", 0..8)` | one bin per value, named `reg0`, `reg1`, and so on |
| `.split("len", 0..1024, 8)` | the range split into 8 equal bins named `len0`..`len7` |
| `.ignore(range)` | values that are neither counted nor reported as out of range |
| `.illegal(range)` | values that fail the running test when sampled |

`point.sample(v)` returns `false` when the value matched no bin. Those are
counted as out of range and shown in the report. An illegal value logs an
error and fails the test at once.

### Crosses

`cg.cross("kind_x_addr", &[&kind_pt, &addr_pt])` counts a tuple each time
every constituent point has been sampled since the last count. Its bins are
named by joining the constituent bin names with ` x `.

### Reporting

| Call | Returns |
|---|---|
| `cg.percent()`, `cg.counts()` | the group's coverage, and `(covered, total)` bins |
| `point.percent()`, `point.hits()` | per-point coverage and hits per bin |
| `cross.percent()`, `cross.hits()` | the same for a cross |
| `rivet::coverage::percent()` | coverage across every registered group |
| `rivet::coverage::render_table()` | the text table below |
| `rivet::coverage::to_json()` | the `coverage.json` format |

At the end of a run the harness prints the table and writes
`sim_build/<sim>/coverage.json`. Nothing is written or reported when the
crate has no covergroups.

```text
covergroup axil : 33/33 bins (100.0%)
  point addr                     9/9
    reg0                             34
    reg1                             22
    reg2                             20
    reg3                             21
    reg4                             21
    reg5                             28
    reg6                             18
    reg7                             21
    out_of_range                    215
  point strb                     4/4
    full                            105
    half                             40
    byte                             16
    none                             17
  point kind                     2/2
    read                            222
    write                           178
  cross kind_x_addr              18/18

RIVET_COVERAGE percent=100.00
```

Uncovered bins are marked `<-- uncovered`, and a point with values outside
every bin gets an `(out of range)` line.

## Merging and thresholds

Coverage from shards and parameter sets is merged into one
`sim_build/<sim>/coverage.json` at the end of `rivet run`, and the merged
percentage is printed:

```text
rivet: functional coverage 100.00% (/path/examples/bus/sim_build/icarus/coverage.json)
```

Fail a run that does not reach a target with `--cov-threshold`:

```sh
rivet run --sim icarus -C examples/bus -j 4 --cov-threshold 95
```

Below the threshold the run exits 1 with:

```text
rivet: coverage 93.10% is below the threshold of 95%
```

A threshold with no coverage recorded at all also fails.

`rivet cov report` merges and prints files without rerunning anything:

```sh
rivet cov report -C examples/bus --threshold 100          # this crate's coverage.json
rivet cov report a.json b.json -o merged.json             # explicit files, write the merge
```
