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
`--test-threads N` (simulator processes), `--format json` and positional
filters, so IDE test runners and `cargo nextest` work unchanged.
