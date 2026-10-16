import { DndContext, type DragEndEvent, useDroppable } from "@dnd-kit/core";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import type { ApplicationCard, BoardColumn } from "@/lib/ipc/bindings";
import { isAppError, renderAppError } from "@/lib/ipc/errors";
import { cn } from "@/lib/utils";
import { Card } from "./card";
import { Detail } from "./detail";
import { useApplicationDetail, useBoard, useTransitionStatus } from "./use-tracker";

/** Where a move is going: the column's stage (for the label) and its status. */
type Target = { stage: string; status: string };

/**
 * The Tracker board (FR-93, EXPERIENCE §9.6). The columns come from the core —
 * the UI keeps no status of its own (AC1) and never moves a card itself: every
 * drop, keyboard move and Undo goes through one `transition` call that logs a
 * `status_event` (AC2/AC4).
 */
export function Board() {
  const { t } = useTranslation("tracker");
  // Errors carry keys from the `common` catalog, the board uses the `tracker` one.
  const { t: tError } = useTranslation();
  const board = useBoard();
  const move = useTransitionStatus();
  const [collapsed, setCollapsed] = useState<string[]>([]);
  const [confirming, setConfirming] = useState<{ card: ApplicationCard; target: Target } | null>(
    null,
  );
  const [open, setOpen] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const detail = useApplicationDetail(open);

  const columns = board.data?.columns ?? [];

  /** Move a card and offer Undo (AC2). */
  const commit = (card: ApplicationCard, target: Target, from: string) => {
    setError(null);
    move.mutate(
      {
        application_id: card.id,
        from_status: from,
        to_status: target.status,
        source: "user",
        reason: null,
      },
      {
        onSuccess: () => {
          toast(t("undo.label", { stage: t(`tracker:stage.${target.stage}`) }), {
            action: {
              label: t("undo.action"),
              // Undo is another move through the same use case (AC4).
              onClick: () =>
                commit(card, { stage: card.stage, status: card.status }, target.status),
            },
          });
        },
        onError: (failure) =>
          setError(isAppError(failure) ? renderAppError(failure, tError) : t("error.move")),
      },
    );
  };

  const requestMove = (card: ApplicationCard, target: Target | undefined) => {
    if (!target || target.status === card.status) return;
    // AC2: moving to Applied asks for a light confirmation.
    if (target.stage === "applied") {
      setConfirming({ card, target });
      return;
    }
    commit(card, target, card.status);
  };

  const targetOf = (index: number): Target | undefined => {
    const column = columns[index];
    return column ? { stage: column.stage, status: column.status } : undefined;
  };

  const keyboardMove = (card: ApplicationCard, direction: -1 | 1) => {
    const index = columns.findIndex((column) => column.stage === card.stage);
    if (index === -1) return;
    requestMove(card, targetOf(index + direction));
  };

  const onDragEnd = (event: DragEndEvent) => {
    if (!event.over) return;
    const card = columns
      .flatMap((column) => column.cards)
      .find((candidate) => candidate.id === event.active.id);
    if (!card) return;
    requestMove(card, targetOf(columns.findIndex((column) => column.stage === event.over?.id)));
  };

  if (board.isPending) {
    return <p className="p-6 text-sm text-fg-muted">{t("loading")}</p>;
  }
  if (board.isError || !board.data) {
    return (
      <p className="p-6 text-sm text-danger">
        {isAppError(board.error) ? renderAppError(board.error, tError) : t("error.load")}
      </p>
    );
  }

  return (
    <div className="flex min-h-0 min-w-0 flex-1 gap-4">
      <DndContext onDragEnd={onDragEnd}>
        <section className="flex min-w-0 flex-1 gap-3 overflow-x-auto" aria-label={t("board.aria")}>
          {columns.map((column) => (
            <Column
              key={column.stage}
              column={column}
              collapsed={collapsed.includes(column.stage)}
              onToggle={() =>
                setCollapsed((current) =>
                  current.includes(column.stage)
                    ? current.filter((stage) => stage !== column.stage)
                    : [...current, column.stage],
                )
              }
              onOpen={setOpen}
              onKeyboardMove={(card, direction) => keyboardMove(card, direction)}
            />
          ))}
        </section>
      </DndContext>

      <div className="flex w-80 shrink-0 flex-col gap-2">
        {error ? <p className="text-sm text-danger">{error}</p> : null}

        {/* AC2: the light confirmation before a card is marked Applied. */}
        {confirming ? (
          <div className="flex flex-col gap-2 rounded-2xl border border-border bg-surface p-3">
            <p className="text-sm text-fg">
              {t("confirm.applied", { title: confirming.card.title })}
            </p>
            <div className="flex gap-2">
              <Button
                size="sm"
                onClick={() => {
                  const { card, target } = confirming;
                  setConfirming(null);
                  commit(card, target, card.status);
                }}
              >
                {t("confirm.yes")}
              </Button>
              <Button variant="ghost" size="sm" onClick={() => setConfirming(null)}>
                {t("confirm.no")}
              </Button>
            </div>
          </div>
        ) : null}

        {open ? (
          <Detail detail={detail.data} loading={detail.isPending} onClose={() => setOpen(null)} />
        ) : columns.every((column) => column.count === 0) ? (
          <EmptyState title={t("empty")} description={t("empty.hint")} />
        ) : null}
      </div>
    </div>
  );
}

/** One Stage column; the four terminal ones collapse (FR-93). */
function Column({
  column,
  collapsed,
  onToggle,
  onOpen,
  onKeyboardMove,
}: {
  column: BoardColumn;
  collapsed: boolean;
  onToggle: () => void;
  onOpen: (id: string) => void;
  onKeyboardMove: (card: ApplicationCard, direction: -1 | 1) => void;
}) {
  const { t } = useTranslation("tracker");
  const { setNodeRef, isOver } = useDroppable({ id: column.stage });
  const label = t(`tracker:stage.${column.stage}`);

  return (
    <section
      ref={setNodeRef}
      className={cn(
        "flex w-56 shrink-0 flex-col gap-2 rounded-2xl border border-border bg-surface-muted/40 p-3",
        isOver && "border-primary",
      )}
    >
      <div className="flex items-center justify-between gap-1">
        <h2 className="font-display text-sm text-fg">
          {label} <span className="text-xs text-fg-subtle">{column.count}</span>
        </h2>
        {column.terminal ? (
          <button
            type="button"
            className="text-xs text-fg-muted underline"
            aria-expanded={!collapsed}
            onClick={onToggle}
          >
            {collapsed
              ? t("column.expand", { stage: label })
              : t("column.collapse", { stage: label })}
          </button>
        ) : null}
      </div>

      {collapsed ? null : (
        <ul className="flex flex-col gap-2">
          {column.cards.map((card) => (
            <Card
              key={card.id}
              card={card}
              onOpen={() => onOpen(card.id)}
              onKeyboardMove={(direction) => onKeyboardMove(card, direction)}
            />
          ))}
          {column.count === 0 ? (
            <li className="text-xs text-fg-subtle">{t("column.empty")}</li>
          ) : null}
        </ul>
      )}
    </section>
  );
}
