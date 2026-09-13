# VHDL

Rivet runs VHDL on two simulators. GHDL goes through its VPI, the same
backend Icarus uses. NVC goes through VHPI, a separate backend in
`crates/rivet-vhpi`, verified on NVC 1.23.

The testbench is the same either way: an `async fn` marked with
`#[rivet::test]` that takes the design as a `Module`. What differs is how
much of the design the simulator's interface lets the harness see.
`examples/vhdl_types` is one design run on both, written so that a test
skips itself where the simulator cannot answer.

## The manifest

```toml
[design]
top = "tb_top"
language = "vhdl"
sources = ["hdl/types_pkg.vhd", "hdl/alu.vhd", "hdl/tb_top.vhd"]

[sim.nvc]
args = ["--std=2008"]

[sim.ghdl]
args = ["--std=08"]
```

`language = "vhdl"` is required. Sources are analysed in the order they are
listed, so a package comes before the entity that uses it.

## One backend per library

A test crate picks its backend with a Cargo feature. A `cdylib` carrying
both VPI and VHPI fails to load on NVC: a VHPI simulator does not provide
the VPI symbols and resolves the library eagerly. `examples/vhdl_types` and
`examples/dff_vhdl` are both written this way:

```toml
[dependencies]
# One backend per simulator: GHDL through VPI, NVC through VHPI.
rivet = { workspace = true, default-features = false }

[features]
default = ["vpi"]
vpi = ["rivet/vpi"]
vhpi = ["rivet/vhpi"]
```

`rivet run --sim nvc` builds the library with
`--no-default-features --features vhpi` for you. Any other simulator uses
the crate's default features.

## The testbench shape

The portable shape is a top-level entity with no ports, with every signal
the harness drives declared inside the architecture. That is the usual VHDL
testbench, and it works on both simulators. From
`examples/vhdl_types/hdl/tb_top.vhd`:

```vhdl
entity tb_top is
  generic (
    WIDTH : positive := 8
  );
end entity;

architecture sim of tb_top is
  signal clk    : std_logic := '0';
  signal rst_n  : std_logic := '0';
  signal cmd    : cmd_t     := (op => OP_NOP, valid => '0', data => (others => '0'));
  signal b      : std_logic_vector(WIDTH - 1 downto 0) := (others => '0');
  signal result : std_logic_vector(WIDTH - 1 downto 0);
  signal done   : std_logic;
  -- A boolean and an integer, to exercise the other type classes.
  signal armed  : boolean := false;
  signal cycles : integer := 0;
begin
  dut : entity work.alu
    generic map (WIDTH => WIDTH, TAPS => 4)
    port map (clk => clk, rst_n => rst_n, cmd => cmd, b => b, result => result, done => done);
end architecture;
```

The design under test is an instance inside it, reached with
`dut.module("dut")`. A top-level entity that does have ports also works, and
`examples/dff_vhdl` is written that way, but the port-less form is what a
VHDL flow usually produces and it needs no elaboration-time driver for the
ports.

## What each simulator exposes

| Property | GHDL 4.1 (VPI) | NVC 1.23 (VHPI) |
|---|---|---|
| Record members by name | no | yes, through `vhpiSelectedNames` |
| Enumeration literal names | no; an enumeration reads back as a vector | yes, `enum_literals()` and `enum_name()` |
| VHDL `integer` signals | read back as a vector | reported as an integer |
| `boolean` signals | one bit | one bit |
| Generate elements | the label resolves to its first element, so elements are found by scanning the enclosing scope | each element is its own region `label(i)`; the backend synthesises the array |
| Generics among a region's children | no | yes, marked constant |
| Case of reported names | as written | upper case; lookups are case-insensitive |
| Deposits | trusted to the simulator | buffered by the runtime and applied at ReadWrite |
| Waveform control from a test | none; `--wave` covers the run | none; `--wave` covers the run |

Both give four-state values, and both report force as supported, though no
example forces a VHDL signal.

`examples/dff_vhdl` logs what each one lists:

```text
NVC:  children: ["CLK:Logic", "RST_N:Logic", "D:LogicVec", "Q:LogicVec",
                 "COUNT:LogicVec", "COUNT_I:LogicVec", "CYCLES:Integer", "WIDTH:Integer"]
GHDL: children: ["clk:Logic", "rst_n:Logic", "d:LogicVec", "q:LogicVec",
                 "count:LogicVec", "count_i:LogicVec", "cycles:LogicVec"]
```

Two things to read out of that. NVC reports the generic `WIDTH` and types
the VHDL `integer` `cycles` as an integer; GHDL lists no generic and gives
`cycles` as a vector. A direct `dut.signal("WIDTH")` lookup still resolves
on GHDL, so a generic is reachable by name even though it is not in the
hierarchy, which means `dut.children()` and `rivet bindgen` do not see it.

## Records

A record port is a module-like object whose members are its children.

