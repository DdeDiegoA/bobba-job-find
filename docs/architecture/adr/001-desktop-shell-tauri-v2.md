# ADR-001 — Desktop shell: Tauri v2

- **Status:** Accepted (validates the locked decision in AGENTS.md) · 2026-10-04
- **Spine:** AD-1, AD-22

## Context
We need a cross-platform desktop app (Windows, macOS, Linux) with a system tray, background scheduling while the user is logged in, native notifications, signed auto-update, secure local storage, and the ability to spawn and supervise third-party CLIs. The user's strengths are TS/React, Rust and Python. Non-technical users must be able to install it with a single installer.

## Decision
Use **Tauri 2.12.x**. The Rust core holds all logic and IO. The React UI runs in the system webview (WKWebView, WebView2, WebKitGTK 4.1). Plugins: single-instance, notification, autostart, updater/process, dialog, opener, window-state, log. (Deep-link was dropped: OAuth is loopback-only, see architecture §11.) The shell, fs, sql and store plugins are not exposed to the webview. Capabilities are minimal, and the CSP is `default-src 'self'`.

## Consequences
- Installers are about 10–20 MB (plus optional embeddings later). Memory use is low, which suits an app that sits in the tray all day.
- One systems language (Rust) for scraping, crypto, PDF, process control.
- Webview differences between OSes (WebKitGTK in particular) need UI testing on all three. E2E runs on Linux and Windows only, because tauri-driver has no macOS WKWebView driver. macOS gets manual QA.
- Linux needs `libwebkit2gtk-4.1` and `libayatana-appindicator3` (AppImage bundles most of it).
- Not shippable to the Mac App Store or Microsoft Store as designed, because sandboxing forbids spawning user CLIs. Direct download only.

## Alternatives
- **Electron.** Mature, with one Chromium everywhere, but about 150 MB and a Node main process (a second backend language), and it is heavier in the tray. Rejected.
- **Wails (Go)** and **Flutter**. Outside the team's skills, and no React reuse.
- **Pure web app plus local server.** Breaks the tray, notifications and keychain story.
