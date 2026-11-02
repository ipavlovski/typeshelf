//! Shape metrics measured from glyph outlines (SuperSearch-style filters):
//! x-height, stem weight, width and stroke contrast, each normalised so fonts compare.

use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::{FontRef, GlyphId, MetadataProvider};
use ts_model::Metrics;
use zeno::{Command, Mask, Point};

/// Pixels per em for measuring; large enough that a hairline is several pixels wide.
const PX: f32 = 256.0;

struct Raster {
    w: usize,
    h: usize,
    alpha: Vec<u8>,
}

impl Raster {
    fn ink(&self, x: usize, y: usize) -> bool {
        self.alpha[y * self.w + x] >= 128
    }

    /// Length of the first ink run along a row, from the left.
    fn first_run_in_row(&self, y: usize) -> usize {
        let start = (0..self.w).find(|&x| self.ink(x, y));
        start.map_or(0, |s| (s..self.w).take_while(|&x| self.ink(x, y)).count())
    }

    /// Length of the first ink run down a column, from the top.
    fn first_run_in_col(&self, x: usize) -> usize {
        let start = (0..self.h).find(|&y| self.ink(x, y));
        start.map_or(0, |s| (s..self.h).take_while(|&y| self.ink(x, y)).count())
    }

    fn rows_with_ink(&self) -> Option<(usize, usize)> {
        let rows: Vec<usize> = (0..self.h).filter(|&y| (0..self.w).any(|x| self.ink(x, y))).collect();
        Some((*rows.first()?, *rows.last()?))
    }

    fn cols_with_ink(&self) -> Option<(usize, usize)> {
        let cols: Vec<usize> = (0..self.w).filter(|&x| (0..self.h).any(|y| self.ink(x, y))).collect();
        Some((*cols.first()?, *cols.last()?))
    }
}

struct Pen {
    cmds: Vec<Command>,
    baseline: f32,
}

impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.cmds.push(Command::MoveTo(Point::new(x + PX * 0.5, self.baseline - y)));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.cmds.push(Command::LineTo(Point::new(x + PX * 0.5, self.baseline - y)));
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        let b = self.baseline;
        self.cmds.push(Command::QuadTo(Point::new(cx + PX * 0.5, b - cy), Point::new(x + PX * 0.5, b - y)));
    }
    fn curve_to(&mut self, a: f32, b_: f32, c: f32, d: f32, x: f32, y: f32) {
        let b = self.baseline;
        self.cmds.push(Command::CurveTo(
            Point::new(a + PX * 0.5, b - b_),
            Point::new(c + PX * 0.5, b - d),
            Point::new(x + PX * 0.5, b - y),
        ));
    }
    fn close(&mut self) {
        self.cmds.push(Command::Close);
    }
}

fn raster(font: &FontRef, ch: char) -> Option<Raster> {
    let gid: GlyphId = font.charmap().map(ch)?;
    let glyph = font.outline_glyphs().get(gid)?;
    let (w, h) = ((PX * 2.0) as usize, (PX * 2.0) as usize);
    let mut pen = Pen { cmds: Vec::new(), baseline: PX * 1.5 };
    glyph.draw(DrawSettings::unhinted(Size::new(PX), LocationRef::default()), &mut pen).ok()?;
    if pen.cmds.is_empty() {
        return None;
    }
    let mut alpha = vec![0u8; w * h];
    Mask::new(&pen.cmds[..]).size(w as u32, h as u32).render_into(&mut alpha, None);
    Some(Raster { w, h, alpha })
}

/// Ink height of a glyph above the baseline, in pixels at [`PX`].
fn top(font: &FontRef, ch: char) -> Option<f32> {
    let r = raster(font, ch)?;
    let (y0, _) = r.rows_with_ink()?;
    Some(PX * 1.5 - y0 as f32)
}

pub fn measure(font: &FontRef) -> Option<Metrics> {
    let cap = top(font, 'H').or_else(|| top(font, 'I'))?;
    if cap < PX * 0.2 {
        return None;
    }
    let x_height = top(font, 'x').map(|x| (x / cap).clamp(0.0, 1.5));

    // Stem thickness across the middle of a straight vertical stroke.
    let weight = ['l', 'I', 'H'].iter().find_map(|&c| {
        let r = raster(font, c)?;
        let (y0, y1) = r.rows_with_ink()?;
        let run = r.first_run_in_row((y0 + y1) / 2);
        (run > 0).then(|| (run as f32 / cap).clamp(0.0, 1.0))
    });

    // Average advance of the alphabet the font has, relative to cap height.
    let advances = font.glyph_metrics(Size::new(PX), LocationRef::default());
    let charmap = font.charmap();
    let widths: Vec<f32> = ('a'..='z')
        .chain('A'..='Z')
        .filter_map(|c| advances.advance_width(charmap.map(c)?))
        .filter(|w| *w > 0.0)
        .collect();
    let width = (!widths.is_empty()).then(|| widths.iter().sum::<f32>() / widths.len() as f32 / cap);

    // Thick (vertical, measured across a row) vs thin (horizontal, down a column) strokes of an O.
    let contrast = ['O', 'o'].iter().find_map(|&c| {
        let r = raster(font, c)?;
        let (y0, y1) = r.rows_with_ink()?;
        let (x0, x1) = r.cols_with_ink()?;
        let thick = r.first_run_in_row((y0 + y1) / 2) as f32;
        let thin = r.first_run_in_col((x0 + x1) / 2) as f32;
        (thick > 0.0 && thin > 0.0).then(|| (1.0 - thin.min(thick) / thick.max(thin)).clamp(0.0, 1.0))
    });

    Some(Metrics { x_height, weight, width, contrast })
}
