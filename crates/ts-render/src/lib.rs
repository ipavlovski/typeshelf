//! Font previews: thumbnails cached on disk as WebP alpha masks, and live renders
//! for custom text. Fonts are read into memory (not memory-mapped, so Windows does
//! not lock the files) and kept in an LRU.

mod pool;
mod raster;

use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use lru::LruCache;
use ts_model::FaceKey;

pub use pool::{CancelToken, Pool};
pub use raster::{Image, LineStyle, LoadedFont, render_line};

/// Thumbnails are capped at this width; the list never shows more.
pub const THUMB_MAX_WIDTH: u32 = 2400;
const LOADED_FONTS: usize = 200;

pub struct Renderer {
    cache_dir: PathBuf,
    fonts: Mutex<LruCache<FaceKey, Arc<LoadedFont>>>,
}

/// What to draw for one face.
#[derive(Clone, Debug)]
pub struct Request<'a> {
    pub key: FaceKey,
    pub path: &'a Path,
    pub text: &'a str,
    pub size: u32,
}

impl Renderer {
    pub fn new(cache_dir: impl Into<PathBuf>) -> Result<Self> {
        let cache_dir = cache_dir.into();
        std::fs::create_dir_all(&cache_dir).with_context(|| format!("creating {}", cache_dir.display()))?;
        Ok(Self { cache_dir, fonts: Mutex::new(LruCache::new(NonZeroUsize::new(LOADED_FONTS).unwrap())) })
    }

    pub fn font(&self, key: FaceKey, path: &Path) -> Result<Arc<LoadedFont>> {
        if let Some(f) = self.fonts.lock().unwrap().get(&key) {
            return Ok(f.clone());
        }
        // Loaded outside the lock so slow disks don't serialise the workers.
        let data = Arc::new(std::fs::read(path).with_context(|| format!("reading {}", path.display()))?);
        let font = Arc::new(LoadedFont::new(data, key.index)?);
        self.fonts.lock().unwrap().put(key, font.clone());
        Ok(font)
    }

    pub fn cache_path(&self, req: &Request) -> PathBuf {
        let text = xxhash_rust::xxh3::xxh3_64(req.text.as_bytes());
        self.cache_dir.join(format!("{}-{}-{text:016x}-{}.webp", req.key.hash, req.key.index, req.size))
    }

    /// A thumbnail as WebP bytes, from the disk cache or freshly rendered.
    /// Text the face cannot draw is replaced by a sample it can.
    pub fn thumbnail(&self, req: &Request) -> Result<Vec<u8>> {
        let cached = self.cache_path(req);
        if let Ok(bytes) = std::fs::read(&cached) {
            return Ok(bytes);
        }
        let font = self.font(req.key, req.path)?;
        let text = font.drawable_text(req.text);
        let img = render_line(&font, &text, req.size as f32, THUMB_MAX_WIDTH, &LineStyle::default())?;
        let bytes = encode_webp(&img)?;
        // Write-then-rename so a reader never sees half a file.
        let tmp = cached.with_extension(format!("tmp{:?}", std::thread::current().id()).replace(['(', ')'], ""));
        if std::fs::write(&tmp, &bytes).is_ok() && std::fs::rename(&tmp, &cached).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        Ok(bytes)
    }

    /// A thumbnail of font data that is not in the library (e.g. a Google Fonts preview
    /// subset), cached under `name`. `load` is only called on a cache miss.
    pub fn thumbnail_of(
        &self,
        name: &str,
        text: &str,
        size: u32,
        load: impl FnOnce() -> Result<Vec<u8>>,
    ) -> Result<Vec<u8>> {
        let key = xxhash_rust::xxh3::xxh3_64(format!("{name}\0{text}").as_bytes());
        let cached = self.cache_dir.join(format!("x{key:016x}-{size}.webp"));
        if let Ok(bytes) = std::fs::read(&cached) {
            return Ok(bytes);
        }
        let font = LoadedFont::new(Arc::new(load()?), 0)?;
        let text = font.drawable_text(text);
        let bytes = encode_webp(&render_line(&font, &text, size as f32, THUMB_MAX_WIDTH, &LineStyle::default())?)?;
        let _ = std::fs::write(&cached, &bytes);
        Ok(bytes)
    }

    /// Renders without touching the disk cache, for live sample text and the detail page.
    pub fn live(&self, req: &Request, max_width: u32, style: &LineStyle) -> Result<Vec<u8>> {
        let font = self.font(req.key, req.path)?;
        let text = font.drawable_text(req.text);
        encode_webp(&render_line(&font, &text, req.size as f32, max_width, style)?)
    }
}

/// Lossless WebP with the coverage in the alpha channel, so the UI can tint it
/// with CSS (`mask-image`) in any theme.
pub fn encode_webp(img: &Image) -> Result<Vec<u8>> {
    let la: Vec<u8> = img.alpha.iter().flat_map(|&a| [0, a]).collect();
    let mut out = Vec::new();
    image_webp::WebPEncoder::new(&mut out).encode(&la, img.width, img.height, image_webp::ColorType::La8)?;
    Ok(out)
}

#[cfg(test)]
mod tests;
