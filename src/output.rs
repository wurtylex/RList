use crate::model::{Paper, Priority, Status};
use std::io::IsTerminal;
use std::sync::OnceLock;
use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

static COLOR: OnceLock<bool> = OnceLock::new();

pub fn init_color(force_off: bool) {
    let enabled =
        !force_off && std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal();
    let _ = COLOR.set(enabled);
}

fn color_on() -> bool {
    *COLOR.get().unwrap_or(&false)
}

pub const RED: &str = "31";
pub const GREEN: &str = "32";
pub const YELLOW: &str = "33";
pub const BLUE: &str = "34";
pub const MAGENTA: &str = "35";
pub const CYAN: &str = "36";
pub const BOLD: &str = "1";
pub const DIM: &str = "2";

pub fn paint(code: &str, s: &str) -> String {
    if color_on() && !s.is_empty() {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

pub fn status_style(s: Status) -> &'static str {
    match s {
        Status::ToRead => YELLOW,
        Status::Reading => BLUE,
        Status::Read => GREEN,
        Status::Dropped => DIM,
    }
}

pub fn priority_style(p: Priority) -> &'static str {
    match p {
        Priority::High => RED,
        Priority::Normal => "0",
        Priority::Low => DIM,
    }
}

pub fn term_width() -> usize {
    terminal_size::terminal_size()
        .map(|(w, _)| w.0 as usize)
        .unwrap_or(100)
        .max(60)
}

/// Truncate to `width` display columns, appending '…' if cut.
pub fn truncate(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = c.width().unwrap_or(0);
        if w + cw > width.saturating_sub(1) {
            break;
        }
        out.push(c);
        w += cw;
    }
    out.push('…');
    out
}

fn pad(s: &str, width: usize) -> String {
    let w = s.width();
    if w >= width {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(width - w))
    }
}

pub fn rating_stars(rating: Option<i64>) -> String {
    match rating {
        Some(r) => "★".repeat(r.clamp(0, 5) as usize) + &"☆".repeat((5 - r.clamp(0, 5)) as usize),
        None => String::new(),
    }
}

/// Render the main paper table.
pub fn table(papers: &[Paper]) {
    if papers.is_empty() {
        println!(
            "{}",
            paint(
                DIM,
                "nothing here, add a paper with `rlist add <arxiv-id|doi|url|title>`"
            )
        );
        return;
    }

    let total = term_width();
    let id_w = papers
        .iter()
        .map(|p| p.id.to_string().len())
        .max()
        .unwrap_or(2)
        .max(2);
    let year_w = 4;
    let glyph_w = 3; // status + priority + space
    let gaps = 5 * 2; // column separators
    let flex = total.saturating_sub(id_w + glyph_w + year_w + gaps).max(30);

    let tag_w = (flex * 22 / 100).min(24);
    let auth_w = (flex * 22 / 100).min(22);
    let title_w = flex - tag_w - auth_w;

    let header = format!(
        "{}  {}  {}  {}  {}  {}",
        pad("ID", id_w),
        pad("", glyph_w - 1),
        pad("TITLE", title_w),
        pad("AUTHORS", auth_w),
        pad("YEAR", year_w),
        pad("TAGS", tag_w),
    );
    println!("{}", paint(DIM, header.trim_end()));

    for p in papers {
        let glyphs = format!(
            "{}{}",
            paint(status_style(p.status), p.status.glyph()),
            paint(priority_style(p.priority), p.priority.glyph()),
        );
        let title = if p.status == Status::Read || p.status == Status::Dropped {
            paint(DIM, &truncate(&p.title, title_w))
        } else {
            truncate(&p.title, title_w)
        };
        // Padding must be computed on the unstyled width.
        let title_pad = " ".repeat(title_w.saturating_sub(truncate(&p.title, title_w).width()));
        let year = p.year.map(|y| y.to_string()).unwrap_or_default();
        let tags = truncate(&p.tags.join(","), tag_w);
        let row = format!(
            "{}  {}  {}{}  {}  {}  {}",
            pad(&p.id.to_string(), id_w),
            glyphs,
            title,
            title_pad,
            paint(DIM, &pad(&truncate(&p.short_authors(), auth_w), auth_w)),
            pad(&year, year_w),
            paint(CYAN, &tags),
        );
        println!("{}", row.trim_end());
    }
}

