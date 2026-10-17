/**
 * The accessibility audit the E2E suite runs on every MVP screen (S10.6,
 * NFR-15, EXPERIENCE §7).
 *
 * It is deliberately dependency-free: the app ships no axe-core (and this slice
 * may not add one — `package.json` is outside S10.6's files). What it checks is
 * the DOM-level floor that a jsdom render can decide: accessible names, label
 * association, heading order, landmarks, id references, tab order and
 * aria-hidden focus leaks. What it cannot decide — real screen-reader output,
 * contrast, 200% zoom, focus painting — is in `docs/qa/accessibility.md` as
 * scripted manual checks. Keep the two in sync: a rule that lands here should
 * lose its manual twin, and vice versa.
 */

export type Violation = {
  /** Stable slug (one of [`Rule`], or an axe-core rule id), so docs and the
   * known-defect baseline can cite one. */
  rule: string;
  /** The offending element, short enough to paste into a bug. */
  element: string;
  /** What is wrong, in one line. */
  detail: string;
};

export type Rule =
  | "control-name"
  | "field-label"
  | "heading-order"
  | "duplicate-id"
  | "aria-labelledby-target"
  | "aria-label-empty"
  | "tabindex-positive"
  | "img-alt"
  | "aria-hidden-focus"
  | "main-landmark"
  | "link-name"
  | "nested-interactive";

const FOCUSABLE =
  'a[href], button, input:not([type="hidden"]), select, textarea, [tabindex]:not([tabindex="-1"])';
const INTERACTIVE_ROLES = "button, link, tab, checkbox, switch, menuitem, option, radio";

/** `aria-labelledby` ids, in order, joined with their text (the AccName rule). */
function labelledByText(el: Element): string {
  const ids = (el.getAttribute("aria-labelledby") ?? "").split(/\s+/).filter(Boolean);
  return ids
    .map((id) => el.ownerDocument.getElementById(id)?.textContent?.trim() ?? "")
    .filter(Boolean)
    .join(" ");
}

/** The name a user would hear for this element, per the HTML-AAM order. */
export function accessibleName(el: Element): string {
  const byLabelledBy = labelledByText(el).trim();
  if (byLabelledBy) return byLabelledBy;

  const label = el.getAttribute("aria-label")?.trim();
  if (label) return label;

  if (el instanceof HTMLInputElement || el instanceof HTMLSelectElement || el instanceof HTMLTextAreaElement) {
    const id = el.getAttribute("id");
    if (id) {
      const forLabel = el.ownerDocument.querySelector(`label[for="${CSS.escape(id)}"]`);
      if (forLabel?.textContent?.trim()) return forLabel.textContent.trim();
    }
    const wrapped = el.closest("label");
    if (wrapped?.textContent?.trim()) {
      // A wrapped label already contains this control; its text is the name.
      return wrapped.textContent.trim();
    }
  }

  if (el instanceof HTMLImageElement && el.alt) return el.alt;

  // Buttons and links take their content, with nested images by their alt.
  const text = [...el.childNodes]
    .map((node) => {
      if (node.nodeType === Node.TEXT_NODE) return node.textContent ?? "";
      if (node instanceof HTMLImageElement) return node.alt;
      if (node instanceof Element && node.getAttribute("aria-hidden") !== "true") {
        // Icons are decorative and aria-hidden; anything else contributes text.
        return node.textContent ?? "";
      }
      return "";
    })
    .join("")
    .replace(/\s+/g, " ")
    .trim();
  if (text) return text;

  return el.getAttribute("title")?.trim() ?? "";
}

function describe(el: Element): string {
  const tag = el.tagName.toLowerCase();
  const id = el.getAttribute("id");
  const cls = (el.getAttribute("class") ?? "").split(/\s+/).filter(Boolean).slice(0, 3).join(".");
  const text = (el.textContent ?? "").replace(/\s+/g, " ").trim().slice(0, 40);
  return `<${tag}${id ? `#${id}` : ""}${cls ? `.${cls}` : ""}>${text ? ` "${text}"` : ""}`;
}

function isVisibleish(el: Element): boolean {
  // jsdom has no layout; hidden-by-attribute and inert subtrees are the cases
  // that would otherwise produce noise.
  if (el.closest("[hidden], [inert]")) return false;
  return true;
}

