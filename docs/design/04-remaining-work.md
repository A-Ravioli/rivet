# Remaining work: what it takes to close each gap

This is the concrete plan for everything `03-status.md` marks as partial,
unverified, or not started. Each item says what is missing, why it matters,
exactly what to build or do, how it gets verified, what it costs, and what
(if anything) has to come from outside this repository.

Ordering is by leverage: items whose verification is available now come
first; items blocked on licences come last, with the preparation that can
be done without them.

## 0. Findings from the internal review of the current code

An internal code review of the last commit found three things. All are
small; the first is a real semantic bug.

1. **Verilator direct writes ignored inertial semantics.** On Verilator
   5.036+, the VPI backend trusted `vpiInertialDelay`, so the runtime
   handed deposits straight to the backend; the direct-access path then
   stored them immediately, so `sig.set(1); sig.get()` read `1` on
   Verilator and `0` on Icarus. Also, honouring `vpiInertialDelay` on 5.036+
   needs the loop to call `VerilatedVpi::doInertialPuts`, which Rivet's
   loop did not. Fix applied: Verilator never trusts inertial writes; the
   runtime buffers deposits until ReadWrite and applies them as immediate
   writes, which is identical on every Verilator version and on the direct
   and VPI paths. Verified on 5.020 and 5.036.
2. **O(n²) deposit dedup.** Replacing the write index with a linear scan
   made loading an N-entry memory O(N²) per timestep. Fix applied: the
   index is back as an `FxHashMap`.
3. **Duplicated X/Z-to-zero logic** in the direct write path. Fix applied:
   scalar writes go through `LogicVec::to_u64_lossy`.

Also found while testing against Verilator 5.036: `VerilatedVar`'s range
accessors were renamed between 5.020 and 5.036 (`packed().elements()` to
`elements(0)`), and generated code is not portable across versions. The
shim now has a version guard and the build helper clears `obj_dir` when
the Verilator version changes.

## 1. VHPI backend (NVC now; Riviera, Xcelium VHDL, Questa VHPI later)

**Why.** VHDL users on anything but GHDL need VHPI. NVC is the open-source
simulator cocotb tests VHPI against, and it now builds from source in this
container (1.23-devel with `vhpi_user.h`), so this is verifiable here.

**What to build: `crates/rivet-vhpi`.**

- `ffi.rs`: hand-written bindings for the sixteen functions cocotb uses:
  `vhpi_get`, `vhpi_get_str`, `vhpi_handle`, `vhpi_handle_by_name`,
  `vhpi_handle_by_index`, `vhpi_iterator`, `vhpi_scan`,
  `vhpi_release_handle`, `vhpi_get_value`, `vhpi_put_value`,
  `vhpi_register_cb`, `vhpi_remove_cb`, `vhpi_get_time`, `vhpi_get_phys`,
  `vhpi_control`, `vhpi_check_error`; the structs `vhpiValueT` (with its
  union and the `numElems`/`bufSize` fields), `vhpiCbDataT`, `vhpiTimeT`,
  `vhpiPhysT`; and the enums for class kinds, properties, one-to-many
  relations, value formats, put modes, and callback reasons, taken from
  cocotb's vendored `vhpi_user.h`.
