import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";

/** Centered placeholder shown by screens and routes without content yet (AC1). */
export function EmptyState({
  icon: Icon,
  title,
  description,
}: {
  icon?: LucideIcon;
  title: ReactNode;
  description?: ReactNode;
}) {
  return (
    <div className="grid place-items-center gap-2 p-10 text-center">
      {Icon ? <Icon aria-hidden className="size-8 text-fg-subtle" /> : null}
      <p className="font-display text-lg text-fg">{title}</p>
      {description ? <p className="max-w-sm text-sm text-fg-muted">{description}</p> : null}
    </div>
  );
}
