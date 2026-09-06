use std::collections::HashMap;

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

/// The five Fitzpatrick modifiers, in the order the settings window offers them.
pub const TONES: [char; 5] = ['\u{1f3fb}', '\u{1f3fc}', '\u{1f3fd}', '\u{1f3fe}', '\u{1f3ff}'];

fn is_tone(c: char) -> bool {
    TONES.contains(&c)
}

pub fn has_tone(ch: &str) -> bool {
    ch.chars().any(is_tone)
}

/// The tone-free form of a sequence, which is also its entry in the table.
fn strip_tone(ch: &str) -> String {
    ch.chars().filter(|c| !is_tone(*c)).collect()
}

/// Only base emoji: the table lists every skin-tone variant as its own entry, and the
/// grid shows one cell per emoji with the chosen tone applied instead.
pub fn by_group(group: &str) -> impl Iterator<Item = &'static Emoji> {
    EMOJI
        .iter()
        .filter(move |e| e.group == group && !has_tone(e.ch))
}

/// base sequence -> that sequence in each of the five tones.
static TONED: std::sync::OnceLock<HashMap<(&'static str, u8), &'static str>> =
    std::sync::OnceLock::new();

fn toned_map() -> &'static HashMap<(&'static str, u8), &'static str> {
    TONED.get_or_init(|| {
        let bases: HashMap<String, &'static str> = EMOJI
            .iter()
            .filter(|e| !has_tone(e.ch))
            .map(|e| (e.ch.to_string(), e.ch))
            .collect();

        let mut map = HashMap::new();
        for e in EMOJI.iter().filter(|e| has_tone(e.ch)) {
            let tones: Vec<char> = e.ch.chars().filter(|c| is_tone(*c)).collect();
            // Sequences mixing two different tones (couples, handshakes) have no single
            // tone to key them under; the base form stands in for those.
            let (Some(first), true) = (tones.first(), tones.windows(2).all(|w| w[0] == w[1]))
            else {
                continue;
            };
            let Some(idx) = TONES.iter().position(|t| t == first) else { continue };
            if let Some(base) = bases.get(&strip_tone(e.ch)) {
                map.insert((*base, idx as u8 + 1), e.ch);
            }
        }
        map
    })
}

/// Apply a tone (1-5; 0 means the default yellow) to a base emoji. Emoji that take no
/// tone come back unchanged.
pub fn with_tone(ch: &'static str, tone: u8) -> &'static str {
    if tone == 0 {
        return ch;
    }
    toned_map().get(&(ch, tone)).copied().unwrap_or(ch)
}

/// Whether this emoji has any skin-tone variants at all.
#[cfg_attr(not(test), allow(dead_code))]
pub fn tonable(ch: &'static str) -> bool {
    (1..=5).any(|t| toned_map().contains_key(&(ch, t)))
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
        // Tone variants would otherwise crowd out everything else: one "waving hand"
        // becomes six near-identical hits.
        if has_tone(e.ch) {
            continue;
        }
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
    fn tones_map_onto_their_base() {
        let wave = find("\u{1f44b}").unwrap().ch;
        assert!(tonable(wave));
        assert_eq!(with_tone(wave, 0), wave);
        assert_eq!(with_tone(wave, 1), "\u{1f44b}\u{1f3fb}");
        assert_eq!(with_tone(wave, 5), "\u{1f44b}\u{1f3ff}");
    }

    #[test]
    fn emoji_without_tones_are_left_alone() {
        let cat = find("\u{1f431}").unwrap().ch;
        assert!(!tonable(cat));
        assert_eq!(with_tone(cat, 3), cat);
    }

    #[test]
    fn browsing_and_search_show_only_base_emoji() {
        assert!(by_group("People & Body").all(|e| !has_tone(e.ch)));
        assert!(search("waving hand").iter().all(|e| !has_tone(e.ch)));
    }

    #[test]
    fn search_finds_by_keyword() {
        let hits = search("kitten");
        assert!(hits.iter().take(10).any(|e| e.ch == "\u{1f431}"), "no cat face in top 10");
    }
}
