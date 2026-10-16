import { OnboardingWizard } from "./wizard";

/**
 * Onboarding screen (owned by S3.1 — backlog ownership rule 7: the first real
 * feature slice replaces S1.1's placeholder container). The wizard itself is
 * the first-run gate: it renders nothing once setup is done.
 *
 * AC1 note (orquestador ruling, 2026-10-15): the first-launch navigation to
 * `/onboarding` lives in routes.tsx (S1.5) and is deferred to S3.8, which lands
 * the wizard's finish step. Visiting `/onboarding` already gives the full AC1
 * behaviour: modal wizard over the app, starting with language and theme.
 */
export default function Onboarding() {
  return <OnboardingWizard />;
}
