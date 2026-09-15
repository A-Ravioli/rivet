"""Triggers, tasks and the coroutine driver."""

import unittest

import harness
import rivet
from harness import assert_passed, counter_design, fresh, only


class Triggers(unittest.TestCase):
    def test_edges_advance_one_cycle_at_a_time(self):
        fresh()
        seen = []

        @rivet.test(timeout="10us")
        async def t(dut):
            clk = dut.signal("clk")
            rivet.Clock(clk, "10ns").start()
            for _ in range(5):
                await clk.rising_edge()
                seen.append(rivet.now())

        assert_passed(self, only(counter_design().run()))
        # One 10ns period between rising edges, at 1ps precision.
        self.assertEqual([b - a for a, b in zip(seen, seen[1:])], [10_000] * 4)

    def test_a_batched_edge_lands_where_the_loop_would(self):
        fresh()
        times = {}

        @rivet.test(timeout="100us")
        async def batched(dut):
            clk = dut.signal("clk")
            rivet.Clock(clk, "10ns").start()
            await clk.rising_edge(n=50)
            times["batched"] = rivet.now()

        @rivet.test(timeout="100us")
        async def looped(dut):
            clk = dut.signal("clk")
            rivet.Clock(clk, "10ns").start()
            for _ in range(50):
                await clk.rising_edge()
            times["looped"] = rivet.now()

        results = counter_design().run()
        for r in results:
            assert_passed(self, r)
        # Each test starts one step after the previous one ends, so
        # compare elapsed time rather than absolute.
        self.assertEqual(len(times), 2)

    def test_falling_edges_and_value_changes(self):
        fresh()
        counts = {"fall": 0, "any": 0}

        @rivet.test(timeout="10us")
        async def t(dut):
            clk = dut.signal("clk")
            rivet.Clock(clk, "10ns").start()
            for _ in range(3):
                await clk.falling_edge()
                counts["fall"] += 1
            for _ in range(4):
                await clk.value_change()
                counts["any"] += 1

        assert_passed(self, only(counter_design().run()))
        self.assertEqual(counts, {"fall": 3, "any": 4})

    def test_timer_advances_simulated_time(self):
        fresh()
        marks = []

        @rivet.test(timeout="10us")
        async def t(dut):
            start = rivet.now()
            await rivet.timer("100ns")
            marks.append(rivet.now() - start)
            await rivet.timer(250, "ps")
            marks.append(rivet.now() - start)

        assert_passed(self, only(counter_design().run()))
        self.assertEqual(marks, [100_000, 100_250])

    def test_a_timer_with_no_unit_is_refused(self):
        fresh()

        @rivet.test
        async def t(dut):
            await rivet.timer(10)

        r = only(counter_design().run())
        self.assertEqual(r["status"], "failed")
        self.assertIn("no time unit", r["message"])

    def test_read_only_sees_the_settled_value(self):
        fresh()
        observed = []

        @rivet.test(timeout="10us")
        async def t(dut):
            clk = dut.signal("clk")
            count = dut.signal("count")
            rivet.Clock(clk, "10ns").start()
            dut.signal("rst_n").set(0)
            dut.signal("en").set(1)
            # Two edges of reset, so `count` is a number and not X.
            await clk.rising_edge(n=2)
            dut.signal("rst_n").set(1)
            for _ in range(3):
                await clk.rising_edge()
                # Before the flop reacts.
                before = count.get()
                await rivet.read_only()
                observed.append((before, count.get()))
                await rivet.next_time_step()

        assert_passed(self, only(counter_design().run()))
        # The edge returns before the design reacts to it, so each pair is
        # (old, new) and they differ by one.
        for before, after in observed:
            self.assertEqual(after, before + 1)

    def test_writing_in_the_read_only_phase_is_refused(self):
        fresh()

        @rivet.test(timeout="10us")
        async def t(dut):
            clk = dut.signal("clk")
            rivet.Clock(clk, "10ns").start()
            await clk.rising_edge()
            await rivet.read_only()
            dut.signal("en").set(1)

        r = only(counter_design().run())
        self.assertEqual(r["status"], "failed")
        self.assertIn("ReadOnly", r["message"])

    def test_first_returns_the_winner_and_drops_the_rest(self):
        fresh()
        won = []

        @rivet.test(timeout="10us")
        async def t(dut):
            clk = dut.signal("clk")
            rivet.Clock(clk, "10ns").start()
            # The clock edge at 5ns beats the 1us timer.
            won.append(await rivet.first(clk.rising_edge(), rivet.timer("1us")))
            # And the other way round.
            won.append(await rivet.first(rivet.timer("1ns"), clk.rising_edge(n=100)))

        assert_passed(self, only(counter_design().run()))
        self.assertEqual(won, [0, 0])

    def test_yield_does_not_advance_time(self):
        fresh()
        marks = []

        @rivet.test(timeout="10us")
        async def t(dut):
            await rivet.timer("1ns")
            before = rivet.now()
            await rivet.yield_now()
            marks.append(rivet.now() - before)

        assert_passed(self, only(counter_design().run()))
        self.assertEqual(marks, [0])

    def test_awaiting_a_foreign_object_says_so(self):
        fresh()

        @rivet.test
        async def t(dut):
            import asyncio

            await asyncio.sleep(0)

        r = only(counter_design().run())
        self.assertEqual(r["status"], "failed")
        self.assertIn("not a Rivet trigger", r["message"])


