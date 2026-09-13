"""Run the cocotb baseline: python3 run.py [icarus|verilator]"""
import sys
from pathlib import Path

from cocotb_tools.runner import get_runner

sim = sys.argv[1] if len(sys.argv) > 1 else "icarus"
here = Path(__file__).parent
runner = get_runner(sim)
runner.build(
    sources=[here.parent / "hdl" / "bench.sv"],
    hdl_toplevel="bench",
    build_dir=here / f"sim_build_{sim}",
    build_args=["-g2012"] if sim == "icarus" else [],
)
runner.test(hdl_toplevel="bench", test_module="test_bench", test_dir=here)
