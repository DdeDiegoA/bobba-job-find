/**
 * The Audit Log's labels (S9.9). The core sends stable codes (`send`,
 * `apply.submitted`, `ok`), and every catalog key is named here explicitly so
 * the i18n gate can see it. An unknown code falls back to the code itself —
 * never to a raw i18n key.
 */
type Translate = (key: string) => string;

export function kindLabel(t: Translate, kind: string): string {
  switch (kind) {
    case "send":
      return t("audit.kind.send");
    case "decision":
      return t("audit.kind.decision");
    case "kill_switch":
      return t("audit.kind.kill_switch");
    case "optin":
      return t("audit.kind.optin");
    case "stage_change":
      return t("audit.kind.stage_change");
    default:
      return t("audit.kind.other");
  }
}

export function eventLabel(t: Translate, action: string): string {
  switch (action) {
    case "apply.submitted":
      return t("audit.event.apply.submitted");
    case "apply.ats_submitted":
      return t("audit.event.apply.ats_submitted");
    case "apply.session_submitted":
      return t("audit.event.apply.session_submitted");
    case "apply.kill_switch":
      return t("audit.event.apply.kill_switch");
    case "auto.decision":
      return t("audit.event.auto.decision");
    case "source.optin":
      return t("audit.event.source.optin");
    case "session.run":
      return t("audit.event.session.run");
    case "stage.saved":
      return t("audit.event.stage.saved");
    case "stage.preparing":
      return t("audit.event.stage.preparing");
    case "stage.applied":
      return t("audit.event.stage.applied");
    case "stage.interview":
      return t("audit.event.stage.interview");
    case "stage.offer":
      return t("audit.event.stage.offer");
    case "stage.hired":
      return t("audit.event.stage.hired");
    case "stage.rejected":
      return t("audit.event.stage.rejected");
    case "stage.no_response":
      return t("audit.event.stage.no_response");
    case "stage.offer_declined":
      return t("audit.event.stage.offer_declined");
    case "stage.withdrawn":
      return t("audit.event.stage.withdrawn");
    default:
      return action;
  }
}

export function outcomeLabel(t: Translate, outcome: string): string {
  switch (outcome) {
    case "ok":
      return t("audit.outcome.ok");
    case "failed":
      return t("audit.outcome.failed");
    case "unknown_after_crash":
      return t("audit.outcome.unknown_after_crash");
    default:
      return outcome;
  }
}
