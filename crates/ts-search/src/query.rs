//! Query syntax: free text plus `key:value` filters.
//!
//! ```text
//! garamond script:cyrillic feat:ss01 wght:>600 col:"2. SANS SERIF" is:active -is:italic #serif/slab
//! ```
//! `#tag` (or `tag:tag`) matches the tag and every tag below it; `#"hand made"` for spaces.
//! `is:tagged` / `is:untagged` match faces with any tag / none.
//! Ranges: `wght:600`, `wght:>600`, `wght:<=300`, `wght:300..500`.

use ts_model::Block;

#[derive(Clone, PartialEq, Debug)]
pub enum Filter {
    Script(Block),
    Feature(String),
    Format(String),
    Active,
    Pinned,
    Variable,
    Italic,
    Favourite,
    /// Any tag at all (`-is:tagged` finds what is left to tag).
    Tagged,
    /// Hierarchical tag, including its sub-tags (case-insensitive).
    Tag(String),
    /// Collection by id, including sub-collections.
    Collection(i64),
    /// Collection by name, case-insensitive, including sub-collections.
    CollectionName(String),
    Folder(i64),
    /// Folder whose path contains this text, case-insensitive.
    FolderPath(String),
    /// Numeric range over a named column (`wght`, `wdth`, `glyphs`, metrics).
    Range {
        key: String,
        min: f32,
        max: f32,
    },
    Not(Box<Filter>),
}

#[derive(Clone, Default, PartialEq, Debug)]
pub struct Query {
    pub text: String,
    pub filters: Vec<Filter>,
}

impl Query {
    pub fn parse(input: &str) -> Query {
        let mut q = Query::default();
        let mut words = Vec::new();
        for token in tokenize(input) {
            match parse_filter(&token) {
                Some(f) => q.filters.push(f),
                None => words.push(token),
            }
        }
        q.text = words.join(" ");
        q
    }

    pub fn with(mut self, f: Filter) -> Query {
        self.filters.push(f);
        self
    }
}

/// Splits on whitespace, keeping `"quoted parts"` together (quotes removed).
fn tokenize(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in input.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn parse_filter(token: &str) -> Option<Filter> {
    if let Some(rest) = token.strip_prefix('-') {
        return parse_filter(rest).map(|f| Filter::Not(Box::new(f)));
    }
    if let Some(rest) = token.strip_prefix('#') {
        return ts_model::normalize_tag(rest).map(Filter::Tag);
    }
    let (key, value) = token.split_once(':')?;
    if value.is_empty() {
        return None;
    }
    let lower = value.to_lowercase();
    Some(match key.to_lowercase().as_str() {
        "script" | "lang" => Filter::Script(Block::from_slug(&lower)?),
        "feat" | "feature" => Filter::Feature(value.to_owned()),
        "format" | "type" => Filter::Format(lower),
        "is" | "has" => match lower.as_str() {
            "active" => Filter::Active,
            "inactive" => Filter::Not(Box::new(Filter::Active)),
            "pinned" => Filter::Pinned,
            "variable" | "var" => Filter::Variable,
            "static" => Filter::Not(Box::new(Filter::Variable)),
            "italic" => Filter::Italic,
            "fav" | "favorite" | "favourite" => Filter::Favourite,
            "tagged" => Filter::Tagged,
            "untagged" => Filter::Not(Box::new(Filter::Tagged)),
            _ => return None,
        },
        "col" | "collection" => Filter::CollectionName(lower),
        "tag" => Filter::Tag(ts_model::normalize_tag(value)?),
        "folder" => Filter::FolderPath(lower),
        key if key.starts_with(|c: char| c.is_ascii_alphabetic()) => {
            let (min, max) = parse_range(value)?;
            Filter::Range { key: key.to_owned(), min, max }
        }
        _ => return None,
    })
}

fn parse_range(v: &str) -> Option<(f32, f32)> {
    let num = |s: &str| s.trim().parse::<f32>().ok();
    if let Some((a, b)) = v.split_once("..") {
        return Some((num(a).unwrap_or(f32::MIN), num(b).unwrap_or(f32::MAX)));
    }
    if let Some(r) = v.strip_prefix(">=") {
        return Some((num(r)?, f32::MAX));
    }
    if let Some(r) = v.strip_prefix("<=") {
        return Some((f32::MIN, num(r)?));
    }
    if let Some(r) = v.strip_prefix('>') {
        return Some((num(r)?.next_up(), f32::MAX));
    }
    if let Some(r) = v.strip_prefix('<') {
        return Some((f32::MIN, num(r)?.next_down()));
    }
    let n = num(v)?;
    Some((n, n))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_documented_example() {
        let q = Query::parse(
            r#"garamond script:cyrillic feat:ss01 wght:>600 col:"2. SANS SERIF" is:active -is:italic #serif/slab"#,
        );
        assert_eq!(q.text, "garamond");
        assert_eq!(
            q.filters,
            [
                Filter::Script(Block::Cyrillic),
                Filter::Feature("ss01".into()),
                Filter::Range { key: "wght".into(), min: 600f32.next_up(), max: f32::MAX },
                Filter::CollectionName("2. sans serif".into()),
                Filter::Active,
                Filter::Not(Box::new(Filter::Italic)),
                Filter::Tag("serif/slab".into()),
            ]
        );
    }

    #[test]
    fn tags() {
        let q = Query::parse(r##"-#display tag:Serif/Slab #"hand made/ brush" # c#"##);
        assert_eq!(q.text, "# c#");
        assert_eq!(
            q.filters,
            [
                Filter::Not(Box::new(Filter::Tag("display".into()))),
                Filter::Tag("Serif/Slab".into()),
                Filter::Tag("hand made/brush".into()),
            ]
        );
    }

    #[test]
    fn ranges() {
        assert_eq!(parse_range("300..500"), Some((300.0, 500.0)));
        assert_eq!(parse_range("..500"), Some((f32::MIN, 500.0)));
        assert_eq!(parse_range("<=300"), Some((f32::MIN, 300.0)));
        assert_eq!(parse_range("400"), Some((400.0, 400.0)));
        assert_eq!(parse_range("bold"), None);
    }

    #[test]
    fn unknown_filters_stay_text() {
        let q = Query::parse("time: 12:30 script:klingon");
        assert_eq!(q.text, "time: 12:30 script:klingon");
        assert!(q.filters.is_empty());
    }
}
