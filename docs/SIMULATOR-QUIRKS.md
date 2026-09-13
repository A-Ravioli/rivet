# Simulator quirks

Every simulator-specific behaviour Rivet carries, where it came from, where it
lives in this repository, and whether it has ever run.

Rivet's backends are ports of cocotb's GPI layer. cocotb's C++ sources are full
of per-simulator guards, each one a fact about a simulator rather than a fact
about cocotb. `docs/design/00-cocotb-analysis.md` §3.6 catalogues them with
source line references. This document is the other half of that table: what
Rivet does about each one.

Rivet is developed against Icarus, Verilator, GHDL and NVC, which are the four
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
backend. The simulator is identified once from the `vpi_get_vlog_info` product
string (`crates/rivet-vpi/src/lib.rs:86-133`) and every quirk below is selected
from that.

### From the cocotb catalogue (§3.6)

Where Rivet applies a cocotb-guarded workaround to every simulator instead of
one, the Simulator column says "all" and the Behaviour column names cocotb's
guard.

| Simulator | Behaviour | cocotb source (§3.6) | Rivet implementation | Verification status |
|---|---|---|---|---|
| GHDL | Do not ask for `vpiPacked`, `vpiConstType` or `vpiSigned`. VHDL has no packed/unpacked distinction and GHDL warns on properties it does not know. | `VpiImpl.cpp:94-100` | `crates/rivet-vpi/src/lib.rs:206`, `:220`, `:241` | verified |
| all (cocotb: Xcelium) | A constant whose type is neither real nor string classifies as a vector, including the `vpiUndefined` that Xcelium reports. | `VpiImpl.cpp:163-169` | `crates/rivet-vpi/src/lib.rs:219-226` | verified |
| Xcelium | Validate a `vpiGenScope` by iterating the parent's internal scopes first. Xcelium answers with a scope that does not exist and crashes when it is used. | `VpiImpl.cpp:382-404` | `crates/rivet-vpi/src/lib.rs:345-369`, applied to every `vpiGenScope` lookup at `:624-629`. Has never executed: the check returns early on anything but Xcelium. | code only |
| all (cocotb: every simulator except Xcelium) | Generate-prefix fallback. `gen[0]` resolves but `gen` does not, so return the parent handle typed as a generate array and index it by name. | `VpiImpl.cpp:406-452` | `crates/rivet-vpi/src/lib.rs:565-610`, indexed at `:640-698` | verified |
| Questa | Hard-coded element counts per type, because Questa reports `vpiSize == 1` for all scalar types. | `VpiImpl.cpp:230-258` | `crates/rivet-vpi/src/lib.rs:187-199`: byte 8, shortint 16, int and integer 32, longint, time and real 64. Has never executed. | code only |
| Questa, VCS | The `vpiRange` iterator restarts at dimension 0, so scan forward to the dimension you want. | `VpiObj.cpp:46-54` | not implemented. Rivet reads `vpiLeftRange` and `vpiRightRange` handles directly and never opens a `vpiRange` iterator (`crates/rivet-vpi/src/lib.rs:242-261`). Multi-dimensional ranges are not modelled. | not implemented |
| Questa | `vpi_handle_by_index` returns NULL on 2-D arrays, so fall back to a name lookup. | `VpiImpl.cpp:504-518` | `crates/rivet-vpi/src/lib.rs:650-662`, applied on every simulator | code only |
| Questa, Xcelium | Deposits to a `vpiStringVar` must use `vpiNoDelay`. | `VpiSignal.cpp:195-206` | `crates/rivet-vpi/src/lib.rs:824-832`. Has never executed. | code only |
| all (cocotb: Icarus) | `vpi_control` returns void on Icarus, so do not check its return value. | `VpiImpl.cpp:763-767` | `crates/rivet-vpi/src/lib.rs:947-951`, which ignores the return value everywhere | verified |
| VCS | VCS loads the plugin at compile time as well as at run time, so bail out when `vpi_get_vlog_info` fails. | `VpiImpl.cpp:843-852` | `crates/rivet-vpi/src/lib.rs:1136-1150`: `startup_with` returns before creating a backend when the call fails, so the compile-time load does nothing. Has never executed. | code only |
| Xcelium | Skip root objects whose full name starts with a backslash. They are virtual classes placed at the top scope. | `VpiImpl.cpp:635-639` | `crates/rivet-vpi/src/lib.rs:520-527`. Has never executed: no free simulator puts one there. | code only |
| Icarus, Xcelium, Questa | Callback re-entrancy queue. These simulators fire value-change callbacks from inside a `vpiNoDelay` write, while the harness is still inside another callback. | `VpiCbHdl.cpp:45-69` | `crates/rivet-core/src/runtime.rs:117-125` and `:474-487`: a `REACTING` flag and a deferred queue drained after the outer event. Exercised on Icarus in CI. | verified |
| Verilator | Remove a one-shot callback after it fires. Verilator treats callbacks as recurring that other simulators free. | `VpiCbHdl.cpp:109-116, 145-161` | `crates/rivet-vpi/src/lib.rs:1066-1071`, with the capability flag at `:469` and `crates/rivet-core/src/backend.rs:138-140`. Under the Verilator backend only value-change callbacks go through VPI; the rest are native (`crates/rivet-verilator/src/sched.rs:320-341`). | verified |
| Xcelium, VCS, Riviera | Call `vpi_free_object` on the callback handle after it fires, which the VPI spec does not require. | `VpiCbHdl.cpp:162-166` | `crates/rivet-vpi/src/lib.rs:1072-1076`, in the same branch that removes Verilator's one-shots. Has never executed. | code only |
| Xcelium | The startup callback must be `cbAfterDelay(0)`. Xcelium does not deliver `cbStartOfSimulation` to a late-loaded library. | `VpiCbHdl.cpp:236-243` | `crates/rivet-vpi/src/lib.rs:1174-1181`. Has never executed. | code only |
| Riviera | Remove `vpiModule`, `vpiModuleArray`, `vpiInterface` and `vpiInterfaceArray` from the iteration lists. Aldec segfaults on mixed-language designs. | `VpiIterator.cpp:25-41` | `crates/rivet-vpi/src/lib.rs:330-336` drops `vpiModule` for Riviera. The other three are not in Rivet's list on any simulator (`:432-445`). Has never executed. | code only |
| Xcelium | Exclude `vpiNetArray` from struct iteration. | `VpiIterator.cpp:51-53` | `crates/rivet-vpi/src/lib.rs:706-713`: struct children are `vpiMember` plus `vpiNetArray`, and the second is dropped on Xcelium. Has never executed. | code only |
| Questa | Iterate `vpiInstance` rather than `vpiPackage`. | `VpiImpl.hpp:266-275` | `crates/rivet-vpi/src/lib.rs:337-341` appends `vpiInstance` to the scope-iteration list on Questa. Has never executed. | code only |
| all (cocotb: Xcelium) | Skip iterated objects that have no name. | `VpiIterator.cpp:228-245` | `crates/rivet-vpi/src/lib.rs:727-731` | verified |
| all | Never remove the startup and shutdown callbacks. Too many simulators object. | `VpiImpl.hpp:127-168` | `crates/rivet-vpi/src/lib.rs:1155-1188`. They are `static` `s_cb_data` with no matching remove, and `remove` only touches the callback map (`:931-945`). | verified |

