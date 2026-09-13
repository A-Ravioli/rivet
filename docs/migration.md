# Migrating from cocotb

Rivet keeps cocotb's model of a testbench: coroutines that await simulator
events, a `dut` handle for the hierarchy, drivers and monitors as
concurrent tasks, one results file per run. What changes is the language
and what the harness does for you. This page maps the cocotb API onto
Rivet and lists the things that have no direct equivalent.

## Test setup

| cocotb | Rivet |
|---|---|
| `Makefile` with `SIM`, `TOPLEVEL`, `VERILOG_SOURCES`, `MODULE` | `rivet.toml` (`[design] top`, `sources`, `[sim.*]`) and a Cargo crate; `rivet run --sim icarus` |
| `cocotb_tools.runner` (Python runner) | `rivet run`, or `cargo test` with `rivet::harness::main()` |
| `@cocotb.test()` | `#[rivet::test]` on an `async fn(dut: Module) -> rivet::Result<()>` |
| `@cocotb.test(timeout_time=10, timeout_unit="us")` | `#[rivet::test(timeout = 10.us())]` |
| `@cocotb.test(expect_fail=True)` / `skip=True` | `#[rivet::test(expect_fail)]` / `#[rivet::test(skip)]` |
| `@cocotb.test(expect_error=ValueError)` | `#[rivet::test(expect_fail = "part of the message")]`: the failure must say what was expected, so a test cannot pass by failing for an unrelated reason |
| a test that is meant to run out of time | `#[rivet::test(expect_timeout)]` |
| `skip=True` decided before the run | `return Err(rivet::skip("reason"))` ends the running test as skipped, so a test can stand down over what the simulator in front of it cannot do |
| `raise TestSuccess` | `rivet::runtime::finish_test()`, callable from any task, so a monitor can end a test early |
| pytest fixtures for setup and teardown | `#[rivet::fixture] async fn ready(dut: Module) -> rivet::Result<Signal>`, asked for by argument name: `async fn t(dut: Module, ready: Signal)`. Teardown is the value's `Drop` |
| `@cocotb.test(stage=1)` | `#[rivet::test(stage = 1)]` |
| `@cocotb.parametrize(width=[8, 16])` | `#[rivet::test(params = [8, 16])] async fn t(dut: Module, width: u32)` (in-process values) or `[design.param_sets]` in `rivet.toml` (rebuilds the design) |
| `TESTCASE=name` / `COCOTB_TEST_FILTER` (`re.search`) | `rivet run --filter name` (regular expressions, comma separated, searched against `module::name` and the bare name) or `cargo test name` |
| `COCOTB_RANDOM_TEST_ORDER=1` | `rivet run --shuffle` (`RIVET_SHUFFLE=1`): shuffles within each stage, and `--seed` replays the same order |
| `RANDOM_SEED=1234` | `rivet run --seed 1234` (every run prints its seed) |
| `COCOTB_LOG_LEVEL=DEBUG` | `rivet run --log debug`; `--log-format json`; per-test files in `sim_build/<sim>/logs/` |
| `results.xml` | the same JUnit format at `sim_build/<sim>/results.xml`, with `file` and `lineno` on every case, plus `results.json` |
| `WAVES=1` | `rivet run --waves` (`--waves-per-test` on Verilator) |
| parallel runs via pytest-xdist and separate `sim_build` dirs | `rivet run -j 4` (shards inside one build) |
| `pytest` collecting cocotb tests | `cargo nextest run` (one test per process, its own results directory, the design build shared behind a lock; settings in `.config/nextest.toml`) |
| `cocotb-config --makefiles` and a copied Makefile to start a project | `rivet new <name>` scaffolds a crate that passes as generated |
| `SIM=ghdl` for VHDL | `rivet run --sim ghdl` (VPI) or `rivet run --sim nvc` (VHPI, which also reaches record members, enumeration literal names and generics) |

## Handles and values

