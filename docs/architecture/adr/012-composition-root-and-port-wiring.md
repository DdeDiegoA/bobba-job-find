# ADR-012 — Composition root and port wiring

- **Status:** Accepted · 2026-10-15 (raised by S3.4)
- **Spine:** AD-3, AD-14, AD-16; ADR-009 (locked state)

## Context
Commands read their ports from Tauri state (`State<Arc<dyn ProfileRepo>>` and so on), but no slice owns opening the store and calling `app.manage(...)`. S1.2b (wave 8) is the natural owner of the unlock flow, so without a decision waves 5–7 ship commands behind state that is never mounted. If each slice instead adds its own wiring to `lib.rs`, every slice conflicts on the same file.

## Decision
**Establish the composition root now in chore CH4, before S1.2b.** It has one fixed entry point and one union-merged registration list, following the same pattern as `ipc/command_list.rs`.

1. **Owner and entry point.** CH4 owns the new module `src-tauri/src/wiring/` and makes the one-time edits to `lib.rs` and `src-tauri/Cargo.toml` (adding the `boba-store` dependency; `Cargo.lock` is regenerated, never hand-merged). In `setup`, `lib.rs` calls `wiring::mount(app)` once. After CH4, no slice edits `lib.rs` for wiring.
2. **Open the store (S1.2a path).** `wiring::mount` calls `Store::open(StoreConfig)` and uses `StoreConfig::dev` under `debug_assertions` (ADR-009). The paths come from Tauri's `app_data_dir`. The result is wrapped in `Arc<Store>`.
3. **Register ports in one list.** `src-tauri/src/wiring/port_list.rs` (`.gitattributes` `merge=union`) holds `fn mount_ports(app: &AppHandle, store: &Arc<Store>)`. Its body is **one `app.manage::<Arc<dyn Port>>(…);` line per port**. A slice that adds a repo or port to the store appends one line there and nothing else. CH4 seeds a line for every port that already has a store implementation. An adapter that needs a constructor uses the same one-line form, for example `Arc::new(SqliteSystemRepo::new(store.clone(), …))`.
4. **Locked or failed open is a typed state, not a panic.** If `Store::open` fails (the key is unavailable or the store is locked), `mount` skips `mount_ports` and manages `StoreStatus::Locked { reason }`. Commands get a port only through the shared helper `wiring::port::<dyn P>(&app) -> AppResult<Arc<dyn P>>`. When the port is not mounted, the helper returns `AppError { code: ErrorCode::Locked, message_key: "errors.store.locked" }`. Commands never call `try_state` for ports themselves.
5. **S1.2b plugs in later.** Its unlock command opens the store with `Store::open_with_key` and then calls the same `wiring::mount_ports` at runtime. Tauri accepts `manage` after setup. The locked path, the unlock screen and the passphrase stay in S1.2b's scope. CH4 only adds the typed `Locked` result.

## Consequences
- Commands built in waves 5–7 work end to end as soon as their port line is appended to `port_list.rs`.
- A bad union merge, such as a duplicate `manage` line, panics at startup, because Tauri rejects a second `manage` of the same type. An app-start smoke test catches this, just as compilation catches errors in `command_list.rs`.
- Tauri state cannot be replaced once managed, so swapping the store after Reset or restore (S10.2) requires a relaunch. This is already the S10.2 plan (`tauri_plugin_process` relaunch) and is acceptable.

## Alternatives
- **(b) Defer all wiring to S1.2b:** three waves of commands could not be exercised in the running app, and S1.2b would grow into a large wiring-plus-security slice that sits behind the Diego gate. Rejected.
- **Each slice edits `lib.rs`:** guaranteed merge conflicts between parallel devs. Rejected for the same reason `registry.rs` was retired (AD-3).
- **One `AppState` struct holding every port:** a single shared struct that every slice edits, the same conflict problem in another file. Rejected.
