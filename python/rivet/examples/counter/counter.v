// A counter with a synchronous, active-low reset and a count enable.
`timescale 1ns / 1ps

module counter #(
    parameter WIDTH = 8
) (
    input  wire              clk,
    input  wire              rst_n,
    input  wire              en,
    input  wire              load,
    input  wire [WIDTH-1:0]  load_value,
    output reg  [WIDTH-1:0]  count
);
    always @(posedge clk) begin
        if (!rst_n)      count <= {WIDTH{1'b0}};
        else if (load)   count <= load_value;
        else if (en)     count <= count + 1'b1;
    end
endmodule
