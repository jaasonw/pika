use std::collections::HashMap;
use std::io::Write;

/// The five Fitzpatrick modifiers, in the order `emoji::TONES` lists them.
const TONES: [char; 5] = ['\u{1f3fb}', '\u{1f3fc}', '\u{1f3fd}', '\u{1f3fe}', '\u{1f3ff}'];

/// Category order shown in the tab bar, and the order base emoji are emitted in. Emitted
/// into the generated table so it cannot drift from `GROUP_RANGES`.
const GROUPS: [&str; 9] = [
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

fn is_tone(c: char) -> bool {
    TONES.contains(&c)
}

fn has_tone(s: &str) -> bool {
    s.chars().any(is_tone)
}

/// The tone-free form of a sequence, which is also its entry in the table.
fn strip_tone(s: &str) -> String {
    s.chars().filter(|c| !is_tone(*c)).collect()
}

/// Which letters a haystack contains, so search can reject a candidate without running
/// the fuzzy matcher. Bits 0-25 are `a`-`z`, bit 26 is any digit, and bit 27 means the
/// field holds non-ASCII — those are never rejected, since the matcher folds accents and
/// the mask cannot model that.
fn mask_of(s: &str) -> u32 {
    let mut m = 0u32;
    for b in s.bytes() {
        match b {
            b'a'..=b'z' => m |= 1 << (b - b'a'),
            b'A'..=b'Z' => m |= 1 << (b - b'A'),
            b'0'..=b'9' => m |= 1 << 26,
            0x80.. => m |= 1 << 27,
            _ => {}
        }
    }
    m
}

struct Row {
    ch: String,
    group: String,
    name: String,
    keywords: String,
}

fn main() {
    println!("cargo:rerun-if-changed=data/emoji.tsv");
    println!("cargo:rerun-if-changed=protocols/input-method-unstable-v1.xml");

    let raw = std::fs::read_to_string("data/emoji.tsv").expect("data/emoji.tsv");

    let mut rows: Vec<Row> = Vec::new();
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut cols = line.split('\t');
        let (Some(ch), Some(group), Some(_sub), Some(name)) =
            (cols.next(), cols.next(), cols.next(), cols.next())
        else {
            continue;
        };
        // "Component" holds skin-tone modifiers and hair styles: not standalone emoji.
        if group == "Component" {
            continue;
        }
        let keywords = cols.next().unwrap_or("").replace(" | ", " ");
        rows.push(Row {
            ch: ch.to_string(),
            group: group.to_string(),
            name: name.to_string(),
            keywords: keywords.trim().to_string(),
        });
    }

    // base sequence -> its form in each of the five tones.
    let mut toned: HashMap<(String, usize), String> = HashMap::new();
    for r in rows.iter().filter(|r| has_tone(&r.ch)) {
        let tones: Vec<char> = r.ch.chars().filter(|c| is_tone(*c)).collect();
        // Sequences mixing two different tones (couples, handshakes) have no single tone
        // to key them under; the base form stands in for those.
        let (Some(first), true) = (tones.first(), tones.windows(2).all(|w| w[0] == w[1])) else {
            continue;
        };
        let Some(idx) = TONES.iter().position(|t| t == first) else {
            continue;
        };
        toned.insert((strip_tone(&r.ch), idx), r.ch.clone());
    }

    // Base emoji first, grouped in tab order, then every tone variant. Search and
    // browsing then read a prefix of the table instead of filtering it, and `find` still
    // sees the whole thing so toned recents keep resolving.
    let mut ordered: Vec<&Row> = Vec::new();
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for g in GROUPS {
        let start = ordered.len();
        ordered.extend(rows.iter().filter(|r| r.group == g && !has_tone(&r.ch)));
        ranges.push((start, ordered.len()));
    }
    let base_count = ordered.len();
    ordered.extend(rows.iter().filter(|r| has_tone(&r.ch)));

    let out = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("emoji_table.rs");
    let mut f = std::io::BufWriter::new(std::fs::File::create(out).unwrap());

    writeln!(f, "/// Category order shown in the tab bar. \"Recents\" is synthesized by the UI.").unwrap();
    writeln!(f, "pub static GROUPS: [&str; {}] = [", GROUPS.len()).unwrap();
    for g in GROUPS {
        writeln!(f, "    {g:?},").unwrap();
    }
    writeln!(f, "];").unwrap();
    writeln!(f, "/// Base emoji occupy `..BASE_COUNT`, tone variants the rest.").unwrap();
    writeln!(f, "pub const BASE_COUNT: usize = {base_count};").unwrap();
    writeln!(f, "/// Half-open bounds of each entry of `GROUPS` within the base range.").unwrap();
    writeln!(f, "pub static GROUP_RANGES: [(usize, usize); {}] = [", ranges.len()).unwrap();
    for (a, b) in &ranges {
        writeln!(f, "    ({a}, {b}),").unwrap();
    }
    writeln!(f, "];").unwrap();

    writeln!(f, "pub static EMOJI: &[Emoji] = &[").unwrap();
    for r in &ordered {
        let tones: Vec<&str> = (0..5)
            .map(|i| {
                toned
                    .get(&(r.ch.clone(), i))
                    .map(String::as_str)
                    // Emoji that take no tone stand in for themselves, so `with_tone`
                    // is an unconditional index.
                    .unwrap_or(r.ch.as_str())
            })
            .collect();
        writeln!(
            f,
            "    Emoji {{ ch: {:?}, group: {:?}, name: {:?}, keywords: {:?}, \
             name_lower: {:?}, name_mask: {:#x}, kw_mask: {:#x}, tones: [{}] }},",
            r.ch,
            r.group,
            r.name,
            r.keywords,
            r.name.to_lowercase(),
            mask_of(&r.name),
            mask_of(&r.keywords),
            tones
                .iter()
                .map(|t| format!("{t:?}"))
                .collect::<Vec<_>>()
                .join(", "),
        )
        .unwrap();
    }
    writeln!(f, "];").unwrap();

    // Indices into EMOJI ordered by `ch`, so `find` is a binary search rather than a
    // scan of all 3,944 entries. It runs per arrow key and once per recent.
    let mut by_ch: Vec<usize> = (0..ordered.len()).collect();
    by_ch.sort_by(|&a, &b| ordered[a].ch.cmp(&ordered[b].ch));
    writeln!(f, "/// Indices into `EMOJI` sorted by `ch`, for `find`.").unwrap();
    writeln!(f, "pub static BY_CH: [u16; {}] = [", by_ch.len()).unwrap();
    for i in &by_ch {
        writeln!(f, "    {i},").unwrap();
    }
    writeln!(f, "];").unwrap();
    assert!(ordered.len() <= u16::MAX as usize, "BY_CH needs a wider index");
}
