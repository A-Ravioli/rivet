# Rivet architecture: a first-principles design

This document reasons from first principles about what a hardware verification
harness *is*, uses the analysis in [`00-cocotb-analysis.md`](00-cocotb-analysis.md)
to identify what cocotb got right and where its costs come from, and then
proposes the architecture of Rivet. The roadmap is in
[`02-roadmap.md`](02-roadmap.md).

## 1. What a harness fundamentally is

Strip away Python, VPI, and Makefiles and a cocotb-style harness is four things:

1. **A co-simulation protocol.** Two engines share one timeline: the HDL
   simulator, which owns time and signal state, and the testbench, which owns
   stimulus and checking. Control passes back and forth at well-defined
   *synchronization points* (a signal changed, time advanced, the timestep is
   about to end). At each point one side can read and write shared state.
2. **A concurrency model for the testbench.** Many independent behaviours
   (drivers, monitors, scoreboards, a clock) run "at the same time" against a
   single simulated timeline. They must be deterministic given a seed, cheap to
   create, and able to block on protocol synchronization points.
3. **A data model for HDL values and hierarchy.** Signals have 4-state packed
   vectors, arrays, records, enums, reals, strings; they live in a named tree.
   The testbench must read and write them without corrupting semantics.
4. **A build/run/report layer.** Compile the design for a given simulator, load
   the harness into it, run a selection of tests with parameters and seeds,
   emit machine-readable results.

Everything else (logging, waveforms, timeouts, structured concurrency, bus
functional models) is a library on top of those four.

The critical performance and correctness questions are all about (1) and (2):

- **How expensive is one synchronization?** Every `await RisingEdge(clk)` is one
  round trip through the protocol. A testbench doing anything per cycle pays
  this cost N times. In cocotb it is on the order of tens of microseconds and
  dominated by Python (see analysis §4). In an in-process Rust executor it can
  be hundreds of nanoseconds, which means the simulator's own evaluation becomes
  the bottleneck again, which is where it should be.
- **How expensive is one value read or write?** cocotb goes through a binary
  string on every access. VPI, VHPI, and FLI all have native packed-vector
  formats (`vpiVectorVal`, `vhpiLogicVecVal`, `mti_GetArrayValue`), so the
  string round trip is a design choice, not a constraint.
- **Where in the timestep are you when you resume?** cocotb's five-phase timing
  model (beginning of timestep, evaluation, values-change, values-settle,
  end-of-timestep) is a well-thought-out common subset of the three PLI
  standards. Rivet keeps it verbatim. Getting this wrong is what makes
  testbenches race with the RTL.

## 2. Design principles

1. **In-process, zero-copy, no interpreter.** The testbench is native code
   linked into the simulator process. There is no interpreter to embed, no GIL,
   no `PYGPI_PYTHON_BIN` dance, no string marshalling.
2. **Keep cocotb's timing model and vocabulary.** `RisingEdge`, `FallingEdge`,
   `ValueChange`, `Timer`, `ReadWrite`, `ReadOnly`, `NextTimeStep`, `Deposit`,
   `Force`, `Release`, `start_soon`, `Event`, `Queue`, `Lock`. These names are
   the shared vocabulary of the field and the semantics are correct. Migration
   cost should be "translate syntax", not "relearn the model".
3. **Port the quirk knowledge, not the code.** cocotb's GPI layer encodes a
   decade of simulator workarounds (callback re-entrancy, untrusted inertial
   writes, Xcelium startup, Verilator's recurring callbacks, VHPI name
   canonicalization). Every one of them is catalogued in the analysis and must
   be reproduced deliberately in Rivet's backends, with a test per quirk.
4. **The simulator is a backend behind a trait, and one backend is fake.**
   A pure-Rust mock simulator lets the executor, triggers, write scheduling,
   and test runner be unit-tested at full speed with no EDA tool installed.
   cocotb cannot do this; every scheduler test needs a real simulator.
5. **Verilator is a first-class target, not a VPI emulation.** Verilator is the
   dominant open-source simulator and it is a C++ library, not a PLI host. Rivet
   owns `main`, drives `eval` directly, and can access signals without VPI.
   Everything else uses the standard VPI/VHPI/FLI path.
