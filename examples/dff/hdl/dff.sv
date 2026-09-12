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

    // Unpacked array written by the testbench and read back through mem_out.
    logic [WIDTH-1:0] mem [0:3];
    logic [WIDTH-1:0] mem_out;
    assign mem_out = mem[d[1:0]];
    integer           cycles;
    real              ratio;
    initial begin
        cycles = 0;
        ratio = 0.0;
    end
    always_ff @(posedge clk) begin
        cycles <= cycles + 1;
        ratio  <= ratio + 0.5;
    end

    // Generate loop: each block has a register holding its index plus d.
    genvar gi;
    generate
        for (gi = 0; gi < 3; gi = gi + 1) begin : gen
            logic [WIDTH-1:0] tap;
            always_ff @(posedge clk) tap <= d + gi;
        end
    endgenerate
endmodule
