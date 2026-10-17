import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import type { DryRunView, OutboxItemView } from "@/lib/ipc/bindings";
import { isAppError, renderAppError } from "@/lib/ipc/errors";
import {
  useActivateRule,
  useCancelItem,
  useEditItem,
  useOutbox,
  useSetDryRun,
  useSetHold,
} from "./use-outbox";

/** The note a stored code names, or the code itself when this build has none. */
function noteText(code: string, t: (key: string) => string): string {
  if (code.startsWith("review:")) {
    return t("note.review");
  }
  const text = t(`outbox:note.${code}`);
  return text === `note.${code}` ? code : text;
}

/** One queued item: what it is, how long it waits, and what the user may do. */
function Item({
  item,
  editing,
  onEdit,
  onCancel,
}: {
  item: OutboxItemView;
  editing: boolean;
  onEdit: () => void;
  onCancel: () => void;
}) {
  const { t } = useTranslation("outbox");
  const state =
    item.state === "held"
      ? item.minutes_left > 0
        ? t("state.held", { minutes: item.minutes_left })
        : t("state.due")
      : t(`outbox:state.${item.state}`);
  const waiting = item.state === "held" || item.state === "would_send";

  return (
    <li
      className="flex flex-col gap-2 rounded-2xl border border-border bg-surface p-4"
      data-testid={`outbox-item-${item.id}`}
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex flex-col">
          <p className="font-medium text-fg">
            {item.title} · {item.company}
          </p>
          <p className="text-sm text-fg-muted">
            {t(`outbox:channel.${item.channel}`)} · {t("item.rule", { name: item.rule_name })}
          </p>
        </div>
        <span className="rounded-xl bg-surface-muted px-2 py-1 text-xs text-fg-muted">{state}</span>
      </div>
      {item.note ? <p className="text-sm text-fg-muted">{noteText(item.note, t)}</p> : null}
      {waiting ? (
        <div className="flex gap-2">
          <Button type="button" variant="outline" size="sm" onClick={onEdit}>
            {t("item.edit")}
          </Button>
          <Button type="button" variant="ghost" size="sm" onClick={onCancel}>
            {t("item.cancel")}
          </Button>
        </div>
      ) : null}
      {editing ? <EditForm item={item} onClose={onEdit} /> : null}
    </li>
  );
}

/** The inline editor: only what the user changes travels with the hold. */
function EditForm({ item, onClose }: { item: OutboxItemView; onClose: () => void }) {
  const { t } = useTranslation("outbox");
  const { t: tError } = useTranslation();
  const edit = useEditItem();
  const [subject, setSubject] = useState(item.subject_override ?? "");
  const [body, setBody] = useState(item.body_override ?? "");

  return (
    <form
      className="flex flex-col gap-2 border-t border-border pt-2"
      onSubmit={(event) => {
        event.preventDefault();
        edit.mutate({ id: item.id, subject: subject.trim() || null, body: body.trim() || null });
      }}
    >
      <p className="font-medium text-fg">{t("edit.title")}</p>
      <label className="flex flex-col gap-1 text-sm text-fg">
        {t("edit.subject")}
        <input
          className="rounded-xl border border-border bg-surface px-3 py-2 text-fg"
          value={subject}
          onChange={(event) => setSubject(event.target.value)}
        />
      </label>
      <label className="flex flex-col gap-1 text-sm text-fg">
        {t("edit.body")}
        <textarea
          className="min-h-24 rounded-xl border border-border bg-surface px-3 py-2 text-fg"
          value={body}
          onChange={(event) => setBody(event.target.value)}
        />
      </label>
      {edit.isError ? (
        <p className="text-sm text-danger">
          {isAppError(edit.error) ? renderAppError(edit.error, tError) : t("edit.error")}
        </p>
      ) : null}
      {edit.isSuccess ? <p className="text-sm text-fg-muted">{t("edit.saved")}</p> : null}
      <div className="flex gap-2">
        <Button type="submit" size="sm" disabled={edit.isPending}>
          {t("edit.save")}
        </Button>
        <Button type="button" variant="ghost" size="sm" onClick={onClose}>
          {t("edit.cancel")}
        </Button>
      </div>
    </form>
  );
}

