# CH4 review — composition root and port wiring (ADR-012)

- **Reviewed:** `hermad/CH4` @ `c8e6527` (base `main` 738f8a0; `339a8d4` adds the wiring, `c8e6527` adds the `errors.store.locked` EN/ES strings)
- **Reviewer:** reviewer-1 (paired with dev-1)
- **Verdict:** APPROVED — no blocking findings. The deny red is the pre-existing `main` baseline (see gate table).

## Acceptance (ADR-012 §1–5)

No CH4 acceptance criteria exist in `stories.yaml`, the backlog, or the slices file; the reviewed scope is ADR-012 §1–5 and its Consequences.

| § | Requirement | Verdict |
|---|---|---|
| 1 | `src-tauri/src/wiring/` owns the mount; `lib.rs` calls `wiring::mount` once in `setup`; `Cargo.toml` adds `boba-store` | ✅ `lib.rs` +4 lines (`pub mod wiring`, one `mount` call); `Cargo.toml` adds `boba-store` and the `tauri` `test` dev-dep |
| 2 | `Store::open` with `StoreConfig::dev` under `debug_assertions`; paths from `app_data_dir`; `Arc<Store>` | ✅ `wiring/mod.rs` `config()`, `mount()`; `open_with_key` creates the parent dir (`crates/boba-store/src/db/mod.rs:94`) |
| 3 | `port_list.rs` `merge=union`, one `mount_port` line per port; seeds every port with a store impl | ✅ 3 seeded lines (`SourcesRepo`, `AgentRepo`, `SystemRepo`); `Store` implements `SourcesRepo` and `AgentRepo`, `SqliteSystemRepo` implements `SystemRepo`; `.gitattributes` union entry present |
| 4 | Failed open → `StoreStatus::Locked { reason }`, no panic; `port::<dyn P>` returns `AppError { Locked, "errors.store.locked" }` | ✅ verified by `unmounted_port_is_a_typed_locked_error` |
| 5 | S1.2b plugs in via `mount_ports` at runtime; no unlock UI in CH4 | ✅ nothing beyond the typed `Locked` result is added |

Dev-1's two scrutiny points, checked:
1. `tauri::Manager::manage` returns `bool` and does not panic; the duplicate-line panic is in `mount_port`'s `assert!` (`wiring/mod.rs`). `duplicate_port_line_panics` passes, so the guard is real.
2. `pub mod wiring` keeps `port()` live under `-D warnings`; clippy passes.

## Findings

No blocking findings.

- **nit (non-blocking):** `wiring/mod.rs` `tests::tmp_dir` creates `boba-wiring-*` dirs under `temp_dir()` and never removes them. Harmless in CI; a `Drop` guard or `tempfile` would keep the temp dir clean.
- **note (for the orquestador, not CH4 scope):** `wiring::mount` opens the encrypted store in `setup` using the keychain service. The real OS-keychain open path is not exercised by any test (tests use `open_with_key` with a fixed key). This is the S1.2b security gate and should be covered there.
- **note:** dev-1's handoff described the deny red as "only zip 4.6.1". The actual red is four `rustls 0.23.45` advisories (RUSTSEC-2026-0346 to -0349) plus the yanked `zip 4.6.1`. These are the known baseline set recorded in `docs/reviews/S4.1c.md` and `main` is red the same way; CH4 does not change `Cargo.lock` beyond what `main` already has.

## Gate (run by reviewer-1 on `c8e6527`)

| Gate | Result |
|---|---|
| `cargo metadata --locked` | ✅ exit 0 |
| `cargo fmt --all -- --check` | ✅ clean |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | ✅ Finished, no warnings |
| `cargo test --workspace --locked` | ✅ all suites pass; `wiring::tests` 3/3 (smoke, duplicate-panic, locked) |
| `cargo deny check` | ❌ exit 1 — **pre-existing baseline only**: `rustls 0.23.45` RUSTSEC-2026-0346…0349 + yanked `zip 4.6.1` (`main` shares the same red, see S4.1c). Not a CH4 regression; per the orquestador rule, treated as blocked-by-CH3 for the zip item |
| `pnpm install --frozen-lockfile` | ✅ |
| `pnpm lint` | ✅ exit 0 (3 pre-existing warnings in untouched TS) |
| `pnpm typecheck` | ✅ |
| `pnpm test` | ✅ 30/30 |
| `node scripts/check-i18n.mjs` | ✅ on `c8e6527` (EN/ES aligned) |
| `node scripts/ci/check-workflow-policy.mjs` | ✅ |
| `node scripts/gen-bindings.mjs --check` | ✅ up to date |

Not covered: a real Tauri dev boot with the OS keychain (see the note above).
