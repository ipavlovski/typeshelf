// Builds the working tree as typeshelfDev (src-tauri/src/channel.rs) and (re)starts it on Windows, next to the
// released typeshelf. Usage: pnpm preview  (from WSL)
//
// Version: previews are labelled with the dev build they lead up to (src/versions.ts): with main at 0.1.0 and
// v0.2.0.2 the last dev build, 0.2.0.3-preview. Nothing is tagged; `pnpm release:dev` does that.

import { spawn } from "node:child_process";
import { copyFileSync, mkdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { git, out, run, uncommitted } from "./lib/run.ts";
import { devSeries, nextDevNumber } from "../src/versions.ts";

const EXE = "typeshelfDev.exe";
if (process.platform !== "linux" || !out("sh", ["-c", "command -v wslpath"], { ok: true })) throw new Error("pnpm preview runs in WSL");
// Windows programs started from a WSL folder print a UNC warning: run them from C:.
const win = { cwd: "/mnt/c" };

const t0 = Date.now();
const step = (msg: string) => console.log(`\n▸ ${msg}  (${((Date.now() - t0) / 1000).toFixed(0)} s)`);

// ── version ─────────────────────────────────────────────────────────────────

const release = (JSON.parse(readFileSync("package.json", "utf8")) as { version: string }).version;
const series = devSeries(release);
const n = nextDevNumber(series, git("tag", "-l").split("\n"));
const label = `${series}.${n}-preview`;
const dirty = uncommitted();
console.log(
  `typeshelfDev ${label}  (main ${release}, ${git("rev-parse", "--abbrev-ref", "HEAD")} @ ${git("rev-parse", "--short", "HEAD")}${dirty.length ? ` + ${dirty.length} uncommitted change(s)` : ""})`,
);

// ── build ───────────────────────────────────────────────────────────────────

step("type check");
run("pnpm", ["-s", "typecheck"]);
step("frontend");
run("pnpm", ["exec", "vite", "build", "--logLevel", "warn"]);
step("app (cargo xwin, x86_64-pc-windows-msvc)");
const window = { label: "main", title: `typeshelfDev ${label}`, width: 1280, height: 820, minWidth: 720, minHeight: 480 };
run("cargo", ["xwin", "build", "--release", "-p", "typeshelf-app", "--features", "custom-protocol", "--target", "x86_64-pc-windows-msvc"], {
  cwd: "..",
  env: {
    ...process.env,
    TYPESHELF_CHANNEL: "dev",
    TYPESHELF_LABEL: label,
    // Merged into tauri.conf.json at compile time: its own identifier keeps its WebView data apart.
    TAURI_CONFIG: JSON.stringify({ identifier: "app.typeshelf.dev", productName: "typeshelfDev", version: `${series}-preview.${n}`, app: { windows: [window] } }),
  },
});
const built = path.resolve("../target/x86_64-pc-windows-msvc/release/typeshelf-app.exe");

// ── deploy ──────────────────────────────────────────────────────────────────

const localAppData = out("wslpath", ["-u", out("cmd.exe", ["/d", "/c", "echo %LOCALAPPDATA%"], win)]);
const dir = path.join(localAppData, "Programs", "typeshelfDev");
const target = path.join(dir, EXE);
const targetWin = out("wslpath", ["-w", target]);

const running = () => out("tasklist.exe", ["/FI", `IMAGENAME eq ${EXE}`, "/NH"], win).includes(EXE);
if (running()) {
  // Closing the window only hides it to the tray, so ask it to quit over its local endpoint: it removes its session
  // fonts on the way out. Forced if it doesn't go within 15 s.
  step("closing the running typeshelfDev");
  const ps = `$w=New-Object System.Net.WebSockets.ClientWebSocket; $w.ConnectAsync([Uri]'ws://127.0.0.1:47216',[Threading.CancellationToken]::None).Wait(3000) | Out-Null; $b=[Text.Encoding]::UTF8.GetBytes('{"type":"quit"}'); $w.SendAsync([ArraySegment[byte]]$b,'Text',$true,[Threading.CancellationToken]::None).Wait(3000) | Out-Null`;
  out("powershell.exe", ["-NoProfile", "-Command", ps], { ...win, ok: true });
  const deadline = Date.now() + 15_000;
  while (running() && Date.now() < deadline) await new Promise((r) => setTimeout(r, 500));
  if (running()) {
    console.log("Still running after 15 s: forcing it.");
    out("taskkill.exe", ["/F", "/T", "/IM", EXE], { ...win, ok: true });
    await new Promise((r) => setTimeout(r, 1000));
  }
}

step(`copying to ${targetWin}`);
mkdirSync(dir, { recursive: true });
copyFileSync(built, target);

step("starting typeshelfDev");
// Detached and without our stdio: an inherited pipe would keep this script waiting until the app quits.
spawn("cmd.exe", ["/d", "/c", "start", '""', targetWin], { ...win, detached: true, stdio: "ignore" }).unref();
console.log(`\ntypeshelfDev ${label} is running (${((Date.now() - t0) / 1000).toFixed(0)} s).`);
