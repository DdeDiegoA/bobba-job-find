import BackupStep, { backupStepIsValid } from "../steps/backup";
import CvImportStep from "../steps/cv-import";
import EmailStep from "../steps/email";
import EngineStep, { engineStepIsValid } from "../steps/engine";
import FinishStep, { finishSetup } from "../steps/finish";
import LanguageStep from "../steps/language";
import PreferencesStep from "../steps/preferences";
import SourcesStep from "../steps/sources";
import type { WizardStep } from "./machine";

/**
 * Wizard order follows PRD §4.1 (FR-1…FR-10). Each later onboarding slice
 * appends its own step here — one line, inside its own `steps/<id>/` folder.
 * Append only: the order is product copy, not free to reorder.
 */
export const WIZARD_STEPS: WizardStep[] = [
  { id: "language", titleKey: "onboarding:steps.language.title", Component: LanguageStep },
  // PRD §4.1 order: the Agent CLI comes before the Profile, and a failing test
  // keeps Next disabled (FR-3); limited mode (S3.3) adds the way out.
  {
    id: "engine",
    titleKey: "onboarding_engine:step.title",
    isValid: engineStepIsValid,
    invalidKey: "onboarding_engine:step.invalid",
    Component: EngineStep,
  },
  // The Master CV comes before Preferences (PRD §4.1, FR-5). Optional: a user
  // with no CV on hand adds facts by hand, and the gap interview fills the rest.
  {
    id: "cv-import",
    titleKey: "onboarding_cv:step.title",
    optional: true,
    Component: CvImportStep,
  },
  { id: "preferences", titleKey: "preferences:step.title", Component: PreferencesStep },
  // Sources are on by default and shown with their Risk Tier (FR-8, S3.8 AC1).
  { id: "sources", titleKey: "onboarding_finish:step.sources.title", Component: SourcesStep },
  // Email is optional: Skip for now continues the wizard (FR-9, AC2).
  {
    id: "email",
    titleKey: "onboarding_finish:step.email.title",
    optional: true,
    Component: EmailStep,
  },
  // Every MVP build is unsigned: no first run until a recovery backup exists
  // (ADR-009, S3.8 AC3). The core's `backup_status` is what unlocks Next.
  {
    id: "backup",
    titleKey: "onboarding_finish:step.backup.title",
    isValid: backupStepIsValid,
    invalidKey: "onboarding_finish:backup.beforeFinish",
    Component: BackupStep,
  },
  // The last step hands over Review mode, 08:00 and launch-at-login off, and
  // its finish action starts the first Search Run (FR-10, AC4).
  {
    id: "finish",
    titleKey: "onboarding_finish:step.finish.title",
    Component: FinishStep,
    onFinish: finishSetup,
  },
];
