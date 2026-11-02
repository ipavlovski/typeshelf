import { spawnSync, type SpawnSyncOptions } from "node:child_process";

// Small process helpers for the release scripts. They run from app/; the repository root is "..".

/** Runs a command with its output shown; throws if it fails. */
export function run(cmd: string, args: string[], opts: SpawnSyncOptions = {}) {
  const r = spawnSync(cmd, args, { stdio: "inherit", ...opts });
  if (r.error) throw r.error;
  if (r.status !== 0) throw new Error(`${cmd} ${args.join(" ")} exited with ${r.status}`);
}

/** Runs a command and returns its trimmed stdout ('' when it fails and `ok` is set). */
export function out(cmd: string, args: string[], opts: SpawnSyncOptions & { ok?: boolean } = {}): string {
  const r = spawnSync(cmd, args, { encoding: "utf8", ...opts });
  if (r.error) throw r.error;
  if (r.status !== 0 && !opts.ok) throw new Error(`${cmd} ${args.join(" ")} exited with ${r.status}: ${r.stderr}`);
  return String(r.stdout ?? "").trim();
}

export const git = (...args: string[]) => out("git", args);

/** Changes that would make a build differ from HEAD: edits and new files in the sources (docs don't count). */
export function uncommitted(): string[] {
  const SOURCES =
    /^(crates|app\/src|app\/src-tauri|app\/scripts|plugins|\.github)\/|^(Cargo\.toml|Cargo\.lock|app\/package\.json|app\/pnpm-lock\.yaml|app\/index\.html|app\/vite\.config\.ts|app\/tsconfig\.json)$/;
  return out("git", ["status", "--porcelain"], { cwd: ".." })
    .split("\n")
    .filter(Boolean)
    .filter((l) => SOURCES.test(l.slice(3)));
}