### Other simulator-specific behaviour in the VPI path

These come from elsewhere in the analysis (§2.2 write scheduling, §3.2 the
value path, §3.3 callback ownership, §3.4 discovery, §3.5 Verilator, §5.1
plugin loading) or from Rivet's own work on the free simulators.

| Simulator | Behaviour | cocotb source | Rivet implementation | Verification status |
|---|---|---|---|---|
| Verilator | Never trust inertial writes. Deposits buffer until ReadWrite and are then applied with `vpiNoDelay`. Honouring `vpiInertialDelay` on 5.036+ would need the loop to call `VerilatedVpi::doInertialPuts`, and would differ from the direct-access path. | §2.2 `handle.py:769-820`, `runner.py:1414-1417` | `crates/rivet-vpi/src/lib.rs:122-133` | verified |
| GHDL | Trust inertial writes. Writes go straight to the simulator instead of the buffer. | §2.2 `runner.py:1560-1563` | `crates/rivet-vpi/src/lib.rs:129-133`, `crates/rivet-core/src/runtime.rs:724-728` | verified |
| Icarus | Do not trust inertial writes. Deposits buffer until the ReadWrite phase, latest write per handle wins. | §2.2 `handle.py:769-820` | `crates/rivet-core/src/runtime.rs:724-744` | verified |
| Questa, Xcelium, VCS, Riviera, DSim | Same buffering as Icarus, by default for every simulator that is not GHDL or Verilator. `RIVET_TRUST_INERTIAL_WRITES=1` turns it off. | §2.2 `Makefile.inc:138-147` | `crates/rivet-vpi/src/lib.rs:129-133` | code only |
| GHDL | Move values as binary strings. GHDL's VPI has no `vpiVectorVal`. | §3.2 `VpiSignal.cpp:103-110` | `crates/rivet-vpi/src/lib.rs:144`, `:786-792`, `:839-846` | verified |
| all | Fall back to a binary string when the width is unknown or the simulator returns a NULL vector pointer. | §3.2 | `crates/rivet-vpi/src/lib.rs:382-390`, `:782-785`, `:796-801` | code only |
| GHDL | A lookup of an unknown child answers with the scope itself. Report that as missing. | none, found in this repository | `crates/rivet-vpi/src/lib.rs:630-635` | verified |
| GHDL | A lookup of a VHDL generate label answers with its first element, `label(0)`. That is the array, not the element, so return a generate array whose elements are found by name. | none, found in this repository | `crates/rivet-vpi/src/lib.rs:613-623`, array built at `:275-302`. Exercised by `examples/vhdl_types` on GHDL (`generate_region_elements_exist`). | verified |
| all | A struct's children are its members (`vpiMember`), not a scope's declarations. | §3.4 `VpiIterator.cpp` | `crates/rivet-vpi/src/lib.rs:425-430`, used at `:706-716`. Icarus and Verilator report no members at all, which is why `bindgen` decodes packed structs from the vector instead. Has never yielded a child. | code only |
| all | Build the path from parent path plus name rather than trusting `vpiFullName`. Simulators disagree, and Verilator reports top-level ports under a `TOP` scope. | §3.4 `GpiCommon.cpp:26-61` | `crates/rivet-vpi/src/lib.rs:154-184` | verified |
| all | Retry a failed child lookup with the fully qualified name. Some simulators only resolve those. | §3.4 | `crates/rivet-vpi/src/lib.rs:560-564` | code only |
| Icarus | Read time precision from the first top-level module. Icarus answers 0 for a NULL object, and nothing answers before elaboration. | none, found in this repository | `crates/rivet-vpi/src/lib.rs:475-501` | verified |
| Icarus | Waveform on and off through a generated `rivet_dump` module holding `$dumpon` and `$dumpoff`. One file per run; a new file per test is refused. | §5.1 `runner.py:932-950` | `crates/rivet-vpi/src/lib.rs:953-982`, module generated by `crates/rivet-cli/src/lib.rs:309-326` and compiled in at `:355-365` | verified |
| Icarus | Bindgen skips the internal scopes Icarus exposes (`$ivl_for_loop0`, `$unm_blk_3`). They exist on no other simulator, so bindings that include them do not port. | none, found in this repository | `crates/rivet-cli/src/bindgen.rs:70-74` | verified |
| Xcelium, CVC | Export a `vlog_startup_routines_bootstrap` symbol. Xcelium loads a plugin as `<lib>:vlog_startup_routines_bootstrap`. | §5.1 plugin-loading table | `crates/rivet-vpi/src/lib.rs:1199-1210`, passed by the Xcelium flow at `crates/rivet-cli/src/lib.rs:480-492` | code only |
| Verilator | Two-state values and no force or release. Tests read this from the capability flags and skip. | §3.5 | `crates/rivet-vpi/src/lib.rs:469-471`, used at `examples/conformance/src/lib.rs:14-15`, `:353` | verified |
| Verilator | Rivet owns `main` and runs the region loop itself: evaluate to a fixpoint, ReadWrite, evaluate again if anything was written, `eval_end_step`, ReadOnly, dump, then jump to the next deadline. The simulation ends when nothing is pending. | §3.5 `verilator.cpp:174-231` | `crates/rivet-verilator/src/lib.rs:136-195`, settle loop at `:75-81`, deadline choice at `:176-183` | verified |
| Verilator | `Verilated::fatalOnVpiError(false)`, otherwise the model aborts on system tasks. | §3.5 `verilator.cpp:154` | `crates/rivet-verilator/src/build.rs:308` | verified |
| Verilator | Close the trace object but never delete it. | §3.5 `verilator.cpp:66-71` | `crates/rivet-verilator/src/build.rs:365-372` | verified |
| Verilator | Verilate with `--cc --vpi --public-flat-rw`. Without `--public-flat-rw` internal signals are neither visible nor writable. | §3.5 `runner.py:1919-1998` | `crates/rivet-verilator/src/build.rs:193` | verified |
| Verilator | Keep timers and phase callbacks in a Rust timer wheel instead of registering `cbAfterDelay`. Rivet owns the loop, so it can schedule them directly. | §3.5 | `crates/rivet-verilator/src/sched.rs:320-341`, wheel at `:114-151` | verified |
| Verilator | Read and write model storage directly through the symbol table. A top-level port exists twice, so root-level names resolve the `TOP` scope first or writes land on the alias. | none, found in this repository | `crates/rivet-verilator/src/sched.rs:178-225`, `:192-201`; shim at `crates/rivet-verilator/src/build.rs:330-345` | verified |
| Verilator | Mark parameters constant from the symbol table's `isParam`. Verilator's VPI types parameters as ordinary variables, so writes would otherwise be accepted. | none, found in this repository | `crates/rivet-verilator/src/sched.rs:168-176`, `crates/rivet-vpi/src/lib.rs:269-273` | verified |
| Verilator | Version guard for the `VerilatedVar` range accessors, renamed between 5.020 and 5.036. Generated code is not portable across versions, so the object directory is cleared when the version changes. | none, found in this repository | `crates/rivet-verilator/src/build.rs:338-342`, object directory cleared at `:176-190` | verified |
| all | Identify the simulator from the `vpi_get_vlog_info` product string, then select every quirk from that. | §3.6 | `crates/rivet-vpi/src/lib.rs:86-133` | verified |
| all | A callback whose removal fails is flagged and squashed when it fires, instead of dereferencing freed memory. | §3.3 `VpiCbHdl.cpp:135-174` | `crates/rivet-vpi/src/lib.rs:931-945`, checked at `:1050-1053` | code only |

