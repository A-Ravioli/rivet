"""Registration, selection and how a test ends.

Python tests go through Rivet's own regression loop, so everything here
is the behaviour `#[rivet::test]` already has, reached from Python.
"""

import unittest

import harness
import rivet
import _rivet
from harness import assert_passed, counter_design, dff_design, fresh, only, statuses

# Tests registered here take their module name from this file.
MOD = __name__


class Registration(unittest.TestCase):
    def test_the_decorator_works_bare_and_with_arguments(self):
        fresh()

        @rivet.test
        async def bare(dut):
            pass

        @rivet.test(timeout="1us")
        async def configured(dut):
            await rivet.timer("1ns")

        # The function itself is returned unchanged.
        self.assertTrue(callable(bare))
        self.assertEqual(bare.__name__, "bare")
        self.assertEqual(_rivet.registered_count(), 2)
        for r in counter_design().run():
            assert_passed(self, r)

    def test_stages_order_the_run(self):
        fresh()
        order = []

        @rivet.test(stage=2)
        async def late(dut):
            order.append("late")

        @rivet.test(stage=0)
        async def early(dut):
            order.append("early")

        @rivet.test(stage=1)
        async def middle(dut):
            order.append("middle")

        counter_design().run()
        self.assertEqual(order, ["early", "middle", "late"])

    def test_a_filter_selects_a_subset(self):
        fresh()
        ran = []

        @rivet.test
        async def alpha(dut):
            ran.append("alpha")

        @rivet.test
        async def beta(dut):
            ran.append("beta")

        results = counter_design().run(filter="alpha")
        self.assertEqual(ran, ["alpha"])
        self.assertEqual(len(results), 1)

    def test_a_custom_name_is_used(self):
        fresh()

        @rivet.test(name="renamed")
        async def original(dut):
            pass

        self.assertEqual(_rivet.registered_names(), [f"{MOD}::renamed"])
        r = only(counter_design().run())
        self.assertEqual(r["name"], "renamed")

    def test_a_result_carries_the_source_location(self):
        fresh()

        @rivet.test
        async def located(dut):
            pass

        r = only(counter_design().run())
        self.assertTrue(r["file"].endswith("test_regression.py"), r["file"])
        self.assertGreater(r["line"], 0)

    def test_a_non_async_function_is_refused(self):
        fresh()

        @rivet.test
        def not_a_coroutine(dut):
            return 1

        r = only(counter_design().run())
        self.assertEqual(r["status"], "failed")
        self.assertIn("async def", r["message"])


