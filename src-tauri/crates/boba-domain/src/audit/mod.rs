//! Audit event model and canonical serialisation (AD-12).
//!
//! Pure: the domain fixes the exact bytes that go into the chain — the RFC 8785
//! (JCS) form of every column except `hash`, and the payload
//! `prev_hash ‖ canonical_json(row)`. The hash function itself (blake3) and the
//! rows live in the `boba-store` adapter, which is where IO lives. Keeping the
//! canonical form here means the chain rule is one reviewable, IO-free function.
//!
//! Every dynamic column is TEXT or INTEGER, so JCS reduces to sorted keys, JSON
//! string escaping and integer formatting — no floating point, no nesting.

use std::fmt::Write as _;

/// `prev_hash` of the first row in the chain: 64 zero hex digits.
pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// Which half of an outbound action a row records (AD-12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditPhase {
    /// Written before the action runs.
    Intent,
    /// Written after the action runs.
    Outcome,
}

impl AuditPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Intent => "intent",
            Self::Outcome => "outcome",
        }
    }

    /// Parse the stored form, `None` for an unknown phase.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "intent" => Some(Self::Intent),
            "outcome" => Some(Self::Outcome),
            _ => None,
        }
    }
}

/// The result recorded on an `outcome` row (AD-12, FR-89).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditOutcome {
    /// The action completed.
    Ok,
    /// The action ran and failed.
    Failed,
    /// An `intent` had no `outcome` at startup (crash); the app appended this
    /// row and the attempt goes to Needs attention (FR-89).
    UnknownAfterCrash,
}

impl AuditOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Failed => "failed",
            Self::UnknownAfterCrash => "unknown_after_crash",
        }
    }

    /// Parse the stored form, `None` for an unknown outcome.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "ok" => Some(Self::Ok),
            "failed" => Some(Self::Failed),
            "unknown_after_crash" => Some(Self::UnknownAfterCrash),
            _ => None,
        }
    }
}

/// One `audit_event` row, every column included (AD-12).
///
/// [`AuditRow::canonical_json`] covers every column **except** `hash`, which is
/// the derived value the chain is built from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRow {
    /// Monotonic chain position assigned on insert; `0` before the first write.
    pub seq: i64,
    /// ULID of this row (AD-17).
    pub id: String,
    /// UTC RFC 3339, written from the injected clock (AD-17).
    pub created_at: String,
    pub phase: AuditPhase,
    /// Stable action name, e.g. `apply.submitted`, `email.sent`.
    pub action: String,
    /// `None` on an `intent` row, the result on an `outcome` row.
    pub outcome: Option<AuditOutcome>,
    /// Groups the `intent`/`outcome` pair for one attempt.
    pub correlation_id: String,
    /// On an `outcome` row, the ULID of the `intent` it completes.
    pub intent_id: Option<String>,
    /// Who initiated the action: `core`, `user` or `auto`.
    pub actor: String,
    /// Kind of target, e.g. `source`, `email`, `application`.
    pub target_kind: String,
    pub target_id: Option<String>,
    /// Metadata JSON (IDs, counts, reasons) — never a payload.
    pub detail_json: String,
    /// blake3 of the payload (Package, email body), never the payload (AD-12).
    pub payload_blake3: Option<String>,
    /// `hash` of the previous row, or [`GENESIS_HASH`] for the first row.
    pub prev_hash: String,
    /// `blake3(prev_hash ‖ canonical_json(row))`, hex.
    pub hash: String,
}

impl AuditRow {
    /// RFC 8785 (JCS) canonical JSON over every column except `hash`.
    ///
    /// Keys are emitted in sorted order and all values are strings, integers or
    /// null; this subset needs no number canonicalisation beyond integer digits.
    pub fn canonical_json(&self) -> String {
        let mut j = Jcs::new();
        j.str_field("action", &self.action);
        j.str_field("actor", &self.actor);
        j.str_field("correlation_id", &self.correlation_id);
        j.str_field("created_at", &self.created_at);
        j.str_field("detail_json", &self.detail_json);
        j.str_field("id", &self.id);
        j.opt_str_field("intent_id", self.intent_id.as_deref());
        j.opt_str_field("outcome", self.outcome.map(AuditOutcome::as_str));
        j.opt_str_field("payload_blake3", self.payload_blake3.as_deref());
        j.str_field("phase", self.phase.as_str());
        j.str_field("prev_hash", &self.prev_hash);
        j.int_field("seq", self.seq);
        j.opt_str_field("target_id", self.target_id.as_deref());
        j.str_field("target_kind", &self.target_kind);
        j.finish()
    }

    /// The byte string hashed into `hash`: `prev_hash ‖ canonical_json(row)`.
    pub fn chain_payload(&self) -> Vec<u8> {
        let canonical = self.canonical_json();
        let mut out = Vec::with_capacity(self.prev_hash.len() + canonical.len());
        out.extend_from_slice(self.prev_hash.as_bytes());
        out.extend_from_slice(canonical.as_bytes());
        out
    }
}

