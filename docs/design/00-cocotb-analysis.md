# cocotb 2.2.0: an analysis

This is a file-level analysis of cocotb 2.2.0 (`src/`, about 39k lines
including vendored headers), written to inform Rivet's design. Line references
are into the cocotb repository at the 2.2.0 tag. The goal is to separate three
things that are tangled together in cocotb:

1. **The model**: the co-simulation protocol and timing model, which is good
   and which Rivet keeps.
2. **The simulator knowledge**: a decade of per-simulator workarounds, which
   Rivet must port item by item.
3. **The implementation choices**: Python embedding, string-typed values,
   dual build flows, which are where the cost and the accidental complexity
   live and which Rivet replaces.

## 1. Shape of the system

cocotb is a simulator plugin that embeds CPython. The layers, bottom up:

| Layer | Files | Lines | Role |
|---|---|---|---|
| Simulator entry | `share/lib/gpi/{vpi,vhpi,fli}/*Impl.cpp` | | Exports `vlog_startup_routines`, `vhpi_startup_routines`, or the FLI `cocotb_init` foreign entry; one `.so` per (interface, simulator) built from the same source with a `-D<SIM>` define |
| GPI (Generic Procedural Interface) | `share/include/gpi.h`, `share/lib/gpi/GpiCommon.cpp` | 591 + 765 | A flat C ABI of about 40 functions normalising VPI, VHPI, and FLI: handles, hierarchy discovery, value get/set, five callback kinds, start/end/finalize hooks |
| Backends | `share/lib/gpi/vpi/`, `vhpi/`, `fli/` | ~1900, ~2600, ~2400 | Type classification, value conversion, callback state machines, and all the simulator-specific behaviour |
| Verilator glue | `share/lib/verilator/verilator.cpp` | 250 | A hand-written `main()` and scheduler loop, because Verilator is a library not a PLI host |
| Python embedding | `share/lib/pygpi/embed.cpp`, `bind.cpp` | ~300, 1528 | Starts CPython inside the simulator process, exposes the `cocotb.simulator` module, re-enters Python from C callbacks |
| Scheduler | `_event_loop.py`, `_base_triggers.py`, `_gpi_triggers.py`, `task.py`, `_task_manager.py` | ~80, 540, 417, 810, 307 | Coroutine tasks, triggers, a FIFO event loop run to exhaustion inside each simulator callback |
| Handles and values | `handle.py`, `types/*` | 1898, ~2000 | The `dut.sub.sig.value` object model, 4-state `Logic`/`LogicArray`, write scheduling |
| Regression | `regression.py`, `_test_manager.py`, `_decorators.py`, `_xunit_reporter.py` | 1196, ~200, 675, 333 | Test discovery, ordering, seeding, timeouts, scoring, JUnit output |
| Launch | `cocotb_tools/runner.py`, `cocotb_tools/makefiles/*` | 2417, ~1500 | Two parallel implementations of "build the design and run the simulator with the plugin loaded" |
| pytest plugin | `cocotb_tools/_pytest/*` | ~3000 | A second pytest session running inside the simulator, reporting to the parent over a socket |

## 2. The scheduler: what happens on `await RisingEdge(clk)`

This is the part of cocotb worth understanding precisely, because Rivet
reproduces its semantics exactly.

**Control flow.** The simulator owns the thread. cocotb only runs inside a
simulator callback. Every callback ends in the same place:
`GPITrigger._react()` (`_gpi_triggers.py:41-50`) sets the current trigger,
runs the trigger's registered callbacks (which schedule tasks), then calls
`cocotb._event_loop._inst.run()`, which pops `ScheduledCallback`s from a
`deque` until it is empty (`_event_loop.py:41-60`). Each callback is a
`Task._resume` that does one `coro.send(None)` (`task.py:326-453`). When the
coroutine yields a `Trigger`, the task calls `trigger._register(self._schedule_resume)`,
which on the first registration calls `trigger._prime()`
(`_base_triggers.py:61-74`). For a GPI trigger, `_prime()` calls into C to
register a simulator callback (`_gpi_triggers.py:140-160, 234-239`). When
the queue is empty, `run()` returns, `_react()` returns, the C callback
returns, and the simulator continues.

So a cocotb "event loop" is not a loop that owns time; it is a drain-to-idle
routine invoked from inside the simulator. That is the correct design for a
PLI application, and it is the design Rivet keeps: an executor whose
`run_until_idle()` is called from inside each callback.

**Trigger lifetimes.** GPI triggers are one-shot. A task awaiting
`RisingEdge(clk)` for the second time re-primes, which means another
`vpi_register_cb`, and after firing the C++ side removes or deletes the
callback handle (`VpiCbHdl.cpp:135-174`). cocotb 2.x reduces the count by
making `signal.rising_edge` a per-signal singleton trigger
(`_gpi_triggers.py:198-206`), so N tasks awaiting the same edge share one
simulator callback; but the registration is still torn down and rebuilt each
edge.

**Edge detection is a string compare.** `cbValueChange` fires on every
change. `VpiValueCbHdl::run` decides whether the edge matches by calling
`get_signal_value_binstr()` and `strcmp`-ing against `"1"` or `"0"`
(`VpiCbHdl.cpp:197-210`). The same pattern is in VHPI
(`VhpiCbHdl.cpp:136-149`) and FLI (`FliCbHdl.cpp:84-98`). So every clock
edge, whether or not it matches, costs a `vpi_get_value(vpiBinStrVal)`, a copy
into a global `std::string`, a `toupper` pass, and a `strcmp`.

