//! Name normalisation and the trigram index used for typo-tolerant matches.

use std::collections::HashMap;

use roaring::RoaringBitmap;
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// Lowercases, strips accents and turns punctuation into spaces: "Café-Neue_Bold" → "cafe neue bold".
pub fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = true;
    for c in s.nfkd().filter(|c| !is_combining_mark(*c)).flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            out.push(c);
            space = false;
        } else if !space {
            out.push(' ');
            space = true;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

fn trigrams(word: &str) -> impl Iterator<Item = [char; 3]> + '_ {
    let chars: Vec<char> = word.chars().collect();
    (0..chars.len().saturating_sub(2)).map(move |i| [chars[i], chars[i + 1], chars[i + 2]])
}

/// Atoms shorter than this match by substring only; longer ones tolerate typos.
pub const FUZZY_ATOM: usize = 5;

/// Trigram → ids of the documents containing it. Trigrams never span words.
#[derive(Default)]
pub struct TrigramIndex {
    postings: HashMap<[char; 3], RoaringBitmap>,
}

impl TrigramIndex {
    pub fn insert(&mut self, id: u32, normalized: &str) {
        for word in normalized.split(' ') {
            for t in trigrams(word) {
                self.postings.entry(t).or_default().insert(id);
            }
        }
    }

    /// Documents that may match `atom` (a normalised word of the query), or `None`
    /// when the atom is too short to prefilter. Short atoms need every trigram
    /// (a substring); longer ones need a third, so "garamnod" and "helvteica" still match.
    pub fn candidates(&self, atom: &str, counts: &mut Vec<u8>) -> Option<RoaringBitmap> {
        let grams: Vec<_> = trigrams(atom).collect();
        if grams.is_empty() {
            return None;
        }
        let empty = RoaringBitmap::new();
        let lists: Vec<&RoaringBitmap> = grams.iter().map(|t| self.postings.get(t).unwrap_or(&empty)).collect();
        if atom.chars().count() < FUZZY_ATOM {
            return Some(lists.iter().fold(lists[0].clone(), |acc, l| acc & *l));
        }
        let need = grams.len().div_ceil(3).max(2) as u8;
        counts.clear();
        let mut out = RoaringBitmap::new();
        for l in lists {
            for id in l {
                let i = id as usize;
                if i >= counts.len() {
                    counts.resize(i + 1, 0);
                }
                counts[i] = counts[i].saturating_add(1);
                if counts[i] == need {
                    out.insert(id);
                }
            }
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes() {
        assert_eq!(normalize("Café-Neue_Bold "), "cafe neue bold");
        assert_eq!(normalize("--"), "");
        assert_eq!(normalize("Ёлка"), "елка");
    }

    #[test]
    fn substring_and_typos() {
        let mut idx = TrigramIndex::default();
        idx.insert(0, "garamond premier");
        idx.insert(1, "helvetica neue");
        idx.insert(2, "eb garamond");
        let mut counts = Vec::new();
        let hits =
            |idx: &TrigramIndex, a: &str, c: &mut Vec<u8>| idx.candidates(a, c).unwrap().iter().collect::<Vec<_>>();
        assert_eq!(hits(&idx, "gar", &mut counts), [0, 2]);
        assert_eq!(hits(&idx, "neue", &mut counts), [1]);
        assert_eq!(hits(&idx, "garamnod", &mut counts), [0, 2]);
        assert!(idx.candidates("ga", &mut counts).is_none());
    }
}
