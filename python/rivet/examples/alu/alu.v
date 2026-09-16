// A registered 32-bit ALU: one cycle from operands to result.
//
// Registering the output is what makes the testbench simple to write
// correctly -- the result for cycle N is sampled at the edge of cycle
// N+1, with no combinational race to reason about.
`timescale 1ns / 1ps

module alu #(
    parameter WIDTH = 32
) (
    input  wire                  clk,
    input  wire                  rst_n,
    input  wire                  valid,
    input  wire [2:0]            op,
    input  wire [WIDTH-1:0]      a,
    input  wire [WIDTH-1:0]      b,
    output reg  [WIDTH-1:0]      result,
    output reg                   zero,
    output reg                   result_valid
);
    localparam OP_ADD = 3'd0;
    localparam OP_SUB = 3'd1;
    localparam OP_AND = 3'd2;
    localparam OP_OR  = 3'd3;
    localparam OP_XOR = 3'd4;
    localparam OP_SLT = 3'd5;   // set-less-than, signed
    localparam OP_SLL = 3'd6;   // shift left by b[4:0]
    localparam OP_SRL = 3'd7;   // shift right by b[4:0], logical

    reg [WIDTH-1:0] next;

    always @(*) begin
        case (op)
            OP_ADD: next = a + b;
            OP_SUB: next = a - b;
            OP_AND: next = a & b;
            OP_OR:  next = a | b;
            OP_XOR: next = a ^ b;
            OP_SLT: next = ($signed(a) < $signed(b)) ? {{WIDTH-1{1'b0}}, 1'b1}
                                                     : {WIDTH{1'b0}};
            OP_SLL: next = a << b[4:0];
            OP_SRL: next = a >> b[4:0];
            default: next = {WIDTH{1'b0}};
        endcase
    end

    always @(posedge clk) begin
        if (!rst_n) begin
            result       <= {WIDTH{1'b0}};
            zero         <= 1'b0;
            result_valid <= 1'b0;
        end else begin
            result_valid <= valid;
            if (valid) begin
                result <= next;
                zero   <= (next == {WIDTH{1'b0}});
            end
        end
    end
endmodule
