"""Run the cocotb + rivet_py test on Icarus with cocotb's Python runner."""

import os
from pathlib import Path

from cocotb_tools.runner import get_runner

here = Path(__file__).resolve().parent
dff = here.parents[3] / "examples" / "dff" / "hdl" / "dff.sv"
runner = get_runner(os.environ.get("SIM", "icarus"))
runner.build(sources=[dff], hdl_toplevel="dff", build_dir=here / "sim_build", build_args=["-g2012"])
runner.test(hdl_toplevel="dff", test_module="test_dff", test_dir=here, build_dir=here / "sim_build")
