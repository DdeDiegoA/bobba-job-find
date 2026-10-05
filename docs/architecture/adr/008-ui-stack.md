# ADR-008 — UI stack: React 19 + Tailwind v4 + shadcn/ui + TanStack Query + tauri-specta

- **Status:** Accepted (coordinated with UX, 2026-10-04) · **Spine:** AD-3, AD-21

## Context
UX's DESIGN.md defines three-layer tokens (primitive → semantic → component) for light and dark themes, targeting Tailwind and shadcn-style CSS variables. It uses Lucide icons and the Fredoka, Nunito and JetBrains Mono fonts. The UI must stay a thin client (AD-2), with typed IPC and EN/ES from day one.

## Decision
- **React 19.3** with **TypeScript 7.0** and **Vite 8.3**. Biome for lint and format.
- **Tailwind 4.3** with tokens in `src/styles/tokens.css` (`:root`, `.dark`, `@theme`). Theme follows the OS (`prefers-color-scheme`), with a manual override class.
- **shadcn/ui** components on Radix. They are owned copies, restyled to the boba look.
- **TanStack Query** for all core data. Core events invalidate query keys. **Zustand** only for UI state.
- **React Router 8** (declarative). **react-hook-form + zod** for forms.
- **i18next + react-i18next + ICU**, with catalogs in `locales/{en,es}` shared with the core.
- **tauri-specta** generates `bindings.ts` for commands and events. The exact rc is pinned, with ts-rs as the fallback.
- **Fonts** are self-hosted through @fontsource. No CDN.

## Consequences
- A typed contract runs end to end, from Rust structs to TS.
- tauri-specta is still a release candidate. If it stalls, we swap to ts-rs plus a thin generated wrapper without changing the UI architecture.

## Alternatives
MUI, Chakra and Mantine (theming fights the custom look); Redux (boilerplate); TanStack Router (more to learn for about 8 routes); ESLint + Prettier (slower, and typescript-eslint lags TS 7). See tech-stack.md.
