import CvImportStep from "../steps/cv-import";
import EngineStep, { engineStepIsValid } from "../steps/engine";
import LanguageStep from "../steps/language";
import PreferencesStep from "../steps/preferences";
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
];
