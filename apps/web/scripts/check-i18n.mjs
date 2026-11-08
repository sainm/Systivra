import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const dictionaries = ["ja", "en", "zh-CN"].map((lang) => ({
  lang,
  messages: JSON.parse(fs.readFileSync(path.join(root, "src/i18n", `${lang}.json`), "utf8")),
}));

const [reference, ...others] = dictionaries;
const referenceKeys = Object.keys(reference.messages).sort();
let failures = 0;

for (const dictionary of others) {
  const keys = Object.keys(dictionary.messages).sort();
  const missing = referenceKeys.filter((key) => !keys.includes(key));
  const extra = keys.filter((key) => !referenceKeys.includes(key));
  if (missing.length > 0 || extra.length > 0) {
    failures += 1;
    console.error(`${dictionary.lang}: missing=${missing.join(",") || "-"} extra=${extra.join(",") || "-"}`);
  }
}

const sourceFiles = [];
function walk(directory) {
  for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
    if (entry.name === "generated" || entry.name === "node_modules") continue;
    const full = path.join(directory, entry.name);
    if (entry.isDirectory()) walk(full);
    else if (/\.(ts|tsx)$/.test(entry.name)) sourceFiles.push(full);
  }
}
walk(path.join(root, "src"));

const used = new Set();
const pattern = /\bt\(\s*["']([^"']+)["']/g;
for (const file of sourceFiles) {
  const source = fs.readFileSync(file, "utf8");
  for (const match of source.matchAll(pattern)) used.add(match[1]);
}

const unknown = [...used].filter((key) => !(key in reference.messages)).sort();
if (unknown.length > 0) {
  failures += 1;
  console.error(`unknown keys used in source: ${unknown.join(",")}`);
}

if (failures > 0) process.exit(1);
console.log(`i18n ok: ${referenceKeys.length} keys, ${used.size} literal keys used`);
