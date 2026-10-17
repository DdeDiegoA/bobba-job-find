import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { useAddRss, useSourceCatalogue } from "@/features/connections/sources/use-sources";
import type { SourceTileView } from "@/lib/ipc/bindings";
import { isAppError, renderAppError } from "@/lib/ipc/errors";
import { cn } from "@/lib/utils";
import type { StepProps } from "../../wizard/machine";

/** How a Risk Tier badge reads (AD-21: resolvable keys, never raw text). */
const RISK_LABELS: Record<string, string> = {
  safe: "onboarding_finish:risk.safe",
  caution: "onboarding_finish:risk.caution",
  prohibited_automation: "onboarding_finish:risk.prohibited_automation",
};

/** How a Risk Tier badge looks: calm, watchful, then clearly not for automation. */
const RISK_STYLE: Record<string, string> = {
  safe: "bg-surface-muted text-fg-muted",
  caution: "bg-warning/15 text-warning",
  prohibited_automation: "bg-danger/15 text-danger",
};

/**
 * Recommended for this install: a Safe Source that is on (FR-8). Nothing here
 * toggles a Source — the core registers the built-in Safe Sources enabled — so
 * the check says what is on, not what the user has to do. The Connections
 * screen owns the toggle (S4.8).
 */
function recommended(tile: SourceTileView): boolean {
  return tile.enabled && tile.risk_tier === "safe";
}

/**
 * Step — Sources (FR-8). The recommended Safe Sources for the user's markets
 * are pre-selected, each tile carries its Risk Tier badge, and a custom feed
 * can be added without leaving setup (AC1).
 */
export default function SourcesStep(_props: StepProps) {
  const { t } = useTranslation("onboarding_finish");
  const { t: tError } = useTranslation();
  const catalogue = useSourceCatalogue();
  const add = useAddRss();
  const [url, setUrl] = useState("");
  const [name, setName] = useState("");

  const sources = catalogue.data ?? [];
  // Recommended first, so the default reading is "these are on".
  const ordered = [...sources].sort((a, b) => Number(recommended(b)) - Number(recommended(a)));
  const addError = add.error && isAppError(add.error) ? renderAppError(add.error, tError) : null;

  return (
    <div className="flex flex-col gap-6">
      <p className="text-fg-muted">{t("sources.body")}</p>

      {catalogue.isPending ? <p className="text-sm text-fg-muted">{t("sources.loading")}</p> : null}
      {catalogue.isError ? <p className="text-sm text-danger">{t("sources.error.load")}</p> : null}

      {catalogue.data && sources.length === 0 ? (
        <p className="text-sm text-fg-muted">{t("sources.empty")}</p>
      ) : null}

      <ul className="flex flex-col gap-2">
        {ordered.map((tile) => (
          <li key={tile.id} className="rounded-xl border border-border bg-surface p-3">
            <label className="flex cursor-default items-start gap-3">
              <input
                type="checkbox"
                className="mt-1"
                checked={recommended(tile)}
                disabled
                onChange={() => undefined}
              />
              <span className="flex flex-col gap-1">
                <span className="font-medium text-fg">{tile.name}</span>
                <span className="flex flex-wrap items-center gap-2 text-xs">
                  <span
                    className={cn(
                      "rounded-xl px-2 py-1",
                      RISK_STYLE[tile.risk_tier] ?? "bg-surface-muted text-fg-muted",
                    )}
                  >
                    {t(RISK_LABELS[tile.risk_tier] ?? tile.risk_tier)}
                  </span>
                  {tile.markets.map((market) => (
                    <span
                      key={market}
                      className="rounded-xl bg-surface-muted px-2 py-1 text-fg-subtle"
                    >
                      {market}
                    </span>
                  ))}
                </span>
              </span>
            </label>
          </li>
        ))}
      </ul>

      <p className="text-sm text-fg-muted">{t("sources.sessionNote")}</p>
      <p className="text-sm text-fg-muted">{t("sources.changeLater")}</p>

      {/* FR-8/§4.4: a custom feed can be added right here (S4.8's probe and
          save; the URL is validated in the core before it is fetched). */}
      <section className="flex flex-col gap-2 rounded-xl border border-border p-4">
        <h3 className="font-medium text-fg">{t("sources.add.title")}</h3>
        <div className="flex flex-col gap-2 sm:flex-row">
          <label className="flex min-w-0 flex-1 flex-col gap-1 text-sm text-fg">
            {t("sources.add.url")}
            <input
              type="url"
              className="h-10 rounded-xl border border-border bg-surface px-3 text-fg"
              placeholder={t("sources.add.placeholder")}
              value={url}
              onChange={(event) => setUrl(event.target.value)}
            />
          </label>
          <label className="flex min-w-0 flex-1 flex-col gap-1 text-sm text-fg">
            {t("sources.add.name")}
            <input
              type="text"
              className="h-10 rounded-xl border border-border bg-surface px-3 text-fg"
              value={name}
              onChange={(event) => setName(event.target.value)}
            />
          </label>
        </div>
        <div>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={url.trim() === "" || add.isPending}
            onClick={() =>
              add.mutate(
                {
                  url: url.trim(),
                  name: name.trim() === "" ? null : name.trim(),
                },
                {
                  onSuccess: () => {
                    setUrl("");
                    setName("");
                  },
                },
              )
            }
          >
            {add.isPending ? t("sources.add.adding") : t("sources.add.action")}
          </Button>
        </div>
        <p className="text-xs text-fg-subtle">{t("sources.add.hint")}</p>
        {addError ? (
          <p role="alert" className="text-sm text-danger">
            {addError}
          </p>
        ) : null}
      </section>
    </div>
  );
}
