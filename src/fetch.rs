use crate::model::squish;
use anyhow::{Context, Result, anyhow, bail};
use std::time::Duration;

/// What kind of reference the user handed to `rlist add`.
#[derive(Debug, Clone, PartialEq)]
pub enum RefKind {
    /// Normalized arXiv id without version (e.g. "1706.03762"), plus the
    /// version-qualified id to fetch if one was given.
    Arxiv {
        id: String,
        fetch_id: String,
    },
    Doi(String),
    Url(String),
    Title(String),
}

/// Metadata fetched from arXiv / Crossref / doi.org.
#[derive(Debug, Default, Clone)]
pub struct Fetched {
    pub title: String,
    pub authors: Vec<String>,
    pub year: Option<i32>,
    pub venue: Option<String>,
    pub abstract_: Option<String>,
    pub url: Option<String>,
    pub pdf_url: Option<String>,
    pub doi: Option<String>,
    pub arxiv_id: Option<String>,
}

fn is_new_style_arxiv(s: &str) -> Option<(String, String)> {
    // NNNN.NNNN or NNNN.NNNNN, optional vN suffix.
    let (base, _version) = split_version(s);
    let bytes = base.as_bytes();
    let dot = base.find('.')?;
    if dot != 4 {
        return None;
    }
    let (a, b) = (&base[..dot], &base[dot + 1..]);
    if a.len() == 4
        && (4..=5).contains(&b.len())
        && a.bytes().all(|c| c.is_ascii_digit())
        && b.bytes().all(|c| c.is_ascii_digit())
        && bytes.iter().all(|c| c.is_ascii_digit() || *c == b'.')
    {
        Some((base.to_string(), s.to_string()))
    } else {
        None
    }
}

fn is_old_style_arxiv(s: &str) -> Option<(String, String)> {
    // e.g. math/0211159, cs.AI/0301001, optional vN.
    let (base, _version) = split_version(s);
    let slash = base.find('/')?;
    let (cat, num) = (&base[..slash], &base[slash + 1..]);
    let cat_ok = !cat.is_empty()
        && cat
            .bytes()
            .all(|c| c.is_ascii_alphabetic() || c == b'-' || c == b'.');
    let num_ok = num.len() == 7 && num.bytes().all(|c| c.is_ascii_digit());
    if cat_ok && num_ok {
        Some((base.to_string(), s.to_string()))
    } else {
        None
    }
}

/// Split a trailing arXiv version suffix: "1706.03762v5" -> ("1706.03762", "v5").
fn split_version(s: &str) -> (&str, &str) {
    if let Some(pos) = s.rfind('v') {
        let tail = &s[pos + 1..];
        if !tail.is_empty() && tail.bytes().all(|c| c.is_ascii_digit()) {
            return (&s[..pos], &s[pos..]);
        }
    }
    (s, "")
}

fn looks_like_doi(s: &str) -> bool {
    s.starts_with("10.")
        && s[3..].find('/').is_some_and(|slash| {
            let registrant = &s[3..3 + slash];
            !registrant.is_empty()
                && registrant.bytes().all(|c| c.is_ascii_digit() || c == b'.')
                && s.len() > 3 + slash + 1
        })
}

fn arxiv_from_str(s: &str) -> Option<RefKind> {
    is_new_style_arxiv(s)
        .or_else(|| is_old_style_arxiv(s))
        .map(|(id, fetch_id)| RefKind::Arxiv { id, fetch_id })
}