class Outcomes(unittest.TestCase):
    def test_an_assertion_failure_carries_its_traceback(self):
        fresh()

        @rivet.test
        async def checks(dut):
            value = 3
            assert value == 4, "the design said 3"

        r = only(counter_design().run())
        self.assertEqual(r["status"], "failed")
        self.assertIn("the design said 3", r["message"])
        self.assertIn("test_regression.py", r["message"])
        self.assertIn("AssertionError", r["message"])

    def test_skip_marks_the_test_skipped(self):
        fresh()

        @rivet.test
        async def by_call(dut):
            rivet.skip("no DDR model in this build")

        @rivet.test(skip=True)
        async def by_flag(dut):
            raise AssertionError("never runs")

        got = statuses(counter_design().run())
        self.assertEqual(set(got.values()), {"skipped"})

    def test_expect_fail_inverts_the_result(self):
        fresh()

        @rivet.test(expect_fail=True)
        async def known_broken(dut):
            assert False, "still broken"

        @rivet.test(expect_fail=True)
        async def unexpectedly_fixed(dut):
            pass

        got = statuses(counter_design().run())
        self.assertEqual(got[(MOD, "known_broken")], "passed")
        self.assertEqual(got[(MOD, "unexpectedly_fixed")], "failed")

    def test_expect_fail_msg_requires_the_right_failure(self):
        fresh()

        @rivet.test(expect_fail_msg="off by one")
        async def right_reason(dut):
            raise AssertionError("off by one at cycle 7")

        @rivet.test(expect_fail_msg="off by one")
        async def wrong_reason(dut):
            raise AssertionError("the clock never started")

        got = statuses(counter_design().run())
        self.assertEqual(got[(MOD, "right_reason")], "passed")
        self.assertEqual(got[(MOD, "wrong_reason")], "failed")

    def test_a_timeout_names_what_every_task_is_waiting_for(self):
        fresh()

        @rivet.test(timeout="100ns")
        async def hangs(dut):
            # Nothing drives the clock, so this never fires.
            await dut.signal("clk").rising_edge()

        r = only(counter_design().run())
        self.assertEqual(r["status"], "failed")
        self.assertIn("timed out after 100ns", r["message"])
        self.assertIn("RisingEdge(top.clk)", r["message"])

    def test_expect_timeout(self):
        fresh()

        @rivet.test(timeout="100ns", expect_timeout=True)
        async def deliberately_hangs(dut):
            await dut.signal("clk").rising_edge()

        assert_passed(self, only(counter_design().run()))

    def test_finish_ends_the_test_as_passed(self):
        fresh()
        reached = []

        @rivet.test(timeout="10us")
        async def early_out(dut):
            clk = dut.signal("clk")
            rivet.Clock(clk, "10ns").start()
            await clk.rising_edge()
            rivet.finish("seen enough")
            reached.append("unreachable")

        assert_passed(self, only(counter_design().run()))
        self.assertEqual(reached, [])

    def test_finish_now_ends_it_from_a_background_task(self):
        fresh()

        @rivet.test(timeout="10us")
        async def watched(dut):
            clk = dut.signal("clk")
            rivet.Clock(clk, "10ns").start()

            async def watcher():
                await clk.rising_edge(n=3)
                rivet.finish_now()

            rivet.start_soon(watcher())
            # Would otherwise run to the timeout.
            await clk.rising_edge(n=100_000)

        assert_passed(self, only(counter_design().run()))

    def test_fail_fails_the_test(self):
        fresh()

        @rivet.test
        async def explicit(dut):
            rivet.fail("the scoreboard is not empty")

        r = only(counter_design().run())
        self.assertEqual(r["status"], "failed")
        self.assertIn("scoreboard is not empty", r["message"])

    def test_each_test_gets_a_seed_derived_from_the_run(self):
        fresh()
        seeds = []

        @rivet.test
        async def a(dut):
            seeds.append(rivet.test_seed())

        @rivet.test
        async def b(dut):
            seeds.append(rivet.test_seed())

        results = counter_design().run()
        self.assertEqual(len(set(seeds)), 2, "different tests get different seeds")
        self.assertEqual([r["seed"] for r in results], seeds)


class Kit(unittest.TestCase):
    def test_a_scoreboard_catches_a_mismatch(self):
        fresh()

        @rivet.test(timeout="10us")
        async def scored(dut):
            clk, d, q = dut.signal("clk"), dut.signal("d"), dut.signal("q")
            rivet.Clock(clk, "10ns").start()
            sb = rivet.Scoreboard("dff")
            for v in (1, 2, 3, 4):
                d.set(v)
                sb.expect(v)
                await clk.rising_edge()
                await rivet.read_only()
                sb.observe(q.get())
                await rivet.next_time_step()
            sb.finish()

        assert_passed(self, only(dff_design().run()))

    def test_the_rng_is_reproducible_from_a_seed(self):
        a = rivet.Rng(seed=7)
        b = rivet.Rng(seed=7)
        self.assertEqual([a.randint(0, 1000) for _ in range(10)],
                         [b.randint(0, 1000) for _ in range(10)])
        self.assertNotEqual(rivet.Rng(seed=8).next_u64(), rivet.Rng(seed=7).next_u64())

    def test_memory_holds_values(self):
        m = rivet.Memory()
        m.write_u32(0x100, 0xDEADBEEF)
        self.assertEqual(m.read_u32(0x100), 0xDEADBEEF)
        m.write_bytes(0, b"\x01\x02\x03\x04")
        self.assertEqual(m.read_bytes(0, 4), b"\x01\x02\x03\x04")
        self.assertEqual(m.read_word(0, 2), 0x0201)

    def test_coverage_records_bins(self):
        rivet.clear_coverage()
        cg = rivet.Covergroup("axi")
        pt = cg.point("len", rivet.Bins().bin("short", 1, 4).bin("long", 5, 16))
        self.assertTrue(pt.sample(2))
        self.assertTrue(pt.sample(9))
        self.assertFalse(pt.sample(99))
        self.assertEqual(dict(pt.hits()), {"short": 1, "long": 1})
        self.assertEqual(pt.percent(), 100.0)
        rivet.clear_coverage()


if __name__ == "__main__":
    unittest.main()
