# Simulator quirks

Every simulator-specific behaviour Rivet carries, where it came from, where it
lives in this repository, and whether it has ever run.

Rivet's backends are ports of cocotb's GPI layer. cocotb's C++ sources are full
of per-simulator guards, each one a fact about a simulator rather than a fact
about cocotb. `docs/design/00-cocotb-analysis.md` §3.6 catalogues them with
source line references. This document is the other half of that table: what
Rivet does about each one.

Rivet is developed against Icarus, Verilator and GHDL, which are the three
simulators CI can install. Everything aimed at a commercial simulator is
written from cocotb's source and has never been executed. If you hold a licence
for Questa, Xcelium, VCS, Riviera or DSim, you are the first person to run this
code. Read the tables below, run `examples/conformance` on your tool (see "How
to verify on your simulator"), and turn each "code only" row into either a
"verified" row or a bug report. A bug report is most useful when it names the
row, the simulator and version, and what the conformance test printed.

## Verification status

| Status | Meaning |
|---|---|
| verified | A test or example in `.github/workflows/ci.yml` runs this code path on that simulator. |
| code only | The code exists but nothing in CI runs it. True for every commercial simulator, and for a few fallback branches that free simulators never take. |
| not implemented | Rivet does not carry the behaviour. The Rivet column says what it does instead, where that matters. |

Line references are to the current branch. cocotb references are the ones
recorded in `docs/design/00-cocotb-analysis.md`; the section number says where
in that document the reference came from.

## VPI

Icarus, Verilator, Questa Verilog, Xcelium, VCS, Riviera, DSim and GHDL all
reach Rivet through `crates/rivet-vpi`. Verilator adds `crates/rivet-verilator`
on top, which owns `main` and delegates hierarchy and values to the VPI
backend.

### From the cocotb catalogue (§3.6)

Where Rivet applies a cocotb-guarded workaround to every simulator instead of
one, the Simulator column says "all" and the Behaviour column names cocotb's
guard.

| Simulator | Behaviour | cocotb source (§3.6) | Rivet implementation | Verification status |
|---|---|---|---|---|
| GHDL | Do not ask for `vpiPacked`, `vpiConstType` or `vpiSigned`. VHDL has no packed/unpacked distinction and GHDL warns on properties it does not know. | `VpiImpl.cpp:94-100` | `crates/rivet-vpi/src/lib.rs:191`, `:205`, `:226` | verified |
| all (cocotb: Xcelium) | A constant whose type is neither real nor string classifies as a vector, including the `vpiUndefined` that Xcelium reports. | `VpiImpl.cpp:163-169` | `crates/rivet-vpi/src/lib.rs:204-211` | verified |
| Xcelium | Validate a `vpiGenScope` by iterating internal scopes first. Xcelium segfaults on an invalid scope and never returns `vpiGenScopeArray`. | `VpiImpl.cpp:382-404` | not implemented. Rivet never asks for `vpiGenScopeArray` on any simulator, so the crashing call is not made: generate arrays always come from the name probe at `crates/rivet-vpi/src/lib.rs:449-478`. | not implemented |
| all (cocotb: every simulator except Xcelium) | Generate-prefix fallback. `gen[0]` resolves but `gen` does not, so return the parent handle typed as a generate array and index it by name. | `VpiImpl.cpp:406-452` | `crates/rivet-vpi/src/lib.rs:449-478`, indexed at `:497-512` | verified |
| Questa | Hard-coded element counts per type, because Questa reports `vpiSize == 1` for all scalar types. | `VpiImpl.cpp:230-258` | not implemented. `classify` trusts `vpiSize` (`crates/rivet-vpi/src/lib.rs:184`, `:212-219`), so scalar widths on Questa are unverified. | not implemented |
| Questa, VCS | The `vpiRange` iterator restarts at dimension 0, so scan forward to the dimension you want. | `VpiObj.cpp:46-54` | not implemented. Rivet reads `vpiLeftRange` and `vpiRightRange` handles directly and never opens a `vpiRange` iterator (`crates/rivet-vpi/src/lib.rs:227-246`). Multi-dimensional ranges are not modelled. | not implemented |
| Questa | `vpi_handle_by_index` returns NULL on 2-D arrays, so fall back to a name lookup. | `VpiImpl.cpp:504-518` | `crates/rivet-vpi/src/lib.rs:499-509`, applied on every simulator | code only |
| Questa, Xcelium | Deposits to a `vpiStringVar` must use `vpiNoDelay`. | `VpiSignal.cpp:195-206` | `crates/rivet-vpi/src/lib.rs:627-635` | code only |
| all (cocotb: Icarus) | `vpi_control` returns void on Icarus, so do not check its return value. | `VpiImpl.cpp:763-767` | `crates/rivet-vpi/src/lib.rs:750-754`, which ignores the return value everywhere | verified |
| VCS | VCS loads the plugin at compile time as well as at run time, so bail out when `vpi_get_vlog_info` fails. | `VpiImpl.cpp:843-852` | not implemented. A failed `vpi_get_vlog_info` leaves the product and version empty and startup continues (`crates/rivet-vpi/src/lib.rs:91-97`). Expect the compile-time load to try to start a regression. | not implemented |
| Xcelium | Skip root objects whose full name starts with a backslash. They are virtual classes placed at the top scope. | `VpiImpl.cpp:635-639` | `crates/rivet-vpi/src/lib.rs:405-411` | code only |
| Icarus, Xcelium, Questa | Callback re-entrancy queue. These simulators fire value-change callbacks from inside a `vpiNoDelay` write, while the harness is still inside another callback. | `VpiCbHdl.cpp:45-69` | `crates/rivet-core/src/runtime.rs:115-123` and `:446-463`: a `REACTING` flag and a deferred queue drained after the outer event. Exercised on Icarus in CI. | verified |
| Verilator | Remove a one-shot callback after it fires. Verilator treats callbacks as recurring that other simulators free. | `VpiCbHdl.cpp:109-116, 145-161` | `crates/rivet-vpi/src/lib.rs:866-876`, with the capability flag at `:353` and `crates/rivet-core/src/backend.rs:138-140`. Under the Verilator backend only value-change callbacks go through VPI; the rest are native (`crates/rivet-verilator/src/sched.rs:320-341`). | verified |
| Xcelium, VCS, Riviera | Call `vpi_release_handle` after a callback fires, which the VPI spec does not require. | `VpiCbHdl.cpp:162-166` | not implemented. Rivet never calls `vpi_release_handle`. Watch for handle-table growth on these three. | not implemented |
| Xcelium | The startup callback must be `cbAfterDelay(0)`. Xcelium does not deliver `cbStartOfSimulation` to a late-loaded library. | `VpiCbHdl.cpp:236-243` | `crates/rivet-vpi/src/lib.rs:959-966` | code only |
| Riviera | Remove `vpiModule`, `vpiModuleArray`, `vpiInterface` and `vpiInterfaceArray` from the iteration lists. Aldec segfaults on mixed-language designs. | `VpiIterator.cpp:25-41` | not implemented. One iteration list serves every simulator and it includes `vpiModule` (`crates/rivet-vpi/src/lib.rs:316-329`). `vpiModuleArray` and the interface relations are not iterated, so only `vpiModule` is a risk. | not implemented |
| Xcelium | Exclude `vpiNetArray` from struct iteration. | `VpiIterator.cpp:51-53` | not implemented. Structs iterate the same list as modules (`crates/rivet-vpi/src/lib.rs:316-329`, `:515-543`). | not implemented |
| Questa | Iterate `vpiInstance` rather than `vpiPackage`. | `VpiImpl.hpp:266-275` | not implemented. Rivet classifies a package (`crates/rivet-vpi/src/lib.rs:189`) but never iterates package contents; `vpiInstance` is declared in `crates/rivet-vpi/src/ffi.rs:60` and unused. | not implemented |
| all (cocotb: Xcelium) | Skip iterated objects that have no name. | `VpiIterator.cpp:228-245` | `crates/rivet-vpi/src/lib.rs:530-534` | verified |
| all | Never remove the startup and shutdown callbacks. Too many simulators object. | `VpiImpl.hpp:127-168` | `crates/rivet-vpi/src/lib.rs:940-972`. They are `static` `s_cb_data` with no matching remove, and `remove` only touches the callback map (`:734-748`). | verified |

### Other simulator-specific behaviour in the VPI path

These come from elsewhere in the analysis (§2.2 write scheduling, §3.2 the
value path, §3.3 callback ownership, §3.4 discovery, §3.5 Verilator, §5.1
plugin loading) or from Rivet's own work on the three free simulators.

| Simulator | Behaviour | cocotb source | Rivet implementation | Verification status |
|---|---|---|---|---|
| Verilator | Never trust inertial writes. Deposits buffer until ReadWrite and are then applied with `vpiNoDelay`. Honouring `vpiInertialDelay` on 5.036+ would need the loop to call `VerilatedVpi::doInertialPuts`, and would differ from the direct-access path. | §2.2 `handle.py:769-820`, `runner.py:1414-1417` | `crates/rivet-vpi/src/lib.rs:119-130` | verified |
| GHDL | Trust inertial writes. Writes go straight to the simulator instead of the buffer. | §2.2 `runner.py:1560-1563` | `crates/rivet-vpi/src/lib.rs:126-130`, `crates/rivet-core/src/runtime.rs:699-703` | verified |
| Icarus | Do not trust inertial writes. Deposits buffer until the ReadWrite phase, latest write per handle wins. | §2.2 `handle.py:769-820` | `crates/rivet-core/src/runtime.rs:699-720` | verified |
| Questa, Xcelium, VCS, Riviera, DSim | Same buffering as Icarus, by default for every simulator that is not GHDL or Verilator. `RIVET_TRUST_INERTIAL_WRITES=1` turns it off. | §2.2 `Makefile.inc:138-147` | `crates/rivet-vpi/src/lib.rs:126-130` | code only |
| GHDL | Move values as binary strings. GHDL's VPI has no `vpiVectorVal`. | §3.2 `VpiSignal.cpp:103-110` | `crates/rivet-vpi/src/lib.rs:141`, `:589-595`, `:642-649` | verified |
| all | Fall back to a binary string when the width is unknown or the simulator returns a NULL vector pointer. | §3.2 | `crates/rivet-vpi/src/lib.rs:271-279`, `:585-588`, `:600-604` | code only |
| GHDL | A lookup of an unknown child answers with the scope itself. Report that as missing. | none, found in this repository | `crates/rivet-vpi/src/lib.rs:481-485` | verified |
| all | Build the path from parent path plus name rather than trusting `vpiFullName`. Simulators disagree, and Verilator reports top-level ports under a `TOP` scope. | §3.4 `GpiCommon.cpp:26-61` | `crates/rivet-vpi/src/lib.rs:151-181` | verified |
| all | Retry a failed child lookup with the fully qualified name. Some simulators only resolve those. | §3.4 | `crates/rivet-vpi/src/lib.rs:443-448` | code only |
| Icarus | Read time precision from the first top-level module. Icarus answers 0 for a NULL object, and nothing answers before elaboration. | none, found in this repository | `crates/rivet-vpi/src/lib.rs:363-385` | verified |
| Icarus | Waveform on and off through a generated `rivet_dump` module holding `$dumpon` and `$dumpoff`. One file per run; a new file per test is refused. | §5.1 `runner.py:932-950` | `crates/rivet-vpi/src/lib.rs:756-785`, generated by `crates/rivet-cli/src/lib.rs:328-337` | verified |
| Icarus | Bindgen skips the internal scopes Icarus exposes (`$ivl_for_loop0`, `$unm_blk_3`). They exist on no other simulator, so bindings that include them do not port. | none, found in this repository | `crates/rivet-cli/src/bindgen.rs:70-74` | verified |
| Xcelium, CVC | Export a `vlog_startup_routines_bootstrap` symbol. Xcelium loads a plugin as `<lib>:vlog_startup_routines_bootstrap`. | §5.1 plugin-loading table | `crates/rivet-vpi/src/lib.rs:980-994` | code only |
| Verilator | Two-state values and no force or release. Tests read this from the capability flags and skip. | §3.5 | `crates/rivet-vpi/src/lib.rs:354-355`, used at `examples/conformance/src/lib.rs:14-15`, `:353` | verified |
| Verilator | Rivet owns `main` and runs the region loop itself: evaluate to a fixpoint, ReadWrite, evaluate again if anything was written, `eval_end_step`, ReadOnly, dump, then jump to the next deadline. The simulation ends when nothing is pending. | §3.5 `verilator.cpp:174-231` | `crates/rivet-verilator/src/lib.rs:136-195`, settle loop at `:75-81`, deadline choice at `:176-183` | verified |
| Verilator | `Verilated::fatalOnVpiError(false)`, otherwise the model aborts on system tasks. | §3.5 `verilator.cpp:154` | `crates/rivet-verilator/src/build.rs:308` | verified |
| Verilator | Close the trace object but never delete it. | §3.5 `verilator.cpp:66-71` | `crates/rivet-verilator/src/build.rs:365-372` | verified |
| Verilator | Verilate with `--cc --vpi --public-flat-rw`. Without `--public-flat-rw` internal signals are neither visible nor writable. | §3.5 `runner.py:1919-1998` | `crates/rivet-verilator/src/build.rs:193` | verified |
| Verilator | Keep timers and phase callbacks in a Rust timer wheel instead of registering `cbAfterDelay`. Rivet owns the loop, so it can schedule them directly. | §3.5 | `crates/rivet-verilator/src/sched.rs:320-341`, wheel at `:114-151` | verified |
| Verilator | Read and write model storage directly through the symbol table. A top-level port exists twice, so root-level names resolve the `TOP` scope first or writes land on the alias. | none, found in this repository | `crates/rivet-verilator/src/sched.rs:178-225`, `:193-201`; shim at `crates/rivet-verilator/src/build.rs:330-345` | verified |
| Verilator | Mark parameters constant from the symbol table's `isParam`. Verilator's VPI types parameters as ordinary variables, so writes would otherwise be accepted. | none, found in this repository | `crates/rivet-verilator/src/sched.rs:168-176`, `crates/rivet-vpi/src/lib.rs:256-258` | verified |
| Verilator | Version guard for the `VerilatedVar` range accessors, renamed between 5.020 and 5.036. Generated code is not portable across versions, so the object directory is cleared when the version changes. | none, found in this repository | `crates/rivet-verilator/src/build.rs:338-342` | verified |
| all | Identify the simulator from the `vpi_get_vlog_info` product string, then select every quirk from that. | §3.6 | `crates/rivet-vpi/src/lib.rs:84-117` | verified |
| all | A callback whose removal fails is flagged and squashed when it fires, instead of dereferencing freed memory. | §3.3 `VpiCbHdl.cpp:135-174` | `crates/rivet-vpi/src/lib.rs:734-748`, checked at `:853-857` | code only |

## VHPI

There is no `crates/rivet-vhpi` in this branch. A VHPI backend is being written
in parallel; the plan, including which of these rows it covers first, is
`docs/design/04-remaining-work.md` §1. Until it lands, VHDL reaches Rivet only
through GHDL's VPI, which is why the GHDL rows above are in the VPI table.

Every row here is therefore "not implemented". They are listed so the new
backend can be checked against the same catalogue.

| Simulator | Behaviour | cocotb source (§3.6) | Rivet implementation | Verification status |
|---|---|---|---|---|
| Xcelium | `vhpiIsUnconstrainedP` is unset, so compare bounds against the magic value 2147483647. | `VhpiObj.cpp:28-34, 60-64`; `VhpiImpl.cpp:719-751` | not implemented | not implemented |
| Xcelium | The unconstrained flag is wrong on the base type; retry on the subtype. | `VhpiImpl.cpp:773-779` | not implemented | not implemented |
| Questa | `vhpiIsUpP` is wrong (cocotb #4236); infer direction from `left < right`. | `VhpiObj.cpp:68-80, 111-120` | not implemented | not implemented |
| Riviera | Generate index separator is `__n`, not `(n)`. | `VhpiImpl.hpp:26-34` | not implemented | not implemented |
| Riviera | Compare enum literals both with and without quotes; Aldec omits them. | `VhpiImpl.cpp:174-201` | not implemented | not implemented |
| Riviera | `vhpiRootInstK` is a null hierarchy level. | `VhpiIterator.cpp:120-123` | not implemented | not implemented |
| NVC | Compare names case-insensitively (nvc#723). | `VhpiImpl.cpp:274-285` | not implemented | not implemented |
| NVC | Upper-case all fully qualified names. | `VhpiImpl.cpp:516-519` | not implemented | not implemented |
| all | `vhpiBaseType`, then `vhpiSubtype`, then `vhpiBaseType` again. Five sites. | `VhpiImpl.cpp:301-312` | not implemented | not implemented |
| all | `vhpiElemType` with a `vhpiElemSubtype` fallback. | `VhpiImpl.cpp:327-332` | not implemented | not implemented |
| all | Record children through `vhpiSelectedNames`. `vhpi_handle_by_name` fails for records inside generics. | `VhpiImpl.cpp:559-578` | not implemented | not implemented |
| all | `vhpi_handle_by_index` with a linear scan of `vhpiIndexedNames` as fallback. | `VhpiImpl.cpp:833-858` | not implemented | not implemented |
| all | Zero-length arrays return an empty string without calling the simulator. | `VhpiSignal.cpp:465-473` | not implemented | not implemented |
| all | Three-stage root discovery, with a `:`-prefixed lookup to disambiguate library objects. | `VhpiImpl.cpp:884-982` | not implemented | not implemented |
| all | `Deposit` and `NoDelay` both map to `vhpiDepositPropagate`. VHPI has no inertial/immediate distinction. | `VhpiSignal.cpp:35-52` | not implemented | not implemented |
| all | VHDL type mapping is by the content of enum types, not by kind: name or literal set decides logic, character and boolean. `bit` and `std_logic` are indistinguishable at this level. | `VhpiImpl.cpp:161-272` | not implemented | not implemented |

## FLI

FLI is Questa's VHDL interface. There is no FLI backend and none is planned
until a Questa licence is available; `docs/design/04-remaining-work.md` §3
records the decision and the reason, which is that Questa VHDL will go through
VHPI first. cocotb keeps FLI because it is faster than Questa's VHPI.

| Simulator | Behaviour | cocotb source (§3.6) | Rivet implementation | Verification status |
|---|---|---|---|---|
| Questa | Handles are three incompatible C types (`mtiRegionIdT`, `mtiSignalIdT`, `mtiVariableIdT`) discriminated out of band, so every access is a pair of branches. | `FliImpl.hpp:173-217` | not implemented | not implemented |
| Questa | Arrays and structures have no value access at all. | `FliObjHdl.cpp:96-156` | not implemented | not implemented |
| Questa | Variables and reals cannot be forced. | `FliObjHdl.cpp:408-474, 612-622` | not implemented | not implemented |
| Questa | `argc` and `argv` must be recovered through the embedded Tcl interpreter, so the library links Tcl. | `FliImpl.cpp:479-536` | not implemented | not implemented |
| Questa | Ending the simulation is `mti_Quit` at time zero and `mti_Break` afterwards. | `FliImpl.cpp:20-27` | not implemented | not implemented |
| Questa | Four generate kinds are `#ifdef`-guarded because the constants do not exist in every Questa version. | `FliImpl.cpp:709-720` | not implemented | not implemented |

## How to verify on your simulator

### The three simulators the CLI drives

`rivet run` supports `icarus`, `verilator` and `ghdl` and rejects anything else
(`crates/rivet-cli/src/lib.rs:563-584`). These are the commands CI runs:

```sh
cargo build -p rivet-cli
target/debug/rivet run --sim icarus    -C examples/conformance
target/debug/rivet run --sim verilator -C examples/conformance
target/debug/rivet run --sim ghdl      -C examples/dff_vhdl
```

The full suite, from `docs/testing.md`:

```sh
cargo test --workspace --no-fail-fast
for e in dff fifo conformance bus; do
  target/debug/rivet run --sim icarus    -C examples/$e
  target/debug/rivet run --sim verilator -C examples/$e
done
target/debug/rivet run --sim icarus -C examples/bus -j 4 --cov-threshold 99
target/debug/rivet run --sim ghdl -C examples/dff_vhdl
```

### Any other simulator

There is no runner for your tool yet, so build the plugin and load it by hand.
The conformance crate builds as a `cdylib` and a `rlib`
(`examples/conformance/Cargo.toml`), and the `cdylib` is the VPI plugin.

1. Build the plugin:

   ```sh
   cargo build -p example-conformance --lib
   # target/debug/libexample_conformance.so
   ```

2. Compile the design, `examples/conformance/hdl/conformance.sv`, with your own
   compiler. The design needs full read and write access, so pass the access
   flag your tool wants: `+acc`, `-access +rwc`, `+access +w_nets` or
   `-voptargs=-access=rw+/.` (`docs/design/00-cocotb-analysis.md` §5.1).
   `examples/conformance/rivet.toml` lists the plusargs the tests expect
   (`+conf=1`).

3. Load the plugin. cocotb's flags, from `docs/design/00-cocotb-analysis.md`
   §5.1:

   | Simulator | How the plugin is loaded |
   |---|---|
   | Icarus | `vvp -M <dir> -m <lib>` |
   | Questa | `-pli <lib>` |
   | Xcelium | `-loadvpisim <lib>:vlog_startup_routines_bootstrap` |
   | VCS | `-load <lib>` at compile time |
   | Riviera / Active-HDL | `asim -pli <lib>` |
   | DSim | `-pli_lib <lib>` on both the image and the run |
   | GHDL | `--vpi=<lib>` |

4. Set the environment the CLI sets (`crates/rivet-cli/src/lib.rs:476-513`).
   The minimum is:

   ```sh
   export RIVET_RESULTS_FILE=$PWD/results.xml
   export RIVET_RESULTS_JSON=$PWD/results.json
   export RIVET_TOPLEVEL=conformance
   export RIVET_LOG=debug              # optional, prints what each test found
   export RIVET_TEST_FILTER=discovery  # optional, one test at a time
   ```

5. Run the simulation and read `results.xml`. Every conformance test prints the
   simulator name with what it observed, so a log from a failing run names both
   the behaviour and the tool.

### Turning a row into a result

Each conformance test maps onto rows above: `discovery` and
`generate_instances` cover iteration and the generate-prefix fallback,
`memory_array` covers index lookup, `string_integer_real` covers string
deposits, `parameters` covers constant classification, `writes_*` cover the
write buffering, `force_and_release` covers force, `x_before_reset` covers
four-state values. If a test passes, the rows it covers are verified on your
simulator and this document can say so. If it fails, the row names the cocotb
source that says why the workaround exists, which is the first thing to check
against your tool's version.

## What Rivet deliberately does not carry

These are cocotb behaviours left out on purpose. None of them is a missing
quirk.

| cocotb behaviour | cocotb source | What Rivet does instead | Reason |
|---|---|---|---|
| Edge detection by reading the signal as an ASCII binary string and `strcmp`-ing it against `"1"` or `"0"` on every change. | §2 `VpiCbHdl.cpp:197-210` | Reads the value into a reused `LogicVec` and compares bit 0 (`crates/rivet-core/src/runtime.rs:564-590`). | The string path costs a format, a copy into a process-global buffer, an upper-case pass and a compare per edge, matching or not. |
| Value transport as ASCII binary strings in both directions. | §3.2 `GpiCommon.cpp:572-580`, `VpiSignal.cpp:158-234` | Moves `aval`/`bval` words through `vpiVectorVal` (`crates/rivet-vpi/src/lib.rs:596-616`, `:650-660`). The string path stays only for GHDL, which has no `vpiVectorVal`. | Same reason, and the global buffer in cocotb invalidates a pointer on the next read anywhere in the process. |
| The edge callback is torn down and re-registered on every edge. | §2 `VpiCbHdl.cpp:135-174` | One persistent value-change callback per handle, shared by all waiters (`crates/rivet-core/src/runtime.rs:593-605`). | Registration is an allocation, a map insert and a removal per edge. |
| One global handle cache keyed by fully qualified name across all backends, never pruned, with a `GPI_NATIVE` path that bypasses it. | §3.4 `GpiCommon.cpp:26-61, 452-463` | Per-backend intern table keyed by the path Rivet builds, with duplicate discovery returning the existing handle (`crates/rivet-vpi/src/lib.rs:151-181`). | Handles are indices into the backend's own table, so there is nothing for a second cache to hold. |
| `COCOTB_TRUST_INERTIAL_WRITES` defaults on for Verilator. | §2.2 `runner.py:1414-1417` | Verilator never trusts inertial writes (`crates/rivet-vpi/src/lib.rs:119-130`). | Honouring `vpiInertialDelay` on Verilator 5.036+ requires the loop to call `VerilatedVpi::doInertialPuts`, and it would make the VPI path disagree with the direct-access path. Buffering behaves the same on every Verilator version. `docs/design/04-remaining-work.md` §0 records the bug this fixed. |
| On Verilator, time advances because a C++ `GpiClock` re-registers `cbAfterDelay` every half period; with no pending cocotb timer the simulation ends. | §3.5 `verilator.cpp:206-218` | Rivet owns `main` and drives its own timer wheel, taking the minimum of its next deadline, the VPI deadline and Verilator's next time slot (`crates/rivet-verilator/src/lib.rs:176-183`, `crates/rivet-verilator/src/sched.rs:114-151`). | No callback registration per half period, and HDL-side events are not lost when no harness timer is pending. |
| An FLI backend for Questa VHDL. | §3.6 FLI | Nothing. Questa VHDL is planned to go through VHPI. | About 2000 lines, and nothing can test it without a Questa licence. `docs/design/04-remaining-work.md` §3. |
| Mixed-language support: several GPI implementations registered at once, with globals routed to `registered_impls[0]` and name lookup fanned out over every implementation. | §3.6 `GpiCommon.cpp:312-334, 680-734` | One backend per run. A `CompositeBackend` is designed but not built. | It can only be verified on a simulator that runs both languages in one process, which means Questa, Xcelium or Riviera. `docs/design/04-remaining-work.md` §2. |
