import { type FormEvent, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { formatMoney } from "@/lib/format";
import type { ManualOffer } from "@/lib/ipc/bindings";
import { commands } from "@/lib/ipc/bindings";
import { isAppError, renderAppError, unwrap } from "@/lib/ipc/errors";
import { cn } from "@/lib/utils";

/**
 * Manual Add (S4.7, FR-25): the supported way in for a job found where
 * automation is not allowed — LinkedIn, Indeed, a company page. A pasted link is
 * fetched (robots rules apply, AC1) or the text is pasted directly; both end as
 * one Offer with Source **Manual** (AC2).
 *
 * The offer's own text is untrusted content (AD-7): the core stores it
 * sanitised, and nothing on this screen renders it as markup.
 */

/** The backend keys that mean "this page could not be fetched" (AC1). */
const FETCH_FAILED_KEY = "errors.manual.fetch_failed";

/** Wording held in data, as resolvable keys (AD-21): a rename fails the build. */
const WORK_MODE_LABELS: Record<string, string> = {
  remote: "manual_add:work_mode.remote",
  hybrid: "manual_add:work_mode.hybrid",
  onsite: "manual_add:work_mode.onsite",
  unknown: "manual_add:work_mode.unknown",
};

const PAY_LABELS: Record<string, string> = {
  year: "manual_add:salary.per_year",
  month: "manual_add:salary.per_month",
  hour: "manual_add:salary.per_hour",
};

type Mode = "url" | "text";

export default function InboxManualAdd() {
  const { t, i18n } = useTranslation("manual_add");
  // Backend errors carry keys from the `common` catalog.
  const { t: tError } = useTranslation();
  const [mode, setMode] = useState<Mode>("url");
  const [url, setUrl] = useState("");
  const [text, setText] = useState("");
  const [offer, setOffer] = useState<ManualOffer | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);

  const value = mode === "url" ? url : text;

  async function submit(event: FormEvent) {
    event.preventDefault();
    setError(null);
    setPending(true);
    try {
      const result =
        mode === "url"
          ? await commands.ingestManualUrl(url)
          : await commands.ingestManualText(text);
      setOffer(unwrap(result));
    } catch (cause) {
      setError(isAppError(cause) ? renderAppError(cause, tError) : tError("errors.unknown"));
      // AC1: when a page cannot be fetched, offer the way that always works.
      if (isAppError(cause) && cause.message_key === FETCH_FAILED_KEY) {
        setMode("text");
      }
    } finally {
      setPending(false);
    }
  }

  return (
    <section className="flex flex-col gap-4 p-6" aria-labelledby="manual-add-title">
      <header className="flex flex-col gap-1">
        <h1 id="manual-add-title" className="font-display text-2xl text-fg">
          {t("title")}
        </h1>
        <p className="max-w-prose text-sm text-fg-muted">{t("hint")}</p>
      </header>

      <form className="flex max-w-2xl flex-col gap-3" onSubmit={submit}>
        <fieldset className="flex flex-wrap gap-2">
          <legend className="mb-1 text-sm text-fg-muted">{t("mode.aria")}</legend>
          {(["url", "text"] as const).map((option) => (
            <label
              key={option}
              className={cn(
                "flex cursor-pointer items-center gap-2 rounded-xl border border-border bg-surface px-3 py-1.5 text-sm text-fg",
                "has-checked:border-primary has-checked:bg-primary-soft",
              )}
            >
              <input
                type="radio"
                name="manual-add-mode"
                checked={mode === option}
                onChange={() => setMode(option)}
              />
              {option === "url" ? t("mode.url") : t("mode.text")}
            </label>
          ))}
        </fieldset>

        {mode === "url" ? (
          <label className="flex flex-col gap-1 text-sm text-fg">
            {t("url.label")}
            <input
              type="url"
              name="url"
              value={url}
              onChange={(event) => setUrl(event.target.value)}
              placeholder={t("url.placeholder")}
              autoComplete="off"
              className="h-10 rounded-xl border border-border bg-surface px-3 text-fg"
            />
          </label>
        ) : (
          <label className="flex flex-col gap-1 text-sm text-fg">
            {t("text.label")}
            <textarea
              name="text"
              value={text}
              onChange={(event) => setText(event.target.value)}
              placeholder={t("text.placeholder")}
              rows={10}
              className="rounded-xl border border-border bg-surface p-3 text-fg"
            />
          </label>
        )}

        <div className="flex items-center gap-3">
          <Button type="submit" disabled={pending || value.trim().length === 0}>
            {pending ? t("submitting") : mode === "url" ? t("submit.url") : t("submit.text")}
          </Button>
          <p className="text-xs text-fg-subtle">{t("fetch.hint")}</p>
        </div>
      </form>

      {error ? (
        <p role="alert" className="max-w-prose text-sm text-danger">
          {error}
        </p>
      ) : null}

      {offer ? <OfferCard offer={offer} language={i18n.language} /> : null}
    </section>
  );
}

function OfferCard({ offer, language }: { offer: ManualOffer; language: string }) {
  const { t } = useTranslation("manual_add");
  const workMode = WORK_MODE_LABELS[offer.work_mode] ?? WORK_MODE_LABELS.unknown;
  const pay = offer.pay_period ? PAY_LABELS[offer.pay_period] : undefined;
  // formatMoney carries the currency's ISO-4217 exponent (AD-17), so KWD/BHD
  // keep their three decimals (CH17).
  const salary =
    offer.amount_minor !== null && offer.currency
      ? formatMoney(offer.amount_minor, offer.currency, language)
      : null;

  return (
    <article
      aria-label={t("result.title")}
      className="flex max-w-2xl flex-col gap-1 rounded-xl border border-border bg-surface p-4"
    >
      <h2 className="font-display text-lg text-fg">{offer.title || t("result.untitled")}</h2>
      <p className="text-sm text-fg-muted">{offer.company || t("result.company_unknown")}</p>
      <p className="text-sm text-fg-muted">{t(workMode)}</p>
      {offer.locations.length > 0 ? (
        <p className="text-sm text-fg-muted">{offer.locations.join(" · ")}</p>
      ) : null}
      {salary ? (
        <p className="text-sm text-fg">
          {salary}
          {pay ? ` ${t(pay)}` : ""}
        </p>
      ) : null}
      <p className="text-sm text-fg">{t("result.source", { source: offer.source_name })}</p>
      <p className="text-xs text-fg-subtle">{t("result.ranking")}</p>
    </article>
  );
}
