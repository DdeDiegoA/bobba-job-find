//! Onboarding completion (S3.8, FR-10): the core remembers that setup finished,
//! so the first-launch gate survives a cleared webview storage and every launch
//! reads the same answer. The value is one `setting` row (S1.7, AD-16), read and
//! written through the mounted [`SystemRepo`] port (ADR-012); no migration is
//! needed.
//!
//! Registration lives in [`crate::ipc::command_list`] (AD-3, merge=union).

use boba_app::error::{AppError, AppResult};
use boba_app::ports::backup::BackupPort;
use boba_app::ports::clock::ClockPort;
use boba_app::ports::repo::system::SystemRepo;
use boba_app::system::SystemClock;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::wiring;

/// The `setting` row holding when the wizard finished (UTC RFC 3339).
pub const COMPLETED_KEY: &str = "onboarding.completed_at";

/// Whether setup finished, and when (FR-10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct OnboardingStatus {
    pub completed: bool,
    /// UTC RFC 3339; `None` until the wizard finishes.
    pub completed_at: Option<String>,
}

/// The completion state the first-launch gate reads.
#[tauri::command]
#[specta::specta]
pub fn onboarding_status(app: tauri::AppHandle) -> AppResult<OnboardingStatus> {
    let system = wiring::port::<dyn SystemRepo>(&app)?;
    read(system.as_ref())
}

/// Record that setup finished. Idempotent: the first completion time wins, so
/// a retried call (or a double effect) never moves it.
///
/// AC3 is enforced here, not only in the wizard: setup cannot be marked
/// complete while `backup_status` says a recovery backup is required (S3.8
/// review F2). An unreadable backup status refuses too, for the same reason.
#[tauri::command]
#[specta::specta]
pub fn onboarding_complete(app: tauri::AppHandle) -> AppResult<OnboardingStatus> {
    let system = wiring::port::<dyn SystemRepo>(&app)?;
    let backup = wiring::port::<dyn BackupPort>(&app)?;
    complete(system.as_ref(), backup.as_ref(), &SystemClock.now_rfc3339())
}

fn read(system: &dyn SystemRepo) -> AppResult<OnboardingStatus> {
    let completed_at = system.setting_get(COMPLETED_KEY).map_err(store_error)?;
    Ok(OnboardingStatus {
        completed: completed_at.is_some(),
        completed_at,
    })
}

fn complete(
    system: &dyn SystemRepo,
    backup: &dyn BackupPort,
    now: &str,
) -> AppResult<OnboardingStatus> {
    // AC3: no recovery backup, no completion — fail closed when the core
    // cannot say (S3.8 review F1/F2).
    let required = backup
        .status()
        .map_err(|error| AppError::internal(error.message_key()))?
        .recovery_required;
    if required {
        return Err(AppError::conflict("errors.backup.recovery_required"));
    }
    if let Some(completed_at) = system.setting_get(COMPLETED_KEY).map_err(store_error)? {
        return Ok(OnboardingStatus {
            completed: true,
            completed_at: Some(completed_at),
        });
    }
    system
        .setting_put(COMPLETED_KEY, now, now)
        .map_err(store_error)?;
    Ok(OnboardingStatus {
        completed: true,
        completed_at: Some(now.to_string()),
    })
}