6. **Typed where possible, dynamic always.** Name-based dynamic access
   (`dut.path("u_core.alu.result")`) always works. Optional generated bindings
   give a typed `dut.u_core.alu.result: Signal<Logic, 32>` with compile-time
   checking of names and widths.
7. **`cargo test` is the regression runner.** A custom libtest-compatible
   harness means tests are discovered, filtered, and reported with the tools
   Rust developers already use. A thin `rivet` CLI wraps build orchestration.
8. **Determinism is a hard requirement.** Same seed, same sources, same
   simulator ⇒ identical trace. FIFO ready queues, seeded per-test RNG, no
   hash-map iteration order leaking into scheduling.

## 3. Execution topology

### 3.1 The choice: in-process plugin

Three topologies were considered.

| Topology | Sync cost | Isolation | Notes |
|---|---|---|---|
| In-process plugin (cocotb's) | ~100 ns–1 µs (function calls) | None: a testbench crash kills the sim | Only option that hits the performance goal |
| Out-of-process over shared memory | ~1–5 µs (futex wake, cache miss) | Full; harness can be any language | Viable later as a second transport behind the same trait |
| Out-of-process over socket | ~20–50 µs | Full | No better than cocotb; rejected |

Decision: in-process. The backend trait is transport-agnostic so a shared-memory
transport can be added later for remote/multi-language use without touching the
executor.

### 3.2 How the harness gets into the simulator

**PLI simulators (Icarus, Questa, Xcelium, VCS, Riviera, GHDL, NVC, DSim).**
The user's test crate is a `cdylib`. It depends on the `rivet` crate, which
exports the PLI entry points:

- VPI: a `vlog_startup_routines` array (`#[no_mangle] static`) containing one
  function that registers `cbStartOfSimulation` and `cbEndOfSimulation`.
- VHPI: `vhpi_startup_routines` likewise.
- FLI: a foreign attribute entry (`rivet_fli_init`) named from the VHDL
  `attribute foreign` string, as cocotb does through `cocotbfli_entry`.

The simulator loads the single `.so` (`vvp -M . -m libmy_tests`, `vsim -pli`,
`xrun -loadvpi`, `ghdl --vpi`, `nvc --load`, and so on). No second library, no
`GPI_EXTRA`, no interpreter. The runner already knows every simulator's flag
from cocotb's `runner.py`; that knowledge ports directly.

**Verilator.** The user's test crate is a `bin`. `build.rs` runs
`verilator --cc --vpi --public-flat-rw --timing --build` to produce
`libVtop.a` and `libverilated.a`, plus a small generated C++ shim exposing
`extern "C"` functions (`rivet_vl_new`, `rivet_vl_eval_step`,
`rivet_vl_eval_end_step`, `rivet_vl_next_time_slot`, `rivet_vl_final`, trace
open/dump/close). Rivet's `main` implements the same loop as cocotb's
`verilator.cpp` (see analysis §3.5), but in Rust and with direct control over
the timer wheel and clock generation. Signals are reachable through Verilator's
VPI as the portable path, and through generated direct accessors as the fast
path (§6.4).

Consequence: `cargo run` builds the design, links the harness, and runs the
simulation as one native executable. `cargo test` runs the regression.

### 3.3 Panics and unwinding across the FFI boundary

Every simulator callback entry (`extern "C" fn`) wraps its body in
`catch_unwind`. A panic inside a test is converted into a test failure, the
current test is torn down, and the next test starts. A panic outside any test
(harness bug) logs, calls the backend's `finish()` (`vpi_control(vpiFinish)`),
and returns. Unwinding into C is undefined behaviour, so this is not optional.

## 4. Concurrency: the executor

### 4.1 Model

Rust `async`/`await` with a single-threaded, `!Send`, deterministic executor.
The mapping from cocotb is exact:

| cocotb | Rivet |
|---|---|
| `Task` wrapping a coroutine | `Task` wrapping `Pin<Box<dyn Future<Output = ()>>>` |
| `Trigger._prime()` on first registration | `Future::poll` on first poll registers the simulator callback and stores the `Waker` |
| `Trigger._react()` → `_do_callbacks()` → `EventLoop.run()` | simulator callback → wake stored wakers → `Executor::run_until_idle()` |
| `EventLoop` deque of `ScheduledCallback` | `VecDeque<TaskId>` ready queue, FIFO |
| `cocotb.start_soon(coro)` | `rivet::spawn(fut) -> JoinHandle` |
| `TaskManager` | `Scope` (structured concurrency, cancels children on error) |
| `task.cancel()` raising `CancelledError` at the await | `JoinHandle::cancel()`: dropping the future runs destructors, which is Rust's cancellation |

The executor's `run_until_idle()` is called from *inside* a simulator callback
and drains the ready queue until every task is parked on a trigger. It then
returns to the simulator. This is exactly cocotb's `GPITrigger._react()`
contract and it is what makes the harness look like a well-behaved PLI
application to the simulator.

### 4.2 Why `async` rather than stackful coroutines

Stackful coroutines (`corosensei`-style) would let user code write
`clk.rising_edge().wait()` from any plain function, which reads like
SystemVerilog `@(posedge clk)`. That is attractive to hardware engineers. The
costs: stack allocation per task (megabytes for thousands of monitors),
no compiler-checked cancellation safety, no `select!`/`join!` ecosystem, and
unsound interactions with `catch_unwind` and thread-locals. `async` is the
idiomatic choice; the ergonomic gap is closed with combinators
(`first!`, `join!`, `with_timeout`) and by making every trigger a plain future.
A stackful shim can be layered on later for users who want it.

### 4.3 Wakers and zero allocation on the hot path

A `RisingEdge` await must not allocate. The trigger future owns a small
inline registration record; the backend stores a raw pointer to the record in
the callback's `user_data`; the record holds the `Waker`. On fire, the backend
clones nothing, calls `waker.wake_by_ref()`, and marks the record fired.
Dropping the future before it fires deregisters the callback (this is the
`_unprime` path, and on some simulators deregistration of an already-fired
callback is an error, so the record tracks state: `Armed`, `Fired`, `Removed`).

Edge triggers on the same signal from many tasks share one simulator
callback (`ValueChange` on the signal) and fan out in Rust, rather than
registering N VPI callbacks. cocotb 2.x already does this via the per-signal
`rising_edge` singleton trigger; Rivet does it at the backend level.

### 4.4 Cancellation and structured concurrency

Rust's cancellation is drop. A `Scope` (cocotb's `TaskManager`) owns child
`JoinHandle`s; leaving the scope (normally or by `?`) drops and thereby
cancels children, deregistering their callbacks. Panics in a child are captured
as `Result<(), TestFailure>` in the handle and, if the scope is configured to
fail-fast, cancel siblings.

