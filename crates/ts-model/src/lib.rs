//! Types shared by every typeshelf crate.

mod coverage;

use std::fmt;

pub use coverage::{Block, Coverage};

/// xxh3-64 of a font file's bytes. Identifies the content, not the path, so
/// it detects duplicates and survives moves.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct FileHash(pub u64);

impl FileHash {
    /// SQLite stores integers as i64; the bit pattern is kept as-is.
    pub fn to_sql(self) -> i64 {
        self.0 as i64
    }

    pub fn from_sql(v: i64) -> Self {
        Self(v as u64)
    }
}

impl fmt::Display for FileHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

/// One face inside a font file: `.ttf`/`.otf` have index 0, collections have several.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct FaceKey {
    pub hash: FileHash,
    pub index: u32,
}

impl fmt::Display for FaceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.hash, self.index)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum FontFormat {
    /// TrueType outlines (`glyf`).
    Ttf,
    /// CFF/CFF2 outlines.
    Otf,
    /// TrueType or OpenType collection.
    Ttc,
    Woff,
    Woff2,
}

impl FontFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ttf => "ttf",
            Self::Otf => "otf",
            Self::Ttc => "ttc",
            Self::Woff => "woff",
            Self::Woff2 => "woff2",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "ttf" => Self::Ttf,
            "otf" => Self::Otf,
            "ttc" => Self::Ttc,
            "woff" => Self::Woff,
            "woff2" => Self::Woff2,
            _ => return None,
        })
    }

    /// Sniffs the format from the first four bytes of a file.
    pub fn sniff(bytes: &[u8]) -> Option<Self> {
        Some(match bytes.get(..4)? {
            [0, 1, 0, 0] | b"true" => Self::Ttf,
            b"OTTO" => Self::Otf,
            b"ttcf" => Self::Ttc,
            b"wOFF" => Self::Woff,
            b"wOF2" => Self::Woff2,
            _ => return None,
        })
    }
}

/// How long an activation lasts.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ActivationMode {
    /// Removed at logoff, on exit, or by the next reconcile.
    Session,
    /// Survives reboots until the user deactivates it.
    Pinned,
}

impl ActivationMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Pinned => "pinned",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "session" => Some(Self::Session),
            "pinned" => Some(Self::Pinned),
            _ => None,
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Axis {
    pub tag: String,
    pub min: f32,
    pub default: f32,
    pub max: f32,
}

/// Shape metrics measured from outlines; `None` when the glyphs needed are missing.
#[derive(Clone, Copy, PartialEq, Default, Debug)]
pub struct Metrics {
    /// x-height / cap height.
    pub x_height: Option<f32>,
    /// Vertical stem thickness / cap height.
    pub weight: Option<f32>,
    /// Average letter advance / cap height.
    pub width: Option<f32>,
    /// 1 - thin stroke / thick stroke of an O (0 = monolinear).
    pub contrast: Option<f32>,
}

/// Everything the indexer extracts from one face.
#[derive(Clone, PartialEq, Debug)]
pub struct FaceInfo {
    pub index: u32,
    /// Sanitised family name used for grouping and display.
    pub family: String,
    /// Family name exactly as the font states it.
    pub family_raw: String,
    pub subfamily: String,
    pub full_name: String,
    pub ps_name: String,
    pub weight: u16,
    pub width: u16,
    pub italic: bool,
    pub format: FontFormat,
    pub glyph_count: u16,
    pub units_per_em: u16,
    pub x_height: Option<i16>,
    pub cap_height: Option<i16>,
    pub axes: Vec<Axis>,
    /// Sorted, deduplicated GSUB/GPOS feature tags.
    pub features: Vec<String>,
    pub coverage: Coverage,
    /// OS/2 achVendID, trimmed.
    pub vendor: String,
    /// Name ID 13 (licence description), truncated.
    pub license: String,
    /// Text the font can actually draw, for previews.
    pub sample: String,
    pub metrics: Metrics,
}

impl FaceInfo {
    pub fn is_variable(&self) -> bool {
        !self.axes.is_empty()
    }
}

/// Canonical form of a hierarchical tag (`serif/slab/egyptian`): leading `#`s are dropped, each `/`-separated segment is trimmed with inner whitespace collapsed, and empty
/// segments are removed. Case is kept. `None` if nothing is left.
pub fn normalize_tag(s: &str) -> Option<String> {
    let segments: Vec<String> = s
        .split('/')
        .map(|seg| seg.trim().trim_start_matches('#').split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|seg| !seg.is_empty())
        .collect();
    (!segments.is_empty()).then(|| segments.join("/"))
}

#[cfg(test)]
mod tag_tests {
    use super::normalize_tag;

    #[test]
    fn normalizes_tags() {
        assert_eq!(normalize_tag("#Serif/ Slab //egyptian ").as_deref(), Some("Serif/Slab/egyptian"));
        assert_eq!(normalize_tag("hand  made").as_deref(), Some("hand made"));
        assert_eq!(normalize_tag(" / # "), None);
        assert_eq!(normalize_tag("#"), None);
    }
}
