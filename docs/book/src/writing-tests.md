# Writing tests

A test is an `async fn` marked with `#[rivet::test]`. It takes the design as
its first argument and returns `rivet::Result<()>`.

```rust
use rivet::prelude::*;

#[rivet::test(timeout = 10.us())]
async fn counter_counts(dut: Module) -> rivet::Result<()> {
    let clk = dut.signal("clk")?;
    let _clock = Clock::start(clk, 10.ns());
    dut.signal("rst_n")?.set(0);
    clk.rising_edge().await;
    dut.signal("rst_n")?.set(1);
    for _ in 0..4 {
        clk.rising_edge().await;
    }
    read_only().await;
    assert_eq!(dut.signal("count")?.get_u64()?, 4);
    Ok(())
}
```

The attribute registers the function in a link-time collection. Tests are
sorted by stage, then module, then name, so runs on different simulators
agree on the order. Inside one simulator process they run sequentially, one
precision step apart, and simulation time does not rewind between them.

A test fails when it returns `Err`, when it panics, when any task it spawned
panics, or when it exceeds a timeout.

## Attributes

| Attribute | Form | Meaning |
|---|---|---|
| `timeout` | `timeout = 10.us()` | simulated-time limit; a `Duration` expression, evaluated when the test starts |
| `wall_timeout` | `wall_timeout = 60` | wall-clock limit in seconds; overrides `RIVET_WALL_TIMEOUT` for this test |
| `skip` | `skip` or `skip = true` | register the test but do not run it; reported as `SKIP` |
| `expect_fail` | `expect_fail` or `expect_fail = true` | a failure is a pass, and a pass is a failure |
| `expect_fail` with a message | `expect_fail = "overflowed"` | the same, but the failure message must contain this text |
| `expect_timeout` | `expect_timeout` or `expect_timeout = true` | the test is expected to run out of simulated time |
| `stage` | `stage = -1` | tests are stable-sorted by stage; the default is 0 |
| `params` | `params = [1u32, 4, 16]` | one test per value, registered as `name[value]`, passed as the second argument |
| `param_sets` | `param_sets = ["init5"]` | run only under these `rivet.toml` parameter sets; empty means all |

Examples from `examples/dff` and `examples/bus`:

```rust
#[rivet::test(stage = -1)]
async fn x_propagates_before_reset(dut: Module) -> rivet::Result<()> { /* ... */ }

#[rivet::test(expect_fail)]
async fn deliberate_failure(dut: Module) -> rivet::Result<()> {
    bail!("this test fails on purpose")
}

#[rivet::test(timeout = 50.ns(), expect_fail)]
async fn times_out(dut: Module) -> rivet::Result<()> { /* never returns */ }

#[rivet::test(params = [1u32, 4, 16], timeout = 500.us())]
async fn axi_mem_bursts(dut: Module, beats: u32) -> rivet::Result<()> { /* ... */ }

#[rivet::test(param_sets = ["init5"])]
async fn regs_reset_to_param(dut: Module) -> rivet::Result<()> {
    assert_eq!(rivet::test::param_set().as_deref(), Some("init5"));
    /* ... */
}
```

A test with `params` must take two arguments. Its registered name carries the
literal as written, so `params = [1u32, 4, 16]` registers
`axi_mem_bursts[1u32]`, `axi_mem_bursts[4]` and `axi_mem_bursts[16]`.

`params` and `param_sets` differ in cost. `params` values are passed
in-process: one build, several tests. `param_sets` rebuilds the design for
each set, because the parameters go to the HDL compiler.

A bare `expect_fail` passes as long as the test fails somehow, which is weak:
a test that was meant to catch an overflow can pass because a signal name
was misspelled. Naming the message pins it down, and `expect_timeout`
expects the timeout specifically.

```rust
#[rivet::test(expect_fail = "counter overflowed")]
async fn overflow_is_caught(dut: Module) -> rivet::Result<()> { /* ... */ }

#[rivet::test(timeout = 20.ns(), expect_timeout)]
async fn never_finishes(dut: Module) -> rivet::Result<()> { /* ... */ }
```

