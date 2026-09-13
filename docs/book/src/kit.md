# The kit

`rivet-kit`, re-exported as `rivet::kit`, holds the verification components:
reset sequences, drivers and monitors, scoreboards, reference models, a
memory model, bus models, background checkers and golden traces. It ships
with the harness rather than as a separate package.

## Driver and Monitor

Two traits, plus helpers that run them as tasks fed by a `Queue`:

```rust
pub trait Driver<T> {
    fn send(&mut self, item: T) -> impl Future<Output = ()>;
}

pub trait Monitor<T> {
    fn recv(&mut self) -> impl Future<Output = T>;
}

pub fn drive_from_queue<T, D: Driver<T>>(name: &str, driver: D, queue: Queue<T>) -> JoinHandle<()>;
pub fn monitor_to_queue<T, M: Monitor<T>>(name: &str, monitor: M, queue: Queue<T>) -> JoinHandle<()>;
```

`drive_from_queue` loops taking items off the queue and sending them;
`monitor_to_queue` loops receiving and putting them on the queue. Both run
until cancelled. The `name` appears in task dumps.

## Valid/ready

`ValidReadySource` drives `valid` and `data` and waits for `ready`.
`ValidReadySink` observes `valid && ready` transfers and can drive `ready`
itself.

| Constructor | Meaning |
|---|---|
| `ValidReadySource::new(clk, valid, ready, data)` | a source; implements `Driver<T>` for any `T: IntoLogicVec`, and has `send_value(v).await` and `idle(cycles).await` |
| `ValidReadySink::observer(clk, valid, ready, data)` | observes only, implements `Monitor<LogicVec>`; something else drives `ready` |
| `ValidReadySink::with_ready(clk, valid, ready, data, policy)` | drives `ready` from `policy(cycle) -> bool` |

`examples/fifo` wires both to queues:

```rust
let source = ValidReadySource::new(p.clk, p.in_valid, p.in_ready, p.in_data);
let sink = ValidReadySink::with_ready(p.clk, p.out_valid, p.out_ready, p.out_data, ready_policy);

let to_send: Queue<u64> = Queue::new();
let received: Queue<LogicVec> = Queue::new();
let driver = drive_from_queue("source", source, to_send.clone());
let monitor = monitor_to_queue("sink", sink, received.clone());
```

The sink samples data in the ReadOnly phase of the transfer's edge.

## Scoreboard

`Scoreboard<T>` compares observed items against expected items in order.
Mismatches are logged and counted; `finish()` turns them into an error.

| Method | Meaning |
|---|---|
| `Scoreboard::new("fifo")` | a named scoreboard, cheap to clone and share |
| `sb.expect(item)` | queue an expected item |
| `sb.observe(item)` | record an observed item and compare it with the next expected one |
| `sb.matched()`, `sb.pending()`, `sb.errors()` | counts and messages |
| `sb.finish()` | `Ok` only if everything matched and nothing is still expected |

```rust
let scoreboard: Scoreboard<u64> = Scoreboard::new("fifo");
scoreboard.expect(v);
scoreboard.observe(got.to_u64()?);
scoreboard.finish()
```

## Model and ModelScoreboard

A `Model<In, Out>` predicts what the design should produce from an input.
Any closure returning `Option<Out>` is a stateless model; returning `None`
means this input produces no output. `ModelScoreboard` feeds the predictions
into a `Scoreboard`.

| Method | Meaning |
|---|---|
| `ModelScoreboard::new("alu", model)` | build one |
| `sb.drive(input)` | feed an input; the prediction becomes expected |
| `sb.observe(out)` | record what the design produced |
| `sb.scoreboard()` | the underlying `Scoreboard`, for sharing with a monitor task |
| `sb.model()`, `sb.driven()`, `sb.reset()` | the model, the count, and a reset to the post-reset state |
| `sb.finish()` | `Ok` if every prediction was observed in order and nothing else |

From `examples/bus`:

```rust
let mut sb = ModelScoreboard::new("alu", |c: &Cmd| {
    Some(match c.op {
        Op::OP_NOP => 0u64,
        Op::OP_ADD => c.a + c.b,
        Op::OP_SUB => (c.a.wrapping_sub(c.b)) & 0xffff,
        Op::OP_MUL => c.a * c.b,
    })
});
let results = sb.scoreboard();
sb.drive(cmd.clone());
results.observe(dut.alu_out.get_u64()?);
sb.finish()?;
```

## A Python reference model

A team moving from cocotb often has a model already written in Python, often
built on numpy. `rivet-kit`'s optional `python` feature turns such a
function into a `Model`, so a Rivet test can score against it instead of the
model being rewritten first.

```toml
[dependencies]
rivet-kit = { version = "0.1.0", features = ["python"] }
```

The feature is off by default because enabling it links libpython into the
test library.

