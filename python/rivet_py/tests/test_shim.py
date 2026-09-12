"""Unit tests for rivet_py (no simulator). Run with `python -m unittest`
after `maturin develop` (or with the built wheel on the path)."""

import os
import tempfile
import unittest

import rivet_py as rv


class MemoryTest(unittest.TestCase):
    def test_words_and_hex(self):
        m = rv.Memory(fill=0xFF)
        self.assertEqual(m.read_u8(1234), 0xFF)
        m.write_u32(0x1000, 0xDEADBEEF)
        self.assertEqual(m.read_u16 if False else m.read_word(0x1002, 2), 0xDEAD)
        m.write_word(0x1000, 4, 0x11223344, strobe=0b0101)
        self.assertEqual(m.read_u32(0x1000), 0xDE22BE44)
        self.assertEqual(m.read_bytes(0x1000, 2), b"\x44\xbe")
        m.write_bytes(0x2000, b"\x01\x02\x03")
        self.assertEqual(m.read_u64(0x2000) & 0xFFFFFF, 0x030201)
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "prog.hex")
            with open(p, "w") as f:
                f.write("deadbeef\n@4 00000001\nxxxxxxxx\n")
            self.assertEqual(m.load_hex(p, base=0x100, word_bytes=4), 2)
            self.assertEqual(m.read_u32(0x100), 0xDEADBEEF)
            self.assertEqual(m.read_u32(0x110), 1)
            out = os.path.join(d, "out.hex")
            m.dump_hex(out, 0x100, 2, 4)
            with open(out) as f:
                self.assertEqual(f.read(), "deadbeef\nffffffff\n")
            with self.assertRaises(RuntimeError):
                m.load_hex(os.path.join(d, "missing"))
        self.assertEqual(rv.parse_readmemh("aa @10 x1"), [(0, 0xAA), (0x10, None)])
        self.assertGreaterEqual(m.pages(), 2)
        m.clear()
        self.assertEqual(m.pages(), 0)


class RngTest(unittest.TestCase):
    def test_deterministic(self):
        a = rv.Rng(42)
        b = rv.Rng(42)
        self.assertEqual([a.next_u64() for _ in range(4)], [b.next_u64() for _ in range(4)])
        # Same first value as the Rust implementation pins.
        self.assertEqual(rv.Rng(42).next_u64(), 1546998764402558742)
        r = rv.Rng(1)
        for _ in range(1000):
            self.assertIn(r.randint(3, 6), (3, 4, 5, 6))
            self.assertTrue(0.0 <= r.random() < 1.0)
        with self.assertRaises(ValueError):
            r.randint(5, 4)
        self.assertIn(r.choice(["a", "b"]), ("a", "b"))
        self.assertEqual(sorted(r.shuffle(list(range(10)))), list(range(10)))
        self.assertEqual(len(r.bytes(5)), 5)
        self.assertNotEqual(r.fork().next_u64(), r.fork().next_u64())
        self.assertEqual(rv.seed_for_test(1, "m::t"), rv.seed_for_test(1, "m::t"))
        self.assertNotEqual(rv.seed_for_test(1, "m::t"), rv.seed_for_test(2, "m::t"))


class ScoreboardTest(unittest.TestCase):
    def test_in_order(self):
        sb = rv.Scoreboard("fifo")
        sb.expect({"a": 1})
        sb.expect([1, 2])
        sb.observe({"a": 1})
        self.assertEqual(sb.matched(), 1)
        self.assertEqual(sb.pending(), 1)
        sb.observe([1, 2])
        sb.finish()
        sb.observe(3)
        self.assertEqual(len(sb.errors()), 1)
        with self.assertRaises(RuntimeError):
            sb.finish()


class CoverageTest(unittest.TestCase):
    def test_bins_and_cross(self):
        rv.clear_coverage()
        cg = rv.Covergroup("axi")
        length = cg.point("len", rv.Bins().bin("one", 1, 1).bin("short", 2, 4).values("pow2", [8, 16]))
        size = cg.point("size", rv.Bins().auto("s", 0, 1))
        cross = cg.cross("len_x_size", [length, size])
        self.assertTrue(length.sample(1))
        self.assertFalse(length.sample(7))
        size.sample(1)
        self.assertEqual(cross.hits(), [("one x s1", 1)])
        self.assertEqual(cg.counts(), (3, 3 + 2 + 6))
        self.assertAlmostEqual(rv.coverage_percent(), 100.0 * 3 / 11)
        self.assertIn('"name":"axi"', rv.coverage_json())
        self.assertIn("covergroup axi", rv.coverage_table())
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "coverage.json")
            rv.write_coverage(p)
            self.assertTrue(os.path.getsize(p) > 0)
        self.assertEqual(len(rv.Bins().split("q", 0, 99, 4)), 4)


if __name__ == "__main__":
    unittest.main()
