use std::io::Write;

fn main() {
    println!("cargo:rerun-if-changed=data/emoji.tsv");

    let raw = std::fs::read_to_string("data/emoji.tsv").expect("data/emoji.tsv");
    let out = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("emoji_table.rs");
    let mut f = std::io::BufWriter::new(std::fs::File::create(out).unwrap());

    writeln!(f, "pub static EMOJI: &[Emoji] = &[").unwrap();
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
        writeln!(
            f,
            "    Emoji {{ ch: {:?}, group: {:?}, name: {:?}, keywords: {:?} }},",
            ch,
            group,
            name,
            keywords.trim()
        )
        .unwrap();
    }
    writeln!(f, "];").unwrap();
}
