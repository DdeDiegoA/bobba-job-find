import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import type { SourceTileView } from "@/lib/ipc/bindings";
import { isAppError, renderAppError } from "@/lib/ipc/errors";
import { cn } from "@/lib/utils";
import {
  type RssProbe,
  useAddRss,
  useRemoveRss,
  useSourceCatalogue,
  useTestRss,
} from "./use-sources";

/** The tile's wording, as resolvable keys (AD-21). */
const TYPE_LABELS: Record<string, string> = {
  api: "connections_sources:tile.type.api",
  ats_board: "connections_sources:tile.type.ats_board",
  feed: "connections_sources:tile.type.feed",
  session_board: "connections_sources:tile.type.session_board",
  manual: "connections_sources:tile.type.manual",
  custom: "connections_sources:tile.type.custom",
};

const RISK_LABELS: Record<string, string> = {
  safe: "connections_sources:tile.risk.safe",
  caution: "connections_sources:tile.risk.caution",
  prohibited_automation: "connections_sources:tile.risk.prohibited_automation",
};

const STATUS_LABELS: Record<SourceTileView["status"], string> = {
  ok: "connections_sources:status.ok",
  degraded: "connections_sources:status.degraded",
  failing: "connections_sources:status.failing",
};

/** How a status pill reads: calm, watchful, then clearly failing. */
const STATUS_STYLE: Record<SourceTileView["status"], string> = {
  ok: "bg-surface-muted text-fg-muted",
  degraded: "bg-warning/15 text-warning",
  failing: "bg-danger/15 text-danger",
};

/** One Source tile: what it is, where it looks, and how it last did (AC1). */
function SourceTile({ tile, onRemove }: { tile: SourceTileView; onRemove: (id: string) => void }) {
  const { t } = useTranslation("connections_sources");
  const risk = tile.risk_tier;
  return (
    <li className="flex flex-col gap-2 rounded-2xl border border-border bg-surface p-4">
      <div className="flex items-baseline justify-between gap-2">
        <h3 className="font-display text-lg text-fg">{tile.name}</h3>
        <span className={cn("rounded-xl px-2 py-1 text-xs", STATUS_STYLE[tile.status])}>
          {t(STATUS_LABELS[tile.status])}
        </span>
      </div>

      <div className="flex flex-wrap items-center gap-2 text-xs">
        <span className="rounded-xl bg-surface-muted px-2 py-1 text-fg-muted">
          {t(TYPE_LABELS[tile.source_type] ?? tile.source_type)}
        </span>
        <span
          className={cn(
            "rounded-xl px-2 py-1",
            risk === "safe"
              ? "bg-surface-muted text-fg-muted"
              : risk === "caution"
                ? "bg-warning/15 text-warning"
                : "bg-danger/15 text-danger",
          )}
        >
          {t(RISK_LABELS[risk] ?? risk)}
        </span>
        {/* S11.3 (FR-91): Auto is on for this board, so the ToS risk persists. */}
        {tile.auto_opt_in ? (
          <span
            className="rounded-xl bg-danger/15 px-2 py-1 text-danger"
            data-tos-risk=""
            data-auto-opt-in=""
          >
            {t("session_auto:badge.tos_risk")}
          </span>
        ) : null}
        {tile.markets.map((market) => (
          <span key={market} className="rounded-xl bg-surface-muted px-2 py-1 text-fg-subtle">
            {market}
          </span>
        ))}
      </div>

      <p className="text-sm text-fg-muted">
        {tile.last_run
          ? t("tile.last_run", { when: new Date(tile.last_run).toLocaleString() })
          : t("tile.never_run")}
      </p>
      <p className="text-sm text-fg-muted">{t("tile.offers", { count: tile.offers_found })}</p>
      {tile.consecutive_failures > 0 ? (
        <p className="text-sm text-fg-muted">
          {t("tile.failures", { count: tile.consecutive_failures })}
        </p>
      ) : null}
      {tile.last_error ? (
        <p className="text-sm text-danger">{t("tile.error", { error: tile.last_error })}</p>
      ) : null}
      {tile.attribution ? <p className="text-xs text-fg-subtle">{tile.attribution}</p> : null}

      {tile.custom ? (
        <Button variant="ghost" size="sm" onClick={() => onRemove(tile.id)}>
          {t("tile.remove")}
        </Button>
      ) : null}
    </li>
  );
}

