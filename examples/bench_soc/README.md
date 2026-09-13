# bench_soc

Harness overhead on a real design. [PicoRV32](https://github.com/YosysHQ/picorv32)
(`hdl/picorv32.v`, ISC licensed, vendored unmodified with its notice) runs a
two-instruction loop out of a memory, and the core's memory interface is
visible to the testbench.

```sh
RIVET_BENCH_N=100000 rivet run --sim icarus --release -C examples/bench_soc
# the same simulation with no harness, for the comparison:
iverilog -g2012 -o bare.vvp hdl/picorv32.v hdl/soc.v hdl/tb_soc.v && vvp bare.vvp +cycles=100000
```

`ci/bench.py` does both and reports the difference. Numbers are in
[`../../docs/benchmarks.md`](../../docs/benchmarks.md).
