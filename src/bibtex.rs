use crate::model::{Paper, squish};
use anyhow::Result;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

const STOPWORDS: &[&str] = &[
    "a", "an", "the", "on", "of", "for", "and", "or", "in", "to", "with", "is", "are", "at", "by",
    "from", "via", "towards", "toward", "do", "does", "what", "how", "why",
];

fn cite_key(p: &Paper, used: &mut Vec<String>) -> String {
    let author_part: String = p
        .author_list()
        .first()
        .map(|a| crate::model::last_name(a))
        .unwrap_or("unknown")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_lowercase();
    let author_part = if author_part.is_empty() {
        "unknown".into()
    } else {
        author_part
    };

    let year_part = p.year.map(|y| y.to_string()).unwrap_or_default();

    let title_word = p
        .title
        .split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .collect::<String>()
                .to_lowercase()
        })
        .find(|w| w.len() > 1 && !STOPWORDS.contains(&w.as_str()))
        .unwrap_or_default();

    let base = format!("{author_part}{year_part}{title_word}");
    let mut key = base.clone();
    let mut n = 2; // "b", "c", … "z", "aa", "ab", …
    while used.contains(&key) {
        key = format!("{base}{}", bijective_suffix(n));
        n += 1;
    }
    used.push(key.clone());
    key
}

/// 1 -> "a", 2 -> "b", … 26 -> "z", 27 -> "aa" (never produces non-letters,
/// no matter how many keys collide).
fn bijective_suffix(mut n: usize) -> String {
    let mut s = Vec::new();
    while n > 0 {
        n -= 1;
        s.push(b'a' + (n % 26) as u8);
        n /= 26;
    }
    s.reverse();
    String::from_utf8(s).unwrap()
}

fn bib_escape(s: &str) -> String {
    // Unbalanced braces would terminate the field value early and corrupt
    // the whole file; balanced ones (protective {DNA}) are legal — keep them.
    let s = if braces_balanced(s) {
        s.to_string()
    } else {
        s.chars().filter(|c| *c != '{' && *c != '}').collect()
    };
    // Keep it conservative: escape the characters that commonly break BibTeX.
    s.replace('\\', "\\textbackslash{}")
        .replace('&', "\\&")
        .replace('%', "\\%")
        .replace('#', "\\#")
        .replace('_', "\\_")
}

fn braces_balanced(s: &str) -> bool {
    let mut depth: i64 = 0;
    for c in s.chars() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth == 0
}

pub fn export(papers: &[Paper]) -> String {
    let mut used_keys = Vec::new();
    let mut out = String::new();
    for p in papers {
        let key = cite_key(p, &mut used_keys);
        let is_arxiv_only = p.arxiv_id.is_some() && p.venue.as_deref().is_none_or(|v| v == "arXiv");
        let is_proceedings = p.venue.as_deref().is_some_and(|v| {
            let v = v.to_lowercase();
            [
                "proceedings",
                "conference",
                "workshop",
                "symposium",
                "meeting",
            ]
            .iter()
            .any(|kw| v.contains(kw))
        });
        let entry_type = if is_arxiv_only {
            "misc"
        } else if is_proceedings {
            "inproceedings"
        } else if p.venue.is_some() {
            "article"
        } else {
            "misc"
        };

        let mut fields: Vec<(&str, String)> = Vec::new();
        fields.push(("title", format!("{{{}}}", bib_escape(&p.title))));
        let authors = p.author_list();
        if !authors.is_empty() {
            fields.push(("author", bib_escape(&authors.join(" and "))));
        }
        if let Some(y) = p.year {
            fields.push(("year", y.to_string()));
        }
        if let Some(v) = &p.venue
            && !is_arxiv_only
        {
            let field = match entry_type {
                "article" => "journal",
                "inproceedings" => "booktitle",
                _ => "howpublished",
            };
            fields.push((field, bib_escape(v)));
        }
        if let Some(a) = &p.arxiv_id {
            fields.push(("eprint", a.clone()));
            fields.push(("archivePrefix", "arXiv".into()));
        }
        if let Some(d) = &p.doi {
            fields.push(("doi", d.clone()));
        }
        if let Some(u) = &p.url {
            fields.push(("url", u.clone()));
        }
        if !p.tags.is_empty() {
            fields.push(("keywords", p.tags.join(", ")));
        }
        if let Some(abs) = &p.abstract_ {
            fields.push(("abstract", bib_escape(abs)));
        }

        out.push_str(&format!("@{entry_type}{{{key},\n"));
        for (name, value) in fields {
            out.push_str(&format!("  {name} = {{{value}}},\n"));
        }
        out.push_str("}\n\n");
    }
    out
}

