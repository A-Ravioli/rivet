# The timing model

Rivet uses cocotb's timing model unchanged: five phases per time step, the
same triggers legal in each phase, and the same illegal transitions. If you
already know cocotb's model, you know Rivet's.

The runtime tracks the phase in `rivet::Phase`, which has one variant per
phase the harness can run in, plus `Startup` before the simulation begins.

## The five phases

| # | Phase | `rivet::Phase` | What resumes here |
|---|---|---|---|
| 1 | Beginning of time step | `BeginTimeStep` | `Timer`, `next_time_step()` |
| 2 | Evaluation | none | nothing: the simulator is evaluating the design |
| 3 | Values change | `ValuesChange` | `rising_edge()`, `falling_edge()`, `value_change()` |
| 4 | Values settle | `ValuesSettle` | `read_write()`, and buffered deposits are applied |
| 5 | End of time step | `EndTimeStep` | `read_only()` |

```text
  time step N                                              time step N+1
  ==========================================================  ===========
  (1) BeginTimeStep      (3) ValuesChange   (4) ValuesSettle   (5) EndTimeStep
       |                       |                  |                  |
  Timer fires            signal changed      ReadWrite           ReadOnly
  next_time_step()       rising_edge()       read_write()        read_only()
       |                 falling_edge()           |                  |
       |                 value_change()           |                  |
       v                       v                  v                  v
  +---------+   (2)      +-----------+      +-----------+      +----------+
  | harness | ------->   |  harness  | ---> |  harness  | ---> | harness  |
  +---------+ simulator  +-----------+      +-----------+      +----------+
       ^      evaluates        ^                  ^                  ^
       |      the design       |                  |                  |
   writes                  writes            buffered writes    NO WRITES
   buffered                buffered          are applied        (panic)
                                             here, then
                                             writes go
                                             straight through
```

Phase 2 has no harness code in it. The simulator evaluates the design and
then reports the value changes that resulted, which is phase 3.

## What each trigger returns in

| Trigger | Returns in | Notes |
|---|---|---|
| `Timer::new(10.ns()).await` | beginning of time step | must advance time; `Timer::steps(0)` panics |
| `next_time_step().await` | beginning of the next time step | |
| `sig.rising_edge().await` | values change | the design has not reacted to this edge yet |
| `sig.falling_edge().await` | values change | |
| `sig.value_change().await` | values change | any transition |
| `read_write().await` | values settle | values have settled; writes are still allowed |
| `read_only().await` | end of time step | no writes allowed |
| `yield_now().await` | the same phase | lets other ready tasks run; no time passes |

The important one is the edge. `clk.rising_edge().await` returns in the
values-change phase, before the flops downstream of that clock have updated.
To read what the design produced on this edge, wait for values to settle
first. This is the pattern in `examples/dff`:

```rust
for i in 0..50u64 {
    let v = (i * 37 + 11) & 0xff;
    clk.rising_edge().await;
    // Values-change phase: the flop has not updated yet.
    assert_eq!(q.get_u64()?, prev_q, "cycle {i}: q before update");
    // Deposit the next value; it is not sampled until the next edge.
    d.set(v);
    read_only().await;
    assert_eq!(q.get_u64()?, last_d, "cycle {i}: q after update");
    prev_q = last_d;
    last_d = v;
}
```

## Write buffering

`Signal::set` is an inertial deposit, cocotb's default. Most simulators do
not apply `vpiInertialDelay` writes where the standard says they should, so
the runtime buffers deposits and flushes them itself.

The rules in `schedule_write`, in order:

1. Writing anything in the end-of-time-step (ReadOnly) phase panics.
2. Writing a constant is an error that fails the test, not a silent drop.
3. The write goes straight to the simulator if any of these holds:
   - the action is not a deposit (`set_now`, `force`, `release`);
   - the backend trusts inertial writes;
   - the phase is already values-settle (ReadWrite).
4. Otherwise the write is buffered. A second write to the same signal in the
   same time step overwrites the first, and the order of first writes is
   preserved. The first buffered write registers a ReadWrite callback.
5. At the ReadWrite callback the buffer is drained into the simulator
   **before** any task waiting on `read_write()` is woken, so a write made
   earlier in the time step is visible to code that resumes in the
   values-settle phase.

| Method | Action | Buffered | Reads back immediately |
|---|---|---|---|
| `sig.set(v)` | `Deposit` | yes, unless the backend trusts inertial writes | no: the old value |
| `sig.set_now(v)` | `NoDelay` | no | yes |
| `sig.force(v)` | `Force` | no | yes, and overrides drivers until released |
| `sig.release()` | `Release` | no | |

Which backends trust inertial writes:

| Backend | `trusts_inertial_writes` | Why |
|---|---|---|
| GHDL | yes | cocotb's default for GHDL |
| Icarus | no | cocotb's default; deposits are buffered and flushed at ReadWrite |
| Verilator | no, always | Rivet owns the simulation loop and does not call `VerilatedVpi::doInertialPuts`; deposits are buffered and applied as immediate writes at the flush, which behaves the same on 5.020 and 5.036 |

`RIVET_TRUST_INERTIAL_WRITES=1` forces trust on every backend except
Verilator, which ignores it.

A consequence to keep in mind: a deposit followed by a read in the same phase
returns the old value. That is the same surprise cocotb users meet, and it is
why `set` and `set_now` are separate methods instead of one assignment.

## Why writing in ReadOnly panics

The end-of-time-step phase is the simulator's promise that nothing more will
change at this time. A write there would either be dropped or would corrupt
the current time step, depending on the simulator, so Rivet fails loudly
instead:

```text
writing to a signal in the ReadOnly phase is not allowed
```

The same applies to awaiting a phase that has already gone by:

```text
illegal transition: awaiting ReadWrite in the ReadOnly phase
```

Both are panics, and a panic in a task fails the running test rather than
bringing down the simulator.

To write again after a `read_only().await`, first leave the time step. Any of
these does it:

```rust
clk.falling_edge().await;   // an edge in a later phase or time step
next_time_step().await;     // the beginning of the next time step
Timer::new(1.ns()).await;   // any advance of time
```

`examples/dff` uses the first form:

```rust
// Leave the ReadOnly phase before writing again.
clk.falling_edge().await;
ratio.set_real(42.25);
read_only().await;
```

The immediate checkers in the kit (`assert_stable`, `assert_becomes`,
`assert_within`) also return at the start of the next time step, because an
earlier version returned in ReadOnly and callers then wrote and panicked.

## Time and precision

Durations are written with the `TimeExt` methods: `10.ns()`, `2.5.us()`,
`100.ps()`, `1.ms()`, `5.sec()`, `3.steps()`. They convert to simulator
precision steps when used. `rivet::now()` is the current time in precision
steps; `rivet::now_in(Unit::Ns)` converts to a unit.

Between tests the simulation advances by exactly one precision step, as in
cocotb. Simulation time never rewinds, so the second test in a run starts at
whatever time the first one reached.
