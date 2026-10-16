/**
 * Profile sections (AC1, EXPERIENCE §9.10). The kind is the value the core
 * stores; the label lives in the `profile` catalog.
 */
export interface ProfileSection {
  /** `FactKind` id, as stored and as sent by the editor. */
  kind: string;
  /** Namespaced i18n key (`profile:sections.<kind>`). */
  labelKey: string;
}

export const PROFILE_SECTIONS: readonly ProfileSection[] = [
  { kind: "identity", labelKey: "profile:sections.identity" },
  { kind: "experience", labelKey: "profile:sections.experience" },
  { kind: "skill", labelKey: "profile:sections.skill" },
  { kind: "education", labelKey: "profile:sections.education" },
  { kind: "certification", labelKey: "profile:sections.certification" },
  { kind: "language", labelKey: "profile:sections.language" },
  { kind: "eligibility", labelKey: "profile:sections.eligibility" },
];

/** The sections that ask for an evidence quote (AC1: skills). */
export function takesEvidence(kind: string): boolean {
  return kind === "skill";
}
