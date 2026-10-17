# Accessibility E2E (S10.6)

Automated half of the accessibility QA (NFR-15, EXPERIENCE §7). The scripts a
machine cannot run — VoiceOver, NVDA, hands-on zoom — and the findings with their
owners are in [`docs/qa/accessibility.md`](../../docs/qa/accessibility.md).

| File | Proves |
|---|---|
| `axe.ts` | axe-core (dev-only, MPL-2.0) over the same DOM. `color-contrast` is off (jsdom cannot paint); contrast lives in `contrast.test.ts`. |
| `audit.ts` | The hand-written audit: accessible names, labels, heading order, ids, `aria-labelledby`, tabindex, alt text, `aria-hidden` focus leaks, nested interactive controls, landmarks. Kept alongside axe on purpose: each rule says why it exists, and it runs without the dependency. |
| `screens.test.tsx` | The seven AC1 screens render with zero unexpected violations; Verdicts and Stages are text; the Auto confirmation moves focus, traps Tab and closes on Esc; tabs expose their panels. `KNOWN` is the allowlist for findings owned by another slice — empty since CH14. |
| `keyboard.test.tsx` | Skip link → `<main>`, native links, Kill Switch keyboard operation, Tracker Cmd/Ctrl + ←/→, wizard focus on open. |
| `contrast.test.ts` | Every normal-text token pair ≥ 4.5:1 in both themes, read from the shipped `tokens.css`. |
| `reduced-motion.test.ts` | The global `prefers-reduced-motion: reduce` rule exists and is not overridden. |
| `zoom.test.ts` | The shell floor vs the window minimum vs the 200% zoom arithmetic (A11Y-6). |

Run everything with:

```sh
pnpm test -- e2e/a11y
```

`KNOWN` (in `screens.test.tsx`) is the only pin left: it is an allowlist for
findings that belong to another slice, one line per follow-up. CH14 fixed
A11Y-1..8, so it is empty and every finding is now a passing assertion — a new
finding must be pinned there (never left unguarded), and the pin disappears the
day its owner ships the fix. There are no `it.fails` or `it.todo` allowances.
