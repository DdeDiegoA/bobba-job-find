import { Search } from "lucide-react";
import { type KeyboardEvent, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router";
import { cn } from "@/lib/utils";
import { offerEntries, type PaletteEntry, pageEntries, rank } from "./entries";
import { useOfferSearch } from "./search";
import { usePaletteStore } from "./store";

/** How many Offers one query may show; the pages are always all listed. */
const MAX_OFFERS = 8;

/** A field the user is typing in: J/K, `/` and the triage keys leave it alone. */
function isEditable(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  if (!el || typeof el.tagName !== "string") return false;
  return (
    el.tagName === "INPUT" ||
    el.tagName === "TEXTAREA" ||
    el.tagName === "SELECT" ||
    el.isContentEditable
  );
}

/**
 * The command palette (S6.7, EXPERIENCE §6): the fast path to any page or Offer.
 *
 * `Cmd/Ctrl+K` (and `/`) open it from anywhere. Pages come from the shell's own
 * navigation table; typing filters them and searches the Offers by title or
 * company. `↑`/`↓` move, `Enter` opens, `Esc` closes and focus returns to
 * wherever it was before — the dialog is `aria-modal` and keeps the focus
 * inside while it is open.
 */
export default function CommandPalette() {
  const open = usePaletteStore((state) => state.open);

  // Openers: Cmd/Ctrl+K is the documented shortcut (DESIGN §10.4); `/` is the
  // shell's "focus search", which here means the palette too.
  useEffect(() => {
    function onKeyDown(event: globalThis.KeyboardEvent) {
      const mod = event.metaKey || event.ctrlKey;
      const key = event.key.toLowerCase();
      if (mod && key === "k" && !event.shiftKey && !event.altKey) {
        event.preventDefault();
        usePaletteStore.getState().show();
        return;
      }
      if (!mod && !event.altKey && event.key === "/" && !isEditable(event.target)) {
        event.preventDefault();
        usePaletteStore.getState().show();
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  // The dialog only exists while it is open, so a closed palette holds no query
  // state and asks the core for nothing.
  return open ? <PaletteDialog /> : null;
}

function PaletteDialog() {
  const { t } = useTranslation("palette");
  const navigate = useNavigate();
  const hide = usePaletteStore((state) => state.hide);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const cards = useOfferSearch(true);

  // With no query the palette lists every page (the "go to" menu); typing filters
  // those and searches the Offers by title or company.
  const needle = query.trim();
  const pageRows = useMemo(() => rank(pageEntries(t), needle), [t, needle]);
  const offerRows = useMemo(
    () =>
      needle ? rank(offerEntries(cards, t("offer.untitled")), needle).slice(0, MAX_OFFERS) : [],
    [needle, cards, t],
  );
  const rows = useMemo(() => [...pageRows, ...offerRows], [pageRows, offerRows]);

  const selected = Math.min(active, Math.max(0, rows.length - 1));
  const activeId = rows.length > 0 ? `palette-row-${selected}` : undefined;

  // The dialog takes the focus while it is open and gives it back on close.
  useEffect(() => {
    const previous = document.activeElement;
    inputRef.current?.focus();
    return () => {
      if (previous instanceof HTMLElement && previous.isConnected) previous.focus();
    };
  }, []);

  const choose = (row: PaletteEntry | undefined) => {
    if (!row) return;
    hide();
    navigate(row.to);
  };

  const move = (delta: number) => {
    if (rows.length === 0) return;
    setActive((selected + delta + rows.length) % rows.length);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === "Escape") {
      event.preventDefault();
      hide();
      return;
    }
    if (event.key === "ArrowDown") {
      event.preventDefault();
      move(1);
      return;
    }
    if (event.key === "ArrowUp") {
      event.preventDefault();
      move(-1);
      return;
    }
    if (event.key === "Home") {
      event.preventDefault();
      setActive(0);
      return;
    }
    if (event.key === "End") {
      event.preventDefault();
      setActive(Math.max(0, rows.length - 1));
      return;
    }
    if (event.key === "Tab") {
      // Focus stays in the dialog while it is open (NFR-15).
      event.preventDefault();
      return;
    }
    if (event.key === "Enter") {
      event.preventDefault();
      choose(rows[selected]);
    }
  };

  // A click outside the panel closes it, the way a modal backdrop would (with
  // no handler on a decorative overlay element).
  const panelRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    function onPointerDown(event: MouseEvent) {
      const panel = panelRef.current;
      if (panel && event.target instanceof Node && !panel.contains(event.target)) hide();
    }
    document.addEventListener("mousedown", onPointerDown);
    return () => document.removeEventListener("mousedown", onPointerDown);
  }, [hide]);

  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center bg-black/40 p-6 pt-24">
      <div
        ref={panelRef}
        role="dialog"
        aria-modal="true"
        aria-label={t("title")}
        onKeyDown={onKeyDown}
        className="flex w-full max-w-xl flex-col overflow-hidden rounded-card border border-border bg-surface shadow-xl"
      >
        <div className="flex items-center gap-2 border-b border-border px-4">
          <Search aria-hidden className="size-4 shrink-0 text-fg-subtle" />
          <input
            ref={inputRef}
            type="text"
            role="combobox"
            aria-expanded={rows.length > 0}
            aria-controls={rows.length > 0 ? "palette-rows" : undefined}
            aria-activedescendant={activeId}
            aria-autocomplete="list"
            aria-label={t("placeholder")}
            placeholder={t("placeholder")}
            value={query}
            onChange={(event) => {
              setQuery(event.target.value);
              setActive(0);
            }}
            className="h-12 w-full bg-transparent text-sm text-fg outline-none placeholder:text-fg-subtle"
          />
        </div>

        {rows.length === 0 ? (
          <p role="status" className="px-2 py-6 text-center text-sm text-fg-muted">
            {t("empty")}
          </p>
        ) : (
          <div
            id="palette-rows"
            role="listbox"
            aria-label={t("results")}
            className="max-h-80 overflow-y-auto p-2"
          >
            {pageRows.length > 0 ? (
              <Group label={t("pages")}>
                {pageRows.map((row, i) => (
                  <Row
                    key={row.id}
                    row={row}
                    index={i}
                    selected={i === selected}
                    onHover={() => setActive(i)}
                    onChoose={() => choose(row)}
                  />
                ))}
              </Group>
            ) : null}

            {offerRows.length > 0 ? (
              <Group label={t("offers")}>
                {offerRows.map((row, i) => {
                  const index = pageRows.length + i;
                  return (
                    <Row
                      key={row.id}
                      row={row}
                      index={index}
                      selected={index === selected}
                      onHover={() => setActive(index)}
                      onChoose={() => choose(row)}
                    />
                  );
                })}
              </Group>
            ) : null}
          </div>
        )}

        <p className="border-t border-border px-4 py-2 text-xs text-fg-subtle">{t("hint")}</p>
      </div>
    </div>
  );
}

/** One named group of rows inside the listbox; the name is visual, the rows speak. */
function Group({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div role="presentation">
      <p aria-hidden="true" className="px-2 pt-2 pb-1 text-xs font-medium text-fg-subtle">
        {label}
      </p>
      {children}
    </div>
  );
}

/** One option: the label, its company when it is an Offer, and the keyboard state. */
function Row({
  row,
  index,
  selected,
  onHover,
  onChoose,
}: {
  row: PaletteEntry;
  index: number;
  selected: boolean;
  onHover: () => void;
  onChoose: () => void;
}) {
  return (
    <div
      id={`palette-row-${index}`}
      role="option"
      aria-selected={selected}
      tabIndex={-1}
      onMouseMove={onHover}
      onMouseDown={onChoose}
      className={cn(
        "flex cursor-pointer flex-col gap-0.5 rounded-xl px-2 py-2 text-sm",
        selected ? "bg-surface-muted text-fg" : "text-fg-muted",
      )}
    >
      <span className="truncate">{row.label}</span>
      {row.detail ? <span className="truncate text-xs text-fg-subtle">{row.detail}</span> : null}
    </div>
  );
}
