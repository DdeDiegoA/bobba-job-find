-- 0011_agent_run_pack_version.sql — the agent pack a run rendered (S2.4, AC2).
-- Never edit once shipped (AD-20).
--
-- Every stored artifact traces back to the pack that produced it, so the
-- version of `agent-pack/` (system.md plus every SKILL.md, hashed) is recorded
-- on the run, next to the `cli` and `model` it already carries (AD-4, AD-5).
-- Rows written before S2.4 carry '' — the version was not recorded then, and a
-- made-up value would be worse than an honest blank.
ALTER TABLE agent_run ADD COLUMN pack_version TEXT NOT NULL DEFAULT '';