/** Run every rule against `root`. Returns one entry per finding, deduplicated. */
export function auditTree(root: ParentNode): Violation[] {
  const out: Violation[] = [];
  const push = (rule: Rule, el: Element, detail: string) => {
    out.push({ rule, element: describe(el), detail });
  };

  // 1. Every control has a name.
  for (const el of root.querySelectorAll(
    `button, a[href], input:not([type="hidden"]), select, textarea, [role="${INTERACTIVE_ROLES.split(", ").join("], [role=")}"]`,
  )) {
    if (!isVisibleish(el)) continue;
    if (el.closest('[aria-hidden="true"]')) continue;
    if (!accessibleName(el)) {
      push("control-name", el, "no accessible name (aria-label, label or text)");
    }
  }

  // 2. Links have text that says where they go.
  for (const el of root.querySelectorAll("a[href]")) {
    if (!isVisibleish(el)) continue;
    if (!accessibleName(el)) push("link-name", el, "empty link");
  }

  // 3. Form fields are labelled (a placeholder is not a label).
  for (const el of root.querySelectorAll('input:not([type="hidden"]), select, textarea')) {
    if (!isVisibleish(el)) continue;
    if (el.closest('[aria-hidden="true"]')) continue;
    if (!accessibleName(el)) {
      push("field-label", el, "no label, aria-label or aria-labelledby");
    }
  }

  // 4. Headings do not skip a level going down.
  let previous = 0;
  for (const heading of root.querySelectorAll("h1, h2, h3, h4, h5, h6")) {
    if (!isVisibleish(heading)) continue;
    const level = Number(heading.tagName[1]);
    if (previous && level > previous + 1) {
      push("heading-order", heading, `h${previous} is followed by h${level}`);
    }
    previous = level;
  }
  const headings = root.querySelectorAll("h1, h2, h3, h4, h5, h6");
  if ((root.querySelector("h1") ?? null) === null && headings.length > 0) {
    push("heading-order", headings[0], "screen has headings but no h1");
  }

  // 5. Ids are unique (aria-labelledby and label[for] resolve by id).
  const seen = new Set<string>();
  for (const el of root.querySelectorAll("[id]")) {
    const id = el.id;
    if (seen.has(id)) push("duplicate-id", el, `id "${id}" appears more than once`);
    seen.add(id);
  }

  // 6. Every aria-labelledby resolves.
  for (const el of root.querySelectorAll("[aria-labelledby]")) {
    const ids = (el.getAttribute("aria-labelledby") ?? "").split(/\s+/).filter(Boolean);
    const missing = ids.filter((id) => !el.ownerDocument.getElementById(id));
    if (missing.length > 0) {
      push("aria-labelledby-target", el, `missing id(s): ${missing.join(", ")}`);
    }
  }

  // 7. An empty aria-label is worse than none: it silences the content.
  for (const el of root.querySelectorAll("[aria-label]")) {
    if (!(el.getAttribute("aria-label") ?? "").trim()) {
      push("aria-label-empty", el, "aria-label is empty");
    }
  }

  // 8. Positive tabindex reorders the document; none should exist.
  for (const el of root.querySelectorAll("[tabindex]")) {
    const value = Number(el.getAttribute("tabindex"));
    if (value > 0) push("tabindex-positive", el, `tabindex="${value}"`);
  }

  // 9. Images carry alt (empty alt means decorative, and is allowed).
  for (const el of root.querySelectorAll("img")) {
    if (el.getAttribute("alt") === null) push("img-alt", el, "img without alt");
  }

  // 10. Nothing focusable lives under aria-hidden.
  for (const hidden of root.querySelectorAll('[aria-hidden="true"]')) {
    for (const el of hidden.querySelectorAll(FOCUSABLE)) {
      push("aria-hidden-focus", el, "focusable inside aria-hidden");
    }
  }

  // 11. At most one main landmark per document.
  const mains = root.querySelectorAll("main, [role=main]");
  if (mains.length > 1) {
    push("main-landmark", mains[1], `${mains.length} main landmarks`);
  }

  // 12. A button inside a button is announced as one control and steals the
  // inner one's activation (WCAG 4.1.2; axe calls it nested-interactive).
  for (const parent of root.querySelectorAll('button, a[href], [role="button"], [role="link"]')) {
    const inner = parent.querySelector(FOCUSABLE);
    if (inner && inner !== parent) {
      push(
        "nested-interactive",
        parent,
        `control contains a focusable <${inner.tagName.toLowerCase()}>`,
      );
    }
  }

  return out;
}

/**
 * A known defect, filed as a follow-up for the slice that owns the screen
 * (S10.6 AC3: this pass never edits `src/`). Each entry names the rule, the
 * element it hits and the follow-up below `docs/qa/accessibility.md` findings.
 * Remove the entry when the owner ships the fix; the suite then guards it.
 */
export type KnownDefect = {
  rule: string;
  /** Matched with `element.startsWith`, so short stable prefixes work. */
  element: string;
  followUp: string;
};

/** Violations that are not in `known`. */
export function unexpected(violations: Violation[], known: KnownDefect[]): Violation[] {
  return violations.filter(
    (violation) =>
      !known.some(
        (defect) => defect.rule === violation.rule && violation.element.startsWith(defect.element),
      ),
  );
}

/** Human-readable failure body: rule, element, detail, one per line. */
export function format(violations: Violation[]): string {
  return violations.map((v) => `[${v.rule}] ${v.element} — ${v.detail}`).join("\n");
}
