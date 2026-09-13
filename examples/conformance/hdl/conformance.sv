`timescale 1ns/1ps
// Conformance design: one instance of every construct the harness must
// handle. Kept synthesizable-ish so every simulator accepts it.
module conformance #(
    parameter int    W  = 8,
    parameter real   RP = 1.5,
    parameter string SP = "hello"
) (
    input  logic               clk,
    input  logic               rst_n,
    input  logic [W-1:0]       din,
    output logic [W-1:0]       dout_reg,
    output logic [W-1:0]       dout_comb,
    input  logic [127:0]       wide_in,
    output logic [127:0]       wide_out,
    input  logic signed [15:0] sin,
    output logic signed [15:0] sneg,
    input  logic [1:0]         addr,
    output logic [W-1:0]       mem_out
);
    logic [W-1:0] mem [0:3];

    assign dout_comb = din + 1'b1;
    assign sneg      = -sin;
    assign mem_out   = mem[addr];

    always_ff @(posedge clk) begin
        if (!rst_n) begin
            dout_reg <= '0;
            wide_out <= '0;
        end else begin
            dout_reg <= din;
            wide_out <= wide_in;
        end
    end

    // Scalar variable types.
    string  s   = "init";
    integer cnt = 0;
    real    acc = 0.0;
    always_ff @(posedge clk) begin
        cnt <= cnt + 1;
        acc <= acc + 0.25;
    end

    // Packed struct viewed as a vector.
    typedef struct packed {
        logic [3:0] hi;
        logic [3:0] lo;
    } pair_t;
    pair_t pr;
    assign pr = {din[7:4], din[3:0]};

    // Generate instances.
    genvar i;
    generate
        for (i = 0; i < 2; i++) begin : blk
            logic [W-1:0] r;
            always_ff @(posedge clk) r <= din ^ W'(i);
        end
    endgenerate

    // HDL-side end of simulation under plusarg control.
    initial begin
        if ($test$plusargs("finish_early")) begin
            #55 $finish;
        end
    end
endmodule
