import { openUrl } from "@tauri-apps/plugin-opener";
import { AlertTriangle, Check, CheckCircle2, ExternalLink, X, XCircle } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import {
  ageDays,
  CHANNEL_LABELS,
  type Detail,
  FACTOR_LABELS,
  factorGroups,
  GATE_LABELS,
  PAY_PERIOD_LABELS,
  payAmount,
  WORK_MODE_LABELS,
  withoutRemoteContent,
} from "./detail";

/**
 * The sections of the Offer detail (FR-65, EXPERIENCE §9.4).
 *
 * Every piece of posting text is rendered as **text** (AD-7/ADR-010): quotes are
 * in `<p>`s, the narration is a paragraph, and the description is the core's
 * sanitiser output with the remote-content strip applied once more.
 */

/** The five Gates, first on the page, each with its verdict and quote (AC1). */
export function Gates({ detail }: { detail: Detail }) {
  const { t } = useTranslation("offer");
  return (
    <section className="flex flex-col gap-2" aria-labelledby="offer-gates-title">
      <h2 id="offer-gates-title" className="font-display text-lg text-fg">
        {t("gates.title")}
      </h2>
      {detail.gates.length === 0 ? (
        <p className="text-sm text-fg-muted">{t("gates.none")}</p>
      ) : (
        <ul className="flex flex-col gap-1.5">
          {detail.gates.map((gate) => (
            <li
              key={gate.gate}
              className="flex flex-wrap items-start gap-x-3 gap-y-1 rounded-card border border-border bg-surface px-3 py-2"
            >
              <span className="inline-flex items-center gap-1.5 text-sm font-medium text-fg">
                <GateIcon verdict={gate.verdict} />
                {t(GATE_LABELS[gate.gate] ?? "offer:gate.other")}
              </span>
              <span className="text-sm text-fg-muted">
                {t(`offer:gates.verdict.${gate.verdict}`)}
              </span>
              {gate.quote ? (
                /* The posting's own words: plain text, never markup (AD-7). */
                <span className="w-full text-sm text-fg-muted">“{gate.quote}”</span>
              ) : null}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function GateIcon({ verdict }: { verdict: string }) {
  if (verdict === "veto") {
    return <XCircle aria-hidden className="size-4 text-danger" />;
  }
  if (verdict === "flag") {
    return <AlertTriangle aria-hidden className="size-4 text-warning" />;
  }
  return <CheckCircle2 aria-hidden className="size-4 text-success" />;
}

/** FR-59's Explanation, when the agent has written one (AC2). */
export function Narration({ detail }: { detail: Detail }) {
  const { t } = useTranslation("offer");
  return (
    <section className="flex flex-col gap-1.5" aria-labelledby="offer-narration-title">
      <h2 id="offer-narration-title" className="font-display text-lg text-fg">
        {t("narration.title")}
      </h2>
      {detail.narration ? (
        <>
          {/* Agent-written, derived from the posting: plain text only (ADR-010). */}
          <p className="whitespace-pre-line text-sm text-fg">{detail.narration.text}</p>
          <p className="text-xs text-fg-subtle">{t("narration.agent")}</p>
        </>
      ) : (
        <p className="text-sm text-fg-muted">{t("narration.none")}</p>
      )}
    </section>
  );
}

/**
 * The factor breakdown (AC1), in two groups the DualScore scrolls to (AC3):
 * score, weight, what matched, what is missing, and where it read it.
 */
export function Breakdown({ detail }: { detail: Detail }) {
  const { t } = useTranslation("offer");
  if (detail.factors.length === 0) {
    return (
      <section className="flex flex-col gap-2" aria-labelledby="offer-factors-title">
        <h2 id="offer-factors-title" className="font-display text-lg text-fg">
          {t("factors.title")}
        </h2>
        <p className="text-sm text-fg-muted">{t("factors.none")}</p>
      </section>
    );
  }
  return (
    <div className="flex flex-col gap-5">
      {factorGroups(detail).map((group) => (
        <section
          key={group.axis}
          id={group.anchor}
          className="flex scroll-mt-6 flex-col gap-2"
          aria-labelledby={`${group.anchor}-title`}
        >
          <h3 id={`${group.anchor}-title`} className="font-display text-base text-fg">
            {t(group.labelKey)}
          </h3>
          <ul className="flex flex-col gap-2">
            {group.factors.map((factor) => (
              <li
                key={factor.factor}
                className="flex flex-col gap-1 rounded-card border border-border bg-surface px-3 py-2"
              >
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <span className="text-sm font-medium text-fg">
                    {t(FACTOR_LABELS[factor.factor] ?? "offer:factor.other")}
                  </span>
                  <span className="text-sm text-fg-muted">
                    {factor.score === null ? (
                      <span className="text-fg-subtle">{t("factors.not_read")}</span>
                    ) : (
                      <>
                        <span className="font-medium text-fg">{Math.round(factor.score)}</span>
                        <span className="ml-2 text-xs">
                          {t("factors.weight", {
                            weight: Math.round((factor.effective_weight ?? 0) * 100),
                          })}
                        </span>
                      </>
                    )}
                  </span>
                </div>

                {factor.matched.length > 0 ? (
                  <ul className="flex flex-wrap gap-1.5" aria-label={t("factors.matched")}>
                    {factor.matched.map((skill) => (
                      <li
                        key={skill}
                        className="inline-flex items-center gap-1 rounded-full border border-success/40 px-2 py-0.5 text-xs text-fg-muted"
                      >
                        <Check aria-hidden className="size-3 text-success" />
                        {skill}
                      </li>
                    ))}
                  </ul>
                ) : null}

                {factor.missing.length > 0 ? (
                  <ul className="flex flex-wrap gap-1.5" aria-label={t("factors.missing")}>
                    {factor.missing.map((skill) => (
                      <li
                        key={skill}
                        className="inline-flex items-center gap-1 rounded-full border border-border px-2 py-0.5 text-xs text-fg-muted"
                      >
                        <X aria-hidden className="size-3 text-fg-subtle" />
                        {skill}
                      </li>
                    ))}
                  </ul>
                ) : null}

                {factor.evidence.length > 0 ? (
                  <ul className="flex flex-col gap-0.5" aria-label={t("factors.evidence")}>
                    {factor.evidence.map((evidence) => (
                      <li
                        // Evidence items have no id; their text and position are the identity.
                        key={`${evidence.kind}-${evidence.text}`}
                        className="text-xs text-fg-subtle"
                      >
                        {evidence.kind === "quote"
                          ? t("factors.quote", { quote: evidence.text })
                          : t("factors.field", { field: evidence.text })}
                      </li>
                    ))}
                  </ul>
                ) : null}
              </li>
            ))}
          </ul>
        </section>
      ))}
    </div>
  );
}

/** The Sources, and the copies dedup merged into this Offer (AC1, AD-19). */
export function Sources({ detail }: { detail: Detail }) {
  const { t } = useTranslation("offer");
  return (
    <section className="flex flex-col gap-1.5" aria-labelledby="offer-sources-title">
      <h2 id="offer-sources-title" className="font-display text-lg text-fg">
        {t("sources.title")}
      </h2>
      <p className="text-sm text-fg-muted">
        {detail.sources.length > 0 ? detail.sources.join(" · ") : t("sources.none")}
      </p>
      <h3 className="mt-2 font-display text-base text-fg">{t("duplicates.title")}</h3>
      {detail.duplicates.length === 0 ? (
        <p className="text-sm text-fg-muted">{t("duplicates.none")}</p>
      ) : (
        <ul className="flex flex-col gap-0.5">
          {detail.duplicates.map((duplicate) => (
            <li
              key={`${duplicate.source}-${duplicate.external_id}`}
              className="text-sm text-fg-muted"
            >
              {t("duplicates.line", {
                source: duplicate.source,
                id: duplicate.external_id,
              })}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

/** The detected Apply Channels and the original link (AC1, FR-81). */
export function Channels({ detail }: { detail: Detail }) {
  const { t } = useTranslation("offer");
  return (
    <section className="flex flex-col gap-2" aria-labelledby="offer-channels-title">
      <h2 id="offer-channels-title" className="font-display text-lg text-fg">
        {t("channels.title")}
      </h2>
      {detail.channels.length === 0 ? (
        <p className="text-sm text-fg-muted">{t("channels.none")}</p>
      ) : (
        <ul className="flex flex-wrap gap-2">
          {detail.channels.map((channel) => (
            <li
              key={channel.kind === "email_apply" ? channel.address : channel.url}
              className="rounded-full border border-border px-2.5 py-1 text-xs text-fg-muted"
            >
              {t(CHANNEL_LABELS[channel.kind] ?? "offer:channel.other")}
              {channel.kind === "email_apply" ? ` · ${channel.address}` : null}
              {channel.kind === "ats_form" ? ` · ${channel.provider}` : null}
            </li>
          ))}
        </ul>
      )}
      <div>
        <Button
          variant="outline"
          size="sm"
          onClick={() => void openUrl(detail.apply_url).catch(() => undefined)}
        >
          <ExternalLink aria-hidden className="size-3.5" />
          {t("open_original")}
        </Button>
      </div>
    </section>
  );
}

/** The posting, sanitised: no remote content is loaded (AC1, AD-7). */
export function Posting({ detail }: { detail: Detail }) {
  const { t } = useTranslation("offer");
  return (
    <section className="flex flex-col gap-2" aria-labelledby="offer-posting-title">
      <h2 id="offer-posting-title" className="font-display text-lg text-fg">
        {t("posting.title")}
      </h2>
      <div
        className="flex flex-col gap-2 text-sm text-fg [&_a]:text-focus [&_a]:underline [&_li]:ml-5 [&_li]:list-disc [&_p]:mb-2"
        // biome-ignore lint/security/noDangerouslySetInnerHtml: `description_html` is the core sanitiser's output (AD-7), and withoutRemoteContent strips every element that could fetch something; rendering it is the one way the posting shows as the posting.
        dangerouslySetInnerHTML={{ __html: withoutRemoteContent(detail.description_html) }}
      />
      {detail.requirements.length > 0 ? (
        <div className="flex flex-col gap-1">
          <h3 className="font-display text-base text-fg">{t("posting.requirements")}</h3>
          <ul className="flex flex-wrap gap-1.5">
            {detail.requirements.map((requirement) => (
              <li
                key={requirement}
                className="rounded-full border border-border px-2 py-0.5 text-xs text-fg-muted"
              >
                {requirement}
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </section>
  );
}

/** The header line: title, company, where, pay, age (EXPERIENCE §9.4). */
export function Header({ detail, locale }: { detail: Detail; locale: string }) {
  const { t } = useTranslation("offer");
  const pay = payAmount(detail, locale);
  const period =
    detail.pay_period && PAY_PERIOD_LABELS[detail.pay_period]
      ? t(PAY_PERIOD_LABELS[detail.pay_period])
      : null;
  const company = [detail.company || null, detail.locations.join(", ") || null]
    .filter(Boolean)
    .join(" · ");

  return (
    <header className="flex flex-col gap-1">
      <h1 id="offer-title" className="font-display text-2xl text-fg">
        {detail.title || t("untitled")}
      </h1>
      {company ? <p className="text-sm text-fg-muted">{company}</p> : null}
      <p className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-fg-subtle">
        <span>{t(WORK_MODE_LABELS[detail.work_mode] ?? "offer:work_mode.unknown")}</span>
        {pay ? (
          <span className="text-fg-muted">
            {pay}
            {period ? ` ${period}` : null}
          </span>
        ) : null}
        <span>{t("age.days", { days: ageDays(detail) })}</span>
      </p>
    </header>
  );
}
