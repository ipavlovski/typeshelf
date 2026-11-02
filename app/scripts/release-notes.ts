// For the release workflow: checks a pushed tag against app/package.json and writes what the GitHub release needs.
// Usage: tsx scripts/release-notes.ts <tag> <notes.md>
// Prints version=, title=, prerelease= lines for $GITHUB_OUTPUT.

import { readFileSync, writeFileSync } from "node:fs";
import { devSeries, sortDevTags, sortTags, tagVersion, versionLabel } from "../src/versions.ts";
import { releaseNotes, type Commit } from "./lib/notes.ts";
import { git } from "./lib/run.ts";

const [tag, notesFile] = process.argv.slice(2);
if (!tag || !notesFile) throw new Error("usage: tsx scripts/release-notes.ts <tag> <notes.md>");
const version = tagVersion(tag);
if (!version) throw new Error(`${tag} is not a release tag (vX.Y.Z or vX.Y.Z.N)`);
const release = (JSON.parse(readFileSync("package.json", "utf8")) as { version: string }).version;
const dev = version.includes("-");
const expected = dev ? devSeries(release) : release;
if (version.split("-")[0] !== expected) throw new Error(`${tag} doesn't match package.json ${release}: expected v${expected}${dev ? ".N" : ""}`);

const lines = (s: string) => s.split("\n").filter(Boolean);
const earlier = sortTags(lines(git("tag", "--merged", tag))).filter((t) => t !== tag);
// A dev build follows the previous build of either kind; a release follows the previous release.
const previous = (dev ? earlier : earlier.filter((t) => !tagVersion(t)!.includes("-"))).at(-1);
const commits: Commit[] = git("log", "--format=%h%x1f%s%x1f%b%x1e", previous ? `${previous}..${tag}` : tag, "-n", "60")
  .split("\x1e")
  .map((r) => r.trim())
  .filter(Boolean)
  .map((r) => {
    const [hash, subject, body] = r.split("\x1f");
    return { hash: hash!, subject: subject!, body: body ?? "" };
  })
  .filter((c) => !/^Release \d/.test(c.subject));
const devTags = dev ? [] : sortDevTags(earlier).filter((t) => tagVersion(t)!.startsWith(`${version}-`));

writeFileSync(notesFile, releaseNotes({ tag, previous, commits, devTags, release }));
console.log(`version=${version}\ntitle=typeshelf ${versionLabel(version)}${dev ? " (dev build)" : ""}\nprerelease=${dev}`);