## VHPI

`crates/rivet-vhpi` is the VHDL side: NVC today, and Questa, Riviera and
Xcelium VHDL on paper. It is about 1250 lines plus 270 lines of hand-written
bindings in `ffi.rs`. `rivet run --sim nvc` drives it, and CI runs
`examples/dff_vhdl` and `examples/vhdl_types` on NVC
(`.github/workflows/ci.yml:115-120`), with the same `examples/vhdl_types` run
again on GHDL through VPI so the two paths are compared on one design.

Three things are worth knowing before reading the tables.

No VHPI row is gated on the tool. The backend detects a `Sim` from the tool
name (`crates/rivet-vhpi/src/lib.rs:138-149`) but uses it only to name itself
(`:548-556`), so every workaround below is applied to every VHPI simulator.
That is deliberate while only one of them has ever run.

Tool identity does not arrive by itself. `vhpi_get_str` on a NULL handle is an
error on NVC, so the constructor falls back to `RIVET_VHPI_TOOL`
(`:122-137`). Nothing in this repository sets that variable, so on NVC the
backend reports its name as `vhpi`.

Development was on NVC 1.23 (`crates/rivet-vhpi/README.md:3`). CI builds NVC
1.17.1 from a release tarball (`.github/workflows/ci.yml:110`), so "verified"
in this section means verified on 1.17.1.

