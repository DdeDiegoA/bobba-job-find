-- 0004_agent_runs.sql — agent run lifecycle (S2.2). Never edit once shipped (AD-20).
--
-- Owner: agent (AD-16). `agent_run` is one row per AgentPort::run; `agent_event`
-- is its ordered stream. `cli`/`model` attribute every run (AD-4). Usage columns
-- are null when the CLI does not report them — the core never guesses.
-- `started_at`/`ended_at`/`at` are UTC RFC 3339 (AD-17).
CREATE TABLE agent_run (
    id             TEXT PRIMARY KEY NOT NULL,
    task_kind      TEXT NOT NULL,
    input_trust    TEXT NOT NULL,
    cli            TEXT NOT NULL,
    model          TEXT,
    status         TEXT NOT NULL,
    failure_reason TEXT,
    started_at     TEXT NOT NULL,
    ended_at       TEXT,
    tokens_in      INTEGER,
    tokens_out     INTEGER,
    cost_usd       REAL
) STRICT;

CREATE INDEX agent_run_status_idx ON agent_run (status);

CREATE TABLE agent_event (
    run_id       TEXT NOT NULL REFERENCES agent_run (id) ON DELETE CASCADE,
    seq          INTEGER NOT NULL,
    kind         TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    at           TEXT NOT NULL,
    PRIMARY KEY (run_id, seq)
) STRICT;
