# typeshelf

A fast, free font manager: index a large font library, preview fonts without
installing them, and activate them per session or pinned. Windows first, then
macOS and Linux. MIT licensed.

> `typeshelf` is a working name.

## Layout

| Path | What |
|---|---|
| `crates/ts-model` | Shared types (`FaceKey`, `FaceInfo`, Unicode block coverage) |
| `crates/ts-store` | SQLite schema, migrations and queries |
| `crates/ts-index` | Folder walking, hashing, font parsing, name cleanup, sample text, watcher |
| `crates/ts-activate` | Crash-safe activation: journal, reconcile, per-OS backends (Windows so far) |
| `crates/ts-cli` | `typeshelf` command-line tool |
| `fixtures/` | Tiny generated test fonts (`python3 tools/make_fixtures.py fixtures`) |

## CLI

```sh
cargo run --release -p ts-cli -- index ~/Fonts          # register + index a watched folder
cargo run --release -p ts-cli -- index --root ~/Library  # register the managed Root folder
cargo run --release -p ts-cli -- stats
cargo run --release -p ts-cli -- dupes
cargo run --release -p ts-cli -- watch

typeshelf activate "Fixture Sans"        # session: gone at next logon
typeshelf activate "Brandon" --pin       # pinned: survives reboots
typeshelf deactivate "Brandon"           # release one holder
typeshelf deactivate --all-session
typeshelf active
typeshelf reconcile [--logon]            # repair after a crash / clear last session
typeshelf autostart on|off|status        # run `reconcile --logon` at every logon
```

### How activation works on Windows

Activating writes a per-user value under
`HKCU\Software\Microsoft\Windows NT\CurrentVersion\Fonts` (tagged `[typeshelf <hash>]`) and
broadcasts `WM_FONTCHANGE`; Windows' font cache makes the font visible to running apps within
~200 ms. Deactivating removes the value and calls `RemoveFontResourceExW` in parallel, because a
font an app has loaded stays in the session until then. Every change is journaled in SQLite
first, so `reconcile` can roll back or finish anything a crash interrupted.

Measured on the author's machine, 300 fonts: activate 34 ms, deactivate 0.3-0.8 s (GDI removal).

The database lives in the per-user data directory (`%LOCALAPPDATA%\typeshelf\data\library.db`
on Windows); override with `--db` or `TYPESHELF_DB`.

## Building for Windows from WSL

```sh
cargo build --release -p ts-cli --target x86_64-pc-windows-gnu   # mingw, no extra setup
cargo xwin build --release -p ts-cli --target x86_64-pc-windows-msvc  # needs clang, lld and llvm
```
