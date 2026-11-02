//! Shapes one line of text with harfrust and rasterises it with zeno into an alpha mask.

use std::sync::Arc;

use anyhow::{Context, Result};
use harfrust::font::Variation;
use harfrust::{Buffer, Feature, ShapeOptions, ShaperFont, Tag};
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::raw::TableProvider;
use skrifa::{FontRef, GlyphId, MetadataProvider};
use zeno::{Command, Mask, Point};

/// A font file loaded once and shared by every render of its faces.
pub struct LoadedFont {
    pub data: Arc<Vec<u8>>,
    pub index: u32,
    shaper: harfrust::Font,
}

impl LoadedFont {
    pub fn new(data: Arc<Vec<u8>>, index: u32) -> Result<Self> {
        let blob: Arc<dyn AsRef<[u8]> + Send + Sync> = data.clone();
        let shaper = harfrust::Font::new(blob, index).context("unsupported font data")?;
        Ok(Self { data, index, shaper })
    }

    pub fn font_ref(&self) -> Result<FontRef<'_>> {
        FontRef::from_index(&self.data, self.index).context("unreadable font")
    }

    pub fn covers(&self, text: &str) -> bool {
        let Ok(font) = self.font_ref() else { return false };
        let cmap = font.charmap();
        text.chars().all(|c| c.is_whitespace() || cmap.map(c).is_some())
    }

    /// Text the face can draw: `preferred` if covered, otherwise a fallback sample.
    pub fn drawable_text(&self, preferred: &str) -> String {
        let Ok(font) = self.font_ref() else { return preferred.to_owned() };
        let cmap = font.charmap();
        ts_index::pick_sample(preferred, |c| cmap.map(c).is_some(), cmap.mappings().map(|(cp, _)| cp))
    }
}

#[derive(Clone, Default, Debug)]
pub struct LineStyle {
    /// OpenType features, e.g. `("ss01", 1)`.
    pub features: Vec<(String, u32)>,
    /// Variable axis values, e.g. `("wght", 700.0)`.
    pub variations: Vec<(String, f32)>,
}

/// 8-bit coverage mask, row-major, `width * height` bytes.
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub alpha: Vec<u8>,
}

/// Row height as a multiple of the font size, so every row in the list lines up.
const LINE_HEIGHT: f32 = 1.4;

/// Renders `text` at `size` pixels per em, clipped to `max_width` pixels.
pub fn render_line(font: &LoadedFont, text: &str, size: f32, max_width: u32, style: &LineStyle) -> Result<Image> {
    let skrifa_font = font.font_ref()?;
    let tag = |s: &str| Tag::new_checked(s.as_bytes()).ok();
    let vars: Vec<(Tag, f32)> = style.variations.iter().filter_map(|(t, v)| Some((tag(t)?, *v))).collect();
    let shaper_font = if vars.is_empty() {
        font.shaper.clone()
    } else {
        font.shaper.instance_builder().variations(vars.iter().map(|&(t, v)| Variation::new(t, v))).build()
    };
    let location = skrifa_font.axes().location(vars.iter().map(|&(t, v)| (t, v)));

    let mut buffer = Buffer::new();
    buffer.push_str(text);
    buffer.guess_segment_properties();
    let features: Vec<Feature> =
        style.features.iter().filter_map(|(t, v)| Some(Feature::new(tag(t)?, *v, ..))).collect();
    let shaper = ShaperFont::new(&shaper_font);
    harfrust::shape(&shaper, &mut buffer, ShapeOptions::new().features(&features)).context("shaping failed")?;

    let upem = skrifa_font.head().map(|h| h.units_per_em()).unwrap_or(1000).max(16) as f32;
    let scale = size / upem;
    let metrics = skrifa_font.metrics(Size::new(size), LocationRef::from(&location));
    // Fonts with wild vertical metrics would otherwise clip or float; keep the
    // ascent/descent box sane and centre it in a fixed-height row.
    let ascent = metrics.ascent.clamp(size * 0.6, size * 1.2);
    let descent = (-metrics.descent).clamp(size * 0.1, size * 0.5);
    let height = (size * LINE_HEIGHT).ceil() as u32;
    let baseline = ((height as f32 - (ascent + descent)) / 2.0 + ascent).round();

    let positions = buffer.glyph_positions();
    let total: f32 = positions.iter().map(|p| p.x_advance as f32 * scale).sum();
    let pad = (size * 0.05).ceil();
    let width = ((total + 2.0 * pad).ceil() as u32).clamp(1, max_width.max(1));

    let outlines = skrifa_font.outline_glyphs();
    let settings_size = Size::new(size);
    let mut pen = PathPen { cmds: Vec::new(), x: 0.0, y: 0.0 };
    let mut x = pad;
    for (info, pos) in buffer.glyph_infos().iter().zip(positions) {
        if x > width as f32 {
            break;
        }
        if let Some(glyph) = outlines.get(GlyphId::new(info.glyph_id)) {
            pen.x = x + pos.x_offset as f32 * scale;
            pen.y = baseline - pos.y_offset as f32 * scale;
            // A broken glyph must not lose the rest of the line.
            let _ = glyph.draw(DrawSettings::unhinted(settings_size, LocationRef::from(&location)), &mut pen);
        }
        x += pos.x_advance as f32 * scale;
    }

    let mut alpha = vec![0u8; (width * height) as usize];
    if !pen.cmds.is_empty() {
        Mask::new(&pen.cmds[..]).size(width, height).render_into(&mut alpha, None);
    }
    Ok(Image { width, height, alpha })
}

/// Collects outline commands in pixel space: y flipped, offset to the glyph origin.
struct PathPen {
    cmds: Vec<Command>,
    x: f32,
    y: f32,
}

impl PathPen {
    fn p(&self, x: f32, y: f32) -> Point {
        Point::new(self.x + x, self.y - y)
    }
}

impl OutlinePen for PathPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.cmds.push(Command::MoveTo(self.p(x, y)));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.cmds.push(Command::LineTo(self.p(x, y)));
    }

    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.cmds.push(Command::QuadTo(self.p(cx, cy), self.p(x, y)));
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.cmds.push(Command::CurveTo(self.p(cx0, cy0), self.p(cx1, cy1), self.p(x, y)));
    }

    fn close(&mut self) {
        self.cmds.push(Command::Close);
    }
}
