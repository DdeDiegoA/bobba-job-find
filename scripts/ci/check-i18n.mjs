#!/usr/bin/env node
// S1.8 wired this path into ci.yml. The i18n gate itself is owned by S1.4 and
// lives at scripts/check-i18n.mjs; this stays a one-line entrypoint so ci.yml
// needs no cross-slice edit.
import "../check-i18n.mjs";