A test with `expect_fail = "..."` that fails for a different reason is
reported as a failure whose message says
`failed as expected, but the message does not contain ...`. `expect_timeout`
matches the runner's own `timed out after ...` message.

## The `dut` argument

The first argument is `Module` for dynamic access, or any type implementing
`rivet::Bind`, which is what `rivet bindgen` generates:

```rust
mod dut;
use dut::Dut;

#[rivet::test]
async fn typed_dut(dut: Dut) -> rivet::Result<()> {
    let _clock = Clock::start(dut.clk, 10.ns());
    dut.d.set(0x5a);
    dut.clk.rising_edge().await;
    read_only().await;
    assert_eq!(dut.q.get_u64()?, 0x5a);
    Ok(())
}
```

With generated bindings a misspelled signal is a compile error and a width
change is caught when the test binds, rather than after a full simulation
run.

## Fixtures

Setup that several tests share can be written once as a fixture: an `async
fn` marked `#[rivet::fixture]` that takes the dut and returns a value. A
test asks for it by naming an argument after the dut, and the macro calls
the fixture of that name. Teardown is the value's `Drop`, which runs when
the test ends, however it ends.

From `examples/dff/src/lib.rs`:

```rust
/// A fixture: the clock started and reset released, shared by the tests
/// below. Teardown is `Drop` on what it returns.
#[rivet::fixture]
async fn ready(dut: Module) -> rivet::Result<Signal> {
    let clk = dut.signal("clk")?;
    Clock::start(clk, 10.ns());
    reset(&dut, clk).await?;
    Ok(clk)
}

#[rivet::test(timeout = 100.us())]
async fn fixture_gives_a_running_clock(dut: Module, ready: Signal) -> rivet::Result<()> {
    // `ready` is the clock, already running, reset already released.
    let d = dut.signal("d")?;
    let q = dut.signal("q")?;
    d.set(0xa5);
    ready.rising_edge().await;
    read_only().await;
    assert_eq!(q.get_u64()?, 0xa5);
    Ok(())
}
```

The rules:

| Rule | Detail |
|---|---|
| The function must be `async` and take exactly one argument, the dut | anything else is a compile error |
| It takes the same dut type the test takes | a test on a generated `Dut` needs a fixture on `Dut`; both are `Clone`, and the fixture gets a clone |
| The argument name in the test is the fixture name | there is no separate registry and no string to keep in step |
| Fixture arguments come after the dut, and after the `params` value | `async fn t(dut: Module, beats: u32, ready: Signal)` |
| A fixture may return `rivet::Result<T>` for any `T` | so it can use `?` internally; returning a plain value works for `Module`, `Signal` and `()` |
| Setup runs once per test that asks for it | it is not shared between tests, and it runs inside the test's own simulated time |
| Teardown is `Drop` | return a value with a `Drop` impl to stop a task, close a file or check an invariant at the end |

A test can take several fixtures. Each is resolved by name in argument
order.

## Hierarchy

| Call | Returns |
|---|---|
| `dut.signal("clk")?` | a `Signal` child by name |
| `dut.module("u_core")?` | a child module by name |
| `dut.child("x")?` | a child as an untyped `Object` |
| `dut.has_child("x")` | `bool`, no error |
| `dut.path_signal("u_core.alu.result")?` | a dotted path, with `[i]` indices |
| `dut.path_module("u_core.alu")?` | the same for modules |
| `dut.index(2)?` | an element of a generate array |
| `dut.children()?` | every immediate child |
| `dut.path()`, `dut.name()` | the full path and the leaf name |

```rust
assert_eq!(dut.path_signal("gen[1].tap")?.get_u64()?, 0x41);
let gen = dut.module("gen")?;
let tap = gen.index(1)?.as_module()?.signal("tap")?;
```

## Values

| Read | Meaning |
|---|---|
| `sig.get()` | the four-state value as a `LogicVec` |
| `sig.read_into(&mut buf)` | the same without allocating |
| `sig.get_u64()?` | unsigned integer; `Err(Unresolved)` if any bit is X or Z |
| `sig.get_i64()?` | signed integer, same error |
| `sig.get_u64_lossy()` | X and Z bits resolve to 0 |
| `sig.is_high()?` | `true` if non-zero, error on X or Z |
| `sig.get_real()`, `sig.get_string()` | real and string objects |
| `sig.width()`, `sig.is_const()`, `sig.kind()`, `sig.info()` | metadata |

