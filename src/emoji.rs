use std::cell::RefCell;

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher};

#[derive(Clone, Copy, Debug)]
pub struct Emoji {
    pub ch: &'static str,
    /// Only read by the tests, which use it to check `GROUP_RANGES` against the data.
    #[cfg_attr(not(test), allow(dead_code))]
    pub group: &'static str,
    pub name: &'static str,
    pub keywords: &'static str,
    /// `name` pre-folded, so the substring bonus in `search` needs no allocation.
    pub name_lower: &'static str,
    /// Which letters `name` and `keywords` contain. See `mask_of` in `build.rs`.
    pub name_mask: u32,
    pub kw_mask: u32,
    /// This sequence in each of the five tones; an emoji that takes no tone repeats
    /// itself, so `with_tone` is an unconditional index.
    pub tones: [&'static str; 5],
}

// Defines EMOJI, GROUPS, BASE_COUNT and GROUP_RANGES. Everything the old lazy statics
// worked out at runtime — which entries are base emoji, which group they fall in, and
// what each one looks like in every tone — is settled at compile time instead.
include!(concat!(env!("OUT_DIR"), "/emoji_table.rs"));

/// The five Fitzpatrick modifiers, in the order the settings window offers them.
pub const TONES: [char; 5] = ['\u{1f3fb}', '\u{1f3fc}', '\u{1f3fd}', '\u{1f3fe}', '\u{1f3ff}'];

/// The base emoji, in tab-bar group order. Tone variants live past this point and are
/// reachable only through `with_tone` and `find`.
fn base() -> &'static [Emoji] {
    &EMOJI[..BASE_COUNT]
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn has_tone(ch: &str) -> bool {
    ch.chars().any(|c| TONES.contains(&c))
}

/// Only base emoji: the table lists every skin-tone variant as its own entry, and the
/// grid shows one cell per emoji with the chosen tone applied instead.
pub fn by_group(group: &str) -> impl Iterator<Item = &'static Emoji> {
    let range = GROUPS
        .iter()
        .position(|g| *g == group)
        .map(|i| GROUP_RANGES[i])
        .unwrap_or((0, 0));
    EMOJI[range.0..range.1].iter()
}

impl Emoji {
    /// Apply a tone (1-5; 0 means the default yellow). Emoji that take no tone come back
    /// unchanged, because `tones` repeats `ch` for those.
    pub fn toned(&self, tone: u8) -> &'static str {
        match tone.checked_sub(1) {
            Some(i) => self.tones[(i as usize).min(4)],
            None => self.ch,
        }
    }

    /// Whether this emoji has any skin-tone variants at all.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn tonable(&self) -> bool {
        self.tones[0] != self.ch
    }
}

/// Tone a sequence given only its characters. The grid already holds `&Emoji` and should
/// call `Emoji::toned`; this is for callers that have just a string, such as the sample
/// hand in the settings window.
pub fn with_tone(ch: &'static str, tone: u8) -> &'static str {
    // An emoji missing from the table stands in for itself, as it did when this walked a
    // runtime map: better a untoned glyph than an empty button.
    find(ch).map(|e| e.toned(tone)).unwrap_or(ch)
}

pub fn find(ch: &str) -> Option<&'static Emoji> {
    BY_CH
        .binary_search_by(|&i| EMOJI[i as usize].ch.cmp(ch))
        .ok()
        .map(|slot| &EMOJI[BY_CH[slot] as usize])
}

/// The grid shows a few dozen hits and nobody scrolls to rank 500. Capping keeps a
/// one-letter query — which matches nearly everything — from sorting and rendering the
/// whole table.
const MAX_HITS: usize = 500;

/// Which letters a query needs, in the encoding `build.rs` used for `name_mask`. Only
/// `[a-z0-9]` contributes: anything else the fuzzy matcher may fold or ignore, so
/// demanding it would reject candidates that do match.
fn query_mask(query: &str) -> u32 {
    let mut m = 0u32;
    for b in query.bytes() {
        match b {
            b'a'..=b'z' => m |= 1 << (b - b'a'),
            b'A'..=b'Z' => m |= 1 << (b - b'A'),
            b'0'..=b'9' => m |= 1 << 26,
            _ => {}
        }
    }
    m
}

/// True when the field cannot possibly match, so the fuzzy matcher can be skipped. Bit 27
/// marks a non-ASCII field, whose folded form the mask cannot model; those always run.
fn cannot_match(field_mask: u32, q: u32) -> bool {
    field_mask & (1 << 27) == 0 && q & !field_mask != 0
}

thread_local! {
    /// Reused across keystrokes; `Matcher` carries scratch buffers worth keeping.
    static MATCHER: RefCell<Matcher> = RefCell::new(Matcher::new(Config::DEFAULT));
}

