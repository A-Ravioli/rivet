// A UART transmitter and receiver, 8N1.
//
// At the default parameters one bit is 50 clocks, and the clock is 20 ns,
// so a bit is exactly 1 us -- which is what lets the testbench talk to the
// wire with a stopwatch instead of counting clock edges.
`timescale 1ns / 1ps

module uart #(
    parameter CLK_HZ = 50_000_000,
    parameter BAUD   = 1_000_000
) (
    input  wire       clk,
    input  wire       rst_n,

    // Transmit
    input  wire       tx_start,
    input  wire [7:0] tx_data,
    output wire       tx_busy,
    output reg        tx_out,

    // Receive
    input  wire       rx_in,
    output reg  [7:0] rx_data,
    output reg        rx_valid,

    // Tie the receiver to our own transmitter instead of `rx_in`.
    input  wire       loopback
);
    localparam integer DIV = CLK_HZ / BAUD;

    // ---- transmitter -----------------------------------------------------
    reg [15:0] tx_cnt;
    reg [3:0]  tx_idx;      // which bit is on the wire: 0 start, 1-8 data, 9 stop
    reg [9:0]  tx_shift;    // {stop, data, start}
    reg        tx_active;

    assign tx_busy = tx_active;

    always @(posedge clk) begin
        if (!rst_n) begin
            tx_out    <= 1'b1;
            tx_active <= 1'b0;
            tx_cnt    <= 16'd0;
            tx_idx    <= 4'd0;
        end else if (!tx_active) begin
            tx_out <= 1'b1;
            if (tx_start) begin
                tx_shift  <= {1'b1, tx_data, 1'b0};
                tx_active <= 1'b1;
                tx_cnt    <= 16'd0;
                tx_idx    <= 4'd0;
                tx_out    <= 1'b0;          // start bit goes out immediately
            end
        end else if (tx_cnt == DIV - 1) begin
            tx_cnt <= 16'd0;
            if (tx_idx == 4'd9) begin        // stop bit has had its full period
                tx_active <= 1'b0;
                tx_out    <= 1'b1;
            end else begin
                tx_idx <= tx_idx + 4'd1;
                tx_out <= tx_shift[tx_idx + 4'd1];
            end
        end else begin
            tx_cnt <= tx_cnt + 16'd1;
        end
    end

    // ---- receiver --------------------------------------------------------
    wire rx_line = loopback ? tx_out : rx_in;

    reg [15:0] rx_cnt;
    reg [3:0]  rx_idx;      // 0 mid-start, 1-8 data samples, 9 mid-stop
    reg [7:0]  rx_shift;
    reg        rx_active;
    reg        rx_line_d;

    always @(posedge clk) begin
        rx_valid <= 1'b0;
        if (!rst_n) begin
            rx_active <= 1'b0;
            rx_line_d <= 1'b1;
            rx_data   <= 8'd0;
        end else begin
            rx_line_d <= rx_line;
            if (!rx_active) begin
                if (rx_line_d && !rx_line) begin        // falling edge: start bit
                    rx_active <= 1'b1;
                    rx_idx    <= 4'd0;
                    rx_cnt    <= DIV / 2;               // first tick lands mid-bit
                end
            end else if (rx_cnt == DIV - 1) begin
                rx_cnt <= 16'd0;
                if (rx_idx == 4'd0) begin
                    rx_idx <= 4'd1;                     // middle of the start bit
                end else if (rx_idx <= 4'd8) begin
                    rx_shift <= {rx_line, rx_shift[7:1]};   // LSB first
                    rx_idx   <= rx_idx + 4'd1;
                end else begin
                    rx_active <= 1'b0;
                    rx_valid  <= 1'b1;
                    rx_data   <= rx_shift;
                end
            end else begin
                rx_cnt <= rx_cnt + 16'd1;
            end
        end
    end
endmodule
