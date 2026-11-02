# typeshelf

## After changing code

When a turn changes the app's code (`crates/`, `app/src/`, `app/src-tauri/`, `app/scripts/`, styles), finish by running `pnpm preview` in `app/` — once, at the end of the turn, after the typecheck and tests pass (`pnpm typecheck`, `pnpm test`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`). It builds the working tree as typeshelfDev and restarts it on Windows so the change can be tried right away; it quits a running typeshelfDev first (cleanly, over its local endpoint), and takes a minute or two. Report if it fails.

Skip it when nothing the app runs changed (docs only, notes, git operations).

## Branches and releases

Work happens on `dev`; `main` only moves by releases. From `app/`:

- `pnpm release:dev` tags dev's HEAD as a dev build (vX.Y.Z.N), published as a GitHub pre-release. Installed apps only get it from Settings → Updates and releases.
- `pnpm release [minor|patch|major]` bumps `app/package.json`, fast-forwards `main`, tags vX.Y.Z and pushes. Installed apps download it in the background and install it on quit.

The release workflow (`.github/workflows/release.yml`) builds the signed installer on Windows. The signing key is `~/.tauri/typeshelf.key` (GitHub secret `TAURI_SIGNING_PRIVATE_KEY`); its public half is in `app/src-tauri/tauri.conf.json`. Losing the key means installed apps can no longer update.
