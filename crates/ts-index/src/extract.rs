//! Reads one font file into [`FaceInfo`]s.

use anyhow::{Context, Result};
use read_fonts::{FileRef, FontRef, TableProvider, types::Tag};
use skrifa::{MetadataProvider, string::StringId};
use ts_model::{Axis, Coverage, FaceInfo, FontFormat};

use crate::names::{clean, sanitize_family};
use crate::sample::{DEFAULT_SAMPLE, pick_sample};

const LICENSE_MAX: usize = 300;

/// Parses every face in a font file. `stem` is the file name without extension,
/// used as the last-resort family name.
pub fn extract(bytes: &[u8], stem: &str) -> Result<Vec<FaceInfo>> {
    let container = FontFormat::sniff(bytes).context("not a font file")?;
    let file = FileRef::new(bytes).context("unreadable font header")?;
    let mut faces = Vec::new();
    for (index, font) in file.fonts().enumerate() {
        let font = font.with_context(|| format!("face {index}"))?;
        faces.push(face(&font, index as u32, container, stem));
    }
    anyhow::ensure!(!faces.is_empty(), "collection has no faces");
    Ok(faces)
}

fn face(font: &FontRef, index: u32, container: FontFormat, stem: &str) -> FaceInfo {
    let name = |id: u16| -> Option<String> {
        let s = font.localized_strings(StringId::new(id)).english_or_first()?.to_string();
        let s = clean(&s);
        (!s.is_empty()).then_some(s)
    };
    let family_raw = name(16).or_else(|| name(1)).unwrap_or_default();
    let full_name = name(4).unwrap_or_default();
    let ps_name = name(6).unwrap_or_default();
    let family = sanitize_family(&family_raw, &[&full_name, &ps_name], stem);
    let subfamily = name(17).or_else(|| name(2)).unwrap_or_else(|| "Regular".into());
    let mut license = name(13).unwrap_or_default();
    if let Some((cut, _)) = license.char_indices().nth(LICENSE_MAX) {
        license.truncate(cut);
    }

    let format = match container {
        FontFormat::Ttc => FontFormat::Ttc,
        _ if font.table_data(Tag::new(b"CFF ")).is_some() || font.table_data(Tag::new(b"CFF2")).is_some() => {
            FontFormat::Otf
        }
        _ => FontFormat::Ttf,
    };

    let os2 = font.os2().ok();
    let head = font.head().ok();
    let mac_italic = head.as_ref().is_some_and(|h| h.mac_style().bits() & 0x2 != 0);
    let italic = match &os2 {
        // ITALIC (bit 0) or OBLIQUE (bit 9).
        Some(os2) => os2.fs_selection().bits() & 0x0201 != 0 || mac_italic,
        None => mac_italic,
    };

    let axes = font
        .axes()
        .iter()
        .map(|a| Axis { tag: a.tag().to_string(), min: a.min_value(), default: a.default_value(), max: a.max_value() })
        .collect();

    let mut features: Vec<String> = Vec::new();
    if let Ok(list) = font.gsub().and_then(|t| t.feature_list()) {
        features.extend(list.feature_records().iter().map(|r| r.feature_tag().to_string()));
    }
    if let Ok(list) = font.gpos().and_then(|t| t.feature_list()) {
        features.extend(list.feature_records().iter().map(|r| r.feature_tag().to_string()));
    }
    features.sort_unstable();
    features.dedup();

    let charmap = font.charmap();
    let coverage = Coverage::from_codepoints(charmap.mappings().map(|(cp, _)| cp));
    let sample = pick_sample(DEFAULT_SAMPLE, |c| charmap.map(c).is_some(), charmap.mappings().map(|(cp, _)| cp));

    let metrics = crate::metrics::measure(font).unwrap_or_default();

    FaceInfo {
        index,
        family,
        family_raw,
        subfamily,
        full_name,
        ps_name,
        weight: os2.as_ref().map_or(400, |t| t.us_weight_class()),
        width: os2.as_ref().map_or(5, |t| t.us_width_class()),
        italic,
        format,
        glyph_count: font.maxp().map_or(0, |m| m.num_glyphs()),
        units_per_em: head.as_ref().map_or(1000, |h| h.units_per_em()),
        x_height: os2.as_ref().and_then(|t| t.sx_height()).filter(|&v| v > 0),
        cap_height: os2.as_ref().and_then(|t| t.s_cap_height()).filter(|&v| v > 0),
        axes,
        features,
        coverage,
        vendor: os2.as_ref().map(|t| clean(&t.ach_vend_id().to_string())).unwrap_or_default(),
        license,
        sample,
        metrics,
    }
}
