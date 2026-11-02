//! Unicode block coverage as a 64-bit set, one bit per [`Block`].

use std::ops::RangeInclusive;

macro_rules! blocks {
    ($($name:ident = $slug:literal, $lo:literal ..= $hi:literal;)*) => {
        /// Unicode blocks typeshelf tracks for filtering (`script:cyrillic`) and sample text.
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
        #[repr(u8)]
        pub enum Block { $($name),* }

        impl Block {
            pub const ALL: &[Block] = &[$(Block::$name),*];

            pub fn slug(self) -> &'static str {
                match self { $(Block::$name => $slug),* }
            }

            pub fn range(self) -> RangeInclusive<u32> {
                match self { $(Block::$name => $lo..=$hi),* }
            }
        }
    };
}

blocks! {
    BasicLatin = "latin", 0x0020..=0x007F;
    Latin1 = "latin-1", 0x00A0..=0x00FF;
    LatinExtA = "latin-ext-a", 0x0100..=0x017F;
    LatinExtB = "latin-ext-b", 0x0180..=0x024F;
    Ipa = "ipa", 0x0250..=0x02AF;
    Greek = "greek", 0x0370..=0x03FF;
    Cyrillic = "cyrillic", 0x0400..=0x04FF;
    CyrillicSup = "cyrillic-sup", 0x0500..=0x052F;
    Armenian = "armenian", 0x0530..=0x058F;
    Hebrew = "hebrew", 0x0590..=0x05FF;
    Arabic = "arabic", 0x0600..=0x06FF;
    Devanagari = "devanagari", 0x0900..=0x097F;
    Bengali = "bengali", 0x0980..=0x09FF;
    Tamil = "tamil", 0x0B80..=0x0BFF;
    Thai = "thai", 0x0E00..=0x0E7F;
    Georgian = "georgian", 0x10A0..=0x10FF;
    Ethiopic = "ethiopic", 0x1200..=0x137F;
    Cherokee = "cherokee", 0x13A0..=0x13FF;
    LatinExtAdditional = "vietnamese", 0x1E00..=0x1EFF;
    GreekExt = "greek-ext", 0x1F00..=0x1FFF;
    Punctuation = "punctuation", 0x2000..=0x206F;
    Currency = "currency", 0x20A0..=0x20CF;
    Letterlike = "letterlike", 0x2100..=0x214F;
    NumberForms = "number-forms", 0x2150..=0x218F;
    Arrows = "arrows", 0x2190..=0x21FF;
    Math = "math", 0x2200..=0x22FF;
    Technical = "technical", 0x2300..=0x23FF;
    BoxDrawing = "box-drawing", 0x2500..=0x257F;
    BlockElements = "block-elements", 0x2580..=0x259F;
    Shapes = "shapes", 0x25A0..=0x25FF;
    Symbols = "symbols", 0x2600..=0x26FF;
    Dingbats = "dingbats", 0x2700..=0x27BF;
    Braille = "braille", 0x2800..=0x28FF;
    CjkSymbols = "cjk-symbols", 0x3000..=0x303F;
    Hiragana = "hiragana", 0x3040..=0x309F;
    Katakana = "katakana", 0x30A0..=0x30FF;
    Cjk = "cjk", 0x4E00..=0x9FFF;
    Hangul = "hangul", 0xAC00..=0xD7AF;
    PrivateUse = "private-use", 0xE000..=0xF8FF;
    PresentationForms = "presentation-forms", 0xFB00..=0xFDFF;
    Fullwidth = "fullwidth", 0xFF00..=0xFFEF;
    Emoji = "emoji", 0x1F300..=0x1FAFF;
}

impl Block {
    pub fn from_slug(slug: &str) -> Option<Block> {
        Block::ALL.iter().copied().find(|b| b.slug() == slug)
    }

    pub fn of(cp: u32) -> Option<Block> {
        Block::ALL.iter().copied().find(|b| b.range().contains(&cp))
    }

    /// Mapped code points needed before a font counts as covering the block.
    /// Low enough that a font with a basic alphabet qualifies, high enough
    /// that a stray currency sign does not make a font "Arabic".
    fn threshold(self) -> u32 {
        let len = self.range().end() - self.range().start() + 1;
        (len / 8).clamp(1, 24)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Debug)]
pub struct Coverage(pub u64);

impl Coverage {
    /// Builds coverage from the code points a font maps.
    pub fn from_codepoints(cps: impl IntoIterator<Item = u32>) -> Self {
        let mut counts = [0u32; 64];
        for cp in cps {
            if let Some(b) = Block::of(cp) {
                counts[b as usize] += 1;
            }
        }
        let mut bits = 0u64;
        for &b in Block::ALL {
            if counts[b as usize] >= b.threshold() {
                bits |= 1 << b as u8;
            }
        }
        Coverage(bits)
    }

    pub fn has(self, b: Block) -> bool {
        self.0 & (1 << b as u8) != 0
    }

    pub fn blocks(self) -> impl Iterator<Item = Block> {
        Block::ALL.iter().copied().filter(move |&b| self.has(b))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_in_u64() {
        assert!(Block::ALL.len() <= 64);
    }

    #[test]
    fn cyrillic_alphabet_counts() {
        let cov = Coverage::from_codepoints(0x410..0x450);
        assert!(cov.has(Block::Cyrillic));
        assert!(!cov.has(Block::BasicLatin));
    }

    #[test]
    fn single_symbol_does_not_count() {
        let cov = Coverage::from_codepoints([0x0627]);
        assert!(!cov.has(Block::Arabic));
    }

    #[test]
    fn slugs_round_trip() {
        for &b in Block::ALL {
            assert_eq!(Block::from_slug(b.slug()), Some(b));
        }
    }
}