/// Word-wrap to `width`, with each line prefixed by `indent`.
pub fn wrap(text: &str, width: usize, indent: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let need = if line.is_empty() {
            word.width()
        } else {
            line.width() + 1 + word.width()
        };
        if need > width && !line.is_empty() {
            lines.push(line.clone());
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
        .iter()
        .map(|l| format!("{indent}{l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Full detail view for `rlist show`.
pub fn detail(p: &Paper) {
    let width = term_width().min(100);
    println!();
    println!(
        "  {}",
        paint(BOLD, &crate::output::wrap_first(&p.title, width - 2))
    );
    if !p.authors.is_empty() {
        println!(
            "  {}",
            paint(DIM, &wrap_first(&p.author_list().join(", "), width - 2))
        );
    }

    let mut meta = Vec::new();
    if let Some(y) = p.year {
        meta.push(y.to_string());
    }
    if let Some(v) = &p.venue {
        meta.push(v.clone());
    }
    if !meta.is_empty() {
        println!("  {}", meta.join(" · "));
    }
    println!();

    let label = |s: &str| paint(DIM, &format!("  {s:<10}"));
    let status_line = format!(
        "{} {} {}",
        label("status"),
        paint(
            status_style(p.status),
            &format!("{} {}", p.status.glyph(), p.status.as_str())
        ),
        match (p.status, &p.started_at, &p.finished_at) {
            (Status::Reading, Some(t), _) => paint(DIM, &format!("(since {})", date_of(t))),
            (Status::Read, _, Some(t)) => paint(DIM, &format!("(finished {})", date_of(t))),
            _ => String::new(),
        }
    );
    println!("{}", status_line.trim_end());
    println!(
        "{} {}",
        label("priority"),
        paint(priority_style(p.priority), p.priority.as_str())
    );
    if p.rating.is_some() {
        println!(
            "{} {}",
            label("rating"),
            paint(YELLOW, &rating_stars(p.rating))
        );
    }
    if !p.tags.is_empty() {
        println!("{} {}", label("tags"), paint(CYAN, &p.tags.join(", ")));
    }
    if let Some(a) = &p.arxiv_id {
        println!("{} {}", label("arxiv"), a);
    }
    if let Some(d) = &p.doi {
        println!("{} {}", label("doi"), d);
    }
    if let Some(u) = &p.url {
        println!("{} {}", label("url"), paint(BLUE, u));
    }
    if let Some(u) = &p.pdf_url {
        println!("{} {}", label("pdf"), paint(BLUE, u));
    }
    println!("{} {}", label("added"), date_of(&p.added_at));

    if let Some(abs) = &p.abstract_ {
        println!();
        println!("{}", paint(DIM, "  abstract"));
        println!("{}", wrap(abs, width - 4, "  "));
    }

    if !p.notes.is_empty() {
        println!();
        println!("{}", paint(DIM, &format!("  notes ({})", p.notes.len())));
        for n in &p.notes {
            // "[YYYY-MM-DD] " is 13 columns, and continuations hang to match.
            println!(
                "  {} {}",
                paint(MAGENTA, &format!("[{}]", date_of(&n.created_at))),
                wrap_hanging(&n.body, width - 4, 13)
            );
        }
    }
    println!();
}

/// First-line-only wrap helper for headers (keeps title on as few lines as
/// possible while still respecting width).
pub fn wrap_first(text: &str, width: usize) -> String {
    wrap(text, width, "").replacen('\n', "\n  ", usize::MAX)
}

fn wrap_hanging(text: &str, width: usize, hang: usize) -> String {
    let wrapped = wrap(text, width.saturating_sub(hang), "");
    wrapped.replace('\n', &format!("\n  {}", " ".repeat(hang)))
}

pub fn date_of(timestamp: &str) -> &str {
    timestamp.split(' ').next().unwrap_or(timestamp)
}

/// A one-line confirmation for a paper, e.g. after add/done.
pub fn confirm_line(verb: &str, p: &Paper) {
    println!(
        "{} {} {}",
        paint(GREEN, verb),
        paint(BOLD, &format!("#{}", p.id)),
        truncate(&p.title, term_width().saturating_sub(verb.len() + 8)),
    );
}

/// Simple horizontal bar for stats.
pub fn bar(count: usize, max: usize, width: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let filled = (count * width).div_ceil(max);
    "█".repeat(filled.min(width))
}
