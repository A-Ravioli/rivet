"""Reading and writing signal values.

This is where cocotb spends most of its time per cycle, because every
value becomes a string on the way in and out. These tests pin down what
the integer path actually does, including at widths no machine word can
hold.
"""

import unittest

import harness
import rivet
from harness import assert_passed, dff_design, fresh, only


class Values(unittest.TestCase):
    def run_body(self, body, timeout="10us"):
        fresh()
        rivet.test(timeout=timeout)(body)
        r = only(dff_design().run())
        assert_passed(self, r)
        return r

    def test_round_trip_through_a_flop(self):
        got = []

        async def t(dut):
            clk, d, q = dut.signal("clk"), dut.signal("d"), dut.signal("q")
            rivet.Clock(clk, "10ns").start()
            for v in (0, 1, 0x5A, 0xFF):
                d.set(v)
                await clk.rising_edge()
                await rivet.read_only()
                got.append(q.get())
                await rivet.next_time_step()

        self.run_body(t)
        self.assertEqual(got, [0, 1, 0x5A, 0xFF])

    def test_a_deposit_lands_when_the_simulator_evaluates(self):
        """The visibility rule, which is the same one the Rust API has.

        `set` is an inertial deposit. Rivet buffers it and hands it to the
        simulator at the ReadWrite phase — and the simulator applies it
        *after* that callback returns, so a read inside ReadWrite still
        sees the old value. It is there at the next evaluation. `set_now`
        skips all of that and is visible to the next read.
        """
        seen = []

        async def t(dut):
            clk, d = dut.signal("clk"), dut.signal("d")
            rivet.Clock(clk, "10ns").start()
            d.set_now(0)

            d.set(0x11)
            seen.append(d.get())          # not yet: still buffered
            await rivet.read_write()
            seen.append(d.get())          # flushed to the simulator, not applied
            await rivet.next_time_step()
            seen.append(d.get())          # applied

            d.set_now(0x22)
            seen.append(d.get())          # immediate writes need no waiting

        self.run_body(t)
        self.assertEqual(seen, [0x00, 0x00, 0x11, 0x22])

    def test_wide_values_round_trip_exactly(self):
        got = []
        # 512 bits: far past u64 and u128, so this exercises the byte path.
        big = (1 << 511) | (0xDEADBEEF << 400) | 0x1234_5678_9ABC_DEF0
        allones = (1 << 512) - 1

        async def t(dut):
            wide = dut.signal("wide")
            for v in (0, 1, big, allones):
                wide.set_now(v)
                got.append(wide.get())

        self.run_body(t)
        self.assertEqual(got, [0, 1, big, allones])

    def test_values_wrap_to_the_signal_width(self):
        got = []

        async def t(dut):
            b = dut.signal("byte")
            b.set_now(0x1FF)
            got.append(b.get())
            b.set_now(-1)
            got.append(b.get())
            b.set_now(-2)
            got.append(b.get())
            got.append(b.get_signed())

        self.run_body(t)
        self.assertEqual(got, [0xFF, 0xFF, 0xFE, -2])

    def test_negative_values_on_a_wide_signal(self):
        got = []

        async def t(dut):
            wide = dut.signal("wide")
            wide.set_now(-1)
            got.append(wide.get())
            wide.set_now(-(1 << 300))
            got.append(wide.get())
            got.append(wide.get_signed())

        self.run_body(t)
        self.assertEqual(got[0], (1 << 512) - 1)
        self.assertEqual(got[1], (1 << 512) - (1 << 300))
        self.assertEqual(got[2], -(1 << 300))

    def test_x_and_z_are_reported_not_guessed(self):
        out = {}

        async def t(dut):
            b = dut.signal("byte")
            b.set_now("1010xxzz")
            out["binstr"] = b.get_binstr()
            out["resolvable"] = b.is_resolvable()
            out["lossy"] = b.get_lossy()
            try:
                b.get()
            except ValueError as e:
                out["error"] = str(e)
            # A short string is zero-extended, exactly as in the Rust
            # API: "x" is one X bit, not eight of them.
            b.set_now("x")
            out["one_x"] = b.get_binstr()
            b.set_now("8'hxx")
            out["all_x"] = b.get_binstr()

        self.run_body(t)
        self.assertEqual(out["binstr"], "1010xxzz")
        self.assertFalse(out["resolvable"])
        self.assertEqual(out["lossy"], 0b10100000)
        self.assertIn("X or Z", out["error"])
        self.assertEqual(out["one_x"], "0000000x")
        self.assertEqual(out["all_x"], "xxxxxxxx")

    def test_strings_are_accepted_in_the_usual_bases(self):
        got = []

        async def t(dut):
            b = dut.signal("byte")
            # Python's prefixes and Verilog's literals both work.
            for s in ("0b1010", "0xA5", "0o17", "11110000", "8'hA5", "8'd42", "8'b1111"):
                b.set_now(s)
                got.append(b.get())

        self.run_body(t)
        self.assertEqual(got, [0b1010, 0xA5, 0o17, 0b11110000, 0xA5, 42, 0b1111])

    def test_slices_read_and_compose(self):
        got = []

        async def t(dut):
            rivet.Clock(dut.signal("clk"), "10ns").start()
            b = dut.signal("byte")
            b.set_now(0xA5)
            got.append(b.slice(7, 4).get())
            got.append(b.slice(3, 0).get())
            # Two slice writes in one time step compose rather than the
            # second losing the first. They are deposits, so they land at
            # the next evaluation.
            b.slice(7, 4).set(0x3)
            b.slice(3, 0).set(0xC)
            await rivet.next_time_step()
            got.append(b.get())

        self.run_body(t)
        self.assertEqual(got, [0xA, 0x5, 0x3C])

    def test_an_out_of_range_slice_is_refused(self):
        out = {}

        async def t(dut):
            try:
                dut.signal("byte").slice(9, 0)
            except IndexError as e:
                out["error"] = str(e)

        self.run_body(t)
        self.assertIn("outside", out["error"])

    def test_force_overrides_the_design_until_released(self):
        got = []

        async def t(dut):
            clk, d, q = dut.signal("clk"), dut.signal("d"), dut.signal("q")
            rivet.Clock(clk, "10ns").start()
            d.set(0x11)
            await clk.rising_edge()
            await rivet.read_only()
            got.append(q.get())
            await rivet.next_time_step()

            q.force(0x99)
            d.set(0x22)
            await clk.rising_edge()
            await rivet.read_only()
            got.append(q.get())
            await rivet.next_time_step()

            q.release()
            d.set(0x33)
            await clk.rising_edge(n=2)
            await rivet.read_only()
            got.append(q.get())

        self.run_body(t)
        self.assertEqual(got, [0x11, 0x99, 0x33])

    def test_the_width_is_visible(self):
        out = {}

        async def t(dut):
            out["byte"] = dut.signal("byte").width
            out["wide"] = dut.signal("wide").width
            out["len"] = len(dut.signal("wide"))

        self.run_body(t)
        self.assertEqual(out, {"byte": 8, "wide": 512, "len": 512})

    def test_a_bad_type_says_what_was_expected(self):
        out = {}

        async def t(dut):
            try:
                dut.signal("byte").set({"not": "a value"})
            except TypeError as e:
                out["error"] = str(e)

        self.run_body(t)
        self.assertIn("expected int, bool, str", out["error"])


