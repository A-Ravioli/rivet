// A synchronous FIFO with show-ahead (first-word-fall-through) reads.
//
// `rd_data` is valid whenever `empty` is low; `rd_en` pops the word that
// is already on the output. Writes while `full` and reads while `empty`
// are ignored.
`timescale 1ns / 1ps

module fifo #(
    parameter WIDTH = 8,
    parameter DEPTH = 16        // must be a power of two
) (
    input  wire              clk,
    input  wire              rst_n,

    input  wire              wr_en,
    input  wire [WIDTH-1:0]  wr_data,
    output wire              full,

    input  wire              rd_en,
    output wire [WIDTH-1:0]  rd_data,
    output wire              empty,

    output wire [ADDR:0]     count
);
    localparam ADDR = $clog2(DEPTH);

    reg [WIDTH-1:0] mem [0:DEPTH-1];
    // One bit wider than the address, so wrap is distinguishable from
    // equality: pointers equal means empty, differing only in the top bit
    // means full.
    reg [ADDR:0] wr_ptr;
    reg [ADDR:0] rd_ptr;

    wire do_write = wr_en && !full;
    wire do_read  = rd_en && !empty;

    assign empty   = (wr_ptr == rd_ptr);
    assign full    = (wr_ptr[ADDR] != rd_ptr[ADDR]) &&
                     (wr_ptr[ADDR-1:0] == rd_ptr[ADDR-1:0]);
    assign count   = wr_ptr - rd_ptr;
    assign rd_data = mem[rd_ptr[ADDR-1:0]];

    always @(posedge clk) begin
        if (!rst_n) begin
            wr_ptr <= {(ADDR+1){1'b0}};
            rd_ptr <= {(ADDR+1){1'b0}};
        end else begin
            if (do_write) begin
                mem[wr_ptr[ADDR-1:0]] <= wr_data;
                wr_ptr <= wr_ptr + 1'b1;
            end
            if (do_read) begin
                rd_ptr <= rd_ptr + 1'b1;
            end
        end
    end
endmodule