```rust
let model = PyModel::from_source("def step(x): return (x * 3) & 0xff", "step")?;
let mut sb = ModelScoreboard::new("alu", model);
sb.drive(7u64);
sb.observe(dut_result);
```

| Call | Meaning |
|---|---|
| `PyModel::from_source(source, function)` | compile inline source and take `function` out of it |
| `PyModel::from_file(path, function)` | the same from a file; the file's directory goes on `sys.path`, so the model can import its own helpers |
| `model.call_u64(x)?`, `model.call_slice(&xs)?` | call it directly, outside a scoreboard |
| `model.name()` | the function's name, for error messages |

`PyModel` implements `Model<u64, u64>` and `Model<Vec<u64>, Vec<u64>>`, so it
drops into `ModelScoreboard` like any other model. The rules a model may use:

- Returning `None` means this input produces no output, which is how a model
  with latency or batching is written.
- A model may keep state between calls.
- An optional module-level `reset()` is picked up automatically and called
  when the scoreboard resets.
- A missing or non-callable function is an error from `from_source` and
  `from_file`, not a panic later.

The interpreter is embedded in the test process. The call happens inside the
test's own task and the GIL is held only for the duration of the call, so
the simulator hot path is untouched. A Python model still costs an
interpreter call per item, so it is for scoring, not for per-cycle work.

## Memory

`Memory` is a sparse, page-backed byte memory shared by handle. It backs the
bus slaves and doubles as a reference model.

| Method | Meaning |
|---|---|
| `Memory::new()`, `Memory::with_fill(byte)` | empty memory; unwritten bytes read as the fill value |
| `read_u8/u16/u32/u64`, `write_u8/u16/u32/u64` | fixed-width access |
| `read_bytes`, `read_vec`, `write_bytes` | byte ranges |
| `read_word(addr, bytes)`, `write_word(addr, bytes, value, strobe)` | width and byte-strobe aware access |
| `load_hex(path, base, word_bytes)` | load a `$readmemh` file |
| `load_bin(path, base)` | load raw bytes |
| `dump_hex(path, base, words, word_bytes)` | write a hex dump |
| `pages()`, `clear()` | allocated pages, and reset |

`load_hex_into(&array_signal, path)` loads a `$readmemh` file straight into an
unpacked array in the design. `parse_readmemh` handles X and Z digits.

## Bus models

Every protocol has a signal bundle, a master and, where it makes sense, a
memory-backed slave.

| Protocol | Master | Slave or sink | Signal bundle |
|---|---|---|---|
| AXI4-Lite | `AxiLiteMaster` | `AxiLiteSlave` | `AxiLite` |
| AXI4 | `AxiMaster` | `AxiSlave` | `Axi` |
| AXI4-Stream | `AxisSource` | `AxisSink` | `Axis` |
| APB | `ApbMaster` | `ApbSlave` | `Apb` |
| Avalon-MM | `AvalonMaster` | `AvalonSlave` | `Avalon` |
| Wishbone (classic) | `WishboneMaster` | `WishboneSlave` | `Wishbone` |

Each bundle has `find(module, clk, prefix)`, which looks up `<prefix><signal>`
and leaves optional signals as `None` when absent:

```rust
let bus = AxiLite::find(&dut, clk, "s_axil_")?;   // s_axil_awaddr, s_axil_wdata, ...
let mut m = AxiLiteMaster::new(bus.clone());
m.write(0x100, 0xa5a5_0000).await?;
assert_eq!(m.read(0x100).await?, 0xa5a5_0000);
assert_eq!(m.write_strb(0x8000, 1, 0xf).await, Resp::SlvErr);
```

The shared timing convention: outputs are driven right after a rising edge,
in the values-change phase, and inputs are sampled at the next rising edge,
so a handshake completes on the edge where `valid` and `ready` were both high
during the preceding cycle.

Responses use `Resp`: `Okay`, `ExOkay`, `SlvErr`, `DecErr`.

AXI4 supports bursts and the three burst types, narrow transfers and IDs:

```rust
let resp = m.write_burst(addr, &data, 2, AxiBurst::Incr).await?;
let (rd, resp) = m.read_burst(addr, beats - 1, 2, AxiBurst::Incr).await?;
let a = rivet::kit::bus::axi::beat_address(addr, 2, beats - 1, burst, i as u32);
```

A memory-backed slave runs as a pair of tasks and can report errors for a
range:

```rust
let mem = Memory::new();
let _slave = AxiLiteSlave::new(q, mem.clone())
    .backpressure(Backpressure::random(0, 3), Backpressure::random(0, 3))
    .error_range(0x8000, 0x9000)
    .run();
```

## Backpressure

Slaves and stream endpoints stall according to a `Backpressure` policy.

