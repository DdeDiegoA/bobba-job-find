import { useNavigate } from "react-router";
import { OnboardingWizard } from "./wizard";

/**
 * Onboarding screen (owned by S3.1 — backlog ownership rule 7: the first real
 * feature slice replaces S1.1's placeholder container). The wizard itself is
 * the first-run gate: it renders nothing once setup is done.
 *
 * AC1 note (orquestador ruling, 2026-10-15; landed by S3.8): the first-launch
 * redirect to `/onboarding` lives in routes.tsx, and finishing the wizard hands
 * over to Today (`/`), where the first Search Run's progress lives (FR-10).
 */
export default function Onboarding() {
  const navigate = useNavigate();
  return <OnboardingWizard onFinished={() => navigate("/", { replace: true })} />;
}