```rust
let cmd = dut.module("cmd")?;
let op = cmd.signal("op")?;
let valid = cmd.signal("valid")?;
let data = cmd.signal("data")?;
assert_eq!(data.width(), 8, "record member width");
```

This works on NVC. GHDL's VPI does not expose record members at all, so
`dut.module("cmd")` fails there.

## Enumerations

Where the simulator reports an enumeration's literals, a test can say what a
value means instead of which position it holds.

```rust
let Some(lits) = op.enum_literals() else {
    return Err(rivet::skip("this simulator does not report enumeration literals"));
};
assert_eq!(lits, ["OP_NOP", "OP_ADD", "OP_SUB", "OP_XOR"], "op_t literals in order");
op.set(2u64);
clk.rising_edge().await;
read_only().await;
assert_eq!(op.enum_name().as_deref(), Some("OP_SUB"));
```

`enum_literals()` returns the literal names in position order, and
`enum_name()` the name of the current value. Both return `None` where the
simulator does not report them, which is GHDL and every Verilog simulator.

Writes still go by position: `op.set(2u64)` deposits `OP_SUB`.

The VHPI backend classifies a type by the content of its enumeration rather
than by kind, because VHPI has no other way to tell `std_logic`, `bit`,
`boolean` and `character` apart: nine logic literals mean logic, `FALSE` and
`TRUE` mean boolean, 256 literals mean a character, anything else is an
ordinary enumeration.

## Generate regions

A VHDL for-generate has no parent object on either simulator. Each element
is its own region named `label(i)`, and the label itself resolves to
nothing on NVC and to the first element on GHDL. Both backends synthesise
the array, so the test is the same:

```rust
let alu = dut.module("dut")?;
let taps = alu.module("tapgen")?;
for i in 0..4 {
    let tap = taps.index(i)?.as_module()?.signal("tap")?;
    assert_eq!(tap.width(), 1, "tapgen[{i}].tap");
}
// The generic reached the instance: TAPS = 4, so there is no fifth.
assert!(taps.index(4).is_err(), "only four taps were generated");
```

This passes on both.

## Skipping instead of failing

A test that needs something the simulator in front of it cannot do should
end as skipped, not as a failure. `rivet::skip(reason)` returns an `Error`
that the runner reports as `SKIP`, so one source file runs everywhere and
the run's exit status still means something.

`examples/vhdl_types` funnels every record access through one helper:

```rust
/// GHDL's VPI does not expose VHDL record members, so tests that need them
/// skip themselves there rather than failing.
fn need_records(dut: &Module) -> rivet::Result<Module> {
    dut.module("cmd").map_err(|_| rivet::skip("this simulator does not expose VHDL record members"))
}
```

and each test that needs a record starts with `let cmd = need_records(&dut)?;`.
The result, on the same six tests:

| Test | NVC 1.23 | GHDL 4.1 |
|---|---|---|
| `clock_and_reset_drive_internal_signals` | pass | pass |
| `enum_literals_have_names` | pass | skip |
| `generate_region_elements_exist` | pass | pass |
| `generate_region_follows_the_accumulator` | pass | skip |
| `record_members_are_addressable` | pass | skip |
| `list_children` (opt-in hierarchy dump) | skip | skip |

Use the same pattern for anything else a backend may not have. Probing the
capability, rather than the simulator name, keeps the test honest when a
tool gains the feature later.

`list_children` is a discovery aid worth copying: it dumps what the
simulator exposes, and skips unless `RIVET_LIST_CHILDREN=1` is set, so it
costs nothing in a normal run.

```sh
RIVET_LIST_CHILDREN=1 rivet run --sim nvc -C examples/vhdl_types --filter list_children
```

## Writes, time and waveforms

VHPI has no inertial/immediate distinction: `set` and `set_now` both become
`vhpiDepositPropagate`, and the runtime buffers deposits until ReadWrite as
it does on Icarus and Verilator. On GHDL the VPI backend trusts the
simulator's own inertial writes, which is cocotb's default for GHDL.

Neither VHDL flow can start or stop a waveform from inside a test.
`rivet run --waves` passes `--wave` to the simulator, which covers the whole
run: `sim_build/ghdl/<top>.ghw` and `sim_build/nvc/<top>.fst`. The functions
in `rivet::waves` return `false` there.

## Running the examples

```sh
rivet run --sim ghdl -C examples/dff_vhdl     # 2 tests
rivet run --sim nvc  -C examples/dff_vhdl     # the same 2 tests
rivet run --sim ghdl -C examples/vhdl_types   # 2 pass, 4 skip
rivet run --sim nvc  -C examples/vhdl_types   # 5 pass, 1 skip
```

Per-simulator behaviour, including which workarounds have run on a tool and
which are carried from cocotb's catalogue unverified, is catalogued in
`docs/SIMULATOR-QUIRKS.md`.
