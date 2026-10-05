#!/usr/bin/env node
// CI gate for S1.8 (AC2): every third-party GitHub Action in .github/workflows
// is pinned to a full commit SHA, and no workflow reaches a job board or an LLM
// endpoint. Local actions (./...) are allowed unpinned.
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

const DIR = ".github/workflows";
const PINNED = /^[^.\s][^@\s]*@[0-9a-f]{40}$/; // owner/repo@<40-char SHA>
const BANNED =
  /linkedin\.com|indeed\.com|glassdoor\.|greenhouse\.io|lever\.co|smartrecruiters\.com|workday\.com|openai\.com|anthropic\.com|generativelanguage\.googleapis\.com|api\.mistral|huggingface\.co/i;

const errs = [];
for (const file of readdirSync(DIR).filter((f) => /\.ya?ml$/.test(f))) {
  const lines = readFileSync(join(DIR, file), "utf8").split("\n");
  lines.forEach((line, i) => {
    const at = `${file}:${i + 1}`;
    const uses = line.match(/^\s*-?\s*uses:\s*(\S+)/);
    if (uses && !uses[1].startsWith("./") && !PINNED.test(uses[1])) {
      errs.push(`${at}: action not pinned to a commit SHA: ${uses[1]}`);
    }
    if (BANNED.test(line) && !/^\s*#/.test(line)) {
      errs.push(`${at}: references a job-board/LLM endpoint`);
    }
  });
}

if (errs.length) {
  for (const e of errs) console.error(`::error::${e}`);
  process.exit(1);
}
console.log("workflow policy ok: actions pinned, no board/LLM endpoints");