| Write | Meaning |
|---|---|
| `sig.set(v)` | inertial deposit, cocotb's default; not readable back in the same phase |
| `sig.set_now(v)` | immediate deposit (`vpiNoDelay`) |
| `sig.force(v)` / `sig.release()` | force a value over the design's drivers, and release it |
| `sig.set_real(x)`, `sig.set_int(i)`, `sig.set_string(s)` | scalar objects |

Writes accept anything implementing `IntoLogicVec`, including integers and
`LogicVec`. Reading or writing sub-objects:

```rust
mem.index(3)?.set(0x10);          // element of an unpacked array
cmd.member("op")?.get_u64()?;     // member of a struct-valued object
```

### Bit ranges

`sig.slice(hi, lo)` is a view of a bit range, inclusive at both ends. Reads
and writes go through the whole vector, so it works on every backend,
including those whose procedural interface has no part-select.

```rust
// Reading a range.
assert_eq!(x.slice(15, 12).get_u64()?, 0xa);
assert_eq!(x.slice(7, 0).get_u64()?, 0xcd);
assert_eq!(x.slice(3, 3).get_u64()?, 1);
assert_eq!(x.slice(7, 4).path(), "top.x[7:4]");

// Writing a range leaves the rest alone, and two writes in one
// phase compose instead of the second dropping the first.
x.slice(7, 0).set(0x12);
x.slice(15, 8).set(0x34);
Timer::steps(1).await;
assert_eq!(x.get_u64()?, 0x3412);
```

A slice write starts from a write already buffered in the same time step, so
a whole-signal write followed by a slice write keeps both: `x.set(0xffff)`
then `x.slice(11, 8).set(0x0)` leaves `0xf0ff`. A slice that is inverted or
runs off the end of the signal is an assertion failure, not a silent
truncation.

| Call | Meaning |
|---|---|
| `sig.slice(hi, lo)` | the view |
| `slice.get()`, `slice.get_u64()?`, `slice.get_u64_lossy()` | read the range |
| `slice.set(v)` | write the range, leaving the other bits alone |
| `slice.width()`, `slice.path()` | metadata |

### Enumeration literals

Where the simulator reports an enumeration's literal names, a test can read
what a value means rather than which position it holds. VHDL tools do;
Verilog simulators do not.

```rust
let Some(lits) = op.enum_literals() else {
    return Err(rivet::skip("this simulator does not report enumeration literals"));
};
assert_eq!(lits, ["OP_NOP", "OP_ADD", "OP_SUB", "OP_XOR"], "op_t literals in order");
op.set(2u64);
clk.rising_edge().await;
read_only().await;
assert_eq!(op.enum_name().as_deref(), Some("OP_SUB"));
```

`enum_literals()` gives the names in position order and `enum_name()` the
name of the current value; both return `None` where the simulator does not
report them. Writes are still by position. See [VHDL](vhdl.md).

Writing a constant fails the test rather than being dropped silently. Writing
in the ReadOnly phase panics; see [The timing model](timing-model.md).

## Tasks

| Call | Meaning |
|---|---|
| `spawn(fut)` | start a concurrent task, returns a `JoinHandle` |
| `spawn_named("monitor", fut)` | the same with a name that appears in task dumps |
| `handle.await` | wait for the task; `Err(Cancelled)` if it was cancelled |
| `handle.cancel()` | drop the task's future, which deregisters its triggers |
| `handle.is_done()`, `handle.name()` | status |
| `Scope::new()` and `scope.spawn(fut)` | structured concurrency: children are cancelled when the scope is dropped |

Dropping a `JoinHandle` detaches the task, it does not stop it. Cancellation
is dropping the future: destructors run, and the triggers the task was
waiting on are deregistered. At the end of every test the runner cancels
everything the test started and discards buffered writes.

```rust
let monitor = spawn(async move {
    loop {
        clk.rising_edge().await;
        read_only().await;
        qv.put(q.get_u64().unwrap()).await;
    }
});
// ... drive stimulus ...
monitor.cancel();
```

