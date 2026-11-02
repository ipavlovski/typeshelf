//! Name cleanup, so junk families like "-", "." or "a" never reach the list.

/// Strips control characters (some fonts pad names with NUL) and collapses whitespace.
pub fn clean(s: &str) -> String {
    s.split(|c: char| c.is_whitespace() || c.is_control()).filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" ")
}

/// A name is junk when it has fewer than two letters or digits.
pub fn is_junk(s: &str) -> bool {
    s.chars().filter(|c| c.is_alphanumeric()).take(2).count() < 2
}

/// Returns `raw` unless it is junk, otherwise the first usable fallback:
/// the other name candidates (full name, PostScript name), then the file stem.
pub fn sanitize_family(raw: &str, candidates: &[&str], stem: &str) -> String {
    if !is_junk(raw) {
        return raw.to_owned();
    }
    for c in candidates {
        // PostScript names are "Family-Style"; keep the family part.
        let c = clean(c.split('-').next().unwrap_or(c));
        if !is_junk(&c) {
            return c;
        }
    }
    let stem = clean(&stem.replace('_', " "));
    let family = stem.split('-').next().unwrap_or(&stem).trim();
    if is_junk(family) { stem } else { family.to_owned() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_real_names() {
        assert_eq!(sanitize_family("Helvetica Neue", &[], "x"), "Helvetica Neue");
        assert_eq!(sanitize_family("21 Cent", &[], "x"), "21 Cent");
    }

    #[test]
    fn replaces_junk() {
        assert_eq!(sanitize_family("-", &["-", "-"], "Junky-Regular"), "Junky");
        assert_eq!(sanitize_family(".", &["", "Brandon-Bold"], "x"), "Brandon");
        assert_eq!(sanitize_family("a", &["Alpha Bold"], "x"), "Alpha Bold");
        assert_eq!(sanitize_family("", &[], "my_font"), "my font");
    }

    #[test]
    fn cleans_control_characters() {
        assert_eq!(clean("Foo\0\0 \t Bar "), "Foo Bar");
    }
}
