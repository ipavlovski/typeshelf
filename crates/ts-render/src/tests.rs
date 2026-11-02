use std::path::PathBuf;

use ts_model::{FaceKey, FileHash};

use super::*;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures").join(name)
}

fn key(n: u64, index: u32) -> FaceKey {
    FaceKey { hash: FileHash(n), index }
}

fn ink(img: &Image) -> usize {
    img.alpha.iter().filter(|&&a| a > 0).count()
}

fn load(name: &str, index: u32) -> LoadedFont {
    LoadedFont::new(Arc::new(std::fs::read(fixture(name)).unwrap()), index).unwrap()
}

#[test]
fn renders_ttf_cff_and_collections() {
    for (file, index) in [("FixtureSans-Light.ttf", 0), ("FixtureSans-Italic.otf", 0), ("FixtureDuo.ttc", 1)] {
        let font = load(file, index);
        let img = render_line(&font, "Abc", 32.0, 1000, &LineStyle::default()).unwrap();
        assert_eq!(img.height, 45, "{file}");
        assert!(ink(&img) > 100, "{file} drew nothing");
    }
}

#[test]
fn width_is_capped() {
    let font = load("FixtureSans-Light.ttf", 0);
    let img = render_line(&font, &"W".repeat(200), 64.0, 300, &LineStyle::default()).unwrap();
    assert_eq!(img.width, 300);
}

#[test]
fn ligature_feature_changes_the_shaping() {
    // The fixture's f_i ligature is one 600-unit glyph instead of two.
    let font = load("FixtureSans-Light.ttf", 0);
    let on = render_line(&font, "fi", 10.0, 1000, &LineStyle::default()).unwrap();
    let off = LineStyle { features: vec![("liga".into(), 0)], ..Default::default() };
    let off = render_line(&font, "fi", 10.0, 1000, &off).unwrap();
    assert!(on.width < off.width, "liga on {} vs off {}", on.width, off.width);
}

#[test]
fn uncovered_text_falls_back_to_drawable_sample() {
    let caps = load("FixtureCaps-Regular.otf", 0);
    assert_eq!(caps.drawable_text("Hello"), "HELLO");
    let dingbats = load("FixtureDingbats-Regular.ttf", 0);
    assert!(!dingbats.covers("Hello"));
    assert!(dingbats.drawable_text("Hello").starts_with('\u{2701}'));
}

#[test]
fn thumbnails_are_cached_webp() {
    let dir = tempfile::tempdir().unwrap();
    let r = Renderer::new(dir.path()).unwrap();
    let path = fixture("FixtureCyr-Regular.ttf");
    let req = Request { key: key(7, 0), path: &path, text: "The quick brown fox", size: 32 };
    let bytes = r.thumbnail(&req).unwrap();
    assert_eq!(&bytes[..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WEBP");
    assert!(r.cache_path(&req).exists());

    // A second call is served from disk, even with the font file gone.
    let moved = dir.path().join("gone.ttf");
    let req2 = Request { path: &moved, ..req.clone() };
    assert_eq!(r.thumbnail(&req2).unwrap(), bytes);
}

#[test]
fn live_render_applies_variations() {
    let dir = tempfile::tempdir().unwrap();
    let r = Renderer::new(dir.path()).unwrap();
    let path = fixture("FixtureVar[wght].ttf");
    let req = Request { key: key(9, 0), path: &path, text: "Abc", size: 24 };
    let style = LineStyle { variations: vec![("wght".into(), 900.0)], ..Default::default() };
    assert!(!r.live(&req, 800, &style).unwrap().is_empty());
}