| Variant | Meaning |
|---|---|
| `Backpressure::None` | accept immediately |
| `Backpressure::Fixed(n)` | always stall `n` cycles |
| `Backpressure::random(min, max)` | uniform in `min..=max`, drawn from the test's seeded stream, so runs replay |
| `Backpressure::Custom(f)` | `f(index) -> cycles`, decided per item |

`AxisSource::gaps(policy)` inserts idle cycles between beats;
`AxisSink::backpressure(policy)` stalls `tready`.

## Reset

`Reset` is a builder. The defaults are active high, synchronous, asserted for
two rising edges, with one settling edge after release. It returns just after
a rising edge, in the values-change phase, with the reset released.

| Method | Meaning |
|---|---|
| `Reset::new(clk, rst)` | start configuring |
| `.active_low()` / `.active_high()` | polarity |
| `.synchronous()` | asserted and released just after rising edges |
| `.asynchronous()` | released half a cycle later, on a falling edge, so release never races the sampling edge |
| `.cycles(n)` | rising edges the reset stays asserted for |
| `.settle(n)` | rising edges to wait after release |
| `.apply().await` | run the sequence |

```rust
Reset::new(clk, dut.signal("rst_n")?).active_low().cycles(3).settle(1).apply().await;
```

The free function `reset(clk, rst, active_low, cycles).await` is the short
form for the common case.

## Checkers

Two kinds. Immediate checkers are awaited and return a `Result`; background
checkers are spawned and fail the test the moment they see a violation.
Cancel the returned handle to stop one.

| Checker | Kind | Meaning |
|---|---|---|
| `assert_stable(clk, sig, cycles)` | immediate | `sig` keeps its value over the next `cycles` rising edges |
| `assert_within(clk, cycles, pred)` | immediate | `pred` holds at some rising edge within the next `cycles` |
| `assert_becomes(clk, sig, value, cycles)` | immediate | `sig` becomes `value` within `cycles` edges |
| `assert_never(name, clk, pred)` | background | fails the first time `pred` is true at a rising edge |
| `assert_always(name, clk, pred)` | background | fails the first time `pred` is false at a rising edge |
| `assert_implies(name, clk, antecedent, consequent, within)` | background | whenever the antecedent holds, the consequent must hold at that edge or within `within` edges; overlapping windows each get their own deadline |
| `assert_no_x(name, clk, signals)` | background | fails if any signal carries an X or Z bit at a rising edge; start it after reset |
| `find_x(clk, signals, cycles)` | immediate | samples every edge and returns the first X or Z as `Option<(Signal, LogicVec, u64)>` instead of failing the test |

The immediate checkers return at the start of the next time step, so it is
safe to write straight afterwards.

```rust
let _no_x = assert_no_x("axil_outputs", clk, vec![bus.bvalid, bus.rvalid, bus.awready, bus.wready]);

let _accepted = assert_implies(
    "input_accepted",
    clk,
    move || s.tvalid.get_u64_lossy() == 1,
    move || s.tvalid.get_u64_lossy() == 1 && s.tready.get_u64_lossy() == 1,
    8,
);

assert_stable(clk, alu_out, 5).await?;
rivet::kit::assert_becomes(clk, alu_out, 7, 3).await?;
```

## Golden traces

A `Trace` collects lines of text and compares them with a golden file.

| Method | Meaning |
|---|---|
| `Trace::new("axil")` | a trace whose entries carry time stamps relative to its creation |
| `Trace::unstamped("axil")` | no time stamps, for designs whose timing differs between simulators |
| `trace.record(item)` | append anything that implements `Display` |
| `trace.text()`, `trace.len()` | the recorded text and the line count |
| `trace.golden_path()?` | where the golden file for this test lives |
| `trace.check_golden()?` | compare, or write the file under `RIVET_UPDATE_GOLDEN=1` |
| `assert_trace!(trace)` | `check_golden` with the error returned from the enclosing test |

```rust
let mut trace = Trace::new("axil");
trace.record(format!("W {addr:#04x} <- {data:#010x}"));
rivet::kit::assert_trace!(trace);
```

The golden file is
`$RIVET_GOLDEN_DIR/<module>_<test>[@<param_set>]__<trace>.trace`, and
`rivet run` sets `RIVET_GOLDEN_DIR` to `<crate>/golden`. So the trace from
`axil_golden_trace` under parameter set `init5` lands at
`golden/example_bus_axil_golden_trace@init5__axil.trace`.

Goldens are per parameter set because the design differs between sets. Time
stamps are relative to the trace's creation because a test's absolute start
time depends on what ran before it and on how the run was sharded.

When a golden is missing or differs, the actual text is written next to it
with a `.trace.actual` extension and the error names both paths. Accept a new
one with `rivet run --update-golden`, which sets `RIVET_UPDATE_GOLDEN=1`.
