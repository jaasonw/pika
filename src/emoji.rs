use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher};

#[derive(Clone, Copy, Debug)]
pub struct Emoji {
    pub ch: &'static str,
    pub group: &'static str,
    pub name: &'static str,
    pub keywords: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/emoji_table.rs"));

/// Category order shown in the tab bar. "Recents" is synthesized by the UI.
pub static GROUPS: &[&str] = &[
    "Smileys & Emotion",
    "People & Body",
    "Animals & Nature",
    "Food & Drink",
    "Travel & Places",
    "Activities",
    "Objects",
    "Symbols",
    "Flags",
];

pub fn by_group(group: &str) -> impl Iterator<Item = &'static Emoji> {
    EMOJI.iter().filter(move |e| e.group == group)
}

pub fn find(ch: &str) -> Option<&'static Emoji> {
    EMOJI.iter().find(|e| e.ch == ch)
}

/// Fuzzy search over name (weighted) and keywords, best match first.
pub fn search(query: &str) -> Vec<&'static Emoji> {
    let mut matcher = Matcher::new(Config::DEFAULT);
    let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
    let mut buf = Vec::new();

    let mut hits: Vec<(u32, usize, &'static Emoji)> = Vec::new();
    for (idx, e) in EMOJI.iter().enumerate() {
        buf.clear();
        let name = pattern.score(
            nucleo_matcher::Utf32Str::new(e.name, &mut buf),
            &mut matcher,
        );
        buf.clear();
        let kw = pattern.score(
            nucleo_matcher::Utf32Str::new(e.keywords, &mut buf),
            &mut matcher,
        );

        // A name hit is worth far more than a keyword hit.
        let mut score = match (name, kw) {
            (Some(n), _) => n * 2,
            (None, Some(k)) => k,
            (None, None) => continue,
        };
        // Whole-word hits beat scattered subsequence matches: "kitten" should land
        // on the cat, not on every emoji whose letters happen to spell it.
        if e.name.eq_ignore_ascii_case(query) {
            score += 10_000;
        } else if e.keywords.split(' ').any(|k| k.eq_ignore_ascii_case(query)) {
            score += 5_000;
        } else if e.name.to_lowercase().contains(&query.to_lowercase()) {
            score += 2_000;
        }
        hits.push((score, idx, e));
    }

    // Ties break on table order, which is the Unicode ordering.
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    hits.into_iter().map(|(_, _, e)| e).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_populated() {
        // Emoji 16 has ~3.9k fully-qualified sequences; a big drop means the
        // regenerated data file lost something.
        assert!(EMOJI.len() > 3500, "got {}", EMOJI.len());
        assert!(EMOJI.iter().all(|e| !e.ch.is_empty() && !e.name.is_empty()));
    }

    #[test]
    fn groups_cover_the_table() {
        for e in EMOJI {
            assert!(GROUPS.contains(&e.group), "unknown group {:?}", e.group);
        }
    }

    #[test]
    fn search_ranks_exact_name_first() {
        assert_eq!(search("grinning face")[0].ch, "\u{1f600}");
        assert_eq!(search("thinking face")[0].ch, "\u{1f914}");
    }

    #[test]
    fn search_finds_by_keyword() {
        let hits = search("kitten");
        assert!(hits.iter().take(10).any(|e| e.ch == "\u{1f431}"), "no cat face in top 10");
    }
}
