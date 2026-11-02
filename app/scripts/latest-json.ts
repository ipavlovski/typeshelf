// For the release workflow: writes latest.json, the manifest the installed apps' updater reads (tauri-plugin-updater),
// from the signed NSIS installer that `tauri build` produced.
// Usage: tsx scripts/latest-json.ts <tag> <version> <notes.md> <nsis bundle dir> <out.json>

import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";

const [tag, version, notesFile, dir, outFile] = process.argv.slice(2);
if (!tag || !version || !notesFile || !dir || !outFile) throw new Error("usage: tsx scripts/latest-json.ts <tag> <version> <notes.md> <dir> <out.json>");

const installer = readdirSync(dir).find((f) => f.endsWith("-setup.exe") && f.includes(version));
if (!installer) throw new Error(`no ${version} installer in ${dir}`);
const signature = readFileSync(path.join(dir, `${installer}.sig`), "utf8").trim();
const url = `https://github.com/ipavlovski/typeshelf/releases/download/${tag}/${encodeURIComponent(installer)}`;
const platform = { signature, url };

writeFileSync(
  outFile,
  JSON.stringify(
    {
      version,
      notes: readFileSync(notesFile, "utf8"),
      pub_date: new Date().toISOString(),
      // The updater looks for "<os>-<arch>-<installer>" first, then "<os>-<arch>".
      platforms: { "windows-x86_64-nsis": platform, "windows-x86_64": platform },
    },
    null,
    2,
  ) + "\n",
);
console.log(`${outFile}: ${installer}`);