`with_timeout(fut, 100.ns())` is a `select` between the future and a `Timer`.
`first(a, b)` returns whichever fires first and drops the other. These are the
combinators cocotb documents as `First`, `Combine`, `with_timeout`.

### 4.5 Determinism

Ready-queue order is FIFO. Wakers enqueue in call order. Value-change fan-out
wakes waiters in registration order. Per-test RNG is `ChaCha8` seeded from
`(run_seed, test_name, param_index)`. No `HashMap` iteration influences
scheduling; pending writes use an insertion-ordered structure.

## 5. The timestep protocol and write scheduling

Rivet adopts cocotb's timing model unchanged: five phases per timestep, with
the same set of triggers legal in each phase and the same illegal transitions
(awaiting `ReadWrite` or `ReadOnly` from within `ReadOnly` is an error).

### 5.1 Writes

cocotb learned (issue history in analysis §3.3) that most simulators do not
apply `vpiInertialDelay` writes at the point the standard says. Its answer is a
per-timestep write cache flushed at the start of the next `ReadWrite` phase,
switchable per simulator with `COCOTB_TRUST_INERTIAL_WRITES`.

Rivet keeps both modes as a backend capability flag, `trusts_inertial_writes`,
set per backend (true for Verilator, GHDL, NVC, Riviera; false elsewhere, per
cocotb's defaults). The pending-write buffer is a `Vec<(HandleId, Value)>` with
a small `HandleId → index` map so a second write to the same handle in one
timestep overwrites the first. One `ReadWrite` callback is registered on the
first pending write. `Force`, `Release`, and `NoDelay` bypass the buffer.

### 5.2 Immediate reads after writes

A deposit followed by a read in the same phase returns the *old* value (the
write is inertial). This surprises new users of cocotb too. Rivet's `Signal`
API makes it explicit: `set()` schedules, `set_now()` is `NoDelay`, and reading
a signal with a pending scheduled write logs a debug hint in verbose mode.

## 6. Values and hierarchy

### 6.1 The value representation

Rivet's core value type is a 4-state packed vector in VPI's own encoding:
two bitplanes, `aval` and `bval`, in 32-bit words. This is also what
`vhpiLogicVecVal` and FLI's `mti_GetArrayValue` can be converted to with a
lookup table per element, and it is what Verilator's `VlWide` is (with
`bval = 0`).

