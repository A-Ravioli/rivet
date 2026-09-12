// A D flip-flop with synchronous active-low reset and a free-running counter.
`timescale 1ns/1ps
module dff #(parameter int WIDTH = 8) (
    input  logic             clk,
    input  logic             rst_n,
    input  logic [WIDTH-1:0] d,
    output logic [WIDTH-1:0] q,
    output logic [WIDTH-1:0] count
);
    always_ff @(posedge clk) begin
        if (!rst_n) begin
            q     <= '0;
            count <= '0;
        end else begin
            q     <= d;
            count <= count + 1'b1;
        end
    end

    logic [WIDTH:0] sum;
    assign sum = d + q;
endmodule
