import { tagVersion, versionLabel } from "../../src/versions.ts";

// Release notes (markdown) for a GitHub release, from the commits since the previous release: each commit's
// subject, with the "- " bullets of its body nested under it. The app's Updates window renders these.

export interface Commit {
  hash: string;
  subject: string;
  body: string;
}

/** The "- " bullets of a commit body (continuation lines joined), without trailers like Co-Authored-By. */
export function bodyBullets(body: string): string[] {
  const out: string[] = [];
  for (const line of body.split("\n")) {
    if (/^- /.test(line)) out.push(line.slice(2).trim());
    else if (/^\s+\S/.test(line) && out.length) out[out.length - 1] += ` ${line.trim()}`;
  }
  return out;
}

export function releaseNotes(o: { tag: string; previous?: string; commits: Commit[]; devTags?: string[]; release: string }): string {
  const version = tagVersion(o.tag)!;
  const dev = version.includes("-");
  const label = (t: string) => versionLabel(tagVersion(t) ?? t);
  const parts: string[] = [];
  parts.push(
    dev
      ? `Dev build of ${version.split("-")[0]} (the current release is ${o.release}). Install it from the app's Updates window.`
      : o.devTags?.length
        ? `Rolls up the dev builds ${o.devTags.map(label).join(", ")}.`
        : "",
  );
  parts.push(`## Changes${o.previous ? ` since ${label(o.previous)}` : ""}`);
  parts.push(
    o.commits.map((c) => [`- ${c.subject} (${c.hash})`, ...bodyBullets(c.body).map((b) => `  - ${b}`)].join("\n")).join("\n") || "- No changes",
  );
  return parts.filter(Boolean).join("\n\n") + "\n";
}