| aval | bval | meaning |
|---|---|---|
| 0 | 0 | `0` |
| 1 | 0 | `1` |
| 0 | 1 | `Z` |
| 1 | 1 | `X` |

Types:

- `Logic`: one 4-state bit, `Copy`, with `is_resolvable()`.
- `LogicVec`: owned, heap-backed for widths over 128 bits, inline otherwise
  (`SmallVec<[u32; 4]>` style), with `width()`, indexing, slicing, `to_u64()`
  returning `Err(Unresolved)` on X/Z, `to_biguint()`, `From<u64>`,
  `From<&str>` for `"8'b1010_xxzz"` literals, and `Display` in binary/hex.
- `Bits<const N: usize>`: 2-state fixed width for the common case, converts
  losslessly to `LogicVec`.
- Scalars: `i64`, `f64`, `String`, and enum variants (name plus ordinal).

Reads use `vpiVectorVal` (VPI), `vhpiLogicVecVal` (VHPI), and
`mti_GetArrayValue` (FLI). Writes use the same. The binary-string path
survives only as a fallback for object kinds where a simulator refuses vector
formats (cocotb's code identifies these; see analysis §3.2).

### 6.2 Handles

A `Handle` is a `Copy` index into a per-backend handle table, not a pointer
the user can misuse. The table caches the simulator handle, the object kind,
width, range, signedness, and constness on first discovery. Kinds mirror
cocotb's `gpi_objtype`: `Module`, `Struct`, `Logic`, `LogicArray`, `Array`,
`Integer`, `Real`, `String`, `Enum`, `Package`, `GenerateArray`.

Typed wrappers: `Module`, `Signal<T>`, `Array<T>`, `Value<T>` where `T`
is one of `Logic`, `LogicVec`, `i64`, `f64`, `String`, `Enum`. Creating a
`Signal<LogicVec>` from a handle whose kind is `Real` is an error at wrap
time, not at first access.

### 6.3 Discovery: dynamic and generated

Dynamic access: `dut.child("u_alu")?`, `dut.path("u_core.u_alu.result")?`,
`module.children()` iteration. Discovery is lazy, cached, and uses
`vpi_handle_by_name` before falling back to iteration, as cocotb does.
Name canonicalization rules (VHPI upper-casing, Xcelium generate block
naming, escaped identifiers) live in the backend and are covered by tests.

Generated access (`rivet-bindgen`): given a hierarchy dump, emit a Rust module
with one struct per instance and typed fields per signal, plus a
`Dut::bind(root)` that resolves every handle at startup and fails fast if the
design does not match the generated bindings. The dump comes from:

- Verilator: `--xml-only` output at build time.
- Any simulator: a `rivet dump-hierarchy` run of the harness that walks the
  design through the backend once and writes JSON. This run is cheap and
  the JSON is checked in.

Typed bindings are optional. They pay for themselves on large designs where
a typo in a signal path currently costs a full simulation run to find.

### 6.4 Verilator direct access

With `--public-flat-rw`, Verilator emits every signal as a public member of
the model class. `rivet-bindgen` (Verilator flavour) generates, from the XML,
a C++ shim with `extern "C"` getters and setters for each public signal and a
Rust side that calls them. Reads and writes then cost one non-virtual function
call and no VPI lookup. Value-change detection for triggers still goes through
Verilator's VPI callbacks (which are just a vector scan inside `callValueCbs`),
or, for clock edges, through Rivet's own clock generator which knows when it
toggled the clock and can wake `RisingEdge` waiters without asking Verilator.

## 7. Backends: the simulator trait

```rust
pub trait Backend {
    // identity and capabilities
    fn name(&self) -> &str;
    fn caps(&self) -> Capabilities;         // trusts_inertial_writes, has_force, four_state, ...
    fn precision_fs(&self) -> u64;
    fn now(&self) -> SimTime;

    // hierarchy
    fn root(&self, name: Option<&str>) -> Result<Handle>;
    fn child_by_name(&self, parent: Handle, name: &str) -> Result<Option<Handle>>;
    fn child_by_index(&self, parent: Handle, idx: i64) -> Result<Option<Handle>>;
    fn iterate(&self, parent: Handle, sel: IterSel) -> Result<Vec<Handle>>;
    fn info(&self, h: Handle) -> &ObjInfo;   // kind, width, range, signed, const, name

    // values
    fn read(&self, h: Handle, out: &mut ValueBuf) -> Result<()>;
    fn write(&self, h: Handle, v: &ValueRef, action: Action) -> Result<()>;

    // callbacks; `reg` is a stable pointer owned by the trigger future
    fn on_value_change(&self, h: Handle, reg: *const CbRecord) -> Result<CbId>;
    fn on_after_delay(&self, steps: u64, reg: *const CbRecord) -> Result<CbId>;
    fn on_read_write(&self, reg: *const CbRecord) -> Result<CbId>;
    fn on_read_only(&self, reg: *const CbRecord) -> Result<CbId>;
    fn on_next_time_step(&self, reg: *const CbRecord) -> Result<CbId>;
    fn remove(&self, id: CbId) -> Result<()>;

    // lifecycle
    fn finish(&self);
    fn stop(&self);                          // drop to the simulator prompt
}
```

Backends:

| Backend | Simulators | Notes |
|---|---|---|
| `mock` | none | Pure-Rust event simulator; used for harness tests and for running testbenches against Rust behavioural models |
| `vpi` | Icarus, Verilator (VPI path), Questa/ModelSim (Verilog), Xcelium, VCS, Riviera, DSim, CVC | Compile-time features select per-simulator quirks, mirroring cocotb's `#ifdef` set |
| `vhpi` | GHDL, NVC, Riviera, Xcelium (VHDL), Questa (VHPI) | |
| `fli` | Questa/ModelSim VHDL | Different callback model (`mti_CreateProcess`); FLI is faster than VHPI on Questa, which is why cocotb keeps it |
| `verilator` | Verilator in-process | Owns `main`, drives `eval`; uses `vpi` for triggers and hierarchy, direct access for values |

Edge filtering (a `cbValueChange` firing when the value went to `1`) is done
by reading the value in the callback. cocotb reads a binary string and
`strcmp`s; Rivet reads the scalar as an integer, or, on Verilator, gets the
new value in `cb_data.value` for free.

Every quirk from the analysis becomes a named item in `Capabilities` or a
`cfg(feature = "sim-xcelium")` block with a comment citing the cocotb issue.

## 8. Test layer

### 8.1 Declaring tests

```rust
#[rivet::test(timeout = 10.us())]
async fn reset_clears_counter(dut: Dut) -> rivet::Result {
    let clk = Clock::start(dut.clk, 10.ns());
    dut.rst_n.set(0);
    clk.cycles(2).await;
    dut.rst_n.set(1);
    dut.clk.rising_edge().await;
    assert_eq!(dut.count.get().to_u64()?, 0);
    Ok(())
}
```

`#[rivet::test]` registers the function in a link-time collection
(`inventory`/`linkme`) with its name, module path, attributes (`timeout`,
`expect_fail`, `skip`, `stage`), and an optional parameter matrix. That is
cocotb's `cocotb.test` decorator plus `TestFactory`, resolved at link time
rather than by module scanning.

### 8.2 Running tests

Inside one simulator process tests run sequentially. Between tests the harness
does what cocotb's `RegressionManager` does: drop all tasks, deregister all
callbacks, flush pending writes, reset the RNG, and start the next test at the
current simulation time. Simulation time does not rewind between tests, as in
cocotb.

Across processes, a regression is one simulator process per (design build,
test shard). `cargo test` and the `rivet` CLI both support `-j`.

### 8.3 `cargo test` as the runner

The test crate sets `harness = false` and `rivet::main!` provides a
libtest-compatible `main`: it understands `--list`, name filters,
`--exact`, `--skip`, `--test-threads`, and `--format json`. When invoked it
either (Verilator) runs the simulation directly or (PLI simulators) spawns the
simulator with the right flags to load itself as a plugin and streams
results back over a pipe. Test outcomes are reported in libtest format so
`cargo test`, `cargo nextest`, and IDE test runners work unchanged.

The harness also writes `results.xml` in the JUnit dialect cocotb produces, so
existing CI dashboards keep working.

### 8.4 Build orchestration

`rivet.toml` (or `[package.metadata.rivet]` in `Cargo.toml`) declares sources,
top level, language, parameters, defines, include dirs, and per-simulator
extra arguments. The build step is content-hashed: sources, flags, and
simulator version go into a key; unchanged inputs skip recompilation. This
replaces cocotb's timestamp-based `always` check and the Makefile flow.

## 9. Library ("kit")

Shipped with the harness, because cocotb's ecosystem shows what happens when
the bus library is a separate, under-maintained project:

- `Clock`: native, driven by the executor's timer wheel, not a task awaiting
  `Timer` twice per period. Supports start/stop, phase offset, and, on
  Verilator, waking edge waiters without a VPI callback.
- `Reset` helper: assert for N cycles, active-high/low.
- Sync: `Event`, `Queue<T>` (bounded/unbounded), `Lock`, `Semaphore`.
- Combinators: `join!`, `first!`, `with_timeout`, `Scope`.
- Transaction framework: `Driver<T>`, `Monitor<T>`, `Scoreboard<T>` traits with
  channels between them; reference implementations for a valid/ready
  handshake, AXI-Lite, and a simple memory model.
- `assert_sim!` family: assertion macros that annotate failures with sim time
  and the current test.
- Logging via `tracing`, with a subscriber that prefixes sim time, and
  per-test log capture.
- Waveforms: enable per test, name file per test.

## 10. Performance model and targets

The costs to measure, in order of importance, with cocotb 2.2 on the same
simulator as the baseline:

1. **Edge round trip**: a single task doing `await clk.rising_edge()` for
   1 million cycles. Icarus and Verilator. Target: harness overhead under
   1 µs per edge on Icarus, under 200 ns on Verilator direct.
2. **Value traffic**: read one 32-bit and one 512-bit signal and write one
   32-bit signal per cycle. Target: no allocation, no strings; on Verilator
   direct, indistinguishable from the eval cost.
3. **Task scaling**: 1000 monitors each awaiting a different signal edge.
   Target: linear, no per-task VPI registration for shared signals.
4. **Startup**: time from process start to first test. cocotb pays for
   interpreter start and module import. Target: under 50 ms.

The benchmark crate is part of the repo and runs in CI against Icarus and
Verilator, which are free.

## 11. What Rivet deliberately does not do (initially)

- No Python API. A PyO3 facade over Rivet's runtime is plausible later and
  would make Rivet a drop-in faster runtime for cocotb-style Python tests, but
  it is a separate project.
- No constrained-random solver. `proptest`-style strategies cover the common
  cases; a real solver is out of scope.
- No SystemC, no mixed-signal.
- No support for simulators cocotb does not support.

## 12. Risks

| Risk | Mitigation |
|---|---|
| Commercial simulator quirks cannot be tested without licenses | Quirks are ported from cocotb with citations; each is isolated behind a capability flag; a `SimQuirks` test document lists what is unverified |
| Verilator VPI coverage is partial (force/release, some object kinds) | Direct access path does not depend on VPI for values; force/release on Verilator is documented as unsupported until verified |
| `cdylib` symbol visibility and static initializers differ per platform | `vlog_startup_routines` is a plain exported data symbol; tested on Linux and macOS in CI with Icarus |
| Unwinding across FFI | `catch_unwind` at every entry point; `panic = "abort"` is rejected in the test crate profile by a compile-time check |
| Simulators that run PLI callbacks on multiple threads (Xcelium multi-core) | Executor asserts thread identity on entry; document the single-thread requirement as cocotb does |
| Async Rust ergonomics for hardware engineers | Kit combinators, a short "cocotb to Rivet" translation table, and examples for every cocotb example |