/// Figure out what the user gave us: arXiv id/URL, DOI, generic URL, or a
/// plain title for a manual entry.
pub fn classify(input: &str) -> RefKind {
    let s = input.trim();

    if let Some(rest) = strip_prefix_ci(s, "arxiv:")
        && let Some(r) = arxiv_from_str(rest.trim())
    {
        return r;
    }
    if let Some(rest) = strip_prefix_ci(s, "doi:") {
        let rest = rest.trim();
        if looks_like_doi(rest) {
            return RefKind::Doi(rest.to_string());
        }
    }

    if s.starts_with("http://") || s.starts_with("https://") {
        return classify_url(s);
    }

    if let Some(r) = arxiv_from_str(s) {
        return r;
    }
    if looks_like_doi(s) {
        return RefKind::Doi(s.to_string());
    }
    RefKind::Title(s.to_string())
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    // The boundary check matters: prefix.len() can fall inside a multibyte
    // character of user input ("arxivé…"), and slicing there would panic.
    if s.len() >= prefix.len()
        && s.is_char_boundary(prefix.len())
        && s[..prefix.len()].eq_ignore_ascii_case(prefix)
    {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

/// Strip a trailing version suffix from an arXiv id ("1706.03762v5" ->
/// "1706.03762"). Used to normalize ids from imports as well.
pub fn normalize_arxiv_id(s: &str) -> String {
    split_version(s.trim()).0.to_string()
}

fn classify_url(url: &str) -> RefKind {
    let without_scheme = url
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let (host, path) = match without_scheme.find('/') {
        Some(i) => (&without_scheme[..i], &without_scheme[i + 1..]),
        None => (without_scheme, ""),
    };
    let host = host.trim_start_matches("www.").to_lowercase();
    let path = path.split(['?', '#']).next().unwrap_or("");

    if host == "arxiv.org" || host == "export.arxiv.org" {
        let id_part = path
            .trim_start_matches("abs/")
            .trim_start_matches("pdf/")
            .trim_start_matches("html/")
            .trim_end_matches(".pdf")
            .trim_end_matches('/');
        if let Some(r) = arxiv_from_str(id_part) {
            return r;
        }
    }
    if host == "doi.org" || host == "dx.doi.org" {
        let doi = percent_decode(path.trim_end_matches('/'));
        if looks_like_doi(&doi) {
            return RefKind::Doi(doi);
        }
    }
    RefKind::Url(url.to_string())
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        // Work on bytes only: '%' followed by part of a multibyte character
        // must fall through as-is, not slice the str mid-character.
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Some(b) = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(20))
        .user_agent(concat!(
            "rlist/",
            env!("CARGO_PKG_VERSION"),
            " (academic reading-list CLI)"
        ))
        .build()
}

pub fn fetch_arxiv(fetch_id: &str, base_id: &str) -> Result<Fetched> {
    let url = format!("https://export.arxiv.org/api/query?id_list={fetch_id}&max_results=1");
    let body = agent()
        .get(&url)
        .call()
        .with_context(|| format!("querying arXiv for {fetch_id}"))?
        .into_string()
        .context("reading arXiv response")?;

    let doc = roxmltree::Document::parse(&body).context("parsing arXiv response XML")?;
    let entry = doc
        .descendants()
        .find(|n| n.has_tag_name(("http://www.w3.org/2005/Atom", "entry")))
        .ok_or_else(|| anyhow!("arXiv returned no entry for {fetch_id} — is the id correct?"))?;

    let text_of = |local: &str| -> Option<String> {
        entry
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == local)
            .and_then(|n| n.text())
            .map(squish)
            .filter(|s| !s.is_empty())
    };

    let title = text_of("title").unwrap_or_default();
    let entry_id = text_of("id").unwrap_or_default();
    if title.eq_ignore_ascii_case("error") || entry_id.contains("/api/errors") {
        let detail = text_of("summary").unwrap_or_else(|| "unknown error".into());
        bail!("arXiv error for {fetch_id}: {detail}");
    }
    if title.is_empty() {
        bail!("arXiv entry for {fetch_id} has no title");
    }

    let authors: Vec<String> = entry
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "author")
        .filter_map(|a| {
            a.children()
                .find(|n| n.is_element() && n.tag_name().name() == "name")
                .and_then(|n| n.text())
                .map(squish)
        })
        .filter(|s| !s.is_empty())
        .collect();

    let year = text_of("published").and_then(|p| p.get(..4).and_then(|y| y.parse::<i32>().ok()));

    let mut pdf_url = None;
    let mut abs_url = None;
    for link in entry
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "link")
    {
        let href = link.attribute("href").unwrap_or("");
        if link.attribute("title") == Some("pdf") {
            pdf_url = Some(href.to_string());
        } else if link.attribute("rel") == Some("alternate") {
            abs_url = Some(href.to_string());
        }
    }

    // arXiv-namespaced extras: journal_ref, doi.
    let arxiv_extra = |local: &str| -> Option<String> {
        entry
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == local)
            .and_then(|n| n.text())
            .map(squish)
            .filter(|s| !s.is_empty())
    };

    Ok(Fetched {
        title,
        authors,
        year,
        venue: arxiv_extra("journal_ref"),
        abstract_: text_of("summary"),
        url: abs_url.or_else(|| Some(format!("https://arxiv.org/abs/{base_id}"))),
        pdf_url: pdf_url.or_else(|| Some(format!("https://arxiv.org/pdf/{base_id}"))),
        doi: arxiv_extra("doi"),
        arxiv_id: Some(base_id.to_string()),
    })
}

