// Version scheme (as in logcadence). main releases are X.Y.Z (app/package.json "version", tag vX.Y.Z). Dev builds
// count towards the next minor release: with main at 0.1.0 they are 0.2.0.1, 0.2.0.2, … (tags v0.2.0.N,
// `pnpm release:dev`), published as GitHub pre-releases, until `pnpm release` rolls them up into v0.2.0.
//
// The app's own version must be semver, so 0.2.0.3 is 0.2.0-dev.3 inside the app (a prerelease of 0.2.0: newer
// than 0.1.0 and every earlier dev build, older than 0.2.0). typeshelfDev previews are 0.2.0-preview.3.
// Used by the UI and by scripts/; src-tauri/src/updater.rs has its own tag_version.

export type Bump = "major" | "minor" | "patch";

export function parseRelease(v: string): [number, number, number] {
  const m = /^(\d+)\.(\d+)\.(\d+)$/.exec(v);
  if (!m) throw new Error(`not an X.Y.Z version: ${v}`);
  return [Number(m[1]), Number(m[2]), Number(m[3])];
}

export function bump(v: string, kind: Bump): string {
  const [a, b, c] = parseRelease(v);
  return kind === "major" ? `${a + 1}.0.0` : kind === "minor" ? `${a}.${b + 1}.0` : `${a}.${b}.${c + 1}`;
}

/** The release that dev builds of `release` lead up to: the next minor. */
export const devSeries = (release: string) => bump(release, "minor");

/** N of the tags vSERIES.N. */
export function devNumbers(series: string, tags: string[]): number[] {
  const prefix = `v${series}.`;
  return tags.flatMap((t) => (t.startsWith(prefix) && /^\d+$/.test(t.slice(prefix.length)) ? [Number(t.slice(prefix.length))] : []));
}

export const nextDevNumber = (series: string, tags: string[]) => Math.max(0, ...devNumbers(series, tags)) + 1;

/** The app version a release tag stands for: v0.2.0 → 0.2.0, v0.2.0.3 → 0.2.0-dev.3; null for other tags. */
export function tagVersion(tag: string): string | null {
  const m = /^v(\d+\.\d+\.\d+)(?:\.(\d+))?$/.exec(tag);
  return m ? (m[2] ? `${m[1]}-dev.${Number(m[2])}` : m[1]!) : null;
}

/** How a version is shown: 0.2.0-dev.3 → 0.2.0.3, 0.2.0-preview.3 → 0.2.0.3-preview. */
export function versionLabel(v: string): string {
  const m = /^(\d+\.\d+\.\d+)-(dev|preview)\.(\d+)(.*)$/.exec(v);
  return m ? `${m[1]}.${m[3]}${m[2] === "preview" ? "-preview" : ""}${m[4]}` : v;
}

/** Orders X.Y.Z and X.Y.Z-dev.N versions (a release after its dev builds); <0, 0 or >0. */
export function compareVersions(a: string, b: string): number {
  const key = (v: string) => {
    const m = /^(\d+)\.(\d+)\.(\d+)(?:-[a-z]+\.(\d+))?/.exec(v);
    if (!m) return [0, 0, 0, 0];
    return [Number(m[1]), Number(m[2]), Number(m[3]), m[4] === undefined ? Infinity : Number(m[4])];
  };
  const [x, y] = [key(a), key(b)];
  for (let i = 0; i < 4; i++) if (x[i] !== y[i]) return x[i]! < y[i]! ? -1 : 1;
  return 0;
}

/** Release tags (vX.Y.Z and dev builds' vX.Y.Z.N), oldest first. */
export const sortTags = (tags: string[]) => tags.filter((t) => tagVersion(t)).sort((x, y) => compareVersions(tagVersion(x)!, tagVersion(y)!));

/** Dev builds' tags (vX.Y.Z.N), oldest first. */
export const sortDevTags = (tags: string[]) => sortTags(tags).filter((t) => tagVersion(t)!.includes("-"));
