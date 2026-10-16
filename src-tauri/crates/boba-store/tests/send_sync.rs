//! Compile-time assertions (S4.1c): every port and repo trait is `Send + Sync`
//! (spine convention), and the store that implements `SourcesRepo` is too, so
//! both can live in Tauri state and async use cases.

use boba_app::ports::repo::sources::SourcesRepo;
use boba_store::db::Store;

/// Compiles only if `T` is `Send + Sync`; `?Sized` covers trait objects.
fn _assert<T: Send + Sync + ?Sized>() {}

#[test]
fn store_is_send_and_sync() {
    _assert::<Store>();
}

#[test]
fn sources_repo_object_is_send_and_sync() {
    _assert::<dyn SourcesRepo>();
}