/// Outcome of walking the chain (AD-12), reported at startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainStatus {
    /// Number of rows walked.
    pub rows: usize,
    /// `hash` of the last row, `None` for an empty log.
    pub head: Option<String>,
    /// `seq` of the first row whose link or hash is wrong.
    pub broken_at: Option<i64>,
}

impl ChainStatus {
    /// True when every row links and re-hashes correctly.
    pub fn is_intact(&self) -> bool {
        self.broken_at.is_none()
    }
}

/// A minimal JCS writer: sorted keys, JSON escaping, integer digits. All values
/// are pre-encoded; callers emit keys in sorted order.
struct Jcs {
    out: String,
    first: bool,
}

impl Jcs {
    fn new() -> Self {
        Self {
            out: String::from("{"),
            first: true,
        }
    }

    fn key(&mut self, key: &str) {
        if self.first {
            self.first = false;
        } else {
            self.out.push(',');
        }
        write_json_string(&mut self.out, key);
        self.out.push(':');
    }

    fn str_field(&mut self, key: &str, value: &str) {
        self.key(key);
        write_json_string(&mut self.out, value);
    }

    fn opt_str_field(&mut self, key: &str, value: Option<&str>) {
        self.key(key);
        match value {
            Some(v) => write_json_string(&mut self.out, v),
            None => self.out.push_str("null"),
        }
    }

    fn int_field(&mut self, key: &str, value: i64) {
        self.key(key);
        let _ = write!(self.out, "{value}");
    }

    fn finish(mut self) -> String {
        self.out.push('}');
        self.out
    }
}

/// RFC 8785 string escaping: `"` and `\`, the short control escapes, `\u00xx`
/// for the rest of U+0000..U+001F, and nothing else — non-ASCII stays verbatim.
fn write_json_string(out: &mut String, value: &str) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    fn intent_row() -> AuditRow {
        AuditRow {
            seq: 1,
            id: "01HZZZZZZZZZZZZZZZZZZZZZZZ".to_string(),
            created_at: "2026-10-05T01:02:03.000Z".to_string(),
            phase: AuditPhase::Intent,
            action: "apply.submitted".to_string(),
            outcome: None,
            correlation_id: "att_01".to_string(),
            intent_id: None,
            actor: "core".to_string(),
            target_kind: "source".to_string(),
            target_id: Some("src_1".to_string()),
            detail_json: "{}".to_string(),
            payload_blake3: Some("ab".to_string()),
            prev_hash: GENESIS_HASH.to_string(),
            hash: String::new(),
        }
    }

    // AD-12: JCS covers every column except `hash`, in sorted key order.
    #[test]
    fn canonical_json_is_sorted_and_excludes_hash() {
        let row = intent_row();
        let json = row.canonical_json();
        assert_eq!(
            json,
            format!(
                concat!(
                    "{{\"action\":\"apply.submitted\",\"actor\":\"core\",",
                    "\"correlation_id\":\"att_01\",\"created_at\":\"2026-10-05T01:02:03.000Z\",",
                    "\"detail_json\":\"{{}}\",\"id\":\"01HZZZZZZZZZZZZZZZZZZZZZZZ\",",
                    "\"intent_id\":null,\"outcome\":null,\"payload_blake3\":\"ab\",",
                    "\"phase\":\"intent\",\"prev_hash\":\"{genesis}\",\"seq\":1,",
                    "\"target_id\":\"src_1\",\"target_kind\":\"source\"}}"
                ),
                genesis = GENESIS_HASH
            )
        );
    }

    // RFC 8785: shortest escapes, lowercase \u, no escaping of `/`.
    #[test]
    fn canonical_json_escapes_control_chars_like_jcs() {
        let mut out = String::new();
        write_json_string(&mut out, "a\"\\\n\r\t/\u{1}b\u{00e9}");
        assert_eq!(out, "\"a\\\"\\\\\\n\\r\\t/\\u0001b\u{00e9}\"");
    }

    // AD-12: the hash input is prev_hash followed by the canonical JSON.
    #[test]
    fn chain_payload_concatenates_prev_hash_and_canonical() {
        let row = intent_row();
        let mut expected = GENESIS_HASH.as_bytes().to_vec();
        expected.extend_from_slice(row.canonical_json().as_bytes());
        assert_eq!(row.chain_payload(), expected);
    }

    // The hash column is not part of the canonical object, so setting it does
    // not change the payload.
    #[test]
    fn hash_column_is_not_part_of_the_canonical_payload() {
        let mut a = intent_row();
        let mut b = intent_row();
        b.hash = "deadbeef".to_string();
        a.hash = String::new();
        assert_eq!(a.chain_payload(), b.chain_payload());
    }

    #[test]
    fn phase_and_outcome_round_trip() {
        for phase in [AuditPhase::Intent, AuditPhase::Outcome] {
            assert_eq!(AuditPhase::parse(phase.as_str()), Some(phase));
        }
        for outcome in [
            AuditOutcome::Ok,
            AuditOutcome::Failed,
            AuditOutcome::UnknownAfterCrash,
        ] {
            assert_eq!(AuditOutcome::parse(outcome.as_str()), Some(outcome));
        }
        assert_eq!(AuditOutcome::parse("nope"), None);
    }
}
