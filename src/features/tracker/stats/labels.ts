/** The Verdict bands' labels (S9.9), named explicitly for the i18n gate. */
type Translate = (key: string) => string;

export function verdictLabel(t: Translate, verdict: string): string {
  switch (verdict) {
    case "strong":
      return t("stats.verdict.strong");
    case "good":
      return t("stats.verdict.good");
    case "moderate":
      return t("stats.verdict.moderate");
    case "weak":
      return t("stats.verdict.weak");
    case "poor":
      return t("stats.verdict.poor");
    case "unscored":
      return t("stats.group.unscored");
    default:
      return verdict;
  }
}
