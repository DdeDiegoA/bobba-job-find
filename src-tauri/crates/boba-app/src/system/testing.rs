//! In-memory `SystemRepo` for use-case tests (test-only).
//!
//! It mirrors the chain contract with a non-cryptographic hash: enough to
//! exercise the app wiring. The real blake3 chain is tested against SQLite in
//! `boba-store`.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use boba_domain::audit::{AuditOutcome, AuditPhase, AuditRow, ChainStatus, GENESIS_HASH};

use crate::ports::repo::system::{NewIntent, SettingRecord, SystemRepo, SystemRepoError};

/// An in-memory `SystemRepo` (settings + audit) for tests.
#[derive(Default)]
pub struct FakeSystemRepo {
    settings: Mutex<BTreeMap<String, SettingRecord>>,
    rows: Mutex<Vec<AuditRow>>,
    anchor: Mutex<Option<String>>,
}

impl FakeSystemRepo {
    /// Seed a stored setting row.
    pub fn seed_setting(&mut self, key: &str, value: &str) {
        self.settings.get_mut().unwrap().insert(
            key.to_string(),
            SettingRecord {
                key: key.to_string(),
                value: value.to_string(),
                updated_at: "seed".to_string(),
            },
        );
    }

    /// Force the keychain anchor to a value (to simulate a mismatch).
    pub fn set_anchor(&self, hash: &str) {
        *self.anchor.lock().unwrap() = Some(hash.to_string());
    }

    /// Every audit row, in append order (a use case's own event inspection).
    pub fn rows(&self) -> Vec<AuditRow> {
        self.rows.lock().unwrap().clone()
    }

    /// Drop the last row, simulating a tail truncation by an attacker.
    pub fn truncate_last(&self) {
        self.rows.lock().unwrap().pop();
    }

    /// Change the last row without re-hashing it, to simulate tampering.
    pub fn corrupt_last(&self) {
        if let Some(last) = self.rows.lock().unwrap().last_mut() {
            last.action.push_str(".evil");
        }
    }

    fn append(&self, mut row: AuditRow) -> AuditRow {
        let mut rows = self.rows.lock().unwrap();
        row.seq = rows.len() as i64 + 1;
        row.prev_hash = rows
            .last()
            .map(|r| r.hash.clone())
            .unwrap_or_else(|| GENESIS_HASH.to_string());
        row.hash = fake_hash(&row.canonical_json());
        rows.push(row.clone());
        // AD-12: the keychain anchor tracks the head on every append.
        *self.anchor.lock().unwrap() = Some(row.hash.clone());
        row
    }

    fn walk(&self) -> ChainStatus {
        let rows = self.rows.lock().unwrap();
        let mut prev = GENESIS_HASH.to_string();
        let mut broken_at = None;
        for row in rows.iter() {
            if row.prev_hash != prev || row.hash != fake_hash(&row.canonical_json()) {
                broken_at = Some(row.seq);
                break;
            }
            prev.clone_from(&row.hash);
        }
        ChainStatus {
            rows: rows.len(),
            head: rows.last().map(|r| r.hash.clone()),
            broken_at,
        }
    }
}

fn fake_hash(canonical: &str) -> String {
    // FNV-1a: deterministic and cheap; the SQLite tests cover the real blake3.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in canonical.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

fn fake_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!("fake{:026}", NEXT.fetch_add(1, Ordering::Relaxed))
}

impl SystemRepo for FakeSystemRepo {
    fn setting_get(&self, key: &str) -> Result<Option<String>, SystemRepoError> {
        Ok(self
            .settings
            .lock()
            .unwrap()
            .get(key)
            .map(|r| r.value.clone()))
    }

    fn setting_put(&self, key: &str, value: &str, updated_at: &str) -> Result<(), SystemRepoError> {
        self.settings.lock().unwrap().insert(
            key.to_string(),
            SettingRecord {
                key: key.to_string(),
                value: value.to_string(),
                updated_at: updated_at.to_string(),
            },
        );
        Ok(())
    }

    fn setting_list(&self) -> Result<Vec<SettingRecord>, SystemRepoError> {
        Ok(self.settings.lock().unwrap().values().cloned().collect())
    }

    fn setting_delete(&self, key: &str) -> Result<bool, SystemRepoError> {
        Ok(self.settings.lock().unwrap().remove(key).is_some())
    }

    fn audit_append_intent(&self, intent: &NewIntent) -> Result<AuditRow, SystemRepoError> {
        Ok(self.append(AuditRow {
            seq: 0,
            id: fake_id(),
            created_at: intent.created_at.clone(),
            phase: AuditPhase::Intent,
            action: intent.action.clone(),
            outcome: None,
            correlation_id: intent.correlation_id.clone(),
            intent_id: None,
            actor: intent.actor.clone(),
            target_kind: intent.target_kind.clone(),
            target_id: intent.target_id.clone(),
            detail_json: intent.detail_json.clone(),
            payload_blake3: intent.payload_blake3.clone(),
            prev_hash: String::new(),
            hash: String::new(),
        }))
    }

    fn audit_append_outcome(
        &self,
        intent: &AuditRow,
        outcome: AuditOutcome,
        detail_json: &str,
        created_at: &str,
    ) -> Result<AuditRow, SystemRepoError> {
        Ok(self.append(AuditRow {
            seq: 0,
            id: fake_id(),
            created_at: created_at.to_string(),
            phase: AuditPhase::Outcome,
            action: intent.action.clone(),
            outcome: Some(outcome),
            correlation_id: intent.correlation_id.clone(),
            intent_id: Some(intent.id.clone()),
            actor: intent.actor.clone(),
            target_kind: intent.target_kind.clone(),
            target_id: intent.target_id.clone(),
            detail_json: detail_json.to_string(),
            payload_blake3: None,
            prev_hash: String::new(),
            hash: String::new(),
        }))
    }

    fn audit_head_hash(&self) -> Result<Option<String>, SystemRepoError> {
        Ok(self.rows.lock().unwrap().last().map(|r| r.hash.clone()))
    }

    fn audit_verify_chain(&self) -> Result<ChainStatus, SystemRepoError> {
        Ok(self.walk())
    }

    fn audit_unresolved_intents(&self) -> Result<Vec<AuditRow>, SystemRepoError> {
        let rows = self.rows.lock().unwrap();
        Ok(rows
            .iter()
            .filter(|r| {
                r.phase == AuditPhase::Intent
                    && !rows.iter().any(|o| {
                        o.phase == AuditPhase::Outcome
                            && o.intent_id.as_deref() == Some(r.id.as_str())
                    })
            })
            .cloned()
            .collect())
    }

    fn audit_anchor_get(&self) -> Result<Option<String>, SystemRepoError> {
        Ok(self.anchor.lock().unwrap().clone())
    }

    fn audit_anchor_is_known(&self, hash: &str) -> Result<bool, SystemRepoError> {
        Ok(self.rows.lock().unwrap().iter().any(|r| r.hash == hash))
    }

    fn audit_anchor_set(&self, hash: &str) -> Result<(), SystemRepoError> {
        *self.anchor.lock().unwrap() = Some(hash.to_string());
        Ok(())
    }
}
