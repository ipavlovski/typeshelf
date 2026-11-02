//! Updates from the GitHub Releases of the public repo (published by
//! .github/workflows/release.yml, signed with the key in tauri.conf.json).
//!
//! The background check follows releases only: GitHub's "latest" never points at a
//! pre-release, and dev builds are pre-releases. A dev build is installed only when
//! picked in the Updates window, which points the updater at that release's
//! latest.json for one check. Updates download in the background and install when
//! the app quits, or right away from "Restart to install".

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State, Url};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::channel;

pub const GITHUB_OWNER: &str = "ipavlovski";
pub const GITHUB_REPO: &str = "typeshelf";
const FIRST_CHECK: Duration = Duration::from_secs(10);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);

#[derive(Serialize, Clone, Default, Debug)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum UpdateStatus {
    #[default]
    None,
    Checking,
    Downloading {
        version: String,
        percent: u32,
    },
    Ready {
        version: String,
    },
    Error {
        message: String,
    },
}

#[derive(Default)]
pub struct Updates {
    status: Mutex<UpdateStatus>,
    /// A downloaded update, installed on quit or by "Restart to install".
    pending: Mutex<Option<(Update, Vec<u8>)>>,
    busy: AtomicBool,
}

fn set(app: &AppHandle, s: UpdateStatus) {
    *app.state::<Updates>().status.lock().unwrap() = s.clone();
    let _ = app.emit("update-status", s);
}

/// Whether this build updates itself: only the installed release, not typeshelfDev or `cargo run`.
fn enabled() -> bool {
    !channel::IS_DEV && !cfg!(debug_assertions)
}

pub fn init(app: &AppHandle) {
    app.manage(Updates::default());
    if !enabled() {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(FIRST_CHECK);
        loop {
            let _ = tauri::async_runtime::block_on(check(&app, None));
            std::thread::sleep(CHECK_EVERY);
        }
    });
}

fn release_url(tag: &str) -> String {
    format!("https://github.com/{GITHUB_OWNER}/{GITHUB_REPO}/releases/download/{tag}/latest.json")
}

/// Checks for (and downloads) an update: the latest release, or the release `tag`.
/// Returns the version downloaded, if any.
async fn check(app: &AppHandle, tag: Option<&str>) -> Result<Option<String>, String> {
    let updates = app.state::<Updates>();
    if updates.busy.swap(true, Ordering::SeqCst) {
        return Err("An update check is already running.".into());
    }
    let result = run_check(app, tag).await;
    updates.busy.store(false, Ordering::SeqCst);
    if let Err(message) = &result {
        set(app, UpdateStatus::Error { message: message.clone() });
    }
    result
}

async fn run_check(app: &AppHandle, tag: Option<&str>) -> Result<Option<String>, String> {
    set(app, UpdateStatus::Checking);
    let mut builder = app.updater_builder();
    if let Some(tag) = tag {
        let url = Url::parse(&release_url(tag)).map_err(|e| e.to_string())?;
        builder = builder.endpoints(vec![url]).map_err(|e| e.to_string())?;
    }
    let updater = builder.build().map_err(|e| e.to_string())?;
    let Some(update) = updater.check().await.map_err(|e| e.to_string())? else {
        // Nothing newer; a download already waiting stays ready.
        let ready = app.state::<Updates>().pending.lock().unwrap().as_ref().map(|(u, _)| u.version.clone());
        set(app, ready.map_or(UpdateStatus::None, |version| UpdateStatus::Ready { version }));
        return Ok(None);
    };
    let version = update.version.clone();
    let (mut done, mut shown) = (0u64, u32::MAX);
    let bytes = update
        .download(
            |chunk, total| {
                done += chunk as u64;
                let percent = total.map_or(0, |t| (done * 100 / t.max(1)) as u32);
                if percent != shown {
                    shown = percent;
                    set(app, UpdateStatus::Downloading { version: version.clone(), percent });
                }
            },
            || {},
        )
        .await
        .map_err(|e| e.to_string())?;
    *app.state::<Updates>().pending.lock().unwrap() = Some((update, bytes));
    set(app, UpdateStatus::Ready { version: version.clone() });
    Ok(Some(version))
}

/// Installs a downloaded update. On Windows the installer runs and restarts the app.
fn install(app: &AppHandle) -> Result<(), String> {
    let Some((update, bytes)) = app.state::<Updates>().pending.lock().unwrap().take() else {
        return Err("No update has been downloaded.".into());
    };
    update.install(bytes).map_err(|e| e.to_string())
}

