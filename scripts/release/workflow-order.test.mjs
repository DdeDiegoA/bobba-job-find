// FX21: the publish job used to verify as "published" before publishing, so
// every run died on "release vX is still a draft; publish it" (beta.1 run
// 39094995728, beta.2 run 39111385539). Verification now brackets the publish:
// draft pass -> publish -> published pass. This test locks the order.
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const WORKFLOW = readFileSync(".github/workflows/release.yml", "utf8");

const stepStart = (name) => {
  const at = WORKFLOW.indexOf(`- name: ${name}`);
  expect(at, `no step named "${name}"`).toBeGreaterThan(-1);
  return at;
};

/** The workflow text of one step, up to the next `- name:` or the file end. */
const stepText = (name) => {
  const start = stepStart(name);
  const next = WORKFLOW.indexOf("- name: ", start + 1);
  return WORKFLOW.slice(start, next === -1 ? undefined : next);
};

const STEPS = [
  "Download the draft assets",
  "latest.json points at public download URLs",
  "Installers and latest.json (draft)",
  "The DMG carries the pinned identity",
  "Publish",
  "Verify the published release",
];

describe("release.yml publish job order", () => {
  it("runs every publish step in the FX21 order", () => {
    const positions = STEPS.map(stepStart);
    expect(positions).toEqual([...positions].sort((a, b) => a - b));
  });

  it("verifies in draft mode before publication, never published mode", () => {
    const draft = stepText("Installers and latest.json (draft)");
    expect(draft).toContain("--draft");
    expect(draft).not.toContain("--published");
  });

  it("verifies in published mode after publication", () => {
    const published = stepText("Verify the published release");
    expect(published).toContain("--published");
    expect(published).not.toContain("--draft");
    expect(stepStart("Verify the published release")).toBeGreaterThan(stepStart("Publish"));
  });

  it("re-reads the attached latest.json for the published pass", () => {
    const published = stepText("Verify the published release");
    expect(published).toContain('gh release download "$TAG"');
    expect(published).toContain("--clobber");
  });
});