fn store_error(error: boba_app::ports::repo::system::SystemRepoError) -> AppError {
    AppError::internal("errors.store.setting").with_detail("reason", error.to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use boba_domain::audit::{AuditOutcome, AuditRow, ChainStatus};

    use super::*;
    use boba_app::ports::backup::{
        BackupError, BackupMeta, BackupPort, BackupStatus, ResetReport, RestoreReport,
    };
    use boba_app::ports::repo::system::{NewIntent, SettingRecord, SystemRepoError};

    /// A `setting` table with every other area unimplemented.
    #[derive(Default)]
    struct FakeSystem {
        settings: Mutex<HashMap<String, String>>,
    }

    impl SystemRepo for FakeSystem {
        fn setting_get(&self, key: &str) -> Result<Option<String>, SystemRepoError> {
            Ok(self.settings.lock().unwrap().get(key).cloned())
        }

        fn setting_put(
            &self,
            key: &str,
            value: &str,
            _updated_at: &str,
        ) -> Result<(), SystemRepoError> {
            self.settings
                .lock()
                .unwrap()
                .insert(key.to_string(), value.to_string());
            Ok(())
        }

        fn setting_list(&self) -> Result<Vec<SettingRecord>, SystemRepoError> {
            unimplemented!()
        }

        fn setting_delete(&self, _key: &str) -> Result<bool, SystemRepoError> {
            unimplemented!()
        }

        fn audit_append_intent(&self, _intent: &NewIntent) -> Result<AuditRow, SystemRepoError> {
            unimplemented!()
        }

        fn audit_append_outcome(
            &self,
            _intent: &AuditRow,
            _outcome: AuditOutcome,
            _detail_json: &str,
            _created_at: &str,
        ) -> Result<AuditRow, SystemRepoError> {
            unimplemented!()
        }

        fn audit_head_hash(&self) -> Result<Option<String>, SystemRepoError> {
            unimplemented!()
        }

        fn audit_verify_chain(&self) -> Result<ChainStatus, SystemRepoError> {
            unimplemented!()
        }

        fn audit_unresolved_intents(&self) -> Result<Vec<AuditRow>, SystemRepoError> {
            unimplemented!()
        }

        fn audit_anchor_get(&self) -> Result<Option<String>, SystemRepoError> {
            unimplemented!()
        }

        fn audit_anchor_is_known(&self, _hash: &str) -> Result<bool, SystemRepoError> {
            unimplemented!()
        }

        fn audit_anchor_set(&self, _hash: &str) -> Result<(), SystemRepoError> {
            unimplemented!()
        }
    }

    #[test]
    fn setup_is_not_done_before_the_wizard_finishes() {
        let system = FakeSystem::default();
        assert_eq!(
            read(&system).expect("status"),
            OnboardingStatus {
                completed: false,
                completed_at: None
            }
        );
    }

    #[test]
    fn finishing_is_recorded_and_the_first_time_wins() {
        let system = FakeSystem::default();
        let backup = FakeBackup {
            required: false,
            unreadable: false,
        };

        let first = complete(&system, &backup, "2026-10-17T09:00:00Z").expect("complete");
        assert_eq!(
            first,
            OnboardingStatus {
                completed: true,
                completed_at: Some("2026-10-17T09:00:00Z".to_string())
            }
        );
        // A second call never moves the completion time.
        let again = complete(&system, &backup, "2026-10-18T10:00:00Z").expect("complete again");
        assert_eq!(again.completed_at.as_deref(), Some("2026-10-17T09:00:00Z"));
        assert_eq!(read(&system).expect("status"), again);
    }

    #[test]
    fn finishing_is_refused_without_a_recovery_backup() {
        let system = FakeSystem::default();
        let backup = FakeBackup {
            required: true,
            unreadable: false,
        };

        let error = complete(&system, &backup, "2026-10-17T09:00:00Z").expect_err("no backup");
        assert_eq!(error.message_key, "errors.backup.recovery_required");
        // Refused means refused: no completion row was written (AC3).
        assert!(!read(&system).expect("status").completed);
        assert!(system.settings.lock().unwrap().get(COMPLETED_KEY).is_none());
    }

    #[test]
    fn an_unreadable_backup_status_blocks_completion() {
        let system = FakeSystem::default();
        let backup = FakeBackup {
            required: false,
            unreadable: true,
        };

        let error = complete(&system, &backup, "2026-10-17T09:00:00Z").expect_err("unreadable");
        assert_eq!(error.message_key, "errors.backup.internal");
        assert!(!read(&system).expect("status").completed);
    }

    /// A backup port that only answers the recovery requirement.
    struct FakeBackup {
        required: bool,
        unreadable: bool,
    }

    impl BackupPort for FakeBackup {
        fn status(&self) -> Result<BackupStatus, BackupError> {
            if self.unreadable {
                return Err(BackupError::Internal);
            }
            Ok(BackupStatus {
                recovery_created_at: None,
                recovery_path: None,
                recovery_present: !self.required,
                recovery_required: self.required,
            })
        }

        fn create(
            &self,
            _dest: &std::path::Path,
            _passphrase: &str,
            _now: &str,
        ) -> Result<BackupMeta, BackupError> {
            Err(BackupError::Internal)
        }

        fn restore(
            &self,
            _src: &std::path::Path,
            _passphrase: &str,
        ) -> Result<RestoreReport, BackupError> {
            Err(BackupError::Internal)
        }

        fn reset(
            &self,
            _service: &str,
            _profiles: &[std::path::PathBuf],
            _confirmation: &str,
        ) -> Result<ResetReport, BackupError> {
            Err(BackupError::Internal)
        }
    }
}
