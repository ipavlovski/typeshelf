//! Google Fonts: the public catalogue, whole-family downloads, and tiny TrueType
//! subsets for previewing a family without downloading it.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

const CATALOGUE_URL: &str = "https://fonts.google.com/metadata/fonts";
const CATALOGUE_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);
const MAX_FONT_BYTES: u64 = 100 << 20;
/// Google serves TrueType (instead of WOFF2) only to very old browsers.
const TRUETYPE_UA: &str = "Mozilla/4.0";

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(60)))
            .user_agent(concat!("typeshelf/", env!("CARGO_PKG_VERSION")))
            .build()
            .into()
    })
}

fn get_bytes(url: &str, user_agent: Option<&str>) -> Result<Vec<u8>> {
    let mut req = agent().get(url);
    if let Some(ua) = user_agent {
        req = req.header("User-Agent", ua);
    }
    let mut res = req.call().with_context(|| format!("GET {url}"))?;
    Ok(res.body_mut().with_config().limit(MAX_FONT_BYTES).read_to_vec()?)
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Family {
    pub family: String,
    pub category: String,
    #[serde(default)]
    pub subsets: Vec<String>,
    /// Style keys like "400", "700i".
    #[serde(default)]
    pub fonts: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    pub axes: Vec<CatalogueAxis>,
    #[serde(default)]
    pub popularity: u32,
    #[serde(default)]
    pub designers: Vec<String>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CatalogueAxis {
    pub tag: String,
    pub min: f32,
    pub max: f32,
}

impl Family {
    pub fn styles(&self) -> usize {
        self.fonts.len()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Catalogue {
    family_metadata_list: Vec<Family>,
}

/// The catalogue, from `cache` when it is younger than a week, otherwise fetched
/// and cached. Falls back to a stale cache when offline.
pub fn catalogue(cache: &Path) -> Result<Vec<Family>> {
    let fresh = std::fs::metadata(cache)
        .and_then(|m| m.modified())
        .is_ok_and(|t| SystemTime::now().duration_since(t).unwrap_or_default() < CATALOGUE_MAX_AGE);
    let bytes = if fresh {
        std::fs::read(cache)?
    } else {
        match get_bytes(CATALOGUE_URL, None) {
            Ok(b) => {
                if let Some(dir) = cache.parent() {
                    std::fs::create_dir_all(dir)?;
                }
                std::fs::write(cache, &b)?;
                b
            }
            Err(e) => std::fs::read(cache).map_err(|_| e)?,
        }
    };
    let mut list =
        serde_json::from_slice::<Catalogue>(&bytes).context("parsing the Google Fonts catalogue")?.family_metadata_list;
    list.sort_by(|a, b| a.family.cmp(&b.family));
    Ok(list)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DownloadList {
    manifest: Manifest,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    file_refs: Vec<FileRef>,
}

#[derive(Deserialize)]
struct FileRef {
    filename: String,
    url: String,
}

/// Downloads a family's font files into `dest`. Variable fonts are preferred over
/// their static instances. Returns the written paths; existing files are kept.
pub fn download_family(family: &str, dest: &Path) -> Result<Vec<PathBuf>> {
    let url = format!("https://fonts.google.com/download/list?family={}", encode(family));
    let raw = String::from_utf8(get_bytes(&url, None)?)?;
    // The response starts with an XSSI guard line: )]}'
    let json = raw.split_once('\n').map_or(raw.as_str(), |(_, rest)| rest);
    let list: DownloadList = serde_json::from_str(json).with_context(|| format!("no download list for {family}"))?;
    let fonts: Vec<&FileRef> = list
        .manifest
        .file_refs
        .iter()
        .filter(|f| f.filename.ends_with(".ttf") || f.filename.ends_with(".otf"))
        .collect();
    if fonts.is_empty() {
        bail!("{family} has no downloadable font files");
    }
    let has_variable = fonts.iter().any(|f| !f.filename.contains('/'));
    std::fs::create_dir_all(dest)?;
    let mut out = Vec::new();
    for f in fonts.iter().filter(|f| !has_variable || !f.filename.contains('/')) {
        let name = f.filename.rsplit('/').next().unwrap_or(&f.filename);
        out.push(download_to(&f.url, &dest.join(name))?);
    }
    Ok(out)
}

/// Downloads one font file to `path` (kept if it already exists).
pub fn download_to(url: &str, path: &Path) -> Result<PathBuf> {
    if !path.exists() {
        let bytes = get_bytes(url, None)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("part");
        std::fs::write(&tmp, &bytes)?;
        std::fs::rename(&tmp, path)?;
    }
    Ok(path.to_owned())
}

/// Where provider fonts are stored inside the Root folder, mirroring FontBase's layout.
pub fn google_dir(root: &Path) -> PathBuf {
    root.join("Providers").join("Google")
}

/// Downloads `(url, path)` pairs on a few threads. `progress` gets `(done, total)`.
/// Returns the failures as `(url, error)`.
pub fn download_many(items: &[(String, PathBuf)], progress: &(dyn Fn(usize, usize) + Sync)) -> Vec<(String, String)> {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let failed = Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..8 {
            s.spawn(|| {
                while let Some((url, path)) = items.get(next.fetch_add(1, Ordering::Relaxed)) {
                    if let Err(e) = download_to(url, path) {
                        failed.lock().unwrap().push((url.clone(), format!("{e:#}")));
                    }
                    progress(done.fetch_add(1, Ordering::Relaxed) + 1, items.len());
                }
            });
        }
    });
    failed.into_inner().unwrap()
}

/// A TrueType subset of `family` containing only the characters of `text`,
/// a few KB, for previews before the family is downloaded.
pub fn preview_subset(family: &str, text: &str) -> Result<Vec<u8>> {
    let css_url = format!("https://fonts.googleapis.com/css2?family={}&text={}", encode(family), encode(text));
    let css = String::from_utf8(get_bytes(&css_url, Some(TRUETYPE_UA))?)?;
    let font_url = css
        .split("url(")
        .nth(1)
        .and_then(|s| s.split(')').next())
        .with_context(|| format!("no font URL for {family}"))?;
    get_bytes(font_url, None)
}

/// Percent-encodes a query parameter value.
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_query_values() {
        assert_eq!(encode("Roboto Flex"), "Roboto%20Flex");
        assert_eq!(encode("Ж&"), "%D0%96%26");
    }

    #[test]
    fn parses_catalogue_entries() {
        let json = r#"{"familyMetadataList":[{"family":"Joan","category":"Serif","subsets":["latin"],
            "fonts":{"400":{}},"axes":[],"popularity":900,"designers":["Paolo Biagini"],"extra":1}]}"#;
        let c: Catalogue = serde_json::from_str(json).unwrap();
        assert_eq!(c.family_metadata_list[0].family, "Joan");
        assert_eq!(c.family_metadata_list[0].styles(), 1);
    }

    /// Network test: `TS_NET=1 cargo test -p ts-providers`.
    #[test]
    fn live_google_fonts() {
        if std::env::var_os("TS_NET").is_none() {
            return;
        }
        let dir = std::env::temp_dir().join("ts-providers-test");
        let list = catalogue(&dir.join("catalogue.json")).unwrap();
        assert!(list.len() > 1000);
        let subset = preview_subset("Joan", "Hello").unwrap();
        assert_eq!(&subset[..4], &[0, 1, 0, 0]);
        let files = download_family("Joan", &dir).unwrap();
        assert!(files[0].exists());
    }
}
