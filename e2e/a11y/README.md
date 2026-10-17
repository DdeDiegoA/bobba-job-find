# Accessibility E2E (S10.6)

Automated half of the accessibility QA (NFR-15, EXPERIENCE §7). The scripts a
machine cannot run — VoiceOver, NVDA, hands-on zoom — and the findings with their
owners are in [`docs/qa/accessibility.md`](../../docs/qa/accessibility.md).

| File | Proves |
|---|---|
| `axe.ts` | axe-core (dev-only, MPL-2.0) over the same DOM. `color-contrast` is off (jsdom cannot paint); contrast lives in `contrast.test.ts`. |
| `audit.ts` | The hand-written audit: accessible names, labels, heading order, ids, `aria-labelledby`, tabindex, alt text, `aria-hidden` focus leaks, nested interactive controls, landmarks. Kept alongside axe on purpose: each rule says why it exists, and it runs without the dependency. |
| `screens.test.tsx` | The seven AC1 screens render with zero unexpected violations; Verdicts and Stages are text; the Auto confirmation is labelled. Findings owned by other slices are pinned in `KNOWN`. |
| `keyboard.test.tsx` | Skip link → `<main>`, native links, Kill Switch keyboard operation, Tracker Cmd/Ctrl + ←/→, wizard focus on open. |
| `contrast.test.ts` | Every normal-text token pair ≥ 4.5:1 in both themes, read from the shipped `tokens.css`. |
| `reduced-motion.test.ts` | The global `prefers-reduced-motion: reduce` rule exists and is not overridden. |
| `zoom.test.ts` | The shell floor vs the window minimum vs the 200% zoom arithmetic (A11Y-6). |

Run everything with:

```sh
pnpm test -- e2e/a11y
```

`KNOWN` (in `screens.test.tsx`), `it.fails` (A11Y-5, A11Y-6) and `it.todo`
(A11Y-4) are pins for findings that belong to another slice — S10.6 never edits
`src/`. When an owner ships the fix, delete the pin; the test then guards it.
