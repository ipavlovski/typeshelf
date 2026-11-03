# typeshelf

A fast, free font manager. It indexes a large font library, previews fonts without
installing them, and activates them for the current session or permanently ("pinned").
It runs on Windows first, with macOS and Linux support. MIT licensed.

> `typeshelf` is a working name.

## What it does

- **Library**: a managed Root folder plus watched folders. Files are indexed in place,
  duplicates are detected by content hash, and folders are re-indexed incrementally when
  their files change.
- **Previews**: one line per family, rendered in Rust (harfrust shaping, skrifa outlines)
  and cached as WebP. The sample text is always one the font can draw: caps-only, Cyrillic
  and symbol fonts never show tofu boxes.
- **Search**: fuzzy names with typo tolerance, plus filters: `script:cyrillic feat:ss01
  wght:>600 col:"2. SANS SERIF" is:active -is:italic folder:archive`.
- **Shape filters ("SuperSearch")**: x-height, stem weight, width and contrast are
  measured from the glyph outlines. They work as range sliders, or in queries as
  `contrast:0.5..1`.
- **Activation**: crash-safe. Every change is journaled first, and a reconcile step at
  launch and at logon repairs whatever a crash or power cut left behind. Session fonts
  disappear at logoff; pinned fonts survive reboots.
- **Collections**: nested, activate a whole branch with one click, favourites.
- **Detail page**: waterfall, OpenType feature toggles, variable-axis sliders and a glyph
  grid you can copy from.
- **FontBase import**: brings over the Root folder, nested collections, favourites,
  active fonts and sample texts. Collection members FontBase never downloaded (Google
  Fonts) are fetched and added.
- **Google Fonts**: browse the catalogue with previews before downloading, then add a
  family (or add and activate it) in one click.
- **Adobe auto-activation**: a UXP panel for Photoshop and InDesign activates the fonts an
  opened document needs (`plugins/adobe-uxp`).
