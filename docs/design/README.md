# Rivet design documents

Read in order.

1. [`00-cocotb-analysis.md`](00-cocotb-analysis.md): what cocotb 2.2.0 actually
   does, file by file: scheduler, GPI layer, the three PLI backends, the
   Verilator loop, the handle/value hot path, the launch and regression layers,
   and a complete table of per-simulator workarounds.
2. [`01-architecture.md`](01-architecture.md): first-principles design of
   Rivet: what a harness is, principles, execution topology, executor, timing
   model and write scheduling, values and hierarchy, the backend trait, the
   test layer, the kit library, performance targets, risks.
3. [`02-roadmap.md`](02-roadmap.md): crate layout and milestones M0 through M7
   with exit criteria.
4. [`03-status.md`](03-status.md): what is implemented, verified, and still
   open, against the roadmap.
5. [`04-remaining-work.md`](04-remaining-work.md): the concrete plan for each
   remaining gap, what it costs, and what needs a licence.
