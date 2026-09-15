# rivet (Python)

Hardware testbenches in Python, on Rivet's native scheduler.

```python
import rivet

@rivet.test(timeout="100us")
async def counts_when_enabled(dut):
    clk = dut.signal("clk")
    rivet.Clock(clk, "10ns").start()

    dut.signal("rst_n").set(0)
    await clk.rising_edge(n=2)
    dut.signal("rst_n").set(1)

    count = dut.signal("count")
    dut.signal("en").set(1)
    for i in range(1, 21):
        await clk.rising_edge()
        await rivet.read_only()
        assert count.get() == i, f"cycle {i}"
        await rivet.next_time_step()
```

```sh
rivet run --python --sim icarus
```

Full documentation: [`docs/python.md`](../../docs/python.md).

## Layout

| path | what it is |
|---|---|
| `src/rivet/` | the Python package: the decorator, the re-exports, `with_timeout`, `log` |
| `crates/bridge/` | the bindings — coroutine driver, triggers, handles, values, registry, kit, mock |
| `crates/ext/` | the `_rivet` extension module, for `import rivet` outside a simulator |
| `crates/plugin/` | the VPI/VHPI plugin the simulator loads, with CPython embedded |
| `tests/` | the suite, run against the in-process mock simulator |
| `examples/counter/` | a testbench and the design it drives |

This is a second cargo workspace. These crates need a Python interpreter
to build, so they stay out of the root one — `cargo test --workspace` at
the top of the repository must not need libpython.

## Building

```sh
# The extension module, for the tests and for `import rivet` on its own.
cargo build -p rivet-python-ext
mkdir -p build && cp target/debug/lib_rivet.so build/_rivet.so
(cd tests && python3 -m unittest discover)

# The plugin the simulator loads. `rivet run --python` builds this for you
# from a checkout; this is the explicit form.
cargo build --release -p rivet-python-plugin
cargo build --release -p rivet-python-plugin --no-default-features --features vhpi   # NVC

# A wheel.
maturin build --release
```

Building anything here needs a Python development install: `libpython`
and its headers.

Do not build the extension module and the plugin in one `cargo build`
invocation: cargo unifies features across a build, and the wheel's
`extension-module` feature tells pyo3 not to link libpython, which the
plugin needs. Build them with separate `-p` selections, as above.
