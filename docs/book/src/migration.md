# Migrating from cocotb

Rivet keeps cocotb's model of a testbench: coroutines that await simulator
events, a `dut` handle for the hierarchy, drivers and monitors as concurrent
tasks, one results file per run. What changes is the language and what the
harness does for you.

The full mapping, including handles and values, verification components and
the things with no direct equivalent, is in
[`docs/migration.md`](https://github.com/A-Ravioli/rivet/blob/main/docs/migration.md).
This page carries the two tables you need first and summarises the rest.

## Test setup

| cocotb | Rivet |
|---|---|
| `Makefile` with `SIM`, `TOPLEVEL`, `VERILOG_SOURCES`, `MODULE` | `rivet.toml` (`[design] top`, `sources`, `[sim.*]`) and a Cargo crate; `rivet run --sim icarus` |
| `cocotb_tools.runner` (Python runner) | `rivet run`, or `cargo test` with `rivet::harness::main()` |
| `@cocotb.test()` | `#[rivet::test]` on an `async fn(dut: Module) -> rivet::Result<()>` |
| `@cocotb.test(timeout_time=10, timeout_unit="us")` | `#[rivet::test(timeout = 10.us())]` |
| `@cocotb.test(expect_fail=True)` / `skip=True` | `#[rivet::test(expect_fail)]` / `#[rivet::test(skip)]` |
| `@cocotb.test(stage=1)` | `#[rivet::test(stage = 1)]` |
| `@cocotb.parametrize(width=[8, 16])` | `#[rivet::test(params = [8, 16])] async fn t(dut: Module, width: u32)` (in-process values) or `[design.param_sets]` in `rivet.toml` (rebuilds the design) |
| `TESTCASE=name` / `COCOTB_TEST_FILTER` | `rivet run --filter name` (substring) or `cargo test name` |
| `RANDOM_SEED=1234` | `rivet run --seed 1234` (every run prints its seed) |
| `COCOTB_LOG_LEVEL=DEBUG` | `rivet run --log debug`; `--log-format json`; per-test files in `sim_build/<sim>/logs/` |
| `results.xml` | the same JUnit format at `sim_build/<sim>/results.xml`, plus `results.json` |
| `WAVES=1` | `rivet run --waves` (`--waves-per-test` on Verilator) |
| parallel runs via pytest-xdist and separate `sim_build` dirs | `rivet run -j 4` (shards inside one build) |

## Triggers and time

| cocotb | Rivet |
|---|---|
| `await Timer(10, unit="ns")` | `Timer::new(10.ns()).await` |
| `await RisingEdge(dut.clk)` / `FallingEdge` / `Edge` | `clk.rising_edge().await` / `falling_edge()` / `value_change()` |
| `await ReadOnly()` / `ReadWrite()` / `NextTimeStep()` | `read_only().await` / `read_write().await` / `next_time_step().await` |
| `await First(a, b)` | `first(a, b).await` (returns `Either`) |
| `await Combine(a, b)` | `join(a, b).await` |
| `await with_timeout(coro, 10, "us")` | `with_timeout(fut, 10.us()).await?` |
| `Clock(dut.clk, 10, unit="ns").start()` | `Clock::start(clk, 10.ns())`; `Clock::builder(clk, 10.ns()).phase(3.ns()).jitter(200.ps()).start()` |
| `get_sim_time("ns")` | `rivet::now_in(Unit::Ns)`; `rivet::now()` in precision steps |
| `Event`, `Queue`, `Lock` | `Event`, `Queue`, `Lock` (same semantics, `await` on `wait()`, `get()`, `acquire()`) |
| `cocotb.start_soon(coro)` | `spawn(fut)` (returns a `JoinHandle`) |
| `task.cancel()` / `await task` | `handle.cancel()` / `handle.await` |
| `await task.join()` | `handle.await?` |
| cancellation on test end | automatic; `Scope` cancels children when dropped |

The phase rules are the same as cocotb's: writes are buffered until the
ReadWrite phase, a write in ReadOnly panics, and `RisingEdge` returns in the
values-change phase before downstream logic has reacted. See
[The timing model](timing-model.md).

## What the other tables cover

- **Handles and values.** `dut.sig` becomes `dut.signal("sig")?`, `sig.value`
  becomes `sig.get()`, `int(sig.value)` becomes `sig.get_u64()?`,
  `sig.setimmediatevalue(5)` becomes `sig.set_now(5)`, and `Force`/`Release`
  become `sig.force(v)` / `sig.release()`. `rivet bindgen` replaces string
  lookups with typed fields. See [Writing tests](writing-tests.md).
- **Verification components.** `cocotb-bus` and the `cocotbext-*` packages map
  onto `rivet-kit`: `AxiLiteMaster`, `AxiMaster`, `AxisSource`/`AxisSink`,
  `ApbMaster`, `AvalonMaster`, `WishboneMaster`, their memory-backed slaves,
  `Scoreboard`, `Reset`, and `Backpressure` in place of pause generators. See
  [The kit](kit.md).
- **Random and coverage.** `random.randint` with `RANDOM_SEED` becomes
  `rivet::rng()` and `#[derive(Randomize)]`; `cocotb-coverage`'s `CoverPoint`
  and `CoverCross` become `Covergroup`, `Bins`, `CoverPoint` and `Cross`. See
  [Random stimulus and coverage](random-and-coverage.md).

## Diagnostics cocotb does not have

- A simulated-time timeout reports every live task and what it waits on;
  `rivet::dump_tasks()` gives the same list on demand.
- `#[rivet::test(wall_timeout = 30)]` or `--wall-timeout` catches a testbench
  that never returns to the simulator, with a watchdog thread behind it.
- Every run prints its seed, and each test's seed is in `results.xml`.

See [Debugging](debugging.md).

## Migrating incrementally

`python/rivet_py` exposes the simulator-independent parts of the kit (the
memory model, scoreboard, seeded random and functional coverage) to Python, so
an existing cocotb testbench can adopt Rivet's models before its tests move to
Rust. Run `maturin develop` in that directory, then `import rivet_py`.

## Not covered yet

- VHPI, FLI and mixed-language designs. GHDL runs through VPI, and NVC is not
  supported; see [Simulators](simulators.md).
- Commercial simulators. Their quirks are implemented in the VPI backend but
  have never been run.
- cocotb's `Freeze` and `Deposit` value objects: use `force` and `set`.
- Python-only conveniences such as `BinaryValue` arithmetic: use Rust integers
  and `LogicVec`.
