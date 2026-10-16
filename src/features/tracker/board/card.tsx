import { useDraggable } from "@dnd-kit/core";
import { useTranslation } from "react-i18next";
import type { ApplicationCard } from "@/lib/ipc/bindings";
import { cn } from "@/lib/utils";

/**
 * One Application card (FR-93). It is draggable and, for keyboard users,
 * `Cmd/Ctrl + ←/→` moves it along the columns (AC2, NFR-15).
 */
export function Card({
  card,
  onOpen,
  onKeyboardMove,
}: {
  card: ApplicationCard;
  onOpen: () => void;
  onKeyboardMove: (direction: -1 | 1) => void;
}) {
  const { t } = useTranslation("tracker");
  const { attributes, listeners, setNodeRef, transform, isDragging } = useDraggable({
    id: card.id,
  });

  return (
    <li
      ref={setNodeRef}
      style={transform ? { transform: `translate(${transform.x}px, ${transform.y}px)` } : undefined}
      {...listeners}
      {...attributes}
      aria-label={t("card.open", { title: card.title })}
      title={t("card.keys")}
      className={cn(
        "flex cursor-grab flex-col gap-1 rounded-2xl border border-border bg-surface p-3 text-left",
        isDragging && "opacity-60",
      )}
      onKeyDown={(event) => {
        if (!(event.metaKey || event.ctrlKey)) return;
        if (event.key === "ArrowLeft") onKeyboardMove(-1);
        else if (event.key === "ArrowRight") onKeyboardMove(1);
        else return;
        event.preventDefault();
      }}
      onDoubleClick={onOpen}
    >
      <button type="button" className="text-left text-sm text-fg" onClick={onOpen}>
        {card.title}
      </button>
      <span className="text-xs text-fg-subtle">{card.company}</span>
      {card.next_action ? <span className="text-xs text-fg-muted">{card.next_action}</span> : null}
    </li>
  );
}