- `VhpiBackend` implementing `Backend`, porting cocotb's `VhpiImpl.cpp`
  logic (analysis §3.6 has the full quirk list):
  - Type classification by *base type*: `vhpiBaseType` with the
    `vhpiSubtype` fallback, then enum-content tests (`is_enum_logic` by
    name `BIT`/`STD_ULOGIC`/`STD_LOGIC` or by the literal set, `is_enum_char`
    for 256 literals, `is_enum_boolean`), arrays via `vhpiElemType` with
    the `vhpiElemSubtype` fallback, records to `ObjKind::Struct`, integers,
    reals, strings.
  - Values: `vhpiLogicVecVal` for vectors, converting each `vhpiEnumT`
    element (`vhpi0`, `vhpi1`, `vhpiZ`, `vhpiX`, and `U`/`W`/`L`/`H`/`-`)
    to `aval`/`bval` bits and back with lookup tables; `vhpiIntVal`,
    `vhpiRealVal`, `vhpiStrVal` for scalars and strings; the
    "zero-length array returns empty" guard.
  - Writes: `vhpiDepositPropagate` for both `Deposit` and `NoDelay`
    (VHPI has no immediate/inertial distinction), `vhpiForcePropagate`,
    `vhpiRelease`.
  - Callbacks: `vhpiCbValueChange` (persistent), `vhpiCbAfterDelay`,
    `vhpiCbRepLastKnownDeltaCycle` for ReadWrite,
    `vhpiCbRepEndOfTimeStep` for ReadOnly, `vhpiCbRepNextTimeStep`;
    remove repetitive callbacks after each fire as cocotb does.
  - Hierarchy: `vhpiInternalRegions`, `vhpiSigDecls`, `vhpiVarDecls`,
    `vhpiPortDecls`, `vhpiGenericDecls`, `vhpiConstDecls`,
    `vhpiCompInstStmts`, `vhpiBlockStmts`; records via `vhpiSelectedNames`;
    arrays via `vhpiIndexedNames` with the linear-scan fallback; skip
    process and signal-assignment statements; the three-stage root
    discovery with the `:` prefix.
  - Time: `vhpi_get_phys(vhpiResolutionLimitP)` for precision.
  - NVC quirks: case-insensitive name comparison, upper-cased full
    names. Aldec/Xcelium/Questa quirks from the analysis table go in
    behind a `Sim` enum like the VPI backend's.
  - Export `vhpi_startup_routines` (and a `_bootstrap` symbol for
    Riviera/Active-HDL).
- Runner: `rivet run --sim nvc`: `nvc -a --std=08 sources; nvc -e top;
  nvc -r top --load=lib.so`, mirroring cocotb's `runner.py:1534-1630`
  including the `entity-arch` top spelling and `--preserve-case` on 1.16+.
- `rivet` facade: `vhpi` feature; the test crate's `cdylib` links both
  backends so one artifact serves VPI and VHPI simulators.

**Verify.** `examples/dff_vhdl` on NVC (reusing the tests written for
GHDL), plus a VHDL design with a record port, an unconstrained array
generic, an enum, and a `for generate`, since those are where VHPI type
classification breaks. Add NVC to CI by building it from source with a
cached tarball, or via the `ghdl/vunit` style Docker images that ship it.

**Cost.** About 1500 lines. The FFI and value transport are half a day;
type classification and hierarchy are the other half; NVC quirks a few
hours. Nothing needed from outside.

## 2. Mixed-language designs

**Why.** Verilog tops with VHDL blocks (or the reverse) are common in
industry and are what `GPI_EXTRA` exists for in cocotb.