**Callback re-entrancy.** Icarus (gh-4067), Xcelium (gh-4013), and Questa
(gh-4105) react to `vpiNoDelay` writes by firing value-change callbacks
*immediately*, from inside the write, while cocotb is still inside another
callback. `handle_vpi_callback` therefore keeps a global `reacting` flag and a
`deque` of deferred callbacks; anything arriving while reacting is queued and
drained after the outer callback finishes (`VpiCbHdl.cpp:40-69`). `remove()`
has to search the deque too (`:104-117`). The VHPI path has no such queue.

**Illegal transitions.** `ReadOnly.__await__` and `ReadWrite.__await__` raise
if the current trigger is `ReadOnly` (`_gpi_triggers.py:165-172, 182-193`).
This enforces the timing model's rule that nothing can be scheduled in the
end-of-timestep phase.

**Cancellation.** `Task.cancel()` schedules a `CancelledError` throw into the
coroutine and expects the coroutine to re-raise; a coroutine that swallows it
and returns normally becomes a `RuntimeError` (`task.py:355-368, 543-621`).
`TaskManager` (`_task_manager.py`) is structured concurrency: children are
cancelled on sibling failure and errors are collected into an
`ExceptionGroup`.

### 2.1 The timing model

`docs/source/timing_model.rst` defines five phases per timestep:
beginning-of-timestep (`Timer`, `NextTimeStep` return here), HDL evaluation,
values-change (`ValueChange`, `RisingEdge`, `FallingEdge` return here, before
downstream HDL has reacted), values-settle (`ReadWrite` returns here, writes
still allowed), end-of-timestep (`ReadOnly` returns here, writes forbidden).
It is a deliberate common subset of the VPI, VHPI, and FLI models and it is
correct. Rivet adopts it verbatim.

Note how differently the three interfaces realise it. VPI has
`cbReadWriteSynch`, `cbReadOnlySynch`, `cbNextSimTime`, `cbAfterDelay`. VHPI
uses *repetitive* callbacks `vhpiCbRepLastKnownDeltaCycle`,
`vhpiCbRepEndOfTimeStep`, `vhpiCbRepNextTimeStep`, which cocotb removes after
each fire (`VhpiCbHdl.cpp:100-116, 175-207`). FLI has no callbacks at all: a
"callback" is an MTI *process* created with a *priority*, and the priority is
the phase (`MTI_PROC_SYNCH` is ReadWrite, `MTI_PROC_POSTPONED` is ReadOnly,
`MTI_PROC_IMMEDIATE` is timer and NextTimeStep), sensitised to a signal or
scheduled with `mti_ScheduleWakeup` (`FliImpl.hpp:488-494`,
`FliCbHdl.cpp:41-126`). MTI processes cannot be destroyed, so cocotb pools
them forever (`FliImpl.hpp:48-82`), and wakeups cannot be cancelled, so a
removed callback is flagged and allowed to fire into a no-op
(`FliCbHdl.cpp:67-74`). Verilator has none of this: its phases are points in
a hand-written loop (§3.5).

The abstraction "ReadWrite means one thing" is therefore already lossy at the
GPI level, and Rivet's `Backend` trait must carry the same per-interface
mapping.

### 2.2 Write scheduling

`handle.py:769-820` is the consequence of simulators not applying
`vpiInertialDelay` writes when the standard says. Deposits are not sent to the
simulator when the user assigns; they go into a `dict` keyed by handle
(latest write wins) and a `ReadWrite` callback is registered once. When it
fires, `_apply_scheduled_writes()` flushes the dict *before* user callbacks
(`_gpi_triggers.py:180-184`). If the user is already in the ReadWrite phase
the write is applied immediately. `Force`, `Release`, `Immediate` (NoDelay)
bypass the buffer. `COCOTB_TRUST_INERTIAL_WRITES=1` turns the buffer off and
is defaulted on for Verilator, GHDL, NVC, and Questa QIS
(`runner.py:1414-1417, 1560-1563, 1879-1882`; `Makefile.inc:138-147`).

Rivet keeps both modes behind a backend capability flag.

## 3. The GPI layer and the three backends

### 3.1 The C ABI

`gpi.h` is a clean, narrow `extern "C"` surface: three opaque handle types,
six enums, about 40 functions, no structs by value. Value access is four
getters (`binstr`, `str`, `real`, `long`) and four setters (`real`, `int32`,
`binstr`, `str`) each taking a `gpi_set_action` of `DEPOSIT`, `FORCE`,
`RELEASE`, or `NO_DELAY` (`gpi.h:274-353`). Callbacks are `int (*)(void*)`
plus `void*` (`gpi.h:405-510`). Logging is dependency-injected across the ABI
with a `va_list` (`gpi.h:556-583`).

What the ABI does *not* have is telling:

