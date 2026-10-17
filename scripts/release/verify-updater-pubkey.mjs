#!/usr/bin/env node
// Release guard (S10.5 AC2, AD-22): the committed updater pubkey is a real
// minisign public key, never a placeholder and never a secret key.
import { readFileSync } from "node:fs";
import { checkPubkey } from "./lib/updater-pubkey.mjs";

const config = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8"));
const pubkey = config.plugins?.updater?.pubkey ?? "";
const result = checkPubkey(pubkey);

if (!result.ok) {
  for (const error of result.errors) console.error(`::error::${error}`);
  process.exit(1);
}
console.log("updater pubkey ok: minisign public key present in tauri.conf.json");