**What to build.** The runtime holds one `Backend`. Add a
`CompositeBackend` that owns a primary and secondary backend and routes:
`root` from the primary; `child_by_name` tries the owning backend, then
the other; handles carry a backend tag in their high bits; callbacks and
time come from the primary (as cocotb's `registered_impls[0]` does). The
cross-boundary hand-off is by fully qualified name, since raw handles are
not shared.

**Verify.** Only possible on a simulator that runs both languages in one
process: Questa, Xcelium, Riviera. GHDL and NVC are VHDL-only. So this is
buildable now but blocked on §5 for verification; keep it small until
then.

**Cost.** About 300 lines once VHPI exists.

## 3. FLI backend (Questa VHDL)

**Why.** cocotb keeps FLI because it is faster than Questa's VHPI. It is
also the only way to reach VHDL variables on Questa in some versions.

**What to build.** `crates/rivet-fli`: bindings for the 57 `mti_*`
functions cocotb uses plus Tcl for argv; the process-based callback model
(`mti_CreateProcessWithPriority` with priority as the phase, a permanent
process pool because processes cannot be destroyed, `m_removed` squashing
because wakeups cannot be cancelled); value access per type class through
enum-index buffers; force via radix strings. Roughly 2000 lines and,
unlike VHPI, nothing to test it on without Questa.

**Decision.** Do not build FLI until Questa is available (§5). Questa's
VHPI path through §1 covers VHDL on Questa first.

## 4. Verilator: newer versions and remaining performance

**Status now.** 5.020 (apt) and 5.036 (from source) both pass every
example after the fixes in §0. cocotb 2.1 runs on 5.036, which gives the
Verilator baseline the benchmark doc was missing:

| Test | cocotb 2.1 on Verilator 5.036 | Rivet on Verilator 5.036 |
|---|---|---|
| edge round trip | 9.34 µs | 1.89 µs |
| edge + ReadOnly + read | 25.9 µs | 2.62 µs |
| 32-bit write, 32-bit + 512-bit read | 371 µs | 2.81 µs |
| 100 tasks awaiting every edge | 366 µs | 20.2 µs |

**To do.**

- CI matrix over Verilator versions: apt 5.020 and a source-built 5.036+
  cached by version key (about 8 minutes to build on 4 cores, cached
  thereafter).
- `--timing` designs: a test with `#delay` and `wait` in the HDL so
  `eventsPending`/`nextTimeSlot` are exercised; today no example uses
  them.
- Force/Release on Verilator: `Capabilities::supports_force` is false. Check
  what 5.036's VPI supports (`vpiForceFlag` landed in the 5.0xx series)
  and either implement force through the direct path (a forced-value
  overlay re-applied after every `eval_step`) or document the limit.
- Edge detection without VPI for harness-driven clocks: the clock task
  knows when it toggled, so `RisingEdge` waiters on that signal can be
  woken from the write instead of from `callValueCbs`. Removes one VPI
  scan per half period.
- Two `eval_step` calls per half period today (after the timer and after
  the ReadWrite flush); with runtime-buffered deposits the first is only
  needed if the timer callbacks wrote with `NoDelay`. Track a
  "wrote since eval" flag and skip.
- `many_tasks` costs about 0.2 µs per task wake, mostly `Arc` waker
  traffic and the `Mutex` on the ready queue. A custom `RawWaker` carrying
  the task key with no allocation, and an `UnsafeCell` ready queue (the
  executor is single-threaded by construction) would roughly halve it.

## 5. Commercial simulators (Questa, Xcelium, VCS, Riviera, DSim)

**What exists.** The VPI backend carries cocotb's quirks for each
(Xcelium startup via `cbAfterDelay(0)`, handle release after fire on
Xcelium/VCS/Aldec, string deposits as `vpiNoDelay` on Questa/Xcelium,
callback re-entrancy) and detects the simulator from `vpi_get_vlog_info`.
None of it has executed.

**What is needed from outside.** A licence. Two are free:

- **Questa Intel FPGA Starter Edition** (free licence from Intel, Linux
  x86_64). Gives VPI, VHPI, and FLI on one tool, so it verifies §1, §2,
  and §3 too. This is the single most valuable download.
- **DSim Desktop** from Metrics (free for non-commercial use). VPI only.

Xcelium, VCS, and Riviera need a commercial licence or a partner with
one.

**What to prepare now, without a licence.**

- Runner flows in `rivet-cli`, ported from cocotb's `runner.py` table in
  the analysis §5.1: Questa `vlog`/`vopt`/`vsim -pli`, `-foreign` for
  FLI/VHPI, `-voptargs=-access=rw+/.`; Xcelium `xrun -loadvpisim
  lib:vlog_startup_routines_bootstrap -access +rwc`; VCS `vcs -load lib
  +acc+3 -debug_access+all`; Riviera `alog`/`asim -pli` in a generated
  `.do`; DSim `dsim -pli_lib`.
- A **conformance crate** (`examples/conformance`): one design and one
  test set that touches every backend feature with the expected outcome
  per simulator recorded in a table: edges, each phase trigger, deposit
  visibility (cocotb's `test_inertial_writes` cases), force/release,
  unpacked arrays, packed structs, generate blocks, parameters,
  integers/reals/strings, `$finish` from HDL, timeouts. Someone with a
  licence runs `rivet run --sim questa -C examples/conformance` and sends
  back `results.xml` and the log; each row then moves from "code only" to
  "verified" or becomes a bug report with a repro.
- A `SIMULATOR-QUIRKS.md` listing every workaround, its cocotb source
  line, and its verification status, as the roadmap already asked for.

**Cost.** Runner flows and the conformance crate: a day. Verification:
whatever the licence takes.

## 6. Benchmarks with statistical treatment and a real design

- Repeat each benchmark five times and report median and spread; make the
  `bench` example print machine-readable lines and add a small script
  that aggregates them.
- Add a realistic design: a single-file RISC-V core (PicoRV32 is ISC
  licensed and one Verilog file) running a small program for 100k cycles,
  with a monitor task on the memory bus. Harness overhead as a fraction
  of a real simulation is the number people actually want.
- A CI benchmark job on Icarus and Verilator with a regression threshold
  (fail if `edge_roundtrip` exceeds twice the recorded baseline) and the
  raw numbers uploaded as an artifact per run.
- Debug-profile numbers alongside release, since that is what
  `cargo test` runs by default.

## 7. Kit and API completeness

Done since this plan was written: per-test seeded RNG and `Randomize`,
bus models (AXI4-Lite, AXI4, AXI4-Stream, APB, Avalon-MM, Wishbone) with
memory-backed slaves and backpressure, the `Memory` model, wall-clock
timeouts with a watchdog, functional coverage, golden traces, checkers,
sharded runs, parameter sets, typed enums and packed structs in `bindgen`.

Still open:

- Struct members: `Object::children` uses module relationships; struct
  variables need `vpiMember` iteration, and `bindgen` should emit nested
  views for them (today packed structs are decoded from the vector).
- Multi-dimensional unpacked arrays in `bindgen` (today only one
  dimension).
- `results.xml` `file`/`line` attributes from `file!()`/`line!()` in the
  macro.
- Better phase errors: writing in ReadOnly should say "await
  `next_time_step()` or an edge first" rather than only naming the phase.
- `cargo nextest` compatibility check (needs `--list --format terse` and
  per-test process isolation semantics; may need a `nextest.toml`).
- Edalize flow API (`edalize.tools`) alongside the legacy tool backend.
- Constrained random beyond rejection sampling (a small solver for linear
  constraints) and coverage-driven stimulus.

## 8. Platforms

- macOS: `cdylib` needs `-undefined dynamic_lookup`; Icarus loads
  `.vpi` regardless of the source extension; Verilator link group flags
  differ (`-Wl,--start-group` is GNU ld only, use `-Wl,-all_load` or
  repeat the archives). Add a macOS CI job with Homebrew `icarus-verilog`
  and `verilator`.
- Windows: cocotb builds import libraries from `.def` files per simulator
  because DLLs cannot have undefined symbols. Rivet would need the same
  (`share/def` in cocotb is reusable). Low priority until a user asks.

## 9. Review

- Internal: the code review above found three issues, now fixed. A
  `security-review` pass over the FFI crates (`rivet-vpi`, `rivet-verilator`)
  is worth running: they hold raw pointers from simulator memory and
  `unsafe` blocks that trust simulator-reported widths.
- External: the PR is a draft. A reviewer who knows cocotb should check
  three things in particular: the timing-model tests in `rivet-mock`
  against their own mental model, the write-buffering rules in
  `runtime.rs`, and the per-simulator quirk table for anything cocotb
  carries that Rivet dropped.

## Suggested order

1. §0 fixes (done in this commit) and the Verilator matrix in CI.
2. §1 VHPI on NVC, then §7's RNG and struct members (small, unblock
   users).
3. §5's runner flows and conformance crate, so a licence holder can
   verify in one command.
4. §6 benchmarks with a real design.
5. §2, §3, §8 as demand appears.