### Found by experiment during NVC bring-up

Each of these cost a debugging session. None of them is in cocotb's catalogue
in this form.

| Simulator | Behaviour | Rivet implementation | Verification status |
|---|---|---|---|
| NVC | A scalar of a logic type needs `vhpiLogicVal`, not `vhpiEnumVal`. NVC accepts `vhpiEnumVal`, reports success, and leaves the signal at `'U'`. | `crates/rivet-vhpi/src/lib.rs:486-505`. Vectors use `vhpiLogicVecVal` (`:513-519`); only enumerations that are not logic use `vhpiEnumVal` (`:924-930`). | verified |
| NVC | `vhpiDeposit` and `vhpiForce` are fatal errors. Only `vhpiDepositPropagate` and `vhpiForcePropagate` exist in practice. | `crates/rivet-vhpi/src/lib.rs:885-893`. `Deposit` and `NoDelay` both map to `vhpiDepositPropagate`, `Force` to `vhpiForcePropagate`. The deposit mapping runs in every test; no example forces a VHDL signal, so the force mapping has not run. | verified |
| NVC | `vhpi_get_value` with `vhpiBinStrVal` works, `vhpi_put_value` with it is fatal. So reads may fall back to a binary string and writes never do. | `crates/rivet-vhpi/src/lib.rs:420-422` states the rule; the read fallback is at `:448` and `:451-476`, and the write path at `:478-526` has no fallback. The fallback has never fired: `vhpiLogicVecVal` reads succeed on NVC. | code only |
| NVC | A callback registered without the `vhpiReturnCb` flag returns NULL on success, which is indistinguishable from failure. | Every registration passes `vhpiReturnCb` and treats NULL as an error: `crates/rivet-vhpi/src/lib.rs:1044-1048`, `:1179`, `:1182`. | verified |
| NVC | `vhpi_get_str` on a NULL handle is an error, so tool identity has to come from elsewhere. | `crates/rivet-vhpi/src/lib.rs:122-137`: the name and version fall back to `RIVET_VHPI_TOOL` and to each other. Nothing sets that variable, so the backend calls itself `vhpi` on NVC. | verified |
| NVC | A VHDL for-generate has no parent object. Each element is a region named `label(i)`, and the label itself resolves to nothing. | `crates/rivet-vhpi/src/lib.rs:623-655` synthesises a generate array when any child's name starts with `label(`; elements are found by scanning `vhpiInternalRegions` at `:681-708`. Exercised by `generate_region_elements_exist` and `generate_region_follows_the_accumulator`. | verified |
| NVC | Names come back upper-cased, so every lookup has to be case-insensitive. | `crates/rivet-vhpi/src/lib.rs:598-603` (root), `:612` and `:619` (children), `:627-631` (generate prefix), `:685-695` (generate elements) | verified |

### From the cocotb catalogue (§3.6)

