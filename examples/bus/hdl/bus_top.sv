// A small SoC-style top for the bus example: register files behind
// AXI4-Lite, APB, Wishbone and Avalon-MM, an AXI4 memory, an AXI4-Stream
// skid buffer, AXI4-Lite and AXI4-Stream register slices (for kit slave
// models), and a typed ALU port.
`timescale 1ns/1ps

package bus_types;
  typedef enum logic [1:0] { OP_NOP = 2'd0, OP_ADD = 2'd1, OP_SUB = 2'd2, OP_MUL = 2'd3 } op_t;
  typedef struct packed {
    op_t        op;
    logic [7:0] a;
    logic [7:0] b;
  } cmd_t;
endpackage

module bus_top #(
    parameter int REG_INIT = 0
) (
    input  logic        clk,
    input  logic        rst_n,

    // AXI4-Lite register file (8 x 32-bit at 0x00..0x1c)
    input  logic [31:0] s_axil_awaddr,
    input  logic        s_axil_awvalid,
    output logic        s_axil_awready,
    input  logic [31:0] s_axil_wdata,
    input  logic [3:0]  s_axil_wstrb,
    input  logic        s_axil_wvalid,
    output logic        s_axil_wready,
    output logic [1:0]  s_axil_bresp,
    output logic        s_axil_bvalid,
    input  logic        s_axil_bready,
    input  logic [31:0] s_axil_araddr,
    input  logic        s_axil_arvalid,
    output logic        s_axil_arready,
    output logic [31:0] s_axil_rdata,
    output logic [1:0]  s_axil_rresp,
    output logic        s_axil_rvalid,
    input  logic        s_axil_rready,

    // AXI4-Lite register slice: p_ in, q_ out (kit master -> kit slave)
    input  logic [31:0] p_axil_awaddr,
    input  logic        p_axil_awvalid,
    output logic        p_axil_awready,
    input  logic [31:0] p_axil_wdata,
    input  logic [3:0]  p_axil_wstrb,
    input  logic        p_axil_wvalid,
    output logic        p_axil_wready,
    output logic [1:0]  p_axil_bresp,
    output logic        p_axil_bvalid,
    input  logic        p_axil_bready,
    input  logic [31:0] p_axil_araddr,
    input  logic        p_axil_arvalid,
    output logic        p_axil_arready,
    output logic [31:0] p_axil_rdata,
    output logic [1:0]  p_axil_rresp,
    output logic        p_axil_rvalid,
    input  logic        p_axil_rready,
    output logic [31:0] q_axil_awaddr,
    output logic        q_axil_awvalid,
    input  logic        q_axil_awready,
    output logic [31:0] q_axil_wdata,
    output logic [3:0]  q_axil_wstrb,
    output logic        q_axil_wvalid,
    input  logic        q_axil_wready,
    input  logic [1:0]  q_axil_bresp,
    input  logic        q_axil_bvalid,
    output logic        q_axil_bready,
    output logic [31:0] q_axil_araddr,
    output logic        q_axil_arvalid,
    input  logic        q_axil_arready,
    input  logic [31:0] q_axil_rdata,
    input  logic [1:0]  q_axil_rresp,
    input  logic        q_axil_rvalid,
    output logic        q_axil_rready,

    // AXI4 memory (1 KiB, 32-bit data)
    input  logic [3:0]  s_axi_awid,
    input  logic [31:0] s_axi_awaddr,
    input  logic [7:0]  s_axi_awlen,
    input  logic [2:0]  s_axi_awsize,
    input  logic [1:0]  s_axi_awburst,
    input  logic        s_axi_awvalid,
    output logic        s_axi_awready,
    input  logic [31:0] s_axi_wdata,
    input  logic [3:0]  s_axi_wstrb,
    input  logic        s_axi_wlast,
    input  logic        s_axi_wvalid,
    output logic        s_axi_wready,
    output logic [3:0]  s_axi_bid,
    output logic [1:0]  s_axi_bresp,
    output logic        s_axi_bvalid,
    input  logic        s_axi_bready,
    input  logic [3:0]  s_axi_arid,
    input  logic [31:0] s_axi_araddr,
    input  logic [7:0]  s_axi_arlen,
    input  logic [2:0]  s_axi_arsize,
    input  logic [1:0]  s_axi_arburst,
    input  logic        s_axi_arvalid,
    output logic        s_axi_arready,
    output logic [3:0]  s_axi_rid,
    output logic [31:0] s_axi_rdata,
    output logic [1:0]  s_axi_rresp,
    output logic        s_axi_rlast,
    output logic        s_axi_rvalid,
    input  logic        s_axi_rready,

    // AXI4-Stream skid buffer
    input  logic        s_axis_tvalid,
    output logic        s_axis_tready,
    input  logic [31:0] s_axis_tdata,
    input  logic [3:0]  s_axis_tkeep,
    input  logic        s_axis_tlast,
    output logic        m_axis_tvalid,
    input  logic        m_axis_tready,
    output logic [31:0] m_axis_tdata,
    output logic [3:0]  m_axis_tkeep,
    output logic        m_axis_tlast,

    // APB register file (4 x 32-bit at 0x00..0x0c), one wait state
    input  logic        apb_psel,
    input  logic        apb_penable,
    input  logic        apb_pwrite,
    input  logic [15:0] apb_paddr,
    input  logic [31:0] apb_pwdata,
    input  logic [3:0]  apb_pstrb,
    output logic [31:0] apb_prdata,
    output logic        apb_pready,
    output logic        apb_pslverr,

    // Wishbone register file (4 x 32-bit, word addresses 0..3)
    input  logic        wb_cyc,
    input  logic        wb_stb,
    input  logic        wb_we,
    input  logic [15:0] wb_adr,
    input  logic [31:0] wb_dat_w,
    input  logic [3:0]  wb_sel,
    output logic [31:0] wb_dat_r,
    output logic        wb_ack,
    output logic        wb_err,

    // Avalon-MM register file (4 x 32-bit, byte addresses), pipelined reads
    input  logic [15:0] av_address,
    input  logic        av_read,
    input  logic        av_write,
    input  logic [31:0] av_writedata,
    input  logic [3:0]  av_byteenable,
    output logic [31:0] av_readdata,
    output logic        av_waitrequest,
    output logic        av_readdatavalid,

    // Typed ALU
    input  bus_types::cmd_t cmd,
    output logic [15:0] alu_out
);
  import bus_types::*;

  // ------------------------------------------------------------------
  // AXI4-Lite register file
  logic [31:0] regs [0:7];
  logic        aw_pending, w_pending;
  logic [31:0] aw_addr, w_data;
  logic [3:0]  w_strb;

  assign s_axil_awready = !aw_pending;
  assign s_axil_wready  = !w_pending;

  function automatic logic axil_ok(input logic [31:0] a);
    return a[31:5] == '0 && a[1:0] == 2'b00;
  endfunction

  always_ff @(posedge clk) begin
    if (!rst_n) begin
      aw_pending <= 1'b0;
      w_pending  <= 1'b0;
      s_axil_bvalid <= 1'b0;
      s_axil_bresp  <= 2'b00;
      for (int i = 0; i < 8; i++) regs[i] <= REG_INIT;
    end else begin
      if (s_axil_awvalid && s_axil_awready) begin
        aw_pending <= 1'b1;
        aw_addr    <= s_axil_awaddr;
      end
      if (s_axil_wvalid && s_axil_wready) begin
        w_pending <= 1'b1;
        w_data    <= s_axil_wdata;
        w_strb    <= s_axil_wstrb;
      end
      if (s_axil_bvalid && s_axil_bready) s_axil_bvalid <= 1'b0;
      if (aw_pending && w_pending && !s_axil_bvalid) begin
        aw_pending <= 1'b0;
        w_pending  <= 1'b0;
        s_axil_bvalid <= 1'b1;
        if (axil_ok(aw_addr)) begin
          s_axil_bresp <= 2'b00;
          for (int b = 0; b < 4; b++)
            if (w_strb[b]) regs[aw_addr[4:2]][8*b +: 8] <= w_data[8*b +: 8];
        end else begin
          s_axil_bresp <= 2'b10;
        end
      end
    end
  end

  assign s_axil_arready = !s_axil_rvalid;
  always_ff @(posedge clk) begin
    if (!rst_n) begin
      s_axil_rvalid <= 1'b0;
      s_axil_rdata  <= '0;
      s_axil_rresp  <= 2'b00;
    end else begin
      if (s_axil_rvalid && s_axil_rready) s_axil_rvalid <= 1'b0;
      if (s_axil_arvalid && s_axil_arready) begin
        s_axil_rvalid <= 1'b1;
        if (axil_ok(s_axil_araddr)) begin
          s_axil_rdata <= regs[s_axil_araddr[4:2]];
          s_axil_rresp <= 2'b00;
        end else begin
          s_axil_rdata <= '0;
          s_axil_rresp <= 2'b10;
        end
      end
    end
  end

  // ------------------------------------------------------------------
  // AXI4-Lite register slice p_ -> q_ (one register stage per channel)
  axil_slice u_axil_slice (
    .clk, .rst_n,
    .s_awaddr(p_axil_awaddr), .s_awvalid(p_axil_awvalid), .s_awready(p_axil_awready),
    .s_wdata(p_axil_wdata), .s_wstrb(p_axil_wstrb), .s_wvalid(p_axil_wvalid), .s_wready(p_axil_wready),
    .s_bresp(p_axil_bresp), .s_bvalid(p_axil_bvalid), .s_bready(p_axil_bready),
    .s_araddr(p_axil_araddr), .s_arvalid(p_axil_arvalid), .s_arready(p_axil_arready),
    .s_rdata(p_axil_rdata), .s_rresp(p_axil_rresp), .s_rvalid(p_axil_rvalid), .s_rready(p_axil_rready),
    .m_awaddr(q_axil_awaddr), .m_awvalid(q_axil_awvalid), .m_awready(q_axil_awready),
    .m_wdata(q_axil_wdata), .m_wstrb(q_axil_wstrb), .m_wvalid(q_axil_wvalid), .m_wready(q_axil_wready),
    .m_bresp(q_axil_bresp), .m_bvalid(q_axil_bvalid), .m_bready(q_axil_bready),
    .m_araddr(q_axil_araddr), .m_arvalid(q_axil_arvalid), .m_arready(q_axil_arready),
    .m_rdata(q_axil_rdata), .m_rresp(q_axil_rresp), .m_rvalid(q_axil_rvalid), .m_rready(q_axil_rready)
  );

  // ------------------------------------------------------------------
  // AXI4 memory: 256 words, FIXED/INCR/WRAP bursts, full-width beats.
  logic [31:0] mem [0:255];
  logic        wr_active, rd_active;
  logic [3:0]  wr_id, rd_id;
  logic [31:0] wr_addr, rd_addr, wr_start, rd_start;
  logic [7:0]  wr_len, rd_len, wr_beat, rd_beat;
  logic [1:0]  wr_burst, rd_burst;
  logic [2:0]  wr_size, rd_size;

  function automatic logic [31:0] next_addr(
      input logic [31:0] cur, input logic [31:0] start, input logic [1:0] burst,
      input logic [7:0] len, input logic [2:0] size);
    logic [31:0] bytes, total, lower, nxt;
    bytes = 32'd1 << size;
    total = bytes * (32'(len) + 32'd1);
    lower = start & ~(total - 32'd1);
    nxt = (cur & ~(bytes - 32'd1)) + bytes;
    case (burst)
      2'b00: return cur;
      2'b10: return (nxt >= lower + total) ? nxt - total : nxt;
      default: return nxt;
    endcase
  endfunction

  assign s_axi_awready = !wr_active && !s_axi_bvalid;
  assign s_axi_wready  = wr_active;
  always_ff @(posedge clk) begin
    if (!rst_n) begin
      wr_active   <= 1'b0;
      s_axi_bvalid <= 1'b0;
      s_axi_bid    <= '0;
      s_axi_bresp  <= '0;
    end else begin
      if (s_axi_bvalid && s_axi_bready) s_axi_bvalid <= 1'b0;
      if (s_axi_awvalid && s_axi_awready) begin
        wr_active <= 1'b1;
        wr_id     <= s_axi_awid;
        wr_addr   <= s_axi_awaddr;
        wr_start  <= s_axi_awaddr;
        wr_len    <= s_axi_awlen;
        wr_burst  <= s_axi_awburst;
        wr_size   <= s_axi_awsize;
        wr_beat   <= '0;
      end
      if (wr_active && s_axi_wvalid) begin
        for (int b = 0; b < 4; b++)
          if (s_axi_wstrb[b]) mem[wr_addr[9:2]][8*b +: 8] <= s_axi_wdata[8*b +: 8];
        wr_addr <= next_addr(wr_addr, wr_start, wr_burst, wr_len, wr_size);
        wr_beat <= wr_beat + 8'd1;
        if (s_axi_wlast || wr_beat == wr_len) begin
          wr_active    <= 1'b0;
          s_axi_bvalid <= 1'b1;
          s_axi_bid    <= wr_id;
          s_axi_bresp  <= (wr_start[31:10] != '0) ? 2'b10 : 2'b00;
        end
      end
    end
  end

  assign s_axi_arready = !rd_active;
  always_ff @(posedge clk) begin
    if (!rst_n) begin
      rd_active   <= 1'b0;
      s_axi_rvalid <= 1'b0;
      s_axi_rlast  <= 1'b0;
      s_axi_rid    <= '0;
      s_axi_rdata  <= '0;
      s_axi_rresp  <= '0;
    end else begin
      if (s_axi_arvalid && s_axi_arready) begin
        rd_active <= 1'b1;
        rd_id     <= s_axi_arid;
        rd_addr   <= s_axi_araddr;
        rd_start  <= s_axi_araddr;
        rd_len    <= s_axi_arlen;
        rd_burst  <= s_axi_arburst;
        rd_size   <= s_axi_arsize;
        rd_beat   <= '0;
        s_axi_rvalid <= 1'b1;
        s_axi_rid    <= s_axi_arid;
        s_axi_rdata  <= mem[s_axi_araddr[9:2]];
        s_axi_rresp  <= (s_axi_araddr[31:10] != '0) ? 2'b10 : 2'b00;
        s_axi_rlast  <= (s_axi_arlen == 8'd0);
      end else if (rd_active && s_axi_rvalid && s_axi_rready) begin
        if (s_axi_rlast) begin
          rd_active    <= 1'b0;
          s_axi_rvalid <= 1'b0;
          s_axi_rlast  <= 1'b0;
        end else begin
          logic [31:0] na;
          na = next_addr(rd_addr, rd_start, rd_burst, rd_len, rd_size);
          rd_addr     <= na;
          rd_beat     <= rd_beat + 8'd1;
          s_axi_rdata <= mem[na[9:2]];
          s_axi_rlast <= (rd_beat + 8'd1 == rd_len);
        end
      end
    end
  end

  // ------------------------------------------------------------------
  // AXI4-Stream skid buffer
  logic        skid_valid;
  logic [31:0] skid_data;
  logic [3:0]  skid_keep;
  logic        skid_last;
  assign s_axis_tready = !skid_valid;
  always_ff @(posedge clk) begin
    if (!rst_n) begin
      m_axis_tvalid <= 1'b0;
      skid_valid    <= 1'b0;
      m_axis_tdata  <= '0;
      m_axis_tkeep  <= '0;
      m_axis_tlast  <= 1'b0;
    end else begin
      if (m_axis_tvalid && m_axis_tready) m_axis_tvalid <= 1'b0;
      if (!m_axis_tvalid || m_axis_tready) begin
        if (skid_valid) begin
          m_axis_tvalid <= 1'b1;
          m_axis_tdata  <= skid_data;
          m_axis_tkeep  <= skid_keep;
          m_axis_tlast  <= skid_last;
          skid_valid    <= 1'b0;
        end else if (s_axis_tvalid && s_axis_tready) begin
          m_axis_tvalid <= 1'b1;
          m_axis_tdata  <= s_axis_tdata;
          m_axis_tkeep  <= s_axis_tkeep;
          m_axis_tlast  <= s_axis_tlast;
        end
      end else if (s_axis_tvalid && s_axis_tready) begin
        skid_valid <= 1'b1;
        skid_data  <= s_axis_tdata;
        skid_keep  <= s_axis_tkeep;
        skid_last  <= s_axis_tlast;
      end
    end
  end

  // ------------------------------------------------------------------
  // APB register file with one wait state
  logic [31:0] apb_regs [0:3];
  logic        apb_wait;
  always_ff @(posedge clk) begin
    if (!rst_n) begin
      apb_pready  <= 1'b0;
      apb_pslverr <= 1'b0;
      apb_prdata  <= '0;
      apb_wait    <= 1'b0;
      for (int i = 0; i < 4; i++) apb_regs[i] <= '0;
    end else begin
      apb_pready  <= 1'b0;
      apb_pslverr <= 1'b0;
      if (apb_psel && apb_penable && !apb_pready) begin
        if (!apb_wait) begin
          apb_wait <= 1'b1;
        end else begin
          apb_wait   <= 1'b0;
          apb_pready <= 1'b1;
          if (apb_paddr >= 16'h0040) begin
            apb_pslverr <= 1'b1;
            apb_prdata  <= '0;
          end else if (apb_pwrite) begin
            for (int b = 0; b < 4; b++)
              if (apb_pstrb[b]) apb_regs[apb_paddr[3:2]][8*b +: 8] <= apb_pwdata[8*b +: 8];
          end else begin
            apb_prdata <= apb_regs[apb_paddr[3:2]];
          end
        end
      end
    end
  end

  // ------------------------------------------------------------------
  // Wishbone register file (word addressed)
  logic [31:0] wb_regs [0:3];
  always_ff @(posedge clk) begin
    if (!rst_n) begin
      wb_ack   <= 1'b0;
      wb_err   <= 1'b0;
      wb_dat_r <= '0;
      for (int i = 0; i < 4; i++) wb_regs[i] <= '0;
    end else begin
      wb_ack <= 1'b0;
      wb_err <= 1'b0;
      if (wb_cyc && wb_stb && !wb_ack && !wb_err) begin
        if (wb_adr >= 16'h0040) begin
          wb_err <= 1'b1;
        end else begin
          wb_ack <= 1'b1;
          if (wb_we) begin
            for (int b = 0; b < 4; b++)
              if (wb_sel[b]) wb_regs[wb_adr[1:0]][8*b +: 8] <= wb_dat_w[8*b +: 8];
          end else begin
            wb_dat_r <= wb_regs[wb_adr[1:0]];
          end
        end
      end
    end
  end

  // ------------------------------------------------------------------
  // Avalon-MM register file: one wait state, readdatavalid a cycle later
  logic [31:0] av_regs [0:3];
  logic        av_busy;
  assign av_waitrequest = !av_busy;
  always_ff @(posedge clk) begin
    if (!rst_n) begin
      av_busy          <= 1'b0;
      av_readdatavalid <= 1'b0;
      av_readdata      <= '0;
      for (int i = 0; i < 4; i++) av_regs[i] <= '0;
    end else begin
      av_readdatavalid <= 1'b0;
      if (!av_busy && (av_read || av_write)) begin
        av_busy <= 1'b1;
      end else if (av_busy) begin
        av_busy <= 1'b0;
        if (av_write) begin
          for (int b = 0; b < 4; b++)
            if (av_byteenable[b]) av_regs[av_address[3:2]][8*b +: 8] <= av_writedata[8*b +: 8];
        end else if (av_read) begin
          av_readdata      <= av_regs[av_address[3:2]];
          av_readdatavalid <= 1'b1;
        end
      end
    end
  end

  // ------------------------------------------------------------------
  // Typed ALU (registered)
  always_ff @(posedge clk) begin
    if (!rst_n) alu_out <= '0;
    else begin
      case (cmd.op)
        OP_ADD: alu_out <= 16'(cmd.a) + 16'(cmd.b);
        OP_SUB: alu_out <= 16'(cmd.a) - 16'(cmd.b);
        OP_MUL: alu_out <= 16'(cmd.a) * 16'(cmd.b);
        default: alu_out <= '0;
      endcase
    end
  end
endmodule

// One register stage per AXI4-Lite channel.
module axil_slice (
    input  logic        clk,
    input  logic        rst_n,
    input  logic [31:0] s_awaddr,  input  logic s_awvalid, output logic s_awready,
    input  logic [31:0] s_wdata,   input  logic [3:0] s_wstrb, input logic s_wvalid, output logic s_wready,
    output logic [1:0]  s_bresp,   output logic s_bvalid,  input  logic s_bready,
    input  logic [31:0] s_araddr,  input  logic s_arvalid, output logic s_arready,
    output logic [31:0] s_rdata,   output logic [1:0] s_rresp, output logic s_rvalid, input logic s_rready,
    output logic [31:0] m_awaddr,  output logic m_awvalid, input  logic m_awready,
    output logic [31:0] m_wdata,   output logic [3:0] m_wstrb, output logic m_wvalid, input logic m_wready,
    input  logic [1:0]  m_bresp,   input  logic m_bvalid,  output logic m_bready,
    output logic [31:0] m_araddr,  output logic m_arvalid, input  logic m_arready,
    input  logic [31:0] m_rdata,   input  logic [1:0] m_rresp, input logic m_rvalid, output logic m_rready
);
  // Forward channels (valid/data registered, ready = !valid || downstream ready).
  assign s_awready = !m_awvalid || m_awready;
  assign s_wready  = !m_wvalid  || m_wready;
  assign s_arready = !m_arvalid || m_arready;
  // Response channels registered the same way in the other direction.
  assign m_bready  = !s_bvalid || s_bready;
  assign m_rready  = !s_rvalid || s_rready;
  always_ff @(posedge clk) begin
    if (!rst_n) begin
      m_awvalid <= 1'b0; m_wvalid <= 1'b0; m_arvalid <= 1'b0;
      s_bvalid  <= 1'b0; s_rvalid <= 1'b0;
      m_awaddr <= '0; m_wdata <= '0; m_wstrb <= '0; m_araddr <= '0;
      s_bresp <= '0; s_rdata <= '0; s_rresp <= '0;
    end else begin
      if (s_awready) begin m_awvalid <= s_awvalid; m_awaddr <= s_awaddr; end
      if (s_wready)  begin m_wvalid  <= s_wvalid;  m_wdata  <= s_wdata; m_wstrb <= s_wstrb; end
      if (s_arready) begin m_arvalid <= s_arvalid; m_araddr <= s_araddr; end
      if (m_bready)  begin s_bvalid  <= m_bvalid;  s_bresp  <= m_bresp; end
      if (m_rready)  begin s_rvalid  <= m_rvalid;  s_rdata  <= m_rdata; s_rresp <= m_rresp; end
    end
  end
endmodule
