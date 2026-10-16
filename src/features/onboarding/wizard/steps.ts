import LanguageStep from "../steps/language";
import type { WizardStep } from "./machine";

/**
 * Wizard order follows PRD §4.1 (FR-1…FR-10). Each later onboarding slice
 * appends its own step here — one line, inside its own `steps/<id>/` folder.
 * Append only: the order is product copy, not free to reorder.
 */
export const WIZARD_STEPS: WizardStep[] = [
  { id: "language", titleKey: "onboarding:steps.language.title", Component: LanguageStep },
];