| Simulator | Behaviour | cocotb source (§3.6) | Rivet implementation | Verification status |
|---|---|---|---|---|
| Xcelium | `vhpiIsUnconstrainedP` is unset, so compare bounds against the magic value 2147483647. | `VhpiObj.cpp:28-34, 60-64`; `VhpiImpl.cpp:719-751` | not implemented. Rivet never reads `vhpiIsUnconstrainedP` (declared and unused at `crates/rivet-vhpi/src/ffi.rs:179`); array width comes from `vhpiSizeP` (`lib.rs:317`). | not implemented |
| Xcelium | The unconstrained flag is wrong on the base type; retry on the subtype. | `VhpiImpl.cpp:773-779` | not implemented, for the same reason. The subtype retry exists for the type itself (`lib.rs:177-192`), not for this flag. | not implemented |
| Questa | `vhpiIsUpP` is wrong (cocotb #4236); infer direction from `left < right`. | `VhpiObj.cpp:68-80, 111-120` | not implemented. Rivet never asks for a direction: it reads `vhpiLeftBoundP` and `vhpiRightBoundP` off the first constraint and records the pair (`lib.rs:350-359`). `vhpiIsUpP` is declared and unused (`ffi.rs:180`). | not implemented |
| Riviera | Generate index separator is `__n`, not `(n)`. | `VhpiImpl.hpp:26-34` | not implemented. The generate synthesis matches `label(i)` only (`lib.rs:627`, `:685`). | not implemented |
| Riviera | Compare enum literals both with and without quotes; Aldec omits them. | `VhpiImpl.cpp:174-201` | `crates/rivet-vhpi/src/lib.rs:112-119`, called for every literal at `:224`. It runs on NVC, but nothing in CI depends on it: logic-type detection succeeds either way. The Aldec behaviour has never been seen. | code only |
| Riviera | `vhpiRootInstK` is a null hierarchy level. | `VhpiIterator.cpp:120-123` | not implemented. `vhpiRootInstK` is classified as an ordinary region (`lib.rs:299-302`), which is what NVC needs. | not implemented |
| NVC | Compare names case-insensitively (nvc#723). | `VhpiImpl.cpp:274-285` | `crates/rivet-vhpi/src/lib.rs:598-603`, `:612`, `:619`. See the NVC table above. | verified |
| NVC | Upper-case all fully qualified names. | `VhpiImpl.cpp:516-519` | not implemented. Rivet keeps the case the tool reports and builds paths from it (`lib.rs:256-283`), then compares case-insensitively. NVC passes CI that way. | not implemented |
| all | `vhpiBaseType`, then `vhpiSubtype`, then `vhpiBaseType` again. Five sites. | `VhpiImpl.cpp:301-312` | `crates/rivet-vhpi/src/lib.rs:177-192`, one function called from `classify`. The first call answers on NVC, so the two fallback branches have not run. | verified |
| all | `vhpiElemType` with a `vhpiElemSubtype` fallback. | `VhpiImpl.cpp:327-332` | `crates/rivet-vhpi/src/lib.rs:194-200`, called for every array type at `:335`. The fallback branch has not run on NVC. | verified |
| all | Record children through `vhpiSelectedNames`. `vhpi_handle_by_name` fails for records inside generics. | `VhpiImpl.cpp:559-578` | `crates/rivet-vhpi/src/lib.rs:737-742`. Exercised by `record_members_are_addressable`, which passes on NVC and skips on GHDL. | verified |
| all | `vhpi_handle_by_index` with a linear scan of `vhpiIndexedNames` as fallback. | `VhpiImpl.cpp:833-858` | `crates/rivet-vhpi/src/lib.rs:709-724`. Neither example indexes a VHDL array; the generate case takes the separate path at `:681-708`. | code only |
| all | Zero-length arrays return an empty string without calling the simulator. | `VhpiSignal.cpp:465-473` | not implemented. `read_logic_vec` asks the simulator whatever the width is (`lib.rs:423-449`). | not implemented |
| all | Three-stage root discovery, with a `:`-prefixed lookup to disambiguate library objects. | `VhpiImpl.cpp:884-982` | not implemented as three stages. `root` takes `vhpiRootInst` and nothing else (`lib.rs:585-605`), which is enough on NVC. The `:`-prefixed fully qualified lookup exists as the last resort for children (`:657-668`) and has never fired. | not implemented |
| all | `Deposit` and `NoDelay` both map to `vhpiDepositPropagate`. VHPI has no inertial/immediate distinction. | `VhpiSignal.cpp:35-52` | `crates/rivet-vhpi/src/lib.rs:885-893`, with `trusts_inertial_writes: false` at `:562-573` so the runtime buffers as it does everywhere else. | verified |
| all | VHDL type mapping is by the content of enum types, not by kind: name or literal set decides logic, character and boolean. `bit` and `std_logic` are indistinguishable at this level. | `VhpiImpl.cpp:161-272` | `crates/rivet-vhpi/src/lib.rs:202-254` builds the map, `:318-333` uses it: nine logic literals give `Logic`, `FALSE`/`TRUE` give `Logic`, 256 literals give a character, anything else is an enumeration. Exercised by `op_t`, `armed` and `std_logic_vector` in `examples/vhdl_types`. | verified |

### Other behaviour in the VHPI path

| Simulator | Behaviour | Rivet implementation | Verification status |
|---|---|---|---|
| all | Time precision is a physical property in femtoseconds on the NULL handle, not an integer exponent. | `crates/rivet-vhpi/src/lib.rs:150-153` converts it to the exponent of ten seconds Rivet uses. | verified |
| all | The one-shot phase callbacks are optional in VHPI; the repetitive forms are what tools implement. Register `vhpiCbRep*` and remove them when they fire. | `crates/rivet-vhpi/src/lib.rs:1013-1018` registers, `:1111-1118` removes, `:568-569` tells the runtime they behave that way. | verified |
| all | Processes and subprogram bodies come back from region iteration and are not design data. | `crates/rivet-vhpi/src/lib.rs:747-751` | verified |
| all | Enumeration literal names come from `vhpiStrValP`, with `vhpiCaseNameP` and `vhpiNameP` as fallbacks. | `crates/rivet-vhpi/src/lib.rs:217-223`, surfaced to tests at `:1066-1072`. Exercised by `enum_literals_have_names`. | verified |
| all | There is no VHPI call to start or stop a waveform. NVC dumps the whole run with `--wave`. | `crates/rivet-vhpi/src/lib.rs:1078-1082` refuses the request; the CLI passes `--wave` for `--waves` instead (`crates/rivet-cli/src/lib.rs:787-789`). | verified |
| all | Handles are interned by `vhpiFullCaseNameP`, or by the path Rivet composes when the tool reports none, and a duplicate is released immediately. | `crates/rivet-vhpi/src/lib.rs:256-283` | verified |

## FLI

FLI is Questa's VHDL interface. There is no FLI backend and none is planned
until a Questa licence is available; `docs/design/04-remaining-work.md` §3
records the decision and the reason, which is that Questa VHDL goes through
VHPI first. That path now exists in `crates/rivet-vhpi`, unverified. cocotb
keeps FLI because it is faster than Questa's VHPI.

| Simulator | Behaviour | cocotb source (§3.6) | Rivet implementation | Verification status |
|---|---|---|---|---|
| Questa | Handles are three incompatible C types (`mtiRegionIdT`, `mtiSignalIdT`, `mtiVariableIdT`) discriminated out of band, so every access is a pair of branches. | `FliImpl.hpp:173-217` | not implemented | not implemented |
| Questa | Arrays and structures have no value access at all. | `FliObjHdl.cpp:96-156` | not implemented | not implemented |
| Questa | Variables and reals cannot be forced. | `FliObjHdl.cpp:408-474, 612-622` | not implemented | not implemented |
| Questa | `argc` and `argv` must be recovered through the embedded Tcl interpreter, so the library links Tcl. | `FliImpl.cpp:479-536` | not implemented | not implemented |
| Questa | Ending the simulation is `mti_Quit` at time zero and `mti_Break` afterwards. | `FliImpl.cpp:20-27` | not implemented | not implemented |
| Questa | Four generate kinds are `#ifdef`-guarded because the constants do not exist in every Questa version. | `FliImpl.cpp:709-720` | not implemented | not implemented |

## How to verify on your simulator

### The simulators CI runs

```sh
cargo build -p rivet-cli
target/debug/rivet run --sim icarus    -C examples/conformance
target/debug/rivet run --sim verilator -C examples/conformance
target/debug/rivet run --sim ghdl      -C examples/dff_vhdl
target/debug/rivet run --sim ghdl      -C examples/vhdl_types
target/debug/rivet run --sim nvc       -C examples/dff_vhdl
target/debug/rivet run --sim nvc       -C examples/vhdl_types
```

The full suite, from `docs/testing.md`:

```sh
cargo test --workspace --no-fail-fast
for e in dff fifo conformance bus; do
  target/debug/rivet run --sim icarus    -C examples/$e
  target/debug/rivet run --sim verilator -C examples/$e
done
target/debug/rivet run --sim icarus -C examples/bus -j 4 --cov-threshold 99
```

### One backend per library

A test crate picks its backend with a cargo feature: `vpi`, `vhpi` or
`verilator`. A `cdylib` that carries two of them fails to load on NVC, because
a VHPI simulator does not provide the VPI symbols and resolves the library
eagerly. So `--sim nvc` builds with `--no-default-features --features vhpi`
(`crates/rivet-cli/src/lib.rs:561-581`, selected at `:835-839`) and every other
simulator gets the crate's default features. The two VHDL examples declare the
features that make this work: `examples/vhdl_types/Cargo.toml` and
`examples/dff_vhdl/Cargo.toml`, both `default = ["vpi"]` with a `vhpi` feature.

`crates/rivet/Cargo.toml:27-29` still says the two features can be enabled
together. They can be compiled together; the result will not load on NVC.

### The commercial simulators

`rivet run` accepts `questa`, `xcelium`, `vcs`, `riviera` and `dsim` and builds
and launches each one (`crates/rivet-cli/src/lib.rs:441-559`, selected at
`:847-851`). None of these flows has ever executed. Running one is the whole
point of this section:

```sh
cargo build -p rivet-cli
target/debug/rivet run --sim questa   -C examples/conformance
target/debug/rivet run --sim xcelium  -C examples/conformance
target/debug/rivet run --sim vcs      -C examples/conformance
target/debug/rivet run --sim riviera  -C examples/conformance
target/debug/rivet run --sim dsim     -C examples/conformance
```

Add `-v` to see each command before it runs. What each flow does, and the
design-access flags it passes, which are the flags that decide whether the
harness can see anything at all:

| Simulator | Compile | Run | Design access, and why |
|---|---|---|---|
| Questa (`lib.rs:457-479`) | `vlog -work work <sources>` | `vsim -c -pli <lib> -voptargs=-access=rw+/. work.<top> -do "run -all; quit -f"` | `-voptargs=-access=rw+/.` keeps read and write access to every net in every scope. Without it `vopt` optimises away the objects the harness looks up, and discovery finds nothing. |
| Xcelium (`lib.rs:480-498`) | one step: `xrun` compiles and runs | `xrun -access +rwc -loadvpisim <lib>:vlog_startup_routines_bootstrap -top <top> <sources>` | `-access +rwc` is read, write and connectivity. The plugin is named `<lib>:<symbol>` because Xcelium loads a named entry point, which is what the exported `vlog_startup_routines_bootstrap` is for. |
| VCS (`lib.rs:499-523`) | `vcs -full64 -sverilog +acc+3 -debug_access+all -load <lib> -o simv -top <top> <sources>` | `./simv` | `+acc+3` plus `-debug_access+all` give full read and write access to nets and registers. `-load` at compile time is why the backend bails out of startup when `vpi_get_vlog_info` fails (`crates/rivet-vpi/src/lib.rs:1136-1150`): the plugin is loaded once with no design present. |
| Riviera (`lib.rs:524-538`) | `alog -work work <sources>` | `vsimsa -do rivet.do`, where the generated script is `asim -pli <lib> work.<top>`, `run -all`, `endsim`, `quit -f` | No access flag is passed. Aldec keeps full visibility by default; if your design is optimised, add the flag through `[sim.riviera] args` in `rivet.toml`. |
| DSim (`lib.rs:539-554`) | `dsim -genimage <image> -pli_lib <lib> -top <top> <sources>` | `dsim -image <image> -pli_lib <lib>` | `-pli_lib` has to be passed to both the image build and the run, which is why it appears twice. |

Parameters and generics are passed the way each tool wants them: `-g<k>=<v>` on
Questa and in the Riviera script, `-defparam <top>.<k>=<v>` on Xcelium,
`-pvalue+<top>.<k>=<v>` on VCS, `-defparam+<top>.<k>=<v>` on DSim. Extra flags
go in `rivet.toml` under `[sim.<name>]`: `args` reaches the compile step,
`run_args` the run.

One thing to add before running the conformance suite: its manifest sets
`run_args = ["+conf=1"]` only for Icarus and Verilator
(`examples/conformance/rivet.toml`). The `plusargs_reach_the_harness` test
checks for that plusarg (`examples/conformance/src/lib.rs:394-397`), so add a
`[sim.<name>] run_args = ["+conf=1"]` block for your tool or expect that one
test to fail.

The environment the CLI sets for every run is in `common_env`
(`crates/rivet-cli/src/lib.rs:693-744`): results files, the top-level name, the
test filter, the seed, log level and log directory. If you drive your simulator
by hand instead, set at least these:

```sh
export RIVET_RESULTS_FILE=$PWD/results.xml
export RIVET_RESULTS_JSON=$PWD/results.json
export RIVET_TOPLEVEL=conformance
export RIVET_LOG=debug              # optional, prints what each test found
export RIVET_TEST_FILTER=discovery  # optional, one test at a time
```

The plugin to load by hand is the crate's `cdylib`
(`examples/conformance/Cargo.toml`), built with `cargo build -p
example-conformance --lib` and found at
`target/debug/libexample_conformance.so`. cocotb's load flags, from
`docs/design/00-cocotb-analysis.md` §5.1:

| Simulator | How the plugin is loaded |
|---|---|
| Icarus | `vvp -M <dir> -m <lib>` |
| Questa | `-pli <lib>` |
| Xcelium | `-loadvpisim <lib>:vlog_startup_routines_bootstrap` |
| VCS | `-load <lib>` at compile time |
| Riviera / Active-HDL | `asim -pli <lib>` |
| DSim | `-pli_lib <lib>` on both the image and the run |
| GHDL | `--vpi=<lib>` |
| NVC | `nvc -r --load <lib>` |

### Turning a row into a result

Each conformance test maps onto rows above: `discovery` and
`generate_instances` cover iteration and the generate-prefix fallback,
`memory_array` covers index lookup, `string_integer_real` covers string
deposits, `parameters` covers constant classification, `writes_*` cover the
write buffering, `force_and_release` covers force, `x_before_reset` covers
four-state values. For VHDL, `examples/vhdl_types` does the same job:
`record_members_are_addressable` covers `vhpiSelectedNames`,
`enum_literals_have_names` covers enum-content type mapping, and the two
`generate_region_*` tests cover the synthesised generate array. If a test
passes, the rows it covers are verified on your simulator and this document can
say so. If it fails, the row names the cocotb source that says why the
workaround exists, which is the first thing to check against your tool's
version.

## What Rivet deliberately does not carry

These are cocotb behaviours left out on purpose. None of them is a missing
quirk.

| cocotb behaviour | cocotb source | What Rivet does instead | Reason |
|---|---|---|---|
| Edge detection by reading the signal as an ASCII binary string and `strcmp`-ing it against `"1"` or `"0"` on every change. | §2 `VpiCbHdl.cpp:197-210` | Reads the value into a reused `LogicVec` and compares bit 0 (`crates/rivet-core/src/runtime.rs:589-615`). | The string path costs a format, a copy into a process-global buffer, an upper-case pass and a compare per edge, matching or not. |
| Value transport as ASCII binary strings in both directions. | §3.2 `GpiCommon.cpp:572-580`, `VpiSignal.cpp:158-234` | Moves `aval`/`bval` words through `vpiVectorVal` (`crates/rivet-vpi/src/lib.rs:793-813`, `:847-857`). The string path stays only for GHDL, which has no `vpiVectorVal`. | Same reason, and the global buffer in cocotb invalidates a pointer on the next read anywhere in the process. |
| The edge callback is torn down and re-registered on every edge. | §2 `VpiCbHdl.cpp:135-174` | One persistent value-change callback per handle, shared by all waiters (`crates/rivet-core/src/runtime.rs:618-630`). | Registration is an allocation, a map insert and a removal per edge. |
| One global handle cache keyed by fully qualified name across all backends, never pruned, with a `GPI_NATIVE` path that bypasses it. | §3.4 `GpiCommon.cpp:26-61, 452-463` | Per-backend intern table keyed by the path Rivet builds, with duplicate discovery returning the existing handle (`crates/rivet-vpi/src/lib.rs:154-184`, `crates/rivet-vhpi/src/lib.rs:256-283`). | Handles are indices into the backend's own table, so there is nothing for a second cache to hold. |
| `COCOTB_TRUST_INERTIAL_WRITES` defaults on for Verilator. | §2.2 `runner.py:1414-1417` | Verilator never trusts inertial writes (`crates/rivet-vpi/src/lib.rs:122-133`). | Honouring `vpiInertialDelay` on Verilator 5.036+ requires the loop to call `VerilatedVpi::doInertialPuts`, and it would make the VPI path disagree with the direct-access path. Buffering behaves the same on every Verilator version. `docs/design/04-remaining-work.md` §0 records the bug this fixed. |
| On Verilator, time advances because a C++ `GpiClock` re-registers `cbAfterDelay` every half period; with no pending cocotb timer the simulation ends. | §3.5 `verilator.cpp:206-218` | Rivet owns `main` and drives its own timer wheel, taking the minimum of its next deadline, the VPI deadline and Verilator's next time slot (`crates/rivet-verilator/src/lib.rs:176-183`, `crates/rivet-verilator/src/sched.rs:114-151`). | No callback registration per half period, and HDL-side events are not lost when no harness timer is pending. |
| An FLI backend for Questa VHDL. | §3.6 FLI | Nothing. Questa VHDL goes through VHPI. | About 2000 lines, and nothing can test it without a Questa licence. `docs/design/04-remaining-work.md` §3. |
| Mixed-language support: several GPI implementations registered at once, with globals routed to `registered_impls[0]` and name lookup fanned out over every implementation. | §3.6 `GpiCommon.cpp:312-334, 680-734` | One backend per run. A `CompositeBackend` is designed but not built. | It can only be verified on a simulator that runs both languages in one process, which means Questa, Xcelium or Riviera. `docs/design/04-remaining-work.md` §2. |

## Counts

| PLI layer | Rows | verified | code only | not implemented |
|---|---|---|---|---|
| VPI, cocotb catalogue | 20 | 8 | 11 | 1 |
| VPI, other | 26 | 20 | 6 | 0 |
| VHPI, found on NVC | 7 | 6 | 1 | 0 |
| VHPI, cocotb catalogue | 16 | 6 | 2 | 8 |
| VHPI, other | 6 | 6 | 0 | 0 |
| FLI | 6 | 0 | 0 | 6 |
| **Total** | **81** | **46** | **20** | **15** |

Twenty rows are code that no one has run. Fifteen are behaviours Rivet does not
carry, and six of those are the whole FLI backend.