/** One rule's dry run: progress, the length the user can change, activation. */
function DryRun({ rule }: { rule: DryRunView }) {
  const { t } = useTranslation("outbox");
  const activate = useActivateRule();
  const setDryRun = useSetDryRun();
  const [matches, setMatches] = useState(rule.required);

  return (
    <li
      className="flex flex-col gap-2 rounded-2xl border border-border p-4"
      data-testid={`outbox-dry-run-${rule.rule_id}`}
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <p className="font-medium text-fg">{rule.rule_name}</p>
        {rule.active ? (
          <span className="rounded-xl bg-primary-soft px-2 py-1 text-xs text-on-primary-soft">
            {t("dry_run.active")}
          </span>
        ) : (
          <span className="text-sm text-fg-muted">
            {t("dry_run.progress", { seen: rule.seen, required: rule.required })}
          </span>
        )}
      </div>
      {rule.active ? null : (
        <>
          {rule.complete ? <p className="text-sm text-fg-muted">{t("dry_run.complete")}</p> : null}
          <div className="flex flex-wrap items-end gap-2">
            <label className="flex flex-col gap-1 text-sm text-fg">
              {t("dry_run.hint")}
              <select
                className="rounded-xl border border-border bg-surface px-3 py-2 text-fg"
                value={matches}
                onChange={(event) => {
                  const next = Number(event.target.value);
                  setMatches(next);
                  setDryRun.mutate({ ruleId: rule.rule_id, matches: next });
                }}
              >
                {Array.from({ length: 10 }, (_, index) => index + 1).map((value) => (
                  <option key={value} value={value}>
                    {value}
                  </option>
                ))}
              </select>
            </label>
            <Button
              type="button"
              size="sm"
              disabled={activate.isPending}
              onClick={() => activate.mutate(rule.rule_id)}
            >
              {t("dry_run.activate")}
            </Button>
          </div>
        </>
      )}
      {activate.isError ? <p className="text-sm text-danger">{t("dry_run.error")}</p> : null}
    </li>
  );
}

/**
 * Today ▸ Outbox (S9.7, FR-84/FR-85, EXPERIENCE §9.9). The panel lists what
 * Auto queued, lets the user edit or cancel an item during its hold, shows the
 * hold setting, and reports each new rule's dry run until the user activates it.
 */
export default function TodayOutbox() {
  const { t } = useTranslation("outbox");
  const outbox = useOutbox();
  const setHold = useSetHold();
  const cancel = useCancelItem();
  const [editing, setEditing] = useState<string | null>(null);
  const [holdDraft, setHoldDraft] = useState<string | null>(null);

  const view = outbox.data;
  const hold = holdDraft ?? String(view?.settings.hold_minutes ?? 30);

  return (
    <section className="flex flex-col gap-4" aria-labelledby="today-outbox-title">
      <div className="flex flex-col gap-1">
        <h2 id="today-outbox-title" className="font-display text-xl text-fg">
          {t("title")}
        </h2>
        <p className="text-sm text-fg-muted">{t("subtitle")}</p>
      </div>

      {outbox.isPending ? <p className="text-sm text-fg-muted">{t("loading")}</p> : null}
      {outbox.isError ? <p className="text-sm text-danger">{t("error.load")}</p> : null}

      {view ? (
        <>
          <h3 className="font-medium text-fg">{t("hold.title")}</h3>
          <form
            className="flex flex-wrap items-end gap-2"
            onSubmit={(event) => {
              event.preventDefault();
              setHold.mutate(Number(hold));
            }}
          >
            <label className="flex flex-col gap-1 text-sm text-fg">
              {t("hold.label")}
              <input
                type="number"
                min={5}
                max={1440}
                className="w-28 rounded-xl border border-border bg-surface px-3 py-2 text-fg"
                value={hold}
                onChange={(event) => setHoldDraft(event.target.value)}
              />
            </label>
            <Button type="submit" variant="outline" size="sm" disabled={setHold.isPending}>
              {t("hold.save")}
            </Button>
            <p className="text-xs text-fg-subtle">{t("hold.hint")}</p>
          </form>
          {setHold.isError ? <p className="text-sm text-danger">{t("hold.error")}</p> : null}
          {setHold.isSuccess ? <p className="text-sm text-fg-muted">{t("hold.saved")}</p> : null}

          {view.items.length === 0 ? (
            <EmptyState title={t("empty.title")} description={t("empty.body")} />
          ) : (
            <ul className="flex flex-col gap-2">
              {view.items.map((item) => (
                <Item
                  key={item.id}
                  item={item}
                  editing={editing === item.id}
                  onEdit={() => setEditing(editing === item.id ? null : item.id)}
                  onCancel={() => {
                    setEditing(null);
                    cancel.mutate(item.id);
                  }}
                />
              ))}
            </ul>
          )}

          {view.rules.length > 0 ? (
            <section className="flex flex-col gap-2" aria-labelledby="today-outbox-dry-run-title">
              <h3 id="today-outbox-dry-run-title" className="font-medium text-fg">
                {t("dry_run.title")}
              </h3>
              <ul className="flex flex-col gap-2">
                {view.rules.map((rule) => (
                  <DryRun key={rule.rule_id} rule={rule} />
                ))}
              </ul>
            </section>
          ) : null}
        </>
      ) : null}
    </section>
  );
}