// ---------------------------------------------------------------------------
// Import: a tolerant hand-rolled parser (brace counting, no grammar games)
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct BibEntry {
    #[allow(dead_code)] // used in tests; kept for parser completeness
    pub entry_type: String,
    pub fields: HashMap<String, String>,
}

pub fn parse(input: &str) -> Result<Vec<BibEntry>> {
    let chars: Vec<char> = input.chars().collect();
    let mut entries = Vec::new();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] != '@' {
            i += 1;
            continue;
        }
        i += 1;
        let type_start = i;
        while i < chars.len() && chars[i] != '{' && chars[i] != '(' {
            i += 1;
        }
        let entry_type: String = chars[type_start..i]
            .iter()
            .collect::<String>()
            .trim()
            .to_lowercase();
        if i >= chars.len() {
            break;
        }
        let close = if chars[i] == '{' { '}' } else { ')' };
        i += 1; // past opening brace

        if entry_type == "comment" || entry_type == "preamble" || entry_type == "string" {
            // Skip the balanced body.
            let mut depth = 1;
            while i < chars.len() && depth > 0 {
                match chars[i] {
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    c if c == close && depth == 1 => depth = 0,
                    _ => {}
                }
                i += 1;
            }
            continue;
        }

        // Citation key: up to first comma.
        let key_start = i;
        while i < chars.len() && chars[i] != ',' && chars[i] != close {
            i += 1;
        }
        let _key: String = chars[key_start..i]
            .iter()
            .collect::<String>()
            .trim()
            .to_string();
        if i < chars.len() && chars[i] == ',' {
            i += 1;
        }

        // Fields.
        let mut fields = HashMap::new();
        loop {
            while i < chars.len() && (chars[i].is_whitespace() || chars[i] == ',') {
                i += 1;
            }
            if i >= chars.len() || chars[i] == close {
                i += 1;
                break;
            }
            let name_start = i;
            while i < chars.len() && chars[i] != '=' && chars[i] != close {
                i += 1;
            }
            if i >= chars.len() || chars[i] == close {
                i += 1;
                break;
            }
            let name: String = chars[name_start..i]
                .iter()
                .collect::<String>()
                .trim()
                .to_lowercase();
            i += 1; // past '='
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            if i >= chars.len() {
                break;
            }

            let value = match chars[i] {
                '{' => {
                    i += 1;
                    let mut depth = 1;
                    let mut v = String::new();
                    while i < chars.len() && depth > 0 {
                        match chars[i] {
                            '{' => {
                                depth += 1;
                                v.push('{');
                            }
                            '}' => {
                                depth -= 1;
                                if depth > 0 {
                                    v.push('}');
                                }
                            }
                            c => v.push(c),
                        }
                        i += 1;
                    }
                    v
                }
                '"' => {
                    i += 1;
                    let mut v = String::new();
                    let mut depth: i64 = 0;
                    while i < chars.len() {
                        match chars[i] {
                            '{' => {
                                depth += 1;
                                v.push('{');
                            }
                            '}' => {
                                // Clamp at 0: a stray '}' must not suppress
                                // the closing quote and swallow the rest of
                                // the file.
                                depth = (depth - 1).max(0);
                                v.push('}');
                            }
                            '"' if depth == 0 => break,
                            c => v.push(c),
                        }
                        i += 1;
                    }
                    i += 1; // past closing quote
                    v
                }
                _ => {
                    let mut v = String::new();
                    while i < chars.len() && chars[i] != ',' && chars[i] != close {
                        v.push(chars[i]);
                        i += 1;
                    }
                    v
                }
            };
            if !name.is_empty() {
                fields.insert(name, clean_value(&value));
            }
        }

        if !entry_type.is_empty() {
            entries.push(BibEntry { entry_type, fields });
        }
    }
    Ok(entries)
}

/// Strip braces and TeX escapes, decode accent commands, collapse whitespace.
fn clean_value(v: &str) -> String {
    let decoded = decode_tex(v);
    let no_braces: String = decoded.chars().filter(|c| *c != '{' && *c != '}').collect();
    let unescaped = no_braces
        .replace("\\&", "&")
        .replace("\\%", "%")
        .replace("\\#", "#")
        .replace("\\_", "_")
        .replace("\\textbackslash", "\\");
    squish(&unescaped)
}

/// Combining mark for each TeX accent command, e.g. \' -> U+0301.
fn accent_mark(cmd: char) -> Option<char> {
    Some(match cmd {
        '\'' => '\u{0301}', // acute
        '`' => '\u{0300}',  // grave
        '^' => '\u{0302}',  // circumflex
        '"' => '\u{0308}',  // umlaut
        '~' => '\u{0303}',  // tilde
        '=' => '\u{0304}',  // macron
        '.' => '\u{0307}',  // dot above
        'u' => '\u{0306}',  // breve
        'v' => '\u{030C}',  // caron
        'H' => '\u{030B}',  // double acute
        'r' => '\u{030A}',  // ring
        'c' => '\u{0327}',  // cedilla
        'k' => '\u{0328}',  // ogonek
        'b' => '\u{0331}',  // bar under
        'd' => '\u{0323}',  // dot under
        _ => return None,
    })
}

