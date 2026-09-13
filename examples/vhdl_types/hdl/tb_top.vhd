-- The top level is a testbench entity with no ports, which is the usual
-- VHDL shape: every signal the harness drives is an internal signal.
library ieee;
use ieee.std_logic_1164.all;
use work.types_pkg.all;

entity tb_top is
  generic (
    WIDTH : positive := 8
  );
end entity;

architecture sim of tb_top is
  signal clk    : std_logic := '0';
  signal rst_n  : std_logic := '0';
  signal cmd    : cmd_t     := (op => OP_NOP, valid => '0', data => (others => '0'));
  signal b      : std_logic_vector(WIDTH - 1 downto 0) := (others => '0');
  signal result : std_logic_vector(WIDTH - 1 downto 0);
  signal done   : std_logic;
  -- A boolean and an integer, to exercise the other type classes.
  signal armed  : boolean := false;
  signal cycles : integer := 0;
begin
  dut : entity work.alu
    generic map (WIDTH => WIDTH, TAPS => 4)
    port map (
      clk    => clk,
      rst_n  => rst_n,
      cmd    => cmd,
      b      => b,
      result => result,
      done   => done
    );

  count : process (clk)
  begin
    if rising_edge(clk) then
      cycles <= cycles + 1;
      armed  <= rst_n = '1';
    end if;
  end process;
end architecture;
