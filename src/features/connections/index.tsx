import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import ConnectionsEmail from "./email";
import SessionOptin from "./session-optin";
import ConnectionsSources from "./sources";
import { useSourceCatalogue } from "./sources/use-sources";

/**
 * Connections (FX3 owns this container): Sources, Email and the per-Source
 * Session Board opt-in. Each section is a leaf its own slice built and tested;
 * this file only mounts them under one page heading.
 */
export default function ConnectionsIndex() {
  const { t } = useTranslation("connections");
  const catalogue = useSourceCatalogue();
  const boards = (catalogue.data ?? []).filter((tile) => tile.source_type === "session_board");

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-6 overflow-y-auto">
      <header className="flex flex-col gap-1 px-6 pt-6">
        <h1 className="font-display text-2xl text-fg">{t("title")}</h1>
        <p className="text-sm text-fg-muted">{t("subtitle")}</p>
      </header>

      <ConnectionsSources />

      <ConnectionsEmail />

      <section
        className="flex flex-col gap-4 px-6 pb-6"
        aria-labelledby="connections-session-title"
      >
        <div className="flex flex-col gap-1">
          <h2 id="connections-session-title" className="font-display text-2xl text-fg">
            {t("session.title")}
          </h2>
          <p className="text-sm text-fg-muted">{t("session.hint")}</p>
        </div>

        {catalogue.isSuccess && boards.length === 0 ? (
          <EmptyState title={t("session.empty.title")} description={t("session.empty.hint")} />
        ) : (
          <ul className="flex flex-col gap-3">
            {boards.map((board) => (
              <SessionBoardRow key={board.id} id={board.id} name={board.name} />
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}

/** One session-board Source: its opt-in card stays folded until asked for. */
function SessionBoardRow({ id, name }: { id: string; name: string }) {
  const { t } = useTranslation("connections");
  const [open, setOpen] = useState(false);
  const panelId = useId();

  return (
    <li className="flex flex-col gap-2">
      <Button
        type="button"
        variant="outline"
        size="sm"
        className="self-start"
        aria-expanded={open}
        aria-controls={panelId}
        onClick={() => setOpen((current) => !current)}
      >
        {t("session.toggle", { source: name })}
      </Button>
      {open ? (
        <div id={panelId}>
          <SessionOptin sourceId={id} sourceName={name} />
        </div>
      ) : null}
    </li>
  );
}
