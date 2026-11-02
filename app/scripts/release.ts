// Publishes the dev branch. Both kinds push a tag that starts the release workflow, which builds the installer and
// publishes it on GitHub, with release notes from the commits since the previous release (scripts/release-notes.ts).
//
//   pnpm release:dev [--yes]                     a dev build: tags dev's HEAD vX.Y.Z.N (src/versions.ts) and
//                                                pushes it as a GitHub pre-release. Installed apps only get it
//                                                from their Updates window.
//   pnpm release [minor|patch|major] [--yes]     a release: bumps app/package.json, fast-forwards main to dev,
//                                                tags vX.Y.Z (listing the dev builds it rolls up) and pushes main,
//                                                dev and the tags. Installed apps update to it on their own.

import { readFileSync, writeFileSync } from "node:fs";
import { createInterface } from "node:readline/promises";
import { git, run, uncommitted } from "./lib/run.ts";
import { bump, devSeries, nextDevNumber, sortDevTags, sortTags, tagVersion, versionLabel, type Bump } from "../src/versions.ts";

const args = process.argv.slice(2);
const devBuild = args.includes("--dev");
const kind = (args.find((a) => !a.startsWith("--")) ?? "minor") as Bump;
if (!["major", "minor", "patch"].includes(kind)) throw new Error("usage: pnpm release [minor|patch|major] [--yes]  |  pnpm release:dev [--yes]");

const fail = (msg: string): never => (console.error(msg), process.exit(1));
if (git("rev-parse", "--abbrev-ref", "HEAD") !== "dev") fail("Release from the dev branch.");
const dirty = uncommitted();
if (dirty.length) fail(`Commit or stash these first:\n${dirty.join("\n")}`);
git("fetch", "origin", "main");
if (git("rev-parse", "main") !== git("rev-parse", "origin/main")) fail("main differs from origin/main: sync it first.");
// main only moves by releases, so dev is always ahead of it.
if (git("merge-base", "main", "dev") !== git("rev-parse", "main")) fail("main has commits that dev lacks: merge main into dev first.");

const pkg = JSON.parse(readFileSync("package.json", "utf8")) as { version: string };
const lines = (s: string) => s.split("\n").filter(Boolean);
// A dev build follows the previous build of either kind; a release follows the previous release, so a release can
// promote the dev build at HEAD (as scripts/release-notes.ts counts them).
const counted = (tags: string[]) => (devBuild ? sortTags(tags) : sortTags(tags).filter((t) => !tagVersion(t)!.includes("-")));
const onHead = counted(lines(git("tag", "--points-at", "HEAD")));
if (onHead.length) fail(`HEAD is already released as ${onHead.join(", ")}.`);
const previous = counted(lines(git("tag", "--merged", "HEAD"))).at(-1);
const log = git("log", "--oneline", previous ? `${previous}..HEAD` : "HEAD", "-n", "40");
if (!log) fail(`Nothing new since ${previous}.`);

async function confirm(question: string) {
  if (args.includes("--yes")) return;
  const rl = createInterface({ input: process.stdin, output: process.stdout });
  const answer = await rl.question(`${question} [y/N] `);
  rl.close();
  if (answer.trim().toLowerCase() !== "y") process.exit(0);
}

// The release workflow runs these too; failing here keeps a broken tag from being pushed.
function check() {
  run("pnpm", ["typecheck"]);
  run("cargo", ["test", "--workspace", "--quiet"], { cwd: ".." });
}

if (devBuild) {
  const series = devSeries(pkg.version);
  const tag = `v${series}.${nextDevNumber(series, lines(git("tag", "-l")))}`;
  console.log(`Dev build ${tag.slice(1)} of ${series} (main is ${pkg.version})\n\nCommits since ${previous ?? "the start"}:\n${log}\n`);
  await confirm(`Tag ${git("rev-parse", "--short", "HEAD")} as ${tag} and push dev + ${tag} to origin?`);
  check();
  git("tag", "-a", tag, "-m", `typeshelf ${tag.slice(1)} (dev build)`);
  run("git", ["push", "origin", "dev", tag]);
} else {
  const version = bump(pkg.version, kind);
  const devTags = sortDevTags(lines(git("tag", "--merged", "dev", "--no-merged", "main")));
  console.log(`Release ${pkg.version} → ${version}\n\nCommits since ${previous ?? "the start"}:\n${log}\n`);
  if (devTags.length) console.log(`Rolls up the dev builds ${devTags.map((t) => versionLabel(t.slice(1))).join(", ")}\n`);
  await confirm(`Commit, tag v${version} and push main + dev to origin?`);
  check();
  writeFileSync("package.json", readFileSync("package.json", "utf8").replace(`"version": "${pkg.version}"`, `"version": "${version}"`));
  git("commit", "-m", `Release ${version}`, "--", "package.json");
  git("checkout", "main");
  git("merge", "--ff-only", "dev");
  git("tag", "-a", `v${version}`, "-m", `typeshelf ${version}${devTags.length ? `\n\nRolls up the dev builds ${devTags.join(", ")}.` : ""}`);
  git("checkout", "dev");
  run("git", ["push", "--follow-tags", "origin", "main", "dev"]);
}
console.log("\nPushed. The release workflow is building it: https://github.com/ipavlovski/typeshelf/actions");