- **MCP server**: Claude (or any MCP client) can browse the library, look at rendered
  specimens and tag families. See [Claude and other MCP clients](#claude-and-other-mcp-clients).

## Layout

| Path | What |
|---|---|
| `crates/ts-model` | Shared types (`FaceKey`, `FaceInfo`, `Metrics`, Unicode block coverage) |
| `crates/ts-store` | SQLite schema and migrations, activation journal, collections, pending members |
| `crates/ts-index` | Folder walking, hashing, parsing, name cleanup, sample text, shape metrics, watcher |
| `crates/ts-search` | In-memory snapshot: fuzzy matching, roaring-bitmap facets, range columns |
| `crates/ts-render` | Shaping and rasterising, thumbnail cache, render pool |
| `crates/ts-activate` | Journal and reconcile; backends for Windows (registry), macOS (CoreText) and Linux (fontconfig) |
| `crates/ts-import-fontbase` | LevelDB and V8 decoders for FontBase's IndexedDB; import plan |
| `crates/ts-providers` | Google Fonts catalogue, downloads, preview subsets |
| `crates/ts-cli` | `typeshelf` command-line tool |
| `app/` | Tauri 2 desktop app (Rust backend in `src-tauri`, React UI in `src`) |
| `plugins/adobe-uxp` | Photoshop and InDesign plugin |
| `fixtures/` | Tiny generated test fonts (`python3 tools/make_fixtures.py fixtures`) |

## Measured on a 13,378-face library (Windows, mid-range laptop)

| Interaction | Budget | Measured |
|---|---|---|
| First index of 13,816 files | < 2 min | 0.7 s (about 1 s with shape metrics) |
| Re-index, nothing changed | n/a | 0.15 s |
| Search per keystroke | < 16 ms | 0.1 to 3 ms |
| Scroll the full list | 60 fps | 60 fps, no frame over 20 ms |
| Activate 300 fonts | < 300 ms | 34 to 40 ms |
| Deactivate 300 fonts | n/a | 0.3 to 0.8 s (GDI unload, runs off the UI thread) |
| Launch to first results | < 400 ms | 0.6 to 0.9 s, of which WebView2 start-up is 0.4 to 0.65 s |
| Idle memory | < 150 MB | 31 MB app process + about 220 MB WebView2 |

The launch and memory budgets are not met. Both overshoots come from WebView2 itself.

## CLI

```sh
typeshelf index ~/Fonts                    # register and index a watched folder
typeshelf index --root ~/Library           # register the managed Root folder
typeshelf search garamond script:cyrillic
typeshelf activate "Garamond" [--pin]
typeshelf deactivate "Garamond" | --all-session
typeshelf reconcile [--logon]
typeshelf autostart on|off|status          # run `reconcile --logon` at every logon
typeshelf import-fontbase [--dry-run] [--download] [--activate session|pinned]
typeshelf google "roboto" [--get]
typeshelf thumbs                           # pre-render every thumbnail (optional)
```

The database lives in the per-user data folder (`%LOCALAPPDATA%\typeshelf\data` on
Windows). Use `--db` or `TYPESHELF_DB` to point at another one.

## Claude and other MCP clients

The running app is an MCP server. `typeshelf.exe --mcp` speaks MCP over stdio and relays to
it (starting the app if it isn't running), so any client that can launch a program can use it,
including Claude Code inside WSL:

```sh
# Windows
claude mcp add typeshelf -- "%LOCALAPPDATA%\typeshelf\typeshelf.exe" --mcp
# WSL (typeshelfDev from `pnpm preview`; it has its own library)
claude mcp add typeshelf-dev -- "$(wslpath "$(cmd.exe /c 'echo %LOCALAPPDATA%' 2>/dev/null | tr -d '\r')")/Programs/typeshelfDev/typeshelfDev.exe" --mcp
```

Tools: `library_overview` (tag tree, collections, how much is untagged), `list_families`
(paged, with the full search syntax, e.g. `is:untagged`), `family_details`,
`preview_families` (a specimen sheet image), `tag_families` (add / remove / replace,
batched), `rename_tag`, `delete_tag`. Changes show up in the open window immediately.

Under the hood the messages are JSON-RPC on the local WebSocket endpoint
(`127.0.0.1:47215`, typeshelfDev `47216`). Connections from web pages (which always send an
`Origin`) are refused.

## How activation works

- **Windows**: writes a per-user value under `HKCU\…\Windows NT\CurrentVersion\Fonts`,
  tagged `[typeshelf <hash>]`, then broadcasts `WM_FONTCHANGE`. Windows' font cache picks
  the font up within about 200 ms. Deactivation also unloads the font from GDI.
  Logoff and shutdown are caught by a hidden window, and a HKCU `Run` entry reconciles
  at the next logon.
- **macOS**: `CTFontManagerRegisterFontsForURL` with session scope for session fonts and
  persistent scope for pinned ones. A LaunchAgent reconciles at login.
- **Linux**: symlinks in `~/.local/share/fonts/typeshelf/{session,pinned}` followed by
  `fc-cache` on that folder. An XDG autostart entry reconciles at login.

Reconcile only ever touches entries typeshelf created.

## Building

Frontend: `cd app && pnpm install && pnpm build`.

From WSL or Linux, cross-compiling for Windows:

```sh
cargo xwin build --release -p ts-cli --target x86_64-pc-windows-msvc
cargo xwin build --release -p typeshelf-app --features custom-protocol --target x86_64-pc-windows-msvc
```

This needs `clang`, `lld`, `llvm` and `cargo-xwin`. To also build the NSIS installer
locally (`pnpm tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc`),
install `libayatana-appindicator` and `nsis` as well.

## Releases and updates

The workflow matches the sibling apps (logcadence, logtracker). Work happens on `dev`; `main`
only moves by releases. From `app/`:

| Command | What it does |
|---|---|
| `pnpm preview` | Builds the working tree as **typeshelfDev** and (re)starts it on Windows, next to the installed typeshelf. It has its own data folder, plugin port, registry tag and logon task. |
| `pnpm release:dev` | Tags dev's HEAD as a dev build `vX.Y.Z.N`, published as a GitHub pre-release. |
| `pnpm release [minor\|patch\|major]` | Bumps `app/package.json`, fast-forwards `main`, and tags and pushes `vX.Y.Z`. |

A pushed tag starts `.github/workflows/release.yml` on `windows-latest`. It checks the
tag against `package.json`, writes release notes from the commits, runs the tests, and
builds the signed NSIS installer plus `latest.json`. It publishes them, along with the CLI
and the UXP plugins, as a GitHub release.

The installed app checks GitHub's latest release 10 s after launch and then every 6 hours.
It downloads the update in the background and installs it on quit, or straight away from
**Settings → Restart to install**. Dev builds are pre-releases, so they are never picked
up automatically: install them from **Settings → Updates and releases**, which lists every
release with its notes.

Updates are signed with `~/.tauri/typeshelf.key` (GitHub secret `TAURI_SIGNING_PRIVATE_KEY`).
The public key is in `app/src-tauri/tauri.conf.json`. Keep a backup of the private key:
without it, installed apps can no longer update.

## Known gaps

- The macOS backend is written against CoreText but is untested. It is compiled and
  tested only in CI.
- The UXP plugin is untested inside Photoshop and InDesign.
- WOFF and WOFF2 files are listed but not indexed yet.
- The glyph grid and the live OpenType preview do not work for `.ttc` collections yet.
- Releases are built for Windows only, like the sibling apps. The macOS and Linux
  backends are covered by CI but are not packaged yet.
