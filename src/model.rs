use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    ToRead,
    Reading,
    Read,
    Dropped,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::ToRead => "to-read",
            Status::Reading => "reading",
            Status::Read => "read",
            Status::Dropped => "dropped",
        }
    }

    pub fn parse(s: &str) -> Option<Status> {
        match s.to_lowercase().as_str() {
            "to-read" | "toread" | "todo" | "unread" | "t" => Some(Status::ToRead),
            "reading" | "started" | "in-progress" | "s" => Some(Status::Reading),
            "read" | "done" | "finished" | "r" | "d" => Some(Status::Read),
            "dropped" | "drop" | "abandoned" | "x" => Some(Status::Dropped),
            _ => None,
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Status::ToRead => "○",
            Status::Reading => "◐",
            Status::Read => "●",
            Status::Dropped => "✗",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    Low = 0,
    Normal = 1,
    High = 2,
}

impl Priority {
    pub fn as_str(self) -> &'static str {
        match self {
            Priority::Low => "low",
            Priority::Normal => "normal",
            Priority::High => "high",
        }
    }

    pub fn parse(s: &str) -> Option<Priority> {
        match s.to_lowercase().as_str() {
            "low" | "l" | "0" => Some(Priority::Low),
            "normal" | "med" | "medium" | "n" | "m" | "1" => Some(Priority::Normal),
            "high" | "h" | "2" => Some(Priority::High),
            _ => None,
        }
    }

    pub fn from_int(i: i64) -> Priority {
        match i {
            0 => Priority::Low,
            2 => Priority::High,
            _ => Priority::Normal,
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Priority::Low => "↓",
            Priority::Normal => " ",
            Priority::High => "↑",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Paper {
    pub id: i64,
    pub title: String,
    /// "First Last; First Last" separated authors.
    pub authors: String,
    pub year: Option<i32>,
    pub venue: Option<String>,
    pub arxiv_id: Option<String>,
    pub doi: Option<String>,
    pub url: Option<String>,
    pub pdf_url: Option<String>,
    #[serde(rename = "abstract")]
    pub abstract_: Option<String>,
    pub status: Status,
    pub priority: Priority,
    pub rating: Option<i64>,
    pub tags: Vec<String>,
    pub added_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<Note>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub created_at: String,
    pub body: String,
}

impl Paper {
    pub fn author_list(&self) -> Vec<&str> {
        self.authors
            .split(';')
            .map(|a| a.trim())
            .filter(|a| !a.is_empty())
            .collect()
    }

    /// Compact author display: "Vaswani +7" / "Smith, Jones".
    pub fn short_authors(&self) -> String {
        let authors = self.author_list();
        match authors.len() {
            0 => String::new(),
            1 => last_name(authors[0]).to_string(),
            2 => format!("{}, {}", last_name(authors[0]), last_name(authors[1])),
            n => format!("{} +{}", last_name(authors[0]), n - 1),
        }
    }

    /// The best link to open in a browser.
    pub fn best_url(&self, prefer_pdf: bool) -> Option<&str> {
        let page = self.url.as_deref().or(self.pdf_url.as_deref());
        let pdf = self.pdf_url.as_deref().or(self.url.as_deref());
        if prefer_pdf { pdf } else { page }
    }
}

pub fn last_name(full: &str) -> &str {
    full.rsplit(' ').next().unwrap_or(full)
}

/// Normalize a tag: lowercase, trim, inner whitespace -> '-'. Commas are
/// stripped because tags are stored comma-joined.
pub fn normalize_tag(tag: &str) -> String {
    tag.trim()
        .to_lowercase()
        .replace(',', "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
}

pub fn normalize_tags(tags: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in tags {
        for part in t.split(',') {
            let n = normalize_tag(part);
            if !n.is_empty() && !out.contains(&n) {
                out.push(n);
            }
        }
    }
    out
}

/// Collapse all whitespace runs (incl. newlines) into single spaces.
pub fn squish(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}