/** The add-feed form: test first, preview, then save (AC2). */
function AddFeedForm({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation("connections_sources");
  const { t: tError } = useTranslation();
  const testRss = useTestRss();
  const addRss = useAddRss();
  const [url, setUrl] = useState("");
  const [name, setName] = useState("");
  const [probe, setProbe] = useState<RssProbe | null>(null);

  const probeError =
    testRss.error && isAppError(testRss.error) ? renderAppError(testRss.error, tError) : null;
  const saveError =
    addRss.error && isAppError(addRss.error) ? renderAppError(addRss.error, tError) : null;

  const trimmedName = name.trim() === "" ? null : name.trim();

  function probeFeed() {
    setProbe(null);
    testRss.mutate({ url, name: trimmedName }, { onSuccess: setProbe });
  }

  return (
    <form
      className="flex flex-col gap-3 rounded-2xl border border-border bg-surface p-4"
      onSubmit={(event) => {
        event.preventDefault();
        addRss.mutate({ url: probe?.url ?? url, name: trimmedName }, { onSuccess: onClose });
      }}
    >
      <h2 className="font-display text-xl text-fg">{t("custom.title")}</h2>

      <label className="flex flex-col gap-1 text-sm text-fg-muted">
        {t("custom.url")}
        <input
          className="rounded-xl border border-border bg-surface px-3 py-2 text-fg"
          type="url"
          value={url}
          onChange={(event) => {
            setUrl(event.target.value);
            setProbe(null);
          }}
          required
        />
      </label>
      <label className="flex flex-col gap-1 text-sm text-fg-muted">
        {t("custom.name")}
        <input
          className="rounded-xl border border-border bg-surface px-3 py-2 text-fg"
          type="text"
          value={name}
          onChange={(event) => setName(event.target.value)}
        />
      </label>

      {probeError ? <p className="text-sm text-danger">{probeError}</p> : null}

      {probe ? (
        <div className="flex flex-col gap-1 text-sm">
          <p className="text-fg">
            {probe.count > 0 ? t("custom.probe", { count: probe.count }) : t("custom.probe_empty")}
          </p>
          <p className="text-fg-muted">{t("custom.sample")}</p>
          <ul className="flex flex-col gap-1">
            {probe.sample.map((item) => (
              <li key={item.url} className="text-fg-muted">
                {item.title} · {item.company}
              </li>
            ))}
          </ul>
        </div>
      ) : null}

      {saveError ? <p className="text-sm text-danger">{saveError}</p> : null}

      <div className="flex gap-2">
        <Button
          type="button"
          variant="outline"
          onClick={probeFeed}
          disabled={url.trim() === "" || testRss.isPending}
        >
          {testRss.isPending ? t("custom.testing") : t("custom.test")}
        </Button>
        {probe ? (
          <Button type="submit" disabled={addRss.isPending}>
            {t("custom.save")}
          </Button>
        ) : null}
        <Button type="button" variant="ghost" onClick={onClose}>
          {t("custom.cancel")}
        </Button>
      </div>
    </form>
  );
}

/**
 * Connections → Sources (S4.8): the catalogue with each Source's health, plus
 * the custom RSS/Atom flow that tests a feed before saving it.
 */
export default function ConnectionsSources() {
  const { t } = useTranslation("connections_sources");
  const { t: tError } = useTranslation();
  const catalogue = useSourceCatalogue();
  const removeRss = useRemoveRss();
  const [adding, setAdding] = useState(false);

  const loadError =
    catalogue.error && isAppError(catalogue.error) ? renderAppError(catalogue.error, tError) : null;

  return (
    <section className="flex flex-col gap-4 p-6" aria-labelledby="ConnectionsSources-title">
      <header className="flex items-baseline justify-between gap-2">
        <h1 id="ConnectionsSources-title" className="font-display text-2xl text-fg">
          {t("title")}
        </h1>
        {adding ? null : (
          <Button variant="outline" size="sm" onClick={() => setAdding(true)}>
            {t("add")}
          </Button>
        )}
      </header>

      {adding ? <AddFeedForm onClose={() => setAdding(false)} /> : null}

      {loadError ? <p className="text-sm text-danger">{loadError}</p> : null}

      {catalogue.isSuccess && catalogue.data.length === 0 ? (
        <EmptyState title={t("empty.title")} description={t("empty.hint")} />
      ) : (
        <ul className="grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-3">
          {(catalogue.data ?? []).map((tile) => (
            <SourceTile key={tile.id} tile={tile} onRemove={(id) => removeRss.mutate(id)} />
          ))}
        </ul>
      )}
    </section>
  );
}
