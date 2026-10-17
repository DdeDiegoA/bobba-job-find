# Onboarding E2E (S3.8)

`wizard-to-ranked-list.test.tsx` walks the whole first run in one test: the app
opens on `/`, the first-launch gate redirects into the wizard, every step is
completed (Language, a fake Agent CLI that answers its test call, CV skipped,
Preferences, fixture Sources, Email skipped, the forced recovery backup, the
finish summary), and finishing starts the Search Run and lands on Today with
the fixture ranked list.

The core is faked at the IPC boundary (`@/lib/ipc/bindings`); the components,
the router, the wizard framework and the i18n catalogs are the real ones. It
runs with `pnpm test`, so the CI vitest job covers it — no WebdriverIO harness
is needed yet (a native-driver suite is a later slice's call).