/// Decode TeX diacritics ({\'{e}}, \"u, \v{s}) and letter commands (\ss, \o)
/// to Unicode, the way Zotero/Google Scholar exports use them. Unknown
/// commands pass through untouched.
fn decode_tex(s: &str) -> String {
    const WORDS: &[(&str, &str)] = &[
        ("ss", "ß"),
        ("ae", "æ"),
        ("AE", "Æ"),
        ("oe", "œ"),
        ("OE", "Œ"),
        ("aa", "å"),
        ("AA", "Å"),
        ("o", "ø"),
        ("O", "Ø"),
        ("l", "ł"),
        ("L", "Ł"),
        ("i", "ı"),
        ("j", "ȷ"),
    ];
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '\\' || i + 1 >= chars.len() {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let cmd = chars[i + 1];

        // Symbol accents: \'e, \'{e}, \"u — anything in accent_mark that
        // isn't a letter (letter-named ones are handled below to avoid
        // colliding with word commands like \v vs \venue).
        if !cmd.is_ascii_alphabetic() {
            if let Some(mark) = accent_mark(cmd)
                && let Some((base, next)) = accent_target(&chars, i + 2)
            {
                out.push(base);
                out.push(mark);
                i = next;
                continue;
            }
            out.push('\\');
            i += 1;
            continue;
        }

        // Word commands: \ss, \o, \ae … must end at a non-letter.
        let word_end = (i + 1..chars.len())
            .find(|&j| !chars[j].is_ascii_alphabetic())
            .unwrap_or(chars.len());
        let word: String = chars[i + 1..word_end].iter().collect();
        if let Some((_, repl)) = WORDS.iter().find(|(w, _)| *w == word) {
            out.push_str(repl);
            i = word_end;
            // Consume one space that only served to end the command (\o slash).
            if i < chars.len() && chars[i] == ' ' {
                i += 1;
            }
            continue;
        }

        // Letter-named accents require a braced or spaced argument: \v{s}, \c c.
        if word.len() == 1
            && let Some(mark) = accent_mark(cmd)
        {
            let mut j = i + 2;
            while j < chars.len() && chars[j] == ' ' {
                j += 1;
            }
            if j < chars.len()
                && (chars[j] == '{' || chars[j].is_ascii_alphabetic())
                && let Some((base, next)) = accent_target(&chars, j)
            {
                out.push(base);
                out.push(mark);
                i = next;
                continue;
            }
        }

        out.push('\\');
        i += 1;
    }
    // NFC so decoded accents match what arXiv/Crossref deliver (é, not e+◌́).
    use unicode_normalization::UnicodeNormalization;
    out.nfc().collect()
}

/// Parse the letter an accent applies to, at `chars[at..]`: either `e`,
/// `{e}`, or `{\i}`. Returns (letter, index after it).
fn accent_target(chars: &[char], at: usize) -> Option<(char, usize)> {
    if at >= chars.len() {
        return None;
    }
    if chars[at] == '{' {
        // {e} or {\i} — under an accent, TeX's dotless \i/\j stand for plain
        // i/j (the accent replaces the dot), so í composes correctly.
        if at + 2 < chars.len() && chars[at + 1] == '\\' && chars[at + 3..].first() == Some(&'}') {
            return Some((chars[at + 2], at + 4));
        }
        if at + 2 < chars.len() && chars[at + 2] == '}' && chars[at + 1].is_alphabetic() {
            return Some((chars[at + 1], at + 3));
        }
        return None;
    }
    if chars[at].is_alphabetic() {
        return Some((chars[at], at + 1));
    }
    None
}

