// A small system around PicoRV32 (ISC licensed, vendored as picorv32.v):
// the core, a memory, and a two-instruction loop. It exists so the
// benchmarks can state harness overhead as a fraction of a simulation that
// is doing real work, rather than of an empty design.
`timescale 1ns/1ps

module soc #(
    parameter integer MEM_WORDS = 1024
) (
    input  wire        clk,
    input  wire        resetn,
    output wire        trap,
    // Visible to the testbench: the core's memory interface.
    output wire        mem_valid,
    output wire        mem_instr,
    output wire        mem_ready,
    output wire [31:0] mem_addr,
    output wire [31:0] mem_wdata,
    output wire [ 3:0] mem_wstrb,
    output wire [31:0] mem_rdata,
    output reg  [31:0] fetches
);
  reg [31:0] mem [0:MEM_WORDS-1];
  reg [31:0] rdata;
  reg        ready;

  initial begin
    // addi x1, x1, 1
    mem[0] = 32'h00108093;
    // jal x0, -4
    mem[1] = 32'hffdff06f;
    for (integer i = 2; i < MEM_WORDS; i = i + 1) mem[i] = 32'h00000013;
  end

  wire [31:0] word_addr = mem_addr >> 2;

  always @(posedge clk) begin
    ready <= 1'b0;
    if (!resetn) begin
      fetches <= 32'd0;
    end else if (mem_valid && !ready) begin
      if (mem_wstrb != 4'b0000) begin
        if (mem_wstrb[0]) mem[word_addr][ 7: 0] <= mem_wdata[ 7: 0];
        if (mem_wstrb[1]) mem[word_addr][15: 8] <= mem_wdata[15: 8];
        if (mem_wstrb[2]) mem[word_addr][23:16] <= mem_wdata[23:16];
        if (mem_wstrb[3]) mem[word_addr][31:24] <= mem_wdata[31:24];
      end else begin
        rdata <= mem[word_addr];
        if (mem_instr) fetches <= fetches + 32'd1;
      end
      ready <= 1'b1;
    end
  end

  assign mem_ready = ready;
  assign mem_rdata = rdata;

  picorv32 #(
      .PROGADDR_RESET(32'h0000_0000)
  ) cpu (
      .clk      (clk),
      .resetn   (resetn),
      .trap     (trap),
      .mem_valid(mem_valid),
      .mem_instr(mem_instr),
      .mem_ready(mem_ready),
      .mem_addr (mem_addr),
      .mem_wdata(mem_wdata),
      .mem_wstrb(mem_wstrb),
      .mem_rdata(mem_rdata)
  );
endmodule
