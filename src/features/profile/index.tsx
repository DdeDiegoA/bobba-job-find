import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import type { FactView } from "@/lib/ipc/bindings";
import { isAppError, renderAppError } from "@/lib/ipc/errors";
import { cn } from "@/lib/utils";
import { FactForm } from "./fact-form";
import { GapsEntry } from "./gaps-entry";
import { FactHistory } from "./history";
import { PROFILE_SECTIONS } from "./sections";
import { useProfile, useSaveFact } from "./use-profile";

/**
 * Profile — facts with EN/ES variants and a version history (S3.4, EXPERIENCE
 * §9.10). Nothing here rewrites a stored fact: every save appends a revision
 * (AC3), and a fact missing a language is flagged, never filled in (AC2).
 */
export default function ProfileIndex() {
  const { t } = useTranslation("profile");
  // Errors carry keys from the `common` catalog, facts use the `profile` one.
  const { t: tError } = useTranslation();
  const profile = useProfile();
  const save = useSaveFact();
  const [section, setSection] = useState("experience");
  const [form, setForm] = useState<{ fact: FactView | null } | null>(null);
  const [historyItem, setHistoryItem] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);

  const facts = profile.data?.facts ?? [];
  const counts = Object.fromEntries(
    PROFILE_SECTIONS.map((entry) => [
      entry.kind,
      facts.filter((fact) => fact.kind === entry.kind).length,
    ]),
  );
  const sectionLabel = t(
    PROFILE_SECTIONS.find((entry) => entry.kind === section)?.labelKey ?? "sections.experience",
  );
  const sectionFacts = facts.filter((fact) => fact.kind === section);

  return (
    <section className="flex min-h-0 flex-1 gap-4 p-6" aria-labelledby="profile-title">
      <div className="flex w-56 shrink-0 flex-col gap-2">
        <h1 id="profile-title" className="font-display text-2xl text-fg">
          {t("title")}
        </h1>
        <nav aria-label={t("sections.aria")} className="flex flex-col gap-1">
          {PROFILE_SECTIONS.map((entry) => (
            <button
              key={entry.kind}
              type="button"
              aria-current={section === entry.kind ? "true" : undefined}
              onClick={() => {
                setSection(entry.kind);
                setForm(null);
              }}
              className={cn(
                "flex items-center justify-between rounded-xl px-3 py-2 text-left text-sm",
                section === entry.kind
                  ? "bg-primary-soft text-on-primary-soft"
                  : "text-fg-muted hover:bg-surface-muted",
              )}
            >
              <span>{t(entry.labelKey)}</span>
              <span className="text-xs text-fg-subtle">{counts[entry.kind] ?? 0}</span>
            </button>
          ))}
        </nav>
      </div>

      <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-4 overflow-y-auto">
        <div className="flex items-center justify-between gap-2">
          <div className="flex items-center gap-2">
            <h2 className="font-display text-xl text-fg">{sectionLabel}</h2>
            {profile.data ? (
              <span
                className="rounded-xl bg-surface-muted px-2 py-1 text-xs text-fg-muted"
                title={t("version.hint")}
              >
                {t("version", { version: profile.data.profile_version })}
              </span>
            ) : null}
          </div>
          <Button
            size="sm"
            onClick={() => {
              setSaveError(null);
              setForm({ fact: null });
            }}
          >
            {t("fact.add", { section: sectionLabel })}
          </Button>
        </div>

        {/* FX9: re-run the gap interview without going back to onboarding. */}
        <GapsEntry />

        {profile.isPending ? <p className="text-sm text-fg-muted">{t("loading")}</p> : null}
        {profile.isError ? (
          <p className="text-sm text-danger">
            {isAppError(profile.error) ? renderAppError(profile.error, tError) : t("error.load")}
          </p>
        ) : null}

        {/* AC4: facts an agent proposed wait here for the user (S3.5 review). */}
        {profile.data && profile.data.pending_proposals > 0 ? (
          <p className="rounded-xl bg-surface-muted px-3 py-2 text-sm text-fg-muted">
            {t("proposals.pending", { count: profile.data.pending_proposals })}
          </p>
        ) : null}

        {form ? (
          <FactForm
            key={form.fact?.item_id ?? `new-${section}`}
            kind={section}
            label={sectionLabel}
            fact={form.fact}
            saving={save.isPending}
            error={saveError}
            onCancel={() => setForm(null)}
            onSave={(input) => {
              setSaveError(null);
              save.mutate(input, {
                onSuccess: () => setForm(null),
                onError: (error) =>
                  setSaveError(isAppError(error) ? renderAppError(error, tError) : t("error.load")),
              });
            }}
          />
        ) : null}

        {historyItem ? (
          <FactHistory
            itemId={historyItem}
            label={sectionLabel}
            onClose={() => setHistoryItem(null)}
          />
        ) : null}

        {sectionFacts.length === 0 && !form ? (
          <EmptyState title={t("empty", { section: sectionLabel })} />
        ) : null}

        <ul className="flex flex-col gap-2">
          {sectionFacts.map((fact) => (
            <li
              key={fact.revision_id}
              className="flex flex-col gap-2 rounded-2xl border border-border bg-surface p-4"
            >
              <p className="text-sm text-fg">{fact.text_en ?? fact.text_es}</p>
              {fact.text_en && fact.text_es ? (
                <p className="text-xs text-fg-subtle">{fact.text_es}</p>
              ) : null}
              {fact.evidence ? (
                <p className="text-xs text-fg-subtle">
                  {t("fact.evidence", { evidence: fact.evidence })}
                </p>
              ) : null}
              {/* AC2: the missing variant is flagged, never invented. */}
              {fact.missing_variant ? (
                <p className="text-xs text-warning">
                  {t(`profile:fact.missing.${fact.missing_variant}`)}
                </p>
              ) : null}
              <div className="flex items-center gap-3">
                <button
                  type="button"
                  className="text-xs text-fg-muted underline"
                  onClick={() => {
                    setSaveError(null);
                    setForm({ fact });
                  }}
                >
                  {t("fact.edit", { section: sectionLabel })}
                </button>
                <button
                  type="button"
                  className="text-xs text-fg-muted underline"
                  onClick={() => setHistoryItem(fact.item_id)}
                >
                  {t("history")}
                </button>
              </div>
            </li>
          ))}
        </ul>
      </div>
    </section>
  );
}