/// "Last, First" -> "First Last"; passthrough otherwise.
pub fn normalize_author(name: &str) -> String {
    match name.split_once(',') {
        Some((last, first)) => squish(&format!("{} {}", first.trim(), last.trim())),
        None => squish(name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_entry() {
        let src = r#"
        @article{vaswani2017attention,
          title = {Attention Is All You Need},
          author = {Vaswani, Ashish and Shazeer, Noam},
          year = 2017,
          journal = "NeurIPS",
        }
        "#;
        let entries = parse(src).unwrap();
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e.entry_type, "article");
        assert_eq!(e.fields["title"], "Attention Is All You Need");
        assert_eq!(e.fields["year"], "2017");
        assert_eq!(e.fields["journal"], "NeurIPS");
        assert_eq!(normalize_author("Vaswani, Ashish"), "Ashish Vaswani");
    }

    #[test]
    fn parses_nested_braces_and_comments() {
        let src = r#"
        @comment{ this { is } ignored }
        @misc{key1,
          title = {{Deep {Learning}} for \& by Everyone},
          note = {multi
                  line}
        }
        "#;
        let entries = parse(src).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].fields["title"],
            "Deep Learning for & by Everyone"
        );
        assert_eq!(entries[0].fields["note"], "multi line");
    }

    #[test]
    fn decodes_tex_accents() {
        assert_eq!(
            clean_value(r"Th{\'{e}}venaz, Cl{\'e}ment"),
            "Thévenaz, Clément"
        );
        assert_eq!(clean_value(r#"M\"uller and G\"{o}del"#), "Müller and Gödel");
        assert_eq!(
            clean_value(r"\v{S}koda \c{c} \~n \o str\o m"),
            "Škoda ç ñ østrøm"
        );
        assert_eq!(clean_value(r"{\ss} and \ae"), "ß and æ");
        assert_eq!(clean_value(r"\'{\i}ndice"), "índice");
        // Unknown commands pass through.
        assert_eq!(clean_value(r"uses \alpha decay"), r"uses \alpha decay");
    }

    #[test]
    fn stray_brace_in_quoted_value_does_not_swallow_file() {
        let src = "@article{a,\n  title = \"Survey of C} languages\",\n  year = \"2020\"\n}\n\
                   @article{b,\n  title = {Second Paper},\n  year = {2021}\n}\n";
        let entries = parse(src).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].fields["title"], "Second Paper");
    }

    #[test]
    fn export_escapes_unbalanced_braces() {
        let mut p = sample_paper();
        p.title = "A Lone Closing Brace } in a Title".into();
        p.arxiv_id = None;
        let bib = export(&[p]);
        let depth = bib.chars().fold(0i64, |d, c| match c {
            '{' => d + 1,
            '}' => d - 1,
            _ => d,
        });
        assert_eq!(depth, 0, "export must stay brace-balanced:\n{bib}");
        // Balanced protective braces survive.
        let mut p2 = sample_paper();
        p2.title = "About {DNA} folding".into();
        assert!(export(&[p2]).contains("About {DNA} folding"));
    }

    #[test]
    fn cite_key_suffixes_stay_alphabetic() {
        let mut used = vec![];
        let p = sample_paper();
        let keys: Vec<String> = (0..30).map(|_| cite_key(&p, &mut used)).collect();
        assert!(
            keys.iter()
                .all(|k| k.chars().all(|c| c.is_ascii_alphanumeric())),
            "{keys:?}"
        );
        assert_eq!(
            keys.len(),
            keys.iter().collect::<std::collections::HashSet<_>>().len()
        );
    }

    fn sample_paper() -> Paper {
        Paper {
            id: 1,
            title: "Attention Is All You Need".into(),
            authors: "Ashish Vaswani; Noam Shazeer".into(),
            year: Some(2017),
            venue: None,
            arxiv_id: Some("1706.03762".into()),
            doi: None,
            url: Some("https://arxiv.org/abs/1706.03762".into()),
            pdf_url: None,
            abstract_: None,
            status: crate::model::Status::ToRead,
            priority: crate::model::Priority::Normal,
            rating: None,
            tags: vec!["transformers".into()],
            added_at: "2026-01-01 00:00:00".into(),
            started_at: None,
            finished_at: None,
            notes: vec![],
        }
    }

    #[test]
    fn export_roundtrip() {
        let p = Paper {
            id: 1,
            title: "Attention Is All You Need".into(),
            authors: "Ashish Vaswani; Noam Shazeer".into(),
            year: Some(2017),
            venue: None,
            arxiv_id: Some("1706.03762".into()),
            doi: None,
            url: Some("https://arxiv.org/abs/1706.03762".into()),
            pdf_url: None,
            abstract_: None,
            status: crate::model::Status::ToRead,
            priority: crate::model::Priority::Normal,
            rating: None,
            tags: vec!["transformers".into()],
            added_at: "2026-01-01 00:00:00".into(),
            started_at: None,
            finished_at: None,
            notes: vec![],
        };
        let bib = export(&[p]);
        assert!(bib.contains("@misc{vaswani2017attention,"));
        let entries = parse(&bib).unwrap();
        assert_eq!(entries[0].fields["title"], "Attention Is All You Need");
        assert_eq!(entries[0].fields["eprint"], "1706.03762");
        assert_eq!(
            normalize_author(entries[0].fields["author"].split(" and ").next().unwrap()),
            "Ashish Vaswani"
        );
    }
}