/// Fuzzy search over name (weighted) and keywords, best match first.
///
/// Only base emoji are considered: tone variants would crowd everything else out, one
/// "waving hand" becoming six near-identical hits.
pub fn search(query: &str) -> Vec<&'static Emoji> {
    let lower = query.to_lowercase();
    let q = query_mask(query);
    let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);

    MATCHER.with(|m| {
        let mut matcher = m.borrow_mut();
        let mut buf = Vec::new();
        let mut hits: Vec<(u32, usize, &'static Emoji)> = Vec::new();

        for (idx, e) in base().iter().enumerate() {
            // The mask rejects most of the table for the cost of two ANDs, before the
            // matcher ever looks at the ~93 characters of name and keywords.
            let skip_name = cannot_match(e.name_mask, q);
            let skip_kw = cannot_match(e.kw_mask, q);
            if skip_name && skip_kw {
                continue;
            }

            let mut score_of = |text: &str| {
                buf.clear();
                pattern.score(nucleo_matcher::Utf32Str::new(text, &mut buf), &mut matcher)
            };
            let name = if skip_name { None } else { score_of(e.name) };
            let kw = if skip_kw { None } else { score_of(e.keywords) };

            // A name hit is worth far more than a keyword hit.
            let mut score = match (name, kw) {
                (Some(n), _) => n * 2,
                (None, Some(k)) => k,
                (None, None) => continue,
            };
            // Whole-word hits beat scattered subsequence matches: "kitten" should land
            // on the cat, not on every emoji whose letters happen to spell it.
            if e.name_lower == lower {
                score += 10_000;
            } else if e.name_lower.starts_with(&lower) {
                // "cat" should reach cat, cat face and cat with wry smile before bobcat.
                score += 7_000;
            } else if e.keywords.split(' ').any(|k| k.eq_ignore_ascii_case(query)) {
                score += 5_000;
            } else if e.name_lower.contains(&lower) {
                score += 2_000;
            }
            hits.push((score, idx, e));
        }

        // Ties break on table order, which is the Unicode ordering within each group.
        let cmp = |a: &(u32, usize, &Emoji), b: &(u32, usize, &Emoji)| {
            b.0.cmp(&a.0).then(a.1.cmp(&b.1))
        };
        if hits.len() > MAX_HITS {
            hits.select_nth_unstable_by(MAX_HITS, cmp);
            hits.truncate(MAX_HITS);
        }
        hits.sort_by(cmp);
        hits.into_iter().map(|(_, _, e)| e).collect()
    })
}

/// Time the table work that sits on the launch and keystroke paths. Reached with
/// `--bench`, alongside the other diagnostic flags, and runs before GTK is touched.
pub fn bench() {
    use std::time::Instant;

    println!("{} entries, {BASE_COUNT} of them base emoji", EMOJI.len());

    // What a rebuild() with an empty query gathers: every group, toned.
    let t = Instant::now();
    let mut cells = 0usize;
    for g in GROUPS {
        cells += by_group(g).map(|e| e.toned(1)).count();
    }
    println!("rebuild gather ({cells} cells over {} groups): {:?}", GROUPS.len(), t.elapsed());

    let t = Instant::now();
    for _ in 0..48 {
        let _ = find("\u{1f600}");
    }
    println!("find() x48 (a full recents list): {:?}", t.elapsed());

    for q in ["a", "cat", "grinning face", "zzz"] {
        let t = Instant::now();
        let hits = search(q);
        println!("search({q:?}) -> {} hits: {:?}", hits.len(), t.elapsed());
    }
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
        let wave = find("\u{1f44b}").unwrap();
        assert!(wave.tonable());
        assert_eq!(wave.toned(0), wave.ch);
        assert_eq!(wave.toned(1), "\u{1f44b}\u{1f3fb}");
        assert_eq!(wave.toned(5), "\u{1f44b}\u{1f3ff}");
    }

    #[test]
    fn emoji_without_tones_are_left_alone() {
        let cat = find("\u{1f431}").unwrap();
        assert!(!cat.tonable());
        assert_eq!(cat.toned(3), cat.ch);
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

    #[test]
    fn search_ranks_prefixes_above_late_substrings() {
        let hits = search("cat");
        let rank = |ch: &str| hits.iter().position(|e| e.ch == ch);
        // "cat face" starts with the query; "bobcat" merely contains it.
        assert!(rank("\u{1f431}") < rank("\u{1f408}\u{200d}\u{2b1b}") || rank("\u{1f408}\u{200d}\u{2b1b}").is_none());
        assert!(hits[0].name_lower.starts_with("cat"), "got {:?}", hits[0].name);
    }

    /// The mask prefilter must never reject a candidate the matcher would have scored.
    #[test]
    fn the_skip_mask_never_hides_a_real_hit() {
        for q in ["cat", "kitten", "waving hand", "zzz", "e", "ok", "flag"] {
            let masked = search(q);
            let brute: Vec<&str> = base()
                .iter()
                .filter(|e| {
                    let p = Pattern::parse(q, CaseMatching::Ignore, Normalization::Smart);
                    let mut m = Matcher::new(Config::DEFAULT);
                    let mut b = Vec::new();
                    p.score(nucleo_matcher::Utf32Str::new(e.name, &mut b), &mut m).is_some() || {
                        b.clear();
                        p.score(nucleo_matcher::Utf32Str::new(e.keywords, &mut b), &mut m)
                            .is_some()
                    }
                })
                .map(|e| e.ch)
                .collect();
            // Only compare when the cap did not truncate, or the counts legitimately differ.
            if brute.len() <= MAX_HITS {
                assert_eq!(masked.len(), brute.len(), "query {q:?} lost or gained hits");
            }
        }
    }

    #[test]
    fn group_ranges_line_up_with_their_group() {
        for (i, g) in GROUPS.iter().enumerate() {
            let (a, b) = GROUP_RANGES[i];
            assert!(a <= b && b <= BASE_COUNT, "bad range for {g}");
            assert!(EMOJI[a..b].iter().all(|e| &e.group == g), "range leaks out of {g}");
        }
        // Every base emoji belongs to exactly one range.
        assert_eq!(GROUP_RANGES.iter().map(|(a, b)| b - a).sum::<usize>(), BASE_COUNT);
    }

    #[test]
    fn find_reaches_both_halves_of_the_table() {
        assert_eq!(find("\u{1f600}").map(|e| e.name), Some("grinning face"));
        // A tone variant lives past BASE_COUNT; recents store these verbatim.
        assert!(find("\u{1f44b}\u{1f3fb}").is_some());
        assert!(find("not an emoji").is_none());
    }
}