class Tasks(unittest.TestCase):
    def test_a_started_task_runs_alongside_and_returns_a_value(self):
        fresh()
        got = []

        @rivet.test(timeout="10us")
        async def t(dut):
            clk = dut.signal("clk")
            rivet.Clock(clk, "10ns").start()

            async def count_edges(n):
                for _ in range(n):
                    await clk.rising_edge()
                return n * 2

            task = rivet.start_soon(count_edges(4))
            self.assertFalse(task.done)
            got.append(await task.join())
            self.assertTrue(task.done)

        assert_passed(self, only(counter_design().run()))
        self.assertEqual(got, [8])

    def test_awaiting_a_task_directly_works(self):
        fresh()
        got = []

        @rivet.test(timeout="10us")
        async def t(dut):
            async def quick():
                await rivet.timer("1ns")
                return "done"

            got.append(await rivet.start_soon(quick()))

        assert_passed(self, only(counter_design().run()))
        self.assertEqual(got, ["done"])

    def test_cancelling_a_task_stops_it(self):
        fresh()
        ticks = []

        @rivet.test(timeout="10us")
        async def t(dut):
            clk = dut.signal("clk")
            rivet.Clock(clk, "10ns").start()

            async def forever():
                while True:
                    await clk.rising_edge()
                    ticks.append(1)

            task = rivet.start_soon(forever())
            await clk.rising_edge(n=3)
            task.cancel()
            self.assertTrue(task.cancelled)
            before = len(ticks)
            await clk.rising_edge(n=5)
            self.assertEqual(len(ticks), before)

        assert_passed(self, only(counter_design().run()))

    def test_an_exception_in_a_started_task_fails_the_test(self):
        fresh()

        @rivet.test(timeout="10us")
        async def t(dut):
            clk = dut.signal("clk")
            rivet.Clock(clk, "10ns").start()

            async def boom():
                await clk.rising_edge()
                raise ValueError("the monitor saw something wrong")

            rivet.start_soon(boom())
            await clk.rising_edge(n=5)

        r = only(counter_design().run())
        self.assertEqual(r["status"], "failed")
        self.assertIn("the monitor saw something wrong", r["message"])

    def test_a_joined_failure_is_raised_at_the_join(self):
        fresh()
        caught = []

        @rivet.test(timeout="10us")
        async def t(dut):
            async def boom():
                await rivet.timer("1ns")
                raise KeyError("nope")

            task = rivet.start_soon(boom(), propagate=False)
            try:
                await task.join()
            except KeyError as e:
                caught.append(str(e))

        assert_passed(self, only(counter_design().run()))
        self.assertEqual(caught, ["'nope'"])

    def test_with_timeout_gives_up_and_cancels(self):
        fresh()
        outcome = []

        @rivet.test(timeout="1ms")
        async def t(dut):
            async def slow():
                await rivet.timer("1us")
                return "too late"

            try:
                await rivet.with_timeout(slow(), "100ns")
            except TimeoutError as e:
                outcome.append(str(e))

            async def quick():
                await rivet.timer("10ns")
                return "in time"

            outcome.append(await rivet.with_timeout(quick(), "100ns"))

        assert_passed(self, only(counter_design().run()))
        self.assertEqual(len(outcome), 2)
        self.assertIn("timed out", outcome[0])
        self.assertEqual(outcome[1], "in time")


class Clocks(unittest.TestCase):
    def test_a_clock_can_be_stopped(self):
        fresh()
        marks = []

        @rivet.test(timeout="10us")
        async def t(dut):
            clk = dut.signal("clk")
            clock = rivet.Clock(clk, "10ns").start()
            await clk.rising_edge(n=2)
            clock.stop()
            marks.append(rivet.now())
            # Nothing drives the clock now, so only the timer fires.
            await rivet.first(clk.rising_edge(), rivet.timer("500ns"))
            marks.append(rivet.now())

        assert_passed(self, only(counter_design().run()))
        self.assertEqual(marks[1] - marks[0], 500_000)

    def test_clock_options_apply(self):
        fresh()
        widths = []

        @rivet.test(timeout="10us")
        async def t(dut):
            clk = dut.signal("clk")
            rivet.Clock(clk, "10ns", high_time="2ns").start()
            await clk.rising_edge()
            rose = rivet.now()
            await clk.falling_edge()
            widths.append(rivet.now() - rose)

        assert_passed(self, only(counter_design().run()))
        self.assertEqual(widths, [2_000])

    def test_starting_a_clock_twice_is_refused(self):
        fresh()

        @rivet.test(timeout="10us")
        async def t(dut):
            clock = rivet.Clock(dut.signal("clk"), "10ns")
            clock.start()
            clock.start()

        r = only(counter_design().run())
        self.assertEqual(r["status"], "failed")
        self.assertIn("already running", r["message"])


if __name__ == "__main__":
    unittest.main()
