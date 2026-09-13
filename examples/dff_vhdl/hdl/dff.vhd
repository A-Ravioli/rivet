library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity dff is
  generic (WIDTH : integer := 8);
  port (
    clk   : in  std_logic;
    rst_n : in  std_logic;
    d     : in  std_logic_vector(WIDTH-1 downto 0);
    q     : out std_logic_vector(WIDTH-1 downto 0);
    count : out std_logic_vector(WIDTH-1 downto 0)
  );
end entity;

architecture rtl of dff is
  signal count_i : unsigned(WIDTH-1 downto 0) := (others => '0');
  signal cycles  : integer := 0;
begin
  process (clk)
  begin
    if rising_edge(clk) then
      if rst_n = '0' then
        q       <= (others => '0');
        count_i <= (others => '0');
      else
        q       <= d;
        count_i <= count_i + 1;
      end if;
      cycles <= cycles + 1;
    end if;
  end process;
  count <= std_logic_vector(count_i);
end architecture;
