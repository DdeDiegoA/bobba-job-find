# ADR-017 — Event names: `<area>:<verb-ed>`, no dots

- **Status:** Accepted · 2026-10-17 (CH10) · amends AD-3
- **Spine:** AD-3

## Context
AD-3 first prescribed `<area>.<thing>.<verb-ed>` (`posting.scored`, `agent.run.progressed`). Tauri 2.12 rejects a `.` in an event name (`EventName::new` returns `IllegalEventName`). Every emit failed, and because the errors were swallowed the app-wide event bus was dead until CH10 found it.

## Decision
- Tauri accepts only alphanumerics and `-`, `/`, `:`, `_`. Events are named `<area>:<verb-ed>` with `_` inside a segment, and never contain a dot. The registered set is `task:progressed`, `task:finished`, `run_report:progressed`, `kill_switch:changed`, `notification:opened`.
- Emit errors are logged (`tracing::warn!`), never swallowed.
- `ipc::tests::every_registered_event_name_is_legal_for_tauri` exports the bindings, asserts the known names are present (so the parser cannot match nothing and pass), and runs each name through `Emitter::emit`. A dotted name fails CI.
- The convention is documented in `src-tauri/src/events/mod.rs`. Audit event names (`apply.intent` and the like) are database values, not Tauri events, and keep their dotted form.

## Consequences
- The UI listens on the colon names from the generated bindings. Nothing hand-writes an event string.
- A Tauri upgrade that tightens the rule fails the validity test, not production.

## Alternatives
- **Keep dots and map them at the emit site:** a second naming layer to keep in sync. Rejected.
