import { useDraggable } from "@dnd-kit/core";
import { GripVertical } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { ApplicationCard } from "@/lib/ipc/bindings";
import { cn } from "@/lib/utils";

/**
 * One Application card (FR-93). It is draggable and, for keyboard users, the
 * grip is a real button (drag with Space, or `Cmd/Ctrl + ←/→` along the
 * columns, AC2, NFR-15). A11Y-2: dnd-kit's `attributes` live on the grip, not
 * on the `<li>`, so the card's open control is not a button inside a button.
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
      className={cn(
        "flex flex-col gap-1 rounded-2xl border border-border bg-surface p-3 text-left",
        isDragging && "opacity-60",
      )}
      onKeyDown={(event) => {
        if (!(event.metaKey || event.ctrlKey)) return;
        if (event.key === "ArrowLeft") onKeyboardMove(-1);
        else if (event.key === "ArrowRight") onKeyboardMove(1);
        else return;
        event.preventDefault();
      }}
    >
      <div className="flex items-start gap-2">
        <button
          type="button"
          {...listeners}
          {...attributes}
          aria-label={t("card.move", { title: card.title })}
          title={t("card.keys")}
          className="cursor-grab touch-none rounded-md p-0.5 text-fg-muted hover:text-fg"
        >
          <GripVertical aria-hidden className="size-4" />
        </button>
        <button
          type="button"
          aria-label={t("card.open", { title: card.title })}
          title={t("card.keys")}
          className="min-w-0 flex-1 text-left text-sm text-fg"
          onClick={onOpen}
        >
          {card.title}
        </button>
      </div>
      <span className="text-xs text-fg-subtle">{card.company}</span>
      {card.next_action ? <span className="text-xs text-fg-muted">{card.next_action}</span> : null}
    </li>
  );
}
