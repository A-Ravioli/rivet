-- Types the VHPI backend has to classify: an enumeration, a record, and an
-- unconstrained array.
library ieee;
use ieee.std_logic_1164.all;

package types_pkg is
  type op_t is (OP_NOP, OP_ADD, OP_SUB, OP_XOR);

  type cmd_t is record
    op    : op_t;
    valid : std_logic;
    data  : std_logic_vector(7 downto 0);
  end record;

  function op_name(o : op_t) return string;
end package;

package body types_pkg is
  function op_name(o : op_t) return string is
  begin
    case o is
      when OP_NOP => return "nop";
      when OP_ADD => return "add";
      when OP_SUB => return "sub";
      when OP_XOR => return "xor";
    end case;
  end function;
end package body;
