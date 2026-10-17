//! Follow-up nudge sweep (S9.5, FR-95, S5.3 quiet hours).

use std::sync::{Arc, Mutex};

use boba_domain::application::{AppStatus, Application, StatusEvent, StatusSource};

use super::*;
use crate::applications::testing::FakeApplyRepo;
use crate::digest::settings::{NotificationSettings, SETTINGS_KEY};
use crate::ports::notifier::{NotifierError, NotifierPort};
use crate::system::FixedClock;
use crate::system::testing::FakeSystemRepo;

const NOW: &str = "2026-10-17T12:00:00Z";

/// A notifier that keeps what the sweep asked to show.
#[derive(Default)]
struct RecordingNotifier {
    shown: Mutex<Vec<Notification>>,
}

impl RecordingNotifier {
    fn shown(&self) -> Vec<Notification> {
        self.shown.lock().unwrap().clone()
    }
}

impl NotifierPort for RecordingNotifier {
    fn notify(&self, notification: Notification) -> Result<(), NotifierError> {
        self.shown.lock().unwrap().push(notification);
        Ok(())
    }
}

fn days_ago(days: i64) -> String {
    let now = OffsetDateTime::parse(NOW, &Rfc3339).expect("valid now");
    (now - time::Duration::days(days))
        .format(&Rfc3339)
        .expect("format")
}

fn application(id: &str, status: AppStatus) -> Application {
    Application {
        id: id.to_string(),
        posting_id: format!("pst_{id}"),
        status,
        offer_snapshot_json: serde_json::json!({
            "title": "Backend Engineer",
            "company": "Nimbus",
        })
        .to_string(),
        notes: None,
        next_action: None,
        created_at: days_ago(40),
        updated_at: days_ago(40),
    }
}

fn applied_event(id: &str, days: i64) -> StatusEvent {
    StatusEvent {
        id: format!("evt_{id}_{days}"),
        application_id: id.to_string(),
        from_status: Some(AppStatus::Drafted),
        to_status: AppStatus::Applied,
        source: StatusSource::User,
        reason: None,
        created_at: days_ago(days),
    }
}

struct Harness {
    system: Arc<FakeSystemRepo>,
    notifier: Arc<RecordingNotifier>,
    service: NudgesService,
}

fn harness(applied_days: Option<i64>, status: AppStatus) -> Harness {
    let apply = Arc::new(FakeApplyRepo::default());
    apply.seed_application(application("app_01", AppStatus::Drafted));
    if let Some(days) = applied_days {
        apply
            .append_status_event(&applied_event("app_01", days))
            .unwrap();
    }
    // The row as the sweep finds it now, after any later moves in the history.
    apply.set_status("app_01", status);
    let system = Arc::new(FakeSystemRepo::default());
    let notifier = Arc::new(RecordingNotifier::default());
    let service = NudgesService::new(
        apply.clone(),
        system.clone(),
        Arc::new(FixedClock::new(NOW)),
        Some(notifier.clone()),
    );
    Harness {
        system,
        notifier,
        service,
    }
}

#[test]
fn a_card_inside_the_first_ten_days_stays_quiet() {
    let h = harness(Some(CHECK_IN_DAYS - 1), AppStatus::Applied);
    assert!(h.service.sweep(10).unwrap().is_empty());
    assert!(h.notifier.shown().is_empty());
}

#[test]
fn ten_quiet_days_send_one_check_in_and_never_twice() {
    let h = harness(Some(CHECK_IN_DAYS), AppStatus::Applied);

    let sent = h.service.sweep(10).unwrap();

    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].kind, NudgeKind::CheckIn);
    assert_eq!(sent[0].days, 10);
    assert_eq!(
        h.notifier.shown(),
        vec![Notification::FollowUpNudge {
            title: "Backend Engineer".into(),
            company: "Nimbus".into(),
            days: 10,
            no_response: false,
        }]
    );
    // The next sweep has nothing new to say about this card (FR-95).
    assert!(h.service.sweep(10).unwrap().is_empty());
    assert_eq!(h.notifier.shown().len(), 1);
}

#[test]
fn thirty_days_suggest_no_response_even_after_the_check_in() {
    let h = harness(Some(NO_RESPONSE_DAYS), AppStatus::Applied);
    h.system
        .setting_put(&format!("{SENT_KEY_PREFIX}app_01.check_in"), NOW, NOW)
        .unwrap();

    let sent = h.service.sweep(10).unwrap();

    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].kind, NudgeKind::NoResponse);
    assert_eq!(
        h.notifier.shown()[0],
        Notification::FollowUpNudge {
            title: "Backend Engineer".into(),
            company: "Nimbus".into(),
            days: NO_RESPONSE_DAYS as u32,
            no_response: true,
        }
    );
}

#[test]
fn a_card_that_left_applied_is_never_nudged() {
    let h = harness(Some(20), AppStatus::Interview);
    assert!(h.service.sweep(10).unwrap().is_empty());
    assert!(h.notifier.shown().is_empty());
}

#[test]
fn quiet_hours_hold_the_nudge_without_consuming_it() {
    let h = harness(Some(CHECK_IN_DAYS), AppStatus::Applied);
    let mut night = NotificationSettings::default();
    night.quiet_hours.enabled = true;
    night.quiet_hours.from = 22;
    night.quiet_hours.to = 7;
    h.system
        .setting_put(SETTINGS_KEY, &serde_json::to_string(&night).unwrap(), NOW)
        .unwrap();

    // 23:00 is inside the window: nothing is sent and nothing is marked.
    assert!(h.service.sweep(23).unwrap().is_empty());
    assert!(h.notifier.shown().is_empty());

    // Morning: the same nudge lands.
    assert_eq!(h.service.sweep(9).unwrap().len(), 1);
    assert_eq!(h.notifier.shown().len(), 1);
}

#[test]
fn a_silenced_digest_silences_the_nudge() {
    let h = harness(Some(CHECK_IN_DAYS), AppStatus::Applied);
    let mut silent = NotificationSettings::default();
    silent.events.digest = false;
    h.system
        .setting_put(SETTINGS_KEY, &serde_json::to_string(&silent).unwrap(), NOW)
        .unwrap();

    assert!(h.service.sweep(10).unwrap().is_empty());
    assert!(h.notifier.shown().is_empty());
}