## Combinators

| Call | Meaning |
|---|---|
| `first(a, b).await` | whichever finishes first, as `Either::Left` or `Either::Right`; the other is dropped |
| `join(a, b).await` | both, as a tuple |
| `with_timeout(fut, 10.us()).await?` | `Err(Error::Timeout)` if the future did not finish in time |
| `yield_now().await` | let other ready tasks run without advancing time |

```rust
let got = with_timeout(received.get(), 10.us()).await?;
```

## Event, Queue and Lock

| Type | Constructor | Operations |
|---|---|---|
| `Event` | `Event::new()` | `set()`, `clear()`, `is_set()`, `wait().await` |
| `Queue<T>` | `Queue::new()`, `Queue::bounded(n)` | `put(v).await`, `get().await`, `try_put(v)`, `try_get()`, `len()`, `is_empty()`, `is_full()` |
| `Lock` | `Lock::new()` | `acquire().await` returning a guard, `is_locked()` |

`Event::set` wakes every waiter and the flag stays set until `clear()`.
`Queue` is a FIFO channel between tasks; `Queue::new()` is unbounded and
`bounded(n)` blocks `put` while full. `Lock` is fair: waiters are woken in
order. All three are cheap to clone and share between tasks.

```rust
let to_send: Queue<u64> = Queue::new();
let received: Queue<LogicVec> = Queue::new();
let driver = drive_from_queue("source", source, to_send.clone());
let monitor = monitor_to_queue("sink", sink, received.clone());
```

## Clock

`Clock::start(signal, period)` starts a 50% duty-cycle clock whose first edge
is rising, as a named task. Dropping the `Clock` does not stop it; call
`stop()`.

| Builder method | Meaning |
|---|---|
| `Clock::builder(sig, 10.ns())` | start configuring |
| `.high_time(4.ns())` | high time within the period; the default is half |
| `.start_high(false)` | first edge falling |
| `.immediate()` | drive with `NoDelay` writes instead of deposits |
| `.phase(3.ns())` | delay before the first edge |
| `.jitter(200.ps())` | uniform jitter in `[-max, +max]` per edge, drawn from the test's seeded stream; the average period is preserved |
| `.start()` | start it, returning a `Clock` |

| `Clock` method | Meaning |
|---|---|
| `clock.cycles(n).await` | wait for `n` rising edges |
| `clock.stop()` | cancel the clock task |
| `clock.signal()`, `clock.period_steps()` | what it drives, and the period in precision steps |

The period must be at least two precision steps, the high time must be inside
the period, and jitter must be smaller than each half period. Each of those
is an assertion, so a bad clock fails immediately.

```rust
let _clock = Clock::start(clk, 10.ns());
let skewed = Clock::builder(clk2, 10.ns()).phase(3.ns()).jitter(200.ps()).start();
```

## Errors

`rivet::Result<T>` is the return type. Two macros build failures with the
message you want:

```rust
ensure!(width.is_const(), "WIDTH should be a parameter");
bail!("this test fails on purpose");
```

Plain `assert_eq!` and `assert!` work too: a panic in a test or in any task
it spawned is caught and recorded as a failure of the running test.

## Ending a test early

Two calls end a running test before its body returns.

| Call | Outcome | Use it for |
|---|---|---|
| `rivet::skip(reason)` | `SKIP` | something the simulator or design in front of the test cannot do |
| `rivet::runtime::finish_test()` | `PASS` | the test is done and the main task would otherwise keep waiting |

These are run-time decisions, unlike the `skip` attribute in the table
above, which keeps a test from starting at all.

`skip` returns an `Error`, so a test ends with it through `?` or `return`:

```rust
/// GHDL's VPI does not expose VHDL record members, so tests that need them
/// skip themselves there rather than failing.
fn need_records(dut: &Module) -> rivet::Result<Module> {
    dut.module("cmd").map_err(|_| rivet::skip("this simulator does not expose VHDL record members"))
}
```

