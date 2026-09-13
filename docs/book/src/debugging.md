# Debugging

What a failing or hanging test looks like, and the tools for finding out why.

## What a failure looks like

A failed test is logged at error level as it happens, appears as `FAIL` in the
summary table, and lands in `results.xml` and `results.json`:

```text
       120ns ERROR rivet_core::test         example_dff::dff_follows_d FAILED: cycle 3: q after update
                                            assertion `left == right` failed
                                              left: 17
                                             right: 42

TEST                              STATUS          SIM TIME        WALL
example_dff::counter_counts       PASS               450ns      0.006s
example_dff::dff_follows_d        FAIL               120ns      0.002s

RIVET_RESULT passed=1 failed=1 skipped=0 seed=7318782485113623909
rivet: 2 tests, 1 failed, 0 skipped (/path/sim_build/icarus/results.xml)
```

The process exits 1. In `results.xml` the case carries a `<failure
message="..."/>` element with the same text, alongside the simulator name,
the simulated duration and the test's `random_seed`.

Whatever the failure, the runner then cancels every task the test started and
discards buffered writes before the next test begins.

A test can fail in four ways: it returns `Err`, it panics, a task it spawned
panics, or it exceeds a timeout. A `#[rivet::test(expect_fail)]` test inverts
the verdict, and the log says so:

```text
example_dff::times_out failed as expected: timed out after 50ns of simulated time
```

## Task dumps on timeout

A simulated-time timeout prints every live task and what it is waiting on.
This is the real output of `examples/dff`'s `times_out`:

```text
     995.008ns INFO  rivet_core::test         example_dff::times_out failed as expected: timed out after 50ns of simulated time
3 live task(s) at 995.008ns (phase BeginTimeStep, 1 armed timer(s), 0 buffered write(s)):
  regression                   running
  times_out                    waiting for RisingEdge(dff.clk) [now 0]
  clock(dff.clk)               waiting for Timer due at 995.008ns (+0ns)
```

The header gives the time, the phase, the number of armed timers and the
number of buffered writes. Each line names a task and its trigger:

| Waiting for | Shown as |
|---|---|
| an edge | `RisingEdge(dff.clk) [now 0]`, `FallingEdge(...)`, `ValueChange(...)`, with the signal's current value |
| a timer | `Timer due at 995.008ns (+0ns)`, absolute and relative |
| a phase | `ReadWrite`, `ReadOnly`, `NextTimeStep` |
| synchronisation | `Event`, `Queue`, `Lock`, `Join`, `Yield` |

Reading the dump above: the clock's timer is due right now and the test is
waiting for a clock edge, so nothing is deadlocked, the test simply ran out of
its 50 ns budget.

Name the tasks that matter with `spawn_named`, and they will be identifiable
here. `Clock` names its own task after the signal it drives.

You do not have to wait for a timeout. Call `rivet::dump_tasks()` anywhere to
get the same string:

```rust
let dump = rivet::dump_tasks();
ensure!(dump.contains("clock(bus_top.clk)"), "{dump}");
```

## Wall timeouts and the watchdog

A simulated-time timeout only fires if simulated time still advances. A
testbench stuck in a loop that never yields, or a simulator that never returns
control, needs a wall-clock limit. There are two layers.

**The in-band check.** `set_wall_limit` records a deadline, and the runtime
checks it every 64 simulator events. Past the deadline the test fails
in-band, with a task dump attached:

```text
wall-clock limit of 60s exceeded
3 live task(s) at ...
```

Set it per test or for the whole run:

```rust
#[rivet::test(wall_timeout = 60)]
async fn checkers_and_waves(dut: Module) -> rivet::Result<()> { /* ... */ }
```

```sh
rivet run --sim icarus --wall-timeout 120     # sets RIVET_WALL_TIMEOUT
```

The per-test attribute wins over the environment variable.

**The watchdog thread.** If the harness is never entered again, the in-band
check never runs. A watchdog thread starts with the first wall limit, wakes
every 500 ms, and aborts the process once the deadline plus a grace period has
passed:

```text
rivet: watchdog: test example_bus::checkers_and_waves exceeded its wall-clock limit and the simulator is not returning control to the harness; aborting the process
```

The simulator process then exits with status 3, and `rivet run` reports
that it produced no results. The grace period is 5 seconds by
default, so the in-band failure has a chance to report cleanly first; set
`RIVET_WATCHDOG_GRACE` to change it.

## A simulator that stops early

If the simulator ends before the regression finishes, for instance on an HDL
`$finish` or a failing HDL assertion, the harness records it as a failure
rather than reporting a short but successful run:

