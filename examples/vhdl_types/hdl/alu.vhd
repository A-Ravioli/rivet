-- A small ALU driven by a record, with a generic width and a for-generate.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.types_pkg.all;

entity alu is
  generic (
    WIDTH : positive := 8;
    TAPS  : positive := 4
  );
  port (
    clk    : in  std_logic;
    rst_n  : in  std_logic;
    cmd    : in  cmd_t;
    b      : in  std_logic_vector(WIDTH - 1 downto 0);
    result : out std_logic_vector(WIDTH - 1 downto 0);
    done   : out std_logic
  );
end entity;

architecture rtl of alu is
  signal acc      : unsigned(WIDTH - 1 downto 0) := (others => '0');
  signal last_op  : op_t := OP_NOP;
  signal tap_bits : std_logic_vector(TAPS - 1 downto 0);
begin
  -- A for-generate, so the backend has a nested region to discover.
  tapgen : for i in 0 to TAPS - 1 generate
    signal tap : std_logic;
  begin
    tap <= acc(i mod WIDTH);
    tap_bits(i) <= tap;
  end generate;

  process (clk, rst_n)
  begin
    if rst_n = '0' then
      acc     <= (others => '0');
      last_op <= OP_NOP;
      done    <= '0';
    elsif rising_edge(clk) then
      last_op <= cmd.op;
      done    <= cmd.valid;
      if cmd.valid = '1' then
        case cmd.op is
          when OP_ADD => acc <= acc + unsigned(b);
          when OP_SUB => acc <= acc - unsigned(b);
          when OP_XOR => acc <= acc xor unsigned(b);
          when OP_NOP => acc <= acc;
        end case;
      end if;
    end if;
  end process;

  result <= std_logic_vector(acc);
end architecture;
