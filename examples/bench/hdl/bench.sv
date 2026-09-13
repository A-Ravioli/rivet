`timescale 1ns/1ps
// Benchmark design: a registered incrementer and a wide shift register.
module bench #(parameter int W = 512) (
    input  logic         clk,
    input  logic [31:0]  din,
    output logic [31:0]  dout,
    output logic [W-1:0] wide
);
    initial begin
        dout = 0;
        wide = 0;
    end
    always_ff @(posedge clk) begin
        dout <= din + 1;
        wide <= {wide[W-33:0], din};
    end
endmodule