pub fn fetch_doi(doi: &str) -> Result<Fetched> {
    match fetch_crossref(doi) {
        Ok(f) => Ok(f),
        Err(crossref_err) => fetch_doi_org(doi).map_err(|fallback_err| {
            anyhow!("could not resolve DOI {doi}\n  crossref: {crossref_err:#}\n  doi.org: {fallback_err:#}")
        }),
    }
}

fn fetch_crossref(doi: &str) -> Result<Fetched> {
    let url = format!(
        "https://api.crossref.org/works/{}",
        percent_encode_path(doi)
    );
    let json: serde_json::Value = agent()
        .get(&url)
        .call()
        .with_context(|| format!("querying Crossref for {doi}"))?
        .into_json()
        .context("parsing Crossref response")?;
    let msg = &json["message"];

    let title = msg["title"][0].as_str().map(squish).unwrap_or_default();
    if title.is_empty() {
        bail!("Crossref record for {doi} has no title");
    }

    let authors: Vec<String> = msg["author"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|a| {
                    if let Some(name) = a["name"].as_str() {
                        return Some(squish(name));
                    }
                    let given = a["given"].as_str().unwrap_or("");
                    let family = a["family"].as_str().unwrap_or("");
                    let full = format!("{given} {family}");
                    let full = squish(&full);
                    if full.is_empty() { None } else { Some(full) }
                })
                .collect()
        })
        .unwrap_or_default();

    let year = ["issued", "published-print", "published-online", "created"]
        .iter()
        .find_map(|k| msg[k]["date-parts"][0][0].as_i64())
        .map(|y| y as i32);

    let venue = msg["container-title"][0]
        .as_str()
        .map(squish)
        .filter(|s| !s.is_empty());

    let pdf_url = msg["link"].as_array().and_then(|links| {
        links
            .iter()
            .find(|l| l["content-type"].as_str() == Some("application/pdf"))
            .and_then(|l| l["URL"].as_str())
            .map(String::from)
    });

    Ok(Fetched {
        title,
        authors,
        year,
        venue,
        abstract_: msg["abstract"]
            .as_str()
            .map(strip_jats)
            .filter(|s| !s.is_empty()),
        url: Some(format!("https://doi.org/{doi}")),
        pdf_url,
        doi: Some(msg["DOI"].as_str().unwrap_or(doi).to_string()),
        arxiv_id: None,
    })
}

