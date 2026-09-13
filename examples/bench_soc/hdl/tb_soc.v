// A pure-Verilog testbench, so the benchmarks can compare a harness-driven
// run with the same simulation driven by the simulator alone.
`timescale 1ns/1ps

module tb_soc;
  reg clk = 1'b0;
  reg resetn = 1'b0;
  wire trap, mem_valid, mem_instr, mem_ready;
  wire [31:0] mem_addr, mem_wdata, mem_rdata, fetches;
  wire [3:0] mem_wstrb;

  soc dut (
      .clk(clk), .resetn(resetn), .trap(trap),
      .mem_valid(mem_valid), .mem_instr(mem_instr), .mem_ready(mem_ready),
      .mem_addr(mem_addr), .mem_wdata(mem_wdata), .mem_wstrb(mem_wstrb),
      .mem_rdata(mem_rdata), .fetches(fetches)
  );

  integer cycles = 0;
  integer limit;

  initial begin
    if (!$value$plusargs("cycles=%d", limit)) limit = 100000;
    repeat (4) @(posedge clk);
    resetn = 1'b1;
  end

  always #5 clk = ~clk;

  always @(posedge clk) begin
    cycles = cycles + 1;
    if (cycles >= limit) begin
      $display("BARE %0d cycles, %0d instruction fetches", cycles, fetches);
      $finish;
    end
  end
endmodule