| cocotb | Rivet |
|---|---|
| `dut.sig` | `dut.signal("sig")?`; `dut.path_signal("u_core.alu.result")?` |
| `dut.sub` (module) | `dut.module("sub")?` |
| `dut.gen[2].tap` | `dut.path_signal("gen[2].tap")?` or `dut.module("gen")?.index(2)?` |
| `dut.mem[3]` (array element) | `dut.signal("mem")?.index(3)?` |
| `sig.value` (`LogicArray`) | `sig.get()` (`LogicVec`, four-state) |
| `int(sig.value)` | `sig.get_u64()?` (errors on X/Z) or `sig.get_u64_lossy()` |
| `sig.value.is_resolvable` | `sig.get().is_resolvable()` |
| `sig.value = 5` | `sig.set(5)` (inertial, cocotb's default) |
| `sig.setimmediatevalue(5)` | `sig.set_now(5)` |
| `sig.value = Force(1)` / `Release()` | `sig.force(1)` / `sig.release()` |
| `len(sig)` / `sig._range` | `sig.width()` / `sig.info().range` |
| `sig.value[7:4]` (slice a `LogicArray`) | `sig.slice(7, 4)`, which reads and writes the bit range through the whole vector on every backend |
| `int(sig.value)` on a VHDL enumeration (its position) | `sig.enum_name()` gives the literal, `sig.enum_literals()` the whole list, where the simulator reports them (NVC through VHPI; `None` elsewhere) |
| `sig.value` on a `real` | `sig.get_real()` / `sig.set_real(x)` |
| `str(sig.value)` on a string variable | `sig.get_string()` |
| `dut._discover_all()` / `dir(dut)` | `dut.children()?`; `rivet bindgen` writes a typed `Dut` struct with a `hierarchy()` method |
| `cocotb.types.LogicArray("1x0z")` | `LogicVec::parse("4'b1x0z")`, `Logic` |

Generated bindings (`rivet bindgen`) replace string lookups with fields:
`dut.clk`, `dut.mem[2]`, `dut.u_sub.x`. `typedef enum` and `typedef
struct packed` in the sources become Rust enums and structs with
`from_signal` / `set_on`.

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

Phase rules are the same as cocotb's: writes are buffered until the
ReadWrite phase, a write in ReadOnly panics, `RisingEdge` returns in the
values-change phase before downstream logic has reacted.

## Verification components

| cocotb (and cocotb-bus / cocotbext) | Rivet kit |
|---|---|
| `cocotb_bus.drivers.Driver` / `monitors.Monitor` | `Driver<T>` / `Monitor<T>` traits, `drive_from_queue`, `monitor_to_queue` |
| `cocotb_bus.scoreboard.Scoreboard` | `Scoreboard<T>`; `ModelScoreboard` feeds it from a `Model` |
| `cocotbext-axi` `AxiLiteMaster` / `AxiLiteRam` | `AxiLiteMaster` / `AxiLiteSlave` (`bus::axi_lite`) |
| `cocotbext-axi` `AxiMaster` / `AxiRam` | `AxiMaster` / `AxiSlave` (bursts, narrow transfers, IDs) |
| `cocotbext-axi` `AxiStreamSource` / `AxiStreamSink` | `AxisSource` / `AxisSink` |
| `cocotbext-apb`, Avalon, Wishbone drivers | `ApbMaster`/`ApbSlave`, `AvalonMaster`/`AvalonSlave`, `WishboneMaster`/`WishboneSlave` |
| `pause_generator` / backpressure callbacks | `Backpressure::{None, Fixed, random, Custom}` |
| `cocotb.utils` reset helpers, hand-written reset coroutines | `Reset::new(clk, rst).active_low().asynchronous().cycles(3).apply()` |
| `random.randint` with `RANDOM_SEED` | `rivet::rng()` (per-test seeded stream), `#[derive(Randomize)]` with constraints |
| `cocotb-coverage` `CoverPoint` / `CoverCross` | `Covergroup`, `Bins`, `CoverPoint`, `Cross`; `rivet cov report` |
| `assert` in a monitor loop | `assert_never` / `assert_always` / `assert_implies` / `assert_no_x` background checkers |
| golden-file comparisons written by hand | `Trace` + `assert_trace!` with `RIVET_UPDATE_GOLDEN=1` |
| `$readmemh` files loaded in HDL | `Memory::load_hex`, `load_hex_into(&array, path)` |

## Diagnostics cocotb does not have

- A simulated-time timeout reports every live task and what it waits on;
  `rivet::dump_tasks()` gives the same list on demand.
- `#[rivet::test(wall_timeout = 30)]` or `--wall-timeout` catches a testbench
  that never returns to the simulator.
- Every run prints its seed, and each test's seed is in `results.xml`.

## Migrating incrementally

`python/rivet_py` exposes the simulator-independent kit (memory model,
scoreboard, seeded random, functional coverage) to Python, so an existing
cocotb testbench can adopt Rivet's models before its tests move to Rust
(`maturin develop` in that directory, then `import rivet_py`).

## Not covered yet

- FLI and mixed-language designs. VHDL runs on NVC through VHPI and on
  GHDL through VPI, where records, enumeration literals and generics are
  not reachable.
- Commercial simulators; see `docs/design/04-remaining-work.md`.
- cocotb's `Freeze`/`Deposit` value objects: use `force`/`set`.
- Python-only conveniences such as `BinaryValue` arithmetic; use Rust integers and `LogicVec`.
