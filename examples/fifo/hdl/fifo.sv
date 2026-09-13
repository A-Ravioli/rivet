`timescale 1ns/1ps
// Synchronous FIFO with valid/ready on both sides.
module fifo #(parameter int WIDTH = 16, parameter int DEPTH = 4) (
    input  logic             clk,
    input  logic             rst_n,
    input  logic             in_valid,
    output logic             in_ready,
    input  logic [WIDTH-1:0] in_data,
    output logic             out_valid,
    input  logic             out_ready,
    output logic [WIDTH-1:0] out_data
);
    localparam int AW = $clog2(DEPTH);
    logic [WIDTH-1:0] mem [0:DEPTH-1];
    logic [AW:0] wr_ptr, rd_ptr;
    logic [AW:0] count;

    assign count     = wr_ptr - rd_ptr;
    assign in_ready  = (count != DEPTH);
    assign out_valid = (count != 0);
    assign out_data  = mem[rd_ptr[AW-1:0]];

    always_ff @(posedge clk) begin
        if (!rst_n) begin
            wr_ptr <= '0;
            rd_ptr <= '0;
        end else begin
            if (in_valid && in_ready) begin
                mem[wr_ptr[AW-1:0]] <= in_data;
                wr_ptr <= wr_ptr + 1'b1;
            end
            if (out_valid && out_ready) begin
                rd_ptr <= rd_ptr + 1'b1;
            end
        end
    end
endmodule