class Vectors(unittest.TestCase):
    def test_logic_vec_basics(self):
        v = rivet.LogicVec("1010xz01")
        self.assertEqual(len(v), 8)
        self.assertEqual(str(v), "1010xz01")
        self.assertTrue(v.has_x())
        self.assertTrue(v.has_z())
        self.assertFalse(v.is_resolvable())
        with self.assertRaises(ValueError):
            v.to_int()

        n = rivet.LogicVec(0xA5, 8)
        self.assertEqual(n.to_int(), 0xA5)
        self.assertEqual(int(n), 0xA5)
        self.assertEqual(n.binstr(), "10100101")
        self.assertEqual(n, rivet.LogicVec("10100101"))
        self.assertEqual(n, 0xA5)
        self.assertNotEqual(n, 0xA4)

    def test_a_vector_can_be_written_to_a_signal(self):
        fresh()
        got = []

        @rivet.test(timeout="10us")
        async def t(dut):
            b = dut.signal("byte")
            b.set_now(rivet.LogicVec("1111xxxx"))
            got.append(b.get_binstr())

        assert_passed(self, only(dff_design().run()))
        self.assertEqual(got, ["1111xxxx"])


class Hierarchy(unittest.TestCase):
    def test_a_misspelled_port_names_the_scope(self):
        fresh()
        out = {}

        @rivet.test
        async def t(dut):
            try:
                dut.signal("conut")
            except AttributeError as e:
                out["error"] = str(e)
            out["children"] = sorted(dut.children())
            out["has"] = (dut.has("clk"), dut.has("nope"))
            out["path"] = dut.signal("clk").path

        assert_passed(self, only(dff_design().run()))
        self.assertIn("conut", out["error"])
        self.assertEqual(out["children"], ["byte", "clk", "d", "q", "wide"])
        self.assertEqual(out["has"], (True, False))
        self.assertEqual(out["path"], "top.clk")

    def test_item_access_and_dotted_paths(self):
        fresh()
        out = {}

        @rivet.test
        async def t(dut):
            out["item"] = dut["clk"].path
            out["at"] = dut.at("q").width
            out["repr"] = repr(dut.signal("byte"))

        assert_passed(self, only(dff_design().run()))
        self.assertEqual(out["item"], "top.clk")
        self.assertEqual(out["at"], 8)
        self.assertIn("top.byte", out["repr"])


if __name__ == "__main__":
    unittest.main()