```text
simulator ended before all tests finished (an HDL $finish or assertion, or no clock running?)
```

## Waveforms

Enable dumping for the whole run:

```sh
rivet run --sim icarus --waves            # sim_build/icarus/<top>.fst
rivet run --sim verilator --waves         # sim_build/verilator/<top>.vcd
rivet run --sim verilator --waves-per-test
rivet run --sim ghdl --waves              # sim_build/ghdl/<top>.ghw
rivet run --sim nvc --waves               # sim_build/nvc/<top>.fst
```

Verilator needs `trace = true` under `[sim.verilator]` in `rivet.toml` for the
model to be built with tracing at all.

`--wave-open` opens the dump the run produced when it finishes, in `surfer`
or, failing that, in `gtkwave`, and prints where the dump is when neither is
installed. It implies `--waves`. On Questa, Xcelium and VCS, `--gui` runs
the tool's own GUI instead and leaves the run under your control.

A whole-run dump of a long regression is large and mostly uninteresting, so
the testbench can open a window around the part that matters:

```rust
rivet::waves::start(Some("alu_window"));
// ... the interesting window ...
rivet::waves::off();
```

| Function | Meaning |
|---|---|
| `waves::start(Some("name"))` | start a new file with this name, where supported, and turn dumping on |
| `waves::start(None)` | turn dumping on, continuing the current file |
| `waves::on()` / `waves::off()` | resume and pause dumping |

Each returns `false` when the simulator cannot do it, and the first failure
logs a warning. What each simulator supports:

| Simulator | on and off | new file per call |
|---|---|---|
| Verilator | yes | yes (`<name>.vcd` or `.fst`) |
| Icarus (`--waves`) | yes (`$dumpon`/`$dumpoff`) | no: one file per run |
| GHDL | no (`--wave` covers the whole run) | no |
| NVC | no (`--wave` covers the whole run) | no |

With `--waves-per-test` the runner calls `start` with `<module>__<test>`
before each test and `off` after it.

## Logs

Log records carry the simulation time, the level, the target and the message:

```text
       250ns INFO  counter_tb               counted to 20 by 250ns
```

The macros are in the prelude: `error!`, `warn!`, `info!`, `debug!`,
`trace!`.

| Variable | CLI flag | Meaning |
|---|---|---|
| `RIVET_LOG` | `--log <level>` | `error`, `warn`, `info` (default), `debug`, `trace`, `off` |
| `RIVET_LOG_FORMAT` | `--log-format <text\|json>` | record format |
| `RIVET_LOG_TIME_UNIT` | none | `ns` (default), `ps`, `us`, `step` |
| `RIVET_LOG_DIR` | `--log-dir <dir>`, `--no-log-dir` | per-test log files |

### Per-test log files

`rivet run` writes one file per test into `sim_build/<sim>/logs/` by default,
named `<module>__<test>.log`. A failing test's log can then be read on its
own, without scrolling through the whole regression:

```text
sim_build/icarus/logs/
├── example_dff__counter_counts.log
├── example_dff__dff_follows_d.log
└── example_dff__times_out.log
```

`--log-dir` moves them, `--no-log-dir` turns them off.

### JSON logs

`--log-format json` writes one object per line, to stderr and to the per-test
files:

```json
{"t":250000,"time":"250ns","level":"INFO","target":"counter_tb","test":"counter_tb::counts_when_enabled","msg":"counted to 20 by 250ns"}
```

| Field | Meaning |
|---|---|
| `t` | simulation time in precision steps, or `null` before the simulation starts |
| `time` | the same time formatted in `RIVET_LOG_TIME_UNIT` |
| `level` | `ERROR`, `WARN`, `INFO`, `DEBUG`, `TRACE` |
| `target` | the log target, normally the crate or module |
| `test` | `module::name`, or `null` outside any test |
| `msg` | the message |

## Reproducing a failure

Every run prints its base seed, and each test's seed is derived from that seed
and the test's name, so a single test reproduces on its own:

```sh
rivet run --sim icarus --seed 7318782485113623909 --filter dff_follows_d --waves --log debug
```

Because the per-test stream is derived from the name rather than the order,
adding `--filter` does not change the values the test sees.

## Sharded runs

With `-j N` each shard is a separate simulator process writing to
`sim_build/<sim>/shard<i>/`. Its output goes to `shard<i>/sim.log` and is
replayed to the terminal when the shard finishes:

```text
rivet: 13 test(s) in 4 shard(s)
rivet: ---- shard 0 (exit status: 0) ----
...
```

Sharding assumes tests are independent. `stage` ordering does not hold across
shards, and a test that relies on the state a previous test left behind must
not be sharded.