- No packed vector format. `vpiVectorVal` (VPI's `aval`/`bval` word pairs) is
  used nowhere in the VPI backend. Every 4-state value crosses as an ASCII
  string of `01xz` (and `uwlh-` from VHDL).
- No 64-bit integer get or set; `gpi_set_signal_value_int` takes `int32_t`.
- No error channel. Failures are NULL, `-1`, `""`, or a log line;
  `check_vpi_error` is a no-op unless debug is on (`VpiImpl.hpp:29-33`), and
  several fatal paths call `exit(1)` from inside the simulator process
  (`VpiImpl.cpp:813`, `GpiCommon.cpp:153,164,175`).

### 3.2 The value path, end to end

The single most important finding for Rivet's performance case.

**VPI read:** the simulator formats the vector as ASCII (`vpiBinStrVal`) into
simulator-owned memory (`VpiSignal.cpp:103-110`); `gpi_get_signal_value_binstr`
copies it into a process-global `static std::string g_binstr`, upper-cases
every character with `std::transform`, and returns `c_str()`
(`GpiCommon.cpp:572-580`); `bind.cpp:476-487` makes a `PyUnicode` from it;
`handle.py` parses that into a `LogicArray`. The global buffer means the
pointer is invalidated by the next read anywhere in the process.

**VHPI read:** the simulator writes ASCII into a pre-allocated
`m_binvalue.value.str`, then the same global copy and `toupper`
(`VhpiSignal.cpp:463-489`).

**FLI read:** the simulator returns an array of enum *indices*; cocotb maps
each through `mti_GetEnumValues(type)[i][1]` (character 1 of the literal
`'0'`) into a buffer, then the same global copy and `toupper`
(`FliObjHdl.cpp:299-333`).

**VPI write:** `std::string` copied into a `std::vector<char>` for
NUL-termination, then `vpi_put_value(vpiBinStrVal)` and the simulator parses
ASCII (`VpiSignal.cpp:158-234`).

**VHPI write:** per-character `chr2vhpi` into `vhpiEnumT[]`, and the string
length must exactly equal the element count or the write errors
(`VhpiSignal.cpp:188-218, 266-287`).

**FLI write:** per-character lookup into a `char[]` of enum indices, pointer
cast to `mtiLongT`, `mti_SetSignalValue`; for *force* the value is instead
formatted as a radix string `"2#0101"` or `"10#-5"` for `mti_ForceSignal`
(`FliObjHdl.cpp:408-474, 612-622`).

Element counts are frequently guessed rather than queried: hard-coded 32/64/8
by VHPI format (`VhpiSignal.cpp:86-119`), hard-coded per-type sizes under
Questa VPI because Questa reports `vpiSize == 1` for scalars
(`VpiImpl.cpp:230-258`), a flat 32 for every FLI integer with a TODO about
VHDL-2019 64-bit integers (`FliObjHdl.cpp:543-552`), and range-derived counts
where `vpiSize` reports the flattened size of a multi-dimensional array
(`VpiObj.cpp:77-91`).

### 3.3 Callback ownership

`GpiCbHdl` (`gpi_priv.hpp:145-196`) has a four-call protocol: `arm()` (once,
after construction, because virtual dispatch does not work in constructors),
`set_cb_info()` (once), `run()` ("should delete the object if it can't fire
again"), `remove()` ("should delete the object"). Whether `run()` does
`delete this` is a per-simulator decision (`VpiCbHdl.cpp:135-174`):

- `VERILATOR`: always `vpi_remove_cb` after firing; Verilator treats callbacks
  as recurring that other simulators do not.
- `IUS || VCS || ALDEC`: `vpi_release_handle` after firing, "despite the VPI
  spec not stating that's necessary".
- everyone else: just `delete this`.
- Startup and shutdown callbacks are never removed because "too many sims get
  upset" (`VpiImpl.hpp:127-168`).

Every backend also carries an `m_removed` flag to squash a callback that fires
after a failed remove. This ownership model is one of the things Rivet must
redesign, not port: a callback record owned by the awaiting future, with a
state machine `Armed → Fired | Removed`, and per-backend knowledge of whether
a fired callback must still be removed or released.

### 3.4 Discovery and the handle cache

`GpiHandleStore` (`GpiCommon.cpp:26-61`) is one global
`std::map<std::string fullname, GpiObjHdl*>`; a duplicate discovery deletes the
new handle and returns the cached one. It is never pruned during a run.
Handle identity is therefore by fully-qualified name string, and each backend
builds that string with its own delimiter (`.` for VPI, `:`+`.` for VHPI, `/`
for FLI) and generate-index separator (`[n]`, `(n)`, or `__n` on Aldec).
`gpi_get_handle_by_name` with `GPI_NATIVE` bypasses the cache entirely
(`GpiCommon.cpp:452-463`).

Pseudo-regions are the messiest area. When `genblk[0]` is reachable but
`genblk` is not, VPI's `get_child_by_name` detects a generate label by string
comparison with `[...]` stripped and then **returns the parent's own
vpiHandle** typed as `GPI_GENARRAY` (`VpiImpl.cpp:373-481`). Indexing such a
handle is `vpi_handle_by_name(parent_fullname + "[n]")` (`:492-503`).
Multi-dimensional arrays similarly count `[` characters in the name to work
out which dimension's range applies (`VpiObj.cpp:12-94`). Because the handle
aliases its parent, destructors must special-case `GPI_GENARRAY` to avoid a
double release (`VhpiObj.cpp:16-24`).

Mixed language: everything global (`sim time`, precision, `sim_end`, all four
phase callbacks) goes to `registered_impls[0]` unconditionally
(`GpiCommon.cpp:312-334, 680-734`); name lookup fans out over every impl;
index lookup crosses none, because Xcelium's VPI once returned VHDL handles it
could not use (`GpiCommon.cpp:471-479`). Cross-language iteration works by
stringifying names or passing raw `void*` handles between impls
(`gpi_next`, `GpiCommon.cpp:519-562`).

### 3.5 Verilator

Verilator is not a simulator you inject into; it emits a C++ model and cocotb
supplies `main()`. The runner invokes
`verilator -cc --exe --vpi --public-flat-rw --prefix Vtop ...` with
`verilator.cpp` in the source list and `-lcocotbvpi_verilator` in `LDFLAGS`,
then `make -f Vtop.mk` (`runner.py:1919-1998`). `--public-flat-rw` is what
makes internal signals visible and writable through VPI.

`verilator.cpp:174-231` is the scheduler:

```
while (!gotFinish) {
  do {
    do { eval_step; clearEvalNeeded; doInertialPuts; settle_value_callbacks }
    while (evalNeeded);
    callCbs(cbReadWriteSynch); doInertialPuts; settle_value_callbacks
  } while (evalNeeded);
  eval_end_step;
  callCbs(cbReadOnlySynch);
  trace dump;
  next = min(VerilatedVpi::cbNextDeadline(), top->nextTimeSlot());
  if (next == none) break;        // no pending cocotb callback ⇒ simulation ends
  main_time = next;
  callCbs(cbNextSimTime); settle_value_callbacks;
  callTimedCbs();       settle_value_callbacks;
}
```

Observations that matter for Rivet:

- `cbReadWriteSynch` and `cbReadOnlySynch` are not simulator events; they are
  points where this loop chooses to call `VerilatedVpi::callCbs`.
- `settle_value_callbacks` (`:49-61`) loops `callValueCbs()` to a fixpoint,
  because value callbacks can write signals. That is the delta-cycle
  substitute.
- Time jumps straight to the next cocotb `cbAfterDelay` deadline or Verilator
  timing event. If cocotb has no pending timer, the simulation ends
  (`:206-218`). The cocotb `Clock` (a C++ `GpiClock` re-registering `cbAfterDelay`
  each half-period, see §4.5) is what drives time forward at all.
- `Verilated::fatalOnVpiError(false)` is required "otherwise it will fail on
  systemtf" (`:154`); the trace object is deliberately leaked "to avoid
  deadlock" (`:66-71`).

Whoever owns `main()` owns all of this. Rivet does, in Rust, and can drive the
clock from its own timer wheel instead of through `cbAfterDelay`.

### 3.6 Simulator-specific behaviour, complete list

Every one of these is a fact about a simulator, not about cocotb, and each
must exist in Rivet's backends with a test where a free simulator allows one.

**VPI**

| Guard | Location | Behaviour |
|---|---|---|
| `GHDL` | `VpiImpl.cpp:94-100` | vectors classify as `LOGIC_ARRAY` not `PACKED`; VHDL has no packed/unpacked |
| `IUS` | `VpiImpl.cpp:163-169` | `vpiUndefined` constant type → guess `LOGIC_ARRAY` |
| `IUS` | `VpiImpl.cpp:382-404` | validate a `vpiGenScope` by iterating internal scopes first; Xcelium segfaults on an invalid scope and never returns `vpiGenScopeArray` |
| `!IUS` | `VpiImpl.cpp:406-452` | generate-prefix fallback for Icarus, Verilator, Questa, which lack `vpiGenScopeArray` |
| `MODELSIM` | `VpiImpl.cpp:230-258` | hard-coded element counts; Questa reports `vpiSize == 1` for all scalar types |
| `MODELSIM \|\| VCS` | `VpiObj.cpp:46-54` | `vpiRange` iterator always restarts at dimension 0; scan forward |
| Questa | `VpiImpl.cpp:504-518` | `vpi_handle_by_index` on 2-D arrays returns NULL → pseudo-handle path |
| `MODELSIM \|\| IUS` | `VpiSignal.cpp:195-206` | `vpiStringVar` deposits must use `vpiNoDelay` |
| `ICARUS` | `VpiImpl.cpp:763-767` | `vpi_control` returns void; skip return check |
| `VCS` | `VpiImpl.cpp:843-852` | VCS loads the plugin at compile time too; bail out if `vpi_get_vlog_info` fails |
| Xcelium | `VpiImpl.cpp:635-639` | skip root objects whose full name starts with `\` (virtual classes at top scope) |
| Icarus gh-4067, Xcelium gh-4013, Questa gh-4105 | `VpiCbHdl.cpp:45-69` | callback re-entrancy queue; these react to `vpiNoDelay` writes inside the write |
| `VERILATOR` | `VpiCbHdl.cpp:109-116, 145-161` | always `vpi_remove_cb` after firing; callbacks are recurring |
| `IUS \|\| VCS \|\| ALDEC` | `VpiCbHdl.cpp:162-166` | `vpi_release_handle` after a callback fires |
| `!IUS` | `VpiCbHdl.cpp:236-243` | startup is `cbStartOfSimulation`; on Xcelium it must be `cbAfterDelay(0)` |
| Aldec | `VpiIterator.cpp:25-41` | `vpiModule`, `vpiModuleArray`, `vpiInterface`, `vpiInterfaceArray` removed from iteration lists: SEGV on mixed language |
| `IUS` | `VpiIterator.cpp:51-53` | `vpiNetArray` excluded from struct iteration |
| Questa | `VpiImpl.hpp:266-275` | iterate `vpiInstance`, not `vpiPackage` |
| Xcelium | `VpiIterator.cpp:228-245` | skip iterated objects with NULL names |
| all | `VpiImpl.hpp:127-168` | never remove startup/shutdown callbacks |

**VHPI**

| Guard | Location | Behaviour |
|---|---|---|
| `IUS` | `VhpiObj.cpp:28-34, 60-64`; `VhpiImpl.cpp:719-751` | `vhpiIsUnconstrainedP` unset; compare bounds against magic `2147483647` |
| `IUS` | `VhpiImpl.cpp:773-779` | unconstrained flag wrong on base type; retry on subtype |
| `MODELSIM` | `VhpiObj.cpp:68-80, 111-120` | `vhpiIsUpP` wrong (cocotb #4236); infer direction from `left < right` |
| `ALDEC` | `VhpiImpl.hpp:26-34` | generate index separator `__n` instead of `(n)` |
| Aldec | `VhpiImpl.cpp:174-201` | enum literals compared with and without quotes; Aldec omits them |
| Aldec | `VhpiIterator.cpp:120-123` | `vhpiRootInstK` is a null hierarchy level |
| `NVC` | `VhpiImpl.cpp:274-285` | case-insensitive name compare (nvc#723) |
| `NVC` | `VhpiImpl.cpp:516-519` | upper-case all fully qualified names |
| all | `VhpiImpl.cpp:301-312` (5×) | `vhpiBaseType` → fallback `vhpiSubtype` → `vhpiBaseType` |
| all | `VhpiImpl.cpp:327-332` | `vhpiElemType` → fallback `vhpiElemSubtype` |
| all | `VhpiImpl.cpp:559-578` | record children via `vhpiSelectedNames`; `vhpi_handle_by_name` fails for records in generics |
| all | `VhpiImpl.cpp:833-858` | `vhpi_handle_by_index` → fallback linear scan of `vhpiIndexedNames` |
| all | `VhpiSignal.cpp:465-473` | zero-length arrays: return `""` without calling the simulator |
| all | `VhpiImpl.cpp:884-982` | three-stage root discovery; `:`-prefixed lookup to disambiguate library objects |
| all | `VhpiSignal.cpp:35-52` | `DEPOSIT` and `NO_DELAY` both map to `vhpiDepositPropagate`; VHPI has no inertial/immediate distinction here |

VHDL type mapping is by *content* of enum types, not by kind:
`is_enum_logic` matches names `BIT`/`STD_ULOGIC`/`STD_LOGIC` or a literal set
of `{'0','1'}` or the nine `std_logic` values; `is_enum_char` is `CHARACTER`
or exactly 256 literals; `is_enum_boolean` is `BOOLEAN` or `{FALSE, TRUE}`
(`VhpiImpl.cpp:161-272`). `bit` and `std_logic` are indistinguishable at the
GPI level.

**FLI** (Questa VHDL): handles are three incompatible C types
(`mtiRegionIdT`, `mtiSignalIdT`, `mtiVariableIdT`) discriminated out of band,
so every access is an `if (m_is_var)` pair (`FliImpl.hpp:173-217`);
`GPI_ARRAY` and `GPI_STRUCTURE` have no value access at all
(`FliObjHdl.cpp:96-156`); variables and reals cannot be forced; argc/argv
must be recovered through the embedded Tcl interpreter (`FliImpl.cpp:479-536`),
so the FLI library links Tcl; `sim_end` is `mti_Quit` at time zero and
`mti_Break` otherwise (`FliImpl.cpp:20-27`); four generate kinds are
`#ifdef`-guarded because the constants do not exist in every Questa version
(`FliImpl.cpp:709-720`). cocotb keeps FLI despite all this because it is faster
than Questa's VHPI.

**GpiCommon**: `gpi_get_handle_by_index` never crosses language boundaries
(`GpiCommon.cpp:471-479`).

## 4. The Python object model and the hot path

### 4.1 Handles

`SimHandleBase` (`handle.py:65`) holds a `sim_obj` C pointer and a path
string; everything else is a `cached_property`. Identity is the pointer, and
`_make_sim_object` (`:1868-1897`) interns Python objects in a process-global
`_handle2obj` dict so handles reached by different routes are the same
object. `dut.sub.sig` is `__getattr__` → `_get` → a `_sub_handles` dict hit,
or on a miss `get_handle_by_name` and a new object (`:292-327, 455-462`).
Full discovery (`_discover_all`, `:253-290`) runs only on iteration, never on
attribute access. Neither cache is ever pruned.

Generate-loop naming is normalised in Python by three regexes tried in order:
`name__X` (Aldec VHPI), `name(X)` (FLI and VHPI), `name[X]` (VPI)
(`:560-578`). Escaped identifiers must be passed as `dut["\\name\\"]`. The
type → class table is at `:1851-1865`; 2.1 added `PackedObject` so Verilog
packed structs read as one vector.

### 4.2 Values

Every 4-state value crosses the boundary as an ASCII binary string.
`LogicObject.get()` is `get_signal_val_binstr()` → `Logic(binstr)`;
`LogicArrayObject.get()` is `get_signal_val_binstr()` →
`LogicArray._from_handle` (`:1162-1165, 1306-1314`). Writes of an `int` up to
32 bits go through `set_signal_val_int`, which is `static_cast<int32_t>` in
`bind.cpp:562`; anything wider is formatted with `f"{value:0{len}b}"` and
sent as a string (`:1261-1304`). `ArrayObject.get()` does one GPI read per
element (`:1010-1028`). `IntegerObject.get()` on anything wider than 32 bits
is `int(get_signal_val_binstr(), 2)` (`:1689-1699`).

`LogicArray` (`types/_logic_array.py:35`) keeps three lazily materialised
representations (`list[Logic]`, `int`, `str`) and funnels conversions through
the string form: array → `"".join(str(v) ...)`, int → `format(v, "0Nb")`,
str → `translate` then `int(s, 2)` (`:319-354`). `_from_handle` discards the
HDL range and substitutes `Range(len-1, "downto", 0)` unconditionally
(`:552-562`), which is the silent 1.x → 2.x indexing change documented in
`update_indexing.rst`. `Logic` is a 9-state `std_ulogic` value interned per
state (`types/_logic.py:118-124`); its operators are tuple lookups
reconstructed through `Logic(...)` (`:149-219`). X resolution
(`COCOTB_RESOLVE_X`) is read once at import and *changes the method bodies*
of `__int__` and `__bool__` at class-creation time (`_logic.py:241-265`,
`_logic_array.py:965-977`).

### 4.3 Cost of one cycle

For `await RisingEdge(clk); x = dut.sig.value; dut.other.value = y`:

**Prime side (per await):** `RisingEdge.__new__` returns the per-signal
cached `rising_edge` trigger, so the trigger is reused. `Task._resume` then
calls `trigger._register`, which allocates a `TriggerCallback`, inserts it in
a dict, and `_prime`s: one C call that does `PyTuple_GetSlice`, `new
PythonCallback` (three increfs), `new VpiValueCbHdl`, `vpi_register_cb`, and
`PyObject_New` for the returned callback handle (`bind.cpp:328-441`).

**Fire side (per edge):** simulator → `VpiValueCbHdl::run` (one
`vpi_get_value(vpiBinStrVal)`, global-string copy, `toupper`, `strcmp`) →
`handle_gpi_callback` (`PyGILState_Ensure`, `PyObject_Call`, `delete`) →
`GPITrigger._react` (a `nullcontext` enter/exit for profiling, a global
assignment) → `_do_callbacks` (swap the dict for a fresh one, iterate) →
`Task._schedule_resume` → `EventLoop.schedule` (allocate `ScheduledCallback`,
deque append) → `EventLoop.run` (popleft, `Task._resume`, `coro.send`) →
`run_bridge_threads` (iterate an empty list) → back in C++,
`vpi_remove_cb` and `delete this`. Roughly 15 to 25 Python-level calls and 6
heap allocations per edge for one task. Falling edges of the clock never
reach Python for a `RisingEdge` waiter, but each still costs the string read
in C++.

**Read:** `__getattr__` → `_get` → dict hit → `value` property → `get()` →
`get_signal_val_binstr`: simulator formats to ASCII, copy into the global
`std::string`, `toupper` over N chars, `PyUnicode_FromString` (third copy),
then `LogicArray._from_handle` (object, five slot stores, a `Range` and a
built-in `range`). About five Python calls, three O(N) copies, three
allocations, one GPI call. `int(x)` adds a `translate` and `int(s, 2)`.

**Write:** property setter does `current_gpi_trigger()` plus `isinstance`
checks; `_set_value` calls `len(self)`, which is an *uncached*
`get_num_elems()` GPI call (`:1364-1367`), then `set_signal_val_int` or an
O(N) format string and a second `len(self)`; `_schedule_write` does another
`current_gpi_trigger()`, a `dict.pop`, a `dict.__setitem__` keyed by handle,
and a tuple allocation. The GPI write itself is deferred to the next
`ReadWrite`, which is one more simulator callback and Python re-entry per
timestep, unless `COCOTB_TRUST_INERTIAL_WRITES` is set. The docs credit that
flag with "noticeable performance improvements" for exactly this reason
(`library_reference.rst:820-846`).

Net: a minimal one-in, one-out cycle is about 40 Python-level operations,
3 to 5 GPI calls, 8 to 12 heap allocations, and at least three full string
copies of every 4-state value in each direction.

### 4.4 The embedding

`embed.cpp:89-206` reads `PYGPI_PYTHON_BIN`, runs `Py_InitializeFromConfig`,
sanity-checks `sys.executable`, and registers start/end/finalize GPI hooks.
Start-of-sim takes the GIL, imports `pygpi.entry`, and calls each `PYGPI_USERS`
entry in order (coverage, logging, `_init`, `_run_regression`). `GPI_USERS`
must list `libpython` *before* the PyGPI entry point so that the interpreter's
symbols are loaded `RTLD_GLOBAL` for extension modules (`runner.py:284-303`,
`dynload.cpp:40`). Every callback into Python is `PyGILState_Ensure` →
`PyObject_Call` → release; an uncaught Python exception is printed, cleared,
and turned into a simulator shutdown because "any subsequent calls will go
back to Python which is now in an unknown state" (`bind.cpp:163-196`).
`PythonCallback` carries an explicit padding field to work around an FLI bug
(`bind.cpp:28-44`).

### 4.5 Clock

cocotb 2.x has two clock implementations selected by `impl`, defaulting to
the C++ `GpiClock` when inertial writes are trusted and the Python one
otherwise (`clock.py:195-203`). `GpiClock` (`bind.cpp:851-931`) toggles the
signal with `gpi_set_signal_value_int` and re-registers a timed callback for
the next edge without entering Python. The Python implementation is a
`while True` over two `Timer`s with a `Deposit` per half-period
(`clock.py:329-339`), so two Python re-entries per period. `Clock.cycles(n)`
replaces `n-1` edge callbacks with one `Timer` (`:359-401`).

### 4.6 Ergonomics to keep and footguns to avoid

Keep: `dut.sub.sig` and `dut["escaped"]`; lazy per-name lookup and full
discovery only on iteration; handle identity by simulator pointer;
`Logic` with `std_ulogic` semantics; `LogicArray` with inclusive HDL ranges,
direction, slicing, and `b`/`x`/`d` format specs; `Deposit`/`Immediate`/
`Force`/`Freeze`/`Release` as first-class values; `is_const`;
`signal.rising_edge` as an attribute; `Clock` with duty cycle and `cycles()`;
sim-time-stamped log lines; `get_sim_time(unit)` with exact rational
arithmetic and explicit rounding modes.

Avoid: `dut.arr.value[0] = 1` silently doing nothing (`handle.py:1045-1049`);
`RisingEdge` firing on any transition *to* 1 on most simulators
(`_gpi_triggers.py:289-291`); `Logic` being unhashable; an environment
variable rewriting `__bool__` semantics at import; `.value` losing the HDL
range; `StringObject` truncating silently but `FixedStringObject` raising
(`:1767-1772, 1815-1829`); `EnumObject` unable to report literal names
(`:1536-1537`); Verilog `integer` and `time` mapping to `LogicArrayObject`
rather than `IntegerObject` (`:1626-1628`); no slicing on any handle; caches
that are never pruned.

## 5. Launch, regression, and pytest

### 5.1 Two implementations of one thing

`runner.py` (2417 lines) and `makefiles/` (~1500 lines) each implement "build
the design, run the simulator with the plugin loaded, read `results.xml`".
The knowledge in them is essential and has drifted between them:

- Questa version auto-detection (dispatch to `Makefile.questa-compat` before
  2025.2 or `Makefile.questa-qisqrun` after) exists only in make; the runner
  requires the user to pick `questa-qisqrun` explicitly (`runner.py:1192-1206`).
- Icarus `+dumpfile_path=` exists only in the runner (`runner.py:932-950`).
- The Verilator minimum-version check (5.036) exists only in make
  (`Makefile.verilator:25-30`).
- The GHDL time-resolution table, the NVC `--preserve-case` probe, the Questa
  VHDL `-t` rule, and the Xcelium `-NEW_VHPI_PROPAGATE_DELAY` define (for
  cocotb #1076) are all duplicated.

Per-simulator plugin loading, which Rivet's runner ports directly:

| Simulator | How the plugin is loaded |
|---|---|
| Icarus | `vvp -m <lib>` |
| Questa | `-pli <lib>` (VPI); `-foreign "cocotb_init <lib>"` (FLI); `-foreign "vhpi_startup_routines_bootstrap <lib>"` plus `-voptargs=-access=rw+/.` (VHPI) |
| GHDL | `--vpi=<lib>` |
| NVC | `--load=<lib>` |
| Riviera / Active-HDL | `asim -pli <lib>` or `-loadvhpi <lib>` inside a generated `.do` script; batch mode is implied by stdout redirection |
| Verilator | linked at build time |
| Xcelium | `-loadvpisim <lib>:vlog_startup_routines_bootstrap`; VHPI is never loaded directly because it errors, VHDL goes through `GPI_EXTRA` |
| VCS | `-load <lib>` at *compile* time |
| DSim | `-pli_lib <lib>` on both image and run |

Design-access flags without which handles are invisible: `+acc`,
`-access +rwc`, `+access +w_nets`, `--public-flat-rw`,
`-voptargs=-access=rw+/.`. A note at `runner.py:276-279` reads "We have to
set all environment variables before building because Xcelium and VCS load
VPI for some reason. TODO: Remove this."

### 5.2 The results contract

The simulator's exit code cannot be set from inside cocotb. `Makefile.inc:48-56`
says so: "Check that the COCOTB_RESULTS_FILE was created, since we can't set
an exit code from cocotb." A test failure does not kill the simulator;
`_tear_down` writes the JUnit XML then calls `stop_simulator()` →
`gpi_finish()` (`regression.py:455-478`). The parent sums
`tests`/`failures`/`errors` across `<testsuite>` elements
(`check_results.py`). A *missing* file means the simulator died. Conversely a
premature simulator end is detected via the end-of-sim callback and turned
into a `SimFailure` with the message about "is your clock running?"
(`regression.py:1111-1128`).

Rivet keeps the JUnit dialect and the properties CI consumes (`cocotb=True`,
`sim_time_*`, `random_seed`, `[[ATTACHMENT|path]]`), and additionally reports
over a pipe to the parent so exit codes are real.

### 5.3 Test lifecycle

`start_regression` optionally shuffles (`COCOTB_RANDOM_TEST_ORDER`), then
stable-sorts by `stage`, then applies `re.search` filters
(`regression.py:344-376`). The first test runs immediately; every later test
is deferred onto `Timer(1)`, so consecutive tests are one timestep apart
(`:417-422`). Per-test seed is
`regression_seed + int(sha1(test.fullname), 16)`, with `random` state saved
and restored around each test (`:435-453, 488-491`). A `TestManager` owns the
main task and every child; a `Timer(timeout)` fires `_abort`, which cancels
every task and waits for the set to empty (`_test_manager.py:34-197`).
Scoring is a precedence ladder: skip → xfail → `TestSuccess` → `expect_error`
→ `expect_fail` → pass/fail (`regression.py:511-647`).

### 5.4 The pytest plugin

Two processes, two pytest sessions. The parent collects, converts
`@cocotb.test` objects into marked plain functions, and launches the simulator
from an `hdl` fixture. The child, inside the simulator, builds a *second*
`RegressionManager` that constructs its own pytest `Config`, re-parses the
parent's argv, and drives `pytest_runtest_protocol` by hand because each
phase must suspend on simulator callbacks (`_pytest/_regression.py:86-283`).
Reports go back over a `multiprocessing.connection` socket to a listener
thread that re-emits `pytest_runtest_logreport` under an `RLock`
(`_controller.py:74-89, 362-384`). Async fixtures are wrapped in
`TestManager`s with a fake cached-result tuple (`_fixture.py:15-54`). pytest's
logging plugin detaches handlers when its context manager exits, which
happens as soon as the async function is *scheduled*, so handlers are
re-attached and stripped around every phase (`_regression.py:809-852`).

The complexity here is not the IPC. It is forcing a synchronous test protocol
to suspend across simulator callbacks. Rivet's test protocol is async from
the start.

### 5.5 Smaller defects worth not repeating

- `test()` catches `CalledProcessError` but the command runner raises
  `RuntimeError`, so the "simulator failed but left results" path is dead
  (`runner.py:656-661, 740-744`).
- `plusargs += [...]` on instance lists in Icarus, DSim, and GHDL runners
  means a second `test()` call accumulates flags (`runner.py:1016-1022,
  1483, 2318-2325`).
- Several builders emit `"-incdir <dir>"` as one argv element with
  `shell=False` (`runner.py:2013-2027, 2065-2087, 2296, 2214`).
- `outdated()` rebuild avoidance is only used by Icarus, Aldec, VCS, DSim;
  Questa, Xcelium, GHDL, NVC, Verilator always rebuild (`runner.py:869-886`,
  `:1932`).
- Aldec `.do` temp files are never deleted (`runner.py:1665-1667, 1777-1779`).
- `TOPLEVEL_LANG` is exported and read nowhere in `src/`.

## 6. What Rivet keeps, ports, and replaces

**Keep as-is (semantics):** the five-phase timing model and its trigger set;
the illegal-transition rules; Deposit/Force/Release/NoDelay; write buffering
with a per-simulator trust flag; drain-to-idle scheduling inside callbacks;
FIFO task order; `Timer(1)` between tests; stage ordering; per-test seeding
from `(run_seed, test name)`; the JUnit output shape; the per-simulator flag
tables.

**Port deliberately (knowledge):** every row in §3.6; the VHDL enum-content
type heuristics; the pseudo-region and multi-dimensional array logic; root
discovery fallbacks; the Verilator loop ordering; the plugin-loading and
design-access flags in §5.1; the results-file-means-alive contract.

**Replace (implementation):**

| cocotb | Cost | Rivet |
|---|---|---|
| Embedded CPython, GIL, `PYGPI_PYTHON_BIN`, `libpython` preload | interpreter start, per-callback GIL, module import | native code in the plugin |
| ASCII `binstr` values through one global `std::string` | alloc + copy + `toupper` per read, parse per write, not re-entrant | `vpiVectorVal` / `vhpiLogicVecVal` / enum-index arrays into an `aval`/`bval` vector, zero-copy where the simulator allows |
| Edge detection by `strcmp` on a binstr | one string read per value change | scalar integer read, or the value delivered in `cb_data` |
| One-shot triggers re-registered every await | `vpi_register_cb` + remove per edge per signal | a persistent per-signal value-change registration fanned out in Rust |
| `GpiCbHdl` self-deleting ownership | per-simulator `delete this` rules | future-owned callback record with an explicit state machine and backend-declared post-fire action |
| Name-keyed global handle map, never pruned | string identity, delimiter per backend | integer handle table with cached `ObjInfo` |
| `dict`-based write cache in Python | hashing per write | insertion-ordered `Vec` with index map |
| `Clock`: C++ `GpiClock` when inertial writes are trusted, otherwise a coroutine awaiting `Timer` twice per period | one timed VPI callback per edge; two Python re-entries per period in the fallback | timer-wheel-driven clock in the executor; on Verilator, edge waiters woken without VPI |
| Two launch implementations (runner + make) | drift | one runner crate; content-hashed builds |
| Two pytest sessions with socket IPC and manual protocol driving | fragility | libtest-compatible harness, async test protocol from the start |
| No exit code from inside the simulator | results file as liveness signal | results file kept for CI, plus a pipe to the parent for real exit codes |
| `set_signal_val_int` capped at `int32_t`; wider writes go through a formatted string | O(N) format per write over 32 bits | `u64`/`u128`/`BigUint` writes straight into the vector encoding |
| `len(self)` on every write is an uncached `get_num_elems()` | extra GPI call per write | width cached in the handle table at discovery |
