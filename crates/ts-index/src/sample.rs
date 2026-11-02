//! Picks preview text a font can actually draw, so previews never show tofu boxes.

pub const DEFAULT_SAMPLE: &str = "The quick brown fox jumps over the lazy dog";

/// Pangrams or common words per script, tried in order when the preferred text is not covered.
const SCRIPT_SAMPLES: &[&str] = &[
    "Съешь же ещё этих мягких французских булок",
    "СЪЕШЬ ЖЕ ЕЩЁ ЭТИХ МЯГКИХ ФРАНЦУЗСКИХ БУЛОК",
    "Ξεσκεπάζω την ψυχοφθόρα βδελυγμία",
    "שפן אכל קצת גזר בטעם חסה",
    "نص حكيم له سر قاطع وذو شأن عظيم",
    "ऋषियों को सताने वाले दुष्ट राक्षसों",
    "เป็นมนุษย์สุดประเสริฐเลิศคุณค่า",
    "いろはにほへと ちりぬるを",
    "永和九年歲在癸丑暮春之初",
    "다람쥐 헌 쳇바퀴에 타고파",
    "0123456789",
];

const FALLBACK_GLYPHS: usize = 30;

/// Returns `preferred` if the font covers it, then its uppercase or lowercase
/// form (for single-case fonts), then a script sample, then the first mapped characters.
pub fn pick_sample(preferred: &str, covers: impl Fn(char) -> bool, mapped: impl Iterator<Item = u32>) -> String {
    let fits = |text: &str| text.chars().all(|c| c.is_whitespace() || covers(c));
    if fits(preferred) {
        return preferred.to_owned();
    }
    for cased in [preferred.to_uppercase(), preferred.to_lowercase()] {
        if fits(&cased) {
            return cased;
        }
    }
    if let Some(s) = SCRIPT_SAMPLES.iter().find(|s| fits(s)) {
        return (*s).to_owned();
    }
    mapped.filter_map(char::from_u32).filter(|c| !c.is_whitespace() && !c.is_control()).take(FALLBACK_GLYPHS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_for(chars: &str) -> String {
        pick_sample(DEFAULT_SAMPLE, |c| chars.contains(c), chars.chars().map(u32::from))
    }

    #[test]
    fn full_latin_gets_preferred() {
        let latin: String = ('a'..='z').chain('A'..='Z').collect();
        assert_eq!(sample_for(&latin), DEFAULT_SAMPLE);
    }

    #[test]
    fn caps_only_gets_uppercase() {
        let caps: String = ('A'..='Z').collect();
        assert_eq!(sample_for(&caps), DEFAULT_SAMPLE.to_uppercase());
    }

    #[test]
    fn lowercase_only_gets_lowercase() {
        let lower: String = ('a'..='z').collect();
        assert_eq!(sample_for(&lower), DEFAULT_SAMPLE.to_lowercase());
    }

    #[test]
    fn cyrillic_gets_cyrillic() {
        let cyr: String = ('\u{410}'..='\u{44F}').chain(['ё']).collect();
        assert_eq!(sample_for(&cyr), SCRIPT_SAMPLES[0]);
    }

    #[test]
    fn symbols_get_their_own_glyphs() {
        let dingbats: String = ('\u{2701}'..='\u{2730}').collect();
        let sample = sample_for(&dingbats);
        assert_eq!(sample.chars().count(), FALLBACK_GLYPHS);
        assert!(sample.starts_with('\u{2701}'));
    }
}