/// Called as the app exits: a downloaded update installs instead of waiting for "Restart".
pub fn install_on_exit(app: &AppHandle) {
    if app.try_state::<Updates>().is_some_and(|u| u.pending.lock().unwrap().is_some()) {
        let _ = install(app);
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    /// "release" or "dev" (typeshelfDev).
    pub channel: &'static str,
    pub label: &'static str,
    pub updates: bool,
}

#[tauri::command]
pub fn app_info(app: AppHandle) -> AppInfo {
    AppInfo {
        version: app.package_info().version.to_string(),
        channel: if channel::IS_DEV { "dev" } else { "release" },
        label: channel::BUILD_LABEL,
        updates: enabled(),
    }
}

#[tauri::command]
pub fn update_status(updates: State<'_, Updates>) -> UpdateStatus {
    updates.status.lock().unwrap().clone()
}

fn not_updatable() -> String {
    if channel::IS_DEV {
        "typeshelfDev is updated with pnpm preview, not from GitHub.".into()
    } else {
        "Updates are only installed by the released app.".into()
    }
}

/// "Check for updates" in Settings. Returns the version downloaded, or `None` when up to date.
#[tauri::command]
pub async fn check_for_updates(app: AppHandle) -> Result<Option<String>, String> {
    if !enabled() {
        return Err(not_updatable());
    }
    check(&app, None).await
}

/// Downloads one release (a dev build, say) from the Updates window.
#[tauri::command]
pub async fn install_release(app: AppHandle, tag: String) -> Result<Option<String>, String> {
    if !enabled() {
        return Err(not_updatable());
    }
    check(&app, Some(&tag)).await
}

#[tauri::command]
pub fn install_update(app: AppHandle) -> Result<(), String> {
    install(&app)?;
    app.restart()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    pub tag: String,
    /// App version: 0.2.0, or 0.2.0-dev.3 for the dev build v0.2.0.3.
    pub version: String,
    pub dev: bool,
    pub title: String,
    /// Markdown release notes (scripts/lib/notes.ts).
    pub notes: String,
    pub published_at: String,
    pub url: String,
}

/// v0.2.0 → 0.2.0, v0.2.0.3 → 0.2.0-dev.3; `None` for other tags (as scripts/versions.ts).
pub fn tag_version(tag: &str) -> Option<String> {
    let parts: Vec<&str> = tag.strip_prefix('v')?.split('.').collect();
    if !(3..=4).contains(&parts.len()) || parts.iter().any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    let release = parts[..3].join(".");
    Some(match parts.get(3) {
        Some(n) => format!("{release}-dev.{n}"),
        None => release,
    })
}

/// The app's GitHub releases, newest first (drafts and other tags left out).
#[tauri::command]
pub async fn list_releases() -> Result<Vec<Release>, String> {
    tauri::async_runtime::spawn_blocking(fetch_releases).await.map_err(|e| e.to_string())?
}

fn fetch_releases() -> Result<Vec<Release>, String> {
    let url = format!("https://api.github.com/repos/{GITHUB_OWNER}/{GITHUB_REPO}/releases?per_page=50");
    let mut res = ureq::get(&url)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "typeshelf")
        .call()
        .map_err(|e| match e {
            ureq::Error::StatusCode(403) => "GitHub rate limit reached; try again in a while.".to_owned(),
            e => format!("GitHub: {e}"),
        })?;
    let body = res.body_mut().read_to_string().map_err(|e| e.to_string())?;
    let list: Vec<serde_json::Value> = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let text = |r: &serde_json::Value, k: &str| r.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_owned();
    Ok(list
        .iter()
        .filter(|r| !r.get("draft").and_then(|d| d.as_bool()).unwrap_or(false))
        .filter_map(|r| {
            let tag = text(r, "tag_name");
            let version = tag_version(&tag)?;
            let title = Some(text(r, "name")).filter(|n| !n.is_empty()).unwrap_or_else(|| tag.clone());
            Some(Release {
                dev: version.contains('-'),
                version,
                title,
                notes: text(r, "body"),
                published_at: Some(text(r, "published_at"))
                    .filter(|p| !p.is_empty())
                    .unwrap_or_else(|| text(r, "created_at")),
                url: text(r, "html_url"),
                tag,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_versions() {
        assert_eq!(tag_version("v0.2.0").as_deref(), Some("0.2.0"));
        assert_eq!(tag_version("v0.2.0.3").as_deref(), Some("0.2.0-dev.3"));
        assert_eq!(tag_version("v1.2"), None);
        assert_eq!(tag_version("nightly"), None);
        assert_eq!(tag_version("v1.2.x"), None);
    }
}