A skipped test is reported as `SKIP`, counted separately in the summary, and
written to `results.xml` with a `<skipped />` element. It does not fail the
run. This is how one source file runs on simulators of differing capability:
`examples/vhdl_types` passes 5 of 6 tests on NVC and 2 of 6 on GHDL, with
the rest skipped rather than failed. Prefer probing the capability, as
above, over checking the simulator's name.

`finish_test()` is cocotb's `TestSuccess`. It ends the test as passed from
any task, so a monitor that has seen everything it needed can stop a test
that would otherwise run to its timeout:

```rust
// A monitor decides the test is done; the main task would otherwise
// wait for a lot longer.
let _m = spawn(async {
    Timer::steps(2).await;
    rivet::runtime::finish_test();
});
Timer::new(1.ms()).await;
Ok(())
```

That test ends two steps in, not at its 500 us timeout. Calling it outside a
test logs an error and does nothing.

## Selecting and ordering tests

| Selection | Where |
|---|---|
| `--filter a,b` (`RIVET_TEST_FILTER`) | `rivet run` |
| positional filters, `--exact`, `--skip` | `cargo test` |
| `--shuffle` (`RIVET_SHUFFLE`) | `rivet run` |

Each `--filter` pattern is tried first as a regular expression, searched
(not anchored) against `module::name` and against the bare test name, which
is what cocotb's `COCOTB_TEST_FILTER` does. A pattern that does not compile,
or that compiles and matches nothing, is then tried as a literal substring.
That fallback is what makes a parametrised name select itself:
`axi_mem_bursts[16]` is a character class to a regular expression engine and
a test name to the person typing it.

```sh
rivet run --filter 'counter|fifo'          # either
rivet run --filter '^example_dff::dff_'    # anchored at the module
rivet run --filter 'axi_mem_bursts[16]'    # literal fallback: one parametrised test
```

`--shuffle` reorders tests within each stage, seeded from the run's base
seed. Stages still run in order, so a stage -1 setup test stays first, and a
shuffled run replays exactly with the `--seed` the run printed. Use it to
find tests that depend on what ran before them.

Every result carries the test's source location, taken from `file!()` and
`line!()` at the registration, and `results.xml` records it as `file` and
`lineno` on each `<testcase>`. CI annotations and IDEs use that to put a
failure on the right line.

## Running through `cargo test`

With the `harness` feature and a `harness = false` test target, `cargo test`
runs the same tests:

```toml
[[test]]
name = "sim"
harness = false
```

```rust
use example_dff as _;

fn main() -> std::process::ExitCode { rivet::harness::main() }
```

```sh
cargo test -p example-dff -- --list        # no simulator needed
cargo test -p example-dff                  # runs on Icarus
RIVET_SIM=verilator cargo test -p example-dff -- counter
cargo test -p example-bus -- --test-threads 2
```

The harness understands `--list`, `--exact`, `--skip <pattern>`,
`--test-threads N` (simulator processes), `--format json`, `--format terse`
and positional filters, so IDE test runners work unchanged.

## Running through `cargo nextest`

`cargo nextest` runs each test in its own process. It works on a Rivet test
crate with no changes to the crate:

```sh
cargo nextest list -p example-dff
cargo nextest run -p example-dff
cargo nextest run -p example-dff --profile ci
```

What makes that work, and what it means for a run:

| Behaviour | Detail |
|---|---|
| Listing | `--list --format terse` prints only `name: test` lines, which is what nextest parses; the count line belongs to the pretty format |
| Ignored tests | `--ignored` lists nothing. Rivet has no ignored tests: `skip` is the harness's decision at run time and is reported as a skip |
| One test per process | each process gets its own results directory under `sim_build/<sim>/one/<module>__<test>/` |
| The design build | a lock file serialises it, so parallel processes neither race on the build nor overwrite each other's results |

One simulator process per test is heavier than one process per suite, so the
repository caps the parallelism in `.config/nextest.toml`:

```toml
[test-groups]
# Every process in this group builds or runs a simulator.
simulator = { max-threads = 4 }

[[profile.default.overrides]]
filter = 'binary(sim) or binary(cli)'
test-group = 'simulator'
```

with a 60 second slow timeout and no retries, on the grounds that simulator
flakiness should be diagnosed rather than retried away. Copy that file into
a testbench repository and adjust `max-threads` to the machine.