/// Fallback for DOIs not registered with Crossref (e.g. DataCite): doi.org
/// content negotiation for CSL JSON.
fn fetch_doi_org(doi: &str) -> Result<Fetched> {
    let url = format!("https://doi.org/{}", percent_encode_path(doi));
    let json: serde_json::Value = agent()
        .get(&url)
        .set("Accept", "application/vnd.citationstyles.csl+json")
        .call()
        .with_context(|| format!("resolving {doi} via doi.org"))?
        .into_json()
        .context("parsing doi.org CSL JSON")?;

    let title = json["title"].as_str().map(squish).unwrap_or_default();
    if title.is_empty() {
        bail!("doi.org record for {doi} has no title");
    }
    let authors: Vec<String> = json["author"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|a| {
                    if let Some(lit) = a["literal"].as_str() {
                        return Some(squish(lit));
                    }
                    let full = format!(
                        "{} {}",
                        a["given"].as_str().unwrap_or(""),
                        a["family"].as_str().unwrap_or("")
                    );
                    let full = squish(&full);
                    if full.is_empty() { None } else { Some(full) }
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(Fetched {
        title,
        authors,
        year: json["issued"]["date-parts"][0][0]
            .as_i64()
            .map(|y| y as i32),
        venue: json["container-title"]
            .as_str()
            .map(squish)
            .filter(|s| !s.is_empty()),
        abstract_: json["abstract"]
            .as_str()
            .map(strip_jats)
            .filter(|s| !s.is_empty()),
        url: Some(format!("https://doi.org/{doi}")),
        pdf_url: None,
        doi: Some(doi.to_string()),
        arxiv_id: None,
    })
}

fn percent_encode_path(s: &str) -> String {
    // Encode characters that are problematic in a URL path; keep '/' which is
    // structural in DOIs and accepted by both APIs.
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'.'
            | b'_'
            | b'~'
            | b'/'
            | b'('
            | b')'
            | b':'
            | b';'
            | b',' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Strip JATS/XML tags from Crossref abstracts ("<jats:p>text</jats:p>").
fn strip_jats(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    let out = out
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'");
    let out = squish(&out);
    out.strip_prefix("Abstract ")
        .map(String::from)
        .unwrap_or(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_arxiv_ids() {
        assert_eq!(
            classify("1706.03762"),
            RefKind::Arxiv {
                id: "1706.03762".into(),
                fetch_id: "1706.03762".into()
            }
        );
        assert_eq!(
            classify("2406.01234v2"),
            RefKind::Arxiv {
                id: "2406.01234".into(),
                fetch_id: "2406.01234v2".into()
            }
        );
        assert_eq!(
            classify("arXiv:1706.03762"),
            RefKind::Arxiv {
                id: "1706.03762".into(),
                fetch_id: "1706.03762".into()
            }
        );
        assert_eq!(
            classify("math/0211159"),
            RefKind::Arxiv {
                id: "math/0211159".into(),
                fetch_id: "math/0211159".into()
            }
        );
        assert_eq!(
            classify("cs.AI/0301001v1"),
            RefKind::Arxiv {
                id: "cs.AI/0301001".into(),
                fetch_id: "cs.AI/0301001v1".into()
            }
        );
    }

    #[test]
    fn classify_arxiv_urls() {
        for url in [
            "https://arxiv.org/abs/1706.03762",
            "https://arxiv.org/pdf/1706.03762.pdf",
            "https://arxiv.org/pdf/1706.03762",
            "http://www.arxiv.org/abs/1706.03762v5",
            "https://arxiv.org/abs/1706.03762?context=cs.LG",
        ] {
            match classify(url) {
                RefKind::Arxiv { id, .. } => assert_eq!(id, "1706.03762", "{url}"),
                other => panic!("{url} classified as {other:?}"),
            }
        }
    }

    #[test]
    fn classify_dois() {
        assert_eq!(
            classify("10.1038/nature14539"),
            RefKind::Doi("10.1038/nature14539".into())
        );
        assert_eq!(
            classify("doi:10.1038/nature14539"),
            RefKind::Doi("10.1038/nature14539".into())
        );
        assert_eq!(
            classify("https://doi.org/10.1038/nature14539"),
            RefKind::Doi("10.1038/nature14539".into())
        );
        assert_eq!(
            classify("https://doi.org/10.1145/3292500.3330701"),
            RefKind::Doi("10.1145/3292500.3330701".into())
        );
    }

    #[test]
    fn classify_other() {
        assert_eq!(
            classify("https://example.com/paper.pdf"),
            RefKind::Url("https://example.com/paper.pdf".into())
        );
        assert_eq!(
            classify("Attention Is All You Need"),
            RefKind::Title("Attention Is All You Need".into())
        );
        // Things that look numeric but aren't arXiv ids stay titles.
        assert_eq!(classify("12.34"), RefKind::Title("12.34".into()));
        assert_eq!(classify("10.5"), RefKind::Title("10.5".into()));
    }

    #[test]
    fn classify_never_panics_on_multibyte_input() {
        // Regression: byte-index slicing panicked when a prefix length fell
        // inside a multibyte character.
        for s in [
            "arxivé test paper",
            "ArXiv’s effect on preprints",
            "doié accents",
            "深層学習による調査",
            "café résumé study",
            "🤖 robots reading papers",
            "Метод обучения",
        ] {
            assert_eq!(classify(s), RefKind::Title(s.into()));
        }
        // percent_decode must not slice mid-character after '%'.
        assert_eq!(
            classify("https://doi.org/10.1234/%€x"),
            RefKind::Doi("10.1234/%€x".into())
        );
        assert_eq!(
            classify("https://doi.org/%€"),
            RefKind::Url("https://doi.org/%€".into())
        );
    }

    #[test]
    fn arxiv_id_normalization() {
        assert_eq!(normalize_arxiv_id("1706.03762v5"), "1706.03762");
        assert_eq!(normalize_arxiv_id(" 2406.01234 "), "2406.01234");
        assert_eq!(normalize_arxiv_id("cs.AI/0301001v2"), "cs.AI/0301001");
    }

    #[test]
    fn jats_stripping() {
        assert_eq!(
            strip_jats("<jats:p>Deep learning is &amp; stays <jats:i>great</jats:i>.</jats:p>"),
            "Deep learning is & stays great."
        );
    }
}
