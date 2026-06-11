mod bibtex;
mod db;
mod fetch;
mod model;
mod output;

use anyhow::{Context, Result, bail};
use clap::{CommandFactory, Parser, Subcommand};
use model::{Paper, Priority, Status, normalize_tags};
use output::{BOLD, CYAN, DIM, GREEN, RED, YELLOW, paint};
use rusqlite::Connection;
use std::cmp::Reverse;
use std::io::Write;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "rlist",
    version,
    about = "A fast reading list for academic papers",
    long_about = "A fast reading list for academic papers.\n\n\
        Add papers by arXiv id, DOI, URL, or plain title, and metadata (title, authors,\n\
        year, venue, abstract, PDF link) is fetched automatically from arXiv/Crossref.\n\
        Track what you're reading, tag and prioritize, take notes, search everything,\n\
        and export to BibTeX.",
    after_help = "Examples:\n  \
        rlist add 1706.03762 -t transformers -p high\n  \
        rlist add 10.1038/nature14539\n  \
        rlist add \"https://arxiv.org/abs/2005.14165\" --note \"recommended by Dana\"\n  \
        rlist                       # show your reading queue\n  \
        rlist next                  # what should I read?\n  \
        rlist start 3               # mark as reading\n  \
        rlist done 3 -r 5           # finished, rated 5/5\n  \
        rlist note 3 \"key idea: scaled dot-product attention\"\n  \
        rlist search attention      # full-text search (incl. abstracts & notes)\n  \
        rlist export -f bibtex -o refs.bib"
)]
struct Cli {
    /// Path to the database (default: ~/.local/share/rlist/rlist.db, or $RLIST_DB)
    #[arg(long, global = true, value_name = "FILE")]
    db: Option<PathBuf>,

    /// Disable colored output (also respects NO_COLOR)
    #[arg(long, global = true)]
    no_color: bool,

    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Add a paper by arXiv id, DOI, URL, or title
    #[command(after_help = "The reference can be:\n  \
        an arXiv id or URL    1706.03762, arXiv:2406.01234v2, https://arxiv.org/abs/...\n  \
        a DOI or doi.org URL  10.1038/nature14539, doi:..., https://doi.org/...\n  \
        any other URL         requires --title\n  \
        a plain title         creates a manual entry (use --authors, --year, ...)")]
    Add {
        /// arXiv id, DOI, URL, or paper title
        #[arg(value_name = "REF")]
        reference: String,
        /// Override / supply the title
        #[arg(long)]
        title: Option<String>,
        /// Authors, separated by ';' (e.g. "Ada Lovelace; Alan Turing")
        #[arg(long)]
        authors: Option<String>,
        /// Publication year
        #[arg(long)]
        year: Option<i32>,
        /// Venue (journal / conference)
        #[arg(long)]
        venue: Option<String>,
        /// Override / supply the paper URL
        #[arg(long)]
        url: Option<String>,
        /// Tag(s), repeatable (comma-separated also works)
        #[arg(short, long = "tag", value_name = "TAG")]
        tags: Vec<String>,
        /// Priority: high, normal, low
        #[arg(short, long, value_name = "PRIO")]
        priority: Option<String>,
        /// Initial status (default: to-read), useful for backfilling read papers
        #[arg(long, value_name = "STATUS")]
        status: Option<String>,
        /// Rating 1-5 (implies --status read)
        #[arg(short, long)]
        rating: Option<i64>,
        /// Attach an initial note
        #[arg(long, value_name = "TEXT")]
        note: Option<String>,
        /// Skip metadata fetching even for arXiv ids / DOIs
        #[arg(long)]
        no_fetch: bool,
    },

    /// List papers (default: your queue of to-read and reading)
    #[command(alias = "ls")]
    List {
        /// Filter by status (to-read, reading, read, dropped), repeatable
        #[arg(short, long = "status", value_name = "STATUS")]
        statuses: Vec<String>,
        /// Filter by tag (all must match), repeatable
        #[arg(short, long = "tag", value_name = "TAG")]
        tags: Vec<String>,
        /// Filter by author substring
        #[arg(short, long)]
        author: Option<String>,
        /// Filter by publication year
        #[arg(short, long)]
        year: Option<i32>,
        /// Sort by: priority, added, year, title, rating
        #[arg(long, default_value = "priority", value_name = "KEY")]
        sort: String,
        /// Reverse the sort order
        #[arg(short = 'R', long)]
        reverse: bool,
        /// Show at most N papers
        #[arg(short = 'n', long, value_name = "N")]
        limit: Option<usize>,
        /// Include every status
        #[arg(short = 'A', long)]
        all: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show full details of a paper (abstract, links, notes)
    Show {
        /// Paper id (as shown in `rlist list`)
        id: i64,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Full-text search across titles, authors, abstracts, tags, and notes
    #[command(alias = "find")]
    Search {
        /// Search terms (the last one matches as a prefix)
        #[arg(required = true)]
        query: Vec<String>,
        /// Show at most N results
        #[arg(short = 'n', long, value_name = "N")]
        limit: Option<usize>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Mark paper(s) as currently reading
    Start {
        /// Paper id(s)
        #[arg(required = true)]
        ids: Vec<i64>,
    },

    /// Mark paper(s) as read (optionally with a rating)
    #[command(alias = "finish", alias = "read")]
    Done {
        /// Paper id(s)
        #[arg(required = true)]
        ids: Vec<i64>,
        /// Rating 1-5
        #[arg(short, long)]
        rating: Option<i64>,
    },

    /// Drop paper(s) you've decided not to read
    Drop {
        /// Paper id(s)
        #[arg(required = true)]
        ids: Vec<i64>,
    },

    /// Edit a paper's metadata, tags, priority, or status
    #[command(
        after_help = "Pass an empty string (or 0 for --year / -r) to clear a field:\n  \
        rlist edit 3 --venue \"\"     # remove the venue\n  \
        rlist edit 3 -r 0           # remove the rating"
    )]
    Edit {
        /// Paper id (as shown in `rlist list`)
        id: i64,
        /// New title
        #[arg(long)]
        title: Option<String>,
        /// Authors, separated by ';'
        #[arg(long)]
        authors: Option<String>,
        /// Publication year (0 clears it)
        #[arg(long)]
        year: Option<i32>,
        /// Venue (empty string clears it)
        #[arg(long)]
        venue: Option<String>,
        /// Paper URL (empty string clears it)
        #[arg(long)]
        url: Option<String>,
        /// PDF URL (empty string clears it)
        #[arg(long)]
        pdf_url: Option<String>,
        /// DOI (empty string clears it)
        #[arg(long)]
        doi: Option<String>,
        /// arXiv id (empty string clears it)
        #[arg(long, value_name = "ID")]
        arxiv: Option<String>,
        /// Add tag(s), repeatable
        #[arg(short = 't', long = "add-tag", value_name = "TAG")]
        add_tags: Vec<String>,
        /// Remove tag(s), repeatable
        #[arg(long = "rm-tag", value_name = "TAG")]
        rm_tags: Vec<String>,
        /// Priority: high, normal, low
        #[arg(short, long, value_name = "PRIO")]
        priority: Option<String>,
        /// Status: to-read, reading, read, dropped
        #[arg(long, value_name = "STATUS")]
        status: Option<String>,
        /// Rating 1-5 (0 clears it)
        #[arg(short, long)]
        rating: Option<i64>,
    },

    /// Add a note to a paper (no text opens $EDITOR)
    Note {
        /// Paper id (as shown in `rlist list`)
        id: i64,
        /// Note text, omit to open your editor
        #[arg(allow_hyphen_values = true, trailing_var_arg = true)]
        text: Vec<String>,
    },

    /// Open a paper's page in the browser, or its PDF in your PDF viewer
    Open {
        /// Paper id (as shown in `rlist list`)
        id: i64,
        /// Download the PDF (cached locally) and open it in your PDF viewer
        #[arg(short, long)]
        pdf: bool,
    },

    /// Remove paper(s) from the list
    #[command(alias = "delete")]
    Rm {
        /// Paper id(s)
        #[arg(required = true)]
        ids: Vec<i64>,
        /// Don't ask for confirmation
        #[arg(short, long)]
        force: bool,
    },

    /// Suggest what to read next
    Next {
        /// Pick at random from the queue instead of by priority
        #[arg(long)]
        random: bool,
        /// Restrict to a tag, repeatable
        #[arg(short, long = "tag", value_name = "TAG")]
        tags: Vec<String>,
    },

    /// List all tags with paper counts
    Tags,

    /// Reading statistics
    Stats,

    /// Export papers (bibtex, json, csv)
    Export {
        /// Output format: bibtex, json, csv
        #[arg(short, long, default_value = "bibtex", value_name = "FMT")]
        format: String,
        /// Write to a file instead of stdout
        #[arg(short, long, value_name = "FILE")]
        output: Option<PathBuf>,
        /// Only papers with this status, repeatable
        #[arg(short, long = "status", value_name = "STATUS")]
        statuses: Vec<String>,
        /// Only papers with this tag, repeatable
        #[arg(short, long = "tag", value_name = "TAG")]
        tags: Vec<String>,
    },

    /// Import papers from a BibTeX or JSON file
    Import {
        /// File to import (.bib or .json, other extensions are sniffed)
        file: PathBuf,
        /// Force format: bibtex or json (default: by file extension)
        #[arg(short, long, value_name = "FMT")]
        format: Option<String>,
    },

    /// Print the database file path
    Path,

    /// Generate shell completions (fish, bash, zsh, ...)
    Completions {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },

    /// Uninstall rlist (keeps your reading list unless --purge)
    #[command(after_help = "Two modes:\n  \
        rlist uninstall           soft: removes the binary and shell completions,\n                            \
        but keeps your reading list and notes\n  \
        rlist uninstall --purge   hard: removes EVERYTHING, including the database")]
    Uninstall {
        /// Also delete the database (reading list, notes, everything)
        #[arg(long)]
        purge: bool,
        /// Don't ask for confirmation
        #[arg(short, long)]
        force: bool,
    },
}

fn main() {
    let cli = Cli::parse();
    output::init_color(cli.no_color);
    if let Err(e) = run(cli) {
        eprintln!("{} {e:#}", paint(RED, "error:"));
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    let db_path = cli.db.clone().unwrap_or_else(db::default_db_path);

    // These don't need a database.
    match &cli.cmd {
        Some(Cmd::Completions { shell }) => {
            let mut cmd = Cli::command();
            clap_complete::generate(*shell, &mut cmd, "rlist", &mut std::io::stdout());
            return Ok(());
        }
        Some(Cmd::Path) => {
            println!("{}", db_path.display());
            return Ok(());
        }
        // Uninstall must not open (and thereby create) the database.
        Some(Cmd::Uninstall { purge, force }) => return cmd_uninstall(&db_path, *purge, *force),
        _ => {}
    }

    let conn = db::open(&db_path)?;

    match cli.cmd.unwrap_or(Cmd::List {
        statuses: vec![],
        tags: vec![],
        author: None,
        year: None,
        sort: "priority".into(),
        reverse: false,
        limit: None,
        all: false,
        json: false,
    }) {
        Cmd::Add {
            reference,
            title,
            authors,
            year,
            venue,
            url,
            tags,
            priority,
            status,
            rating,
            note,
            no_fetch,
        } => cmd_add(
            &conn, &reference, title, authors, year, venue, url, tags, priority, status, rating,
            note, no_fetch,
        ),
        Cmd::List {
            statuses,
            tags,
            author,
            year,
            sort,
            reverse,
            limit,
            all,
            json,
        } => cmd_list(
            &conn, statuses, tags, author, year, &sort, reverse, limit, all, json,
        ),
        Cmd::Show { id, json } => cmd_show(&conn, id, json),
        Cmd::Search { query, limit, json } => cmd_search(&conn, &query.join(" "), limit, json),
        Cmd::Start { ids } => cmd_status_change(&conn, &ids, Status::Reading, None),
        Cmd::Done { ids, rating } => cmd_status_change(&conn, &ids, Status::Read, rating),
        Cmd::Drop { ids } => cmd_status_change(&conn, &ids, Status::Dropped, None),
        Cmd::Edit {
            id,
            title,
            authors,
            year,
            venue,
            url,
            pdf_url,
            doi,
            arxiv,
            add_tags,
            rm_tags,
            priority,
            status,
            rating,
        } => cmd_edit(
            &conn, id, title, authors, year, venue, url, pdf_url, doi, arxiv, add_tags, rm_tags,
            priority, status, rating,
        ),
        Cmd::Note { id, text } => cmd_note(&conn, id, &text.join(" ")),
        Cmd::Open { id, pdf } => cmd_open(&conn, id, pdf),
        Cmd::Rm { ids, force } => cmd_rm(&conn, &ids, force),
        Cmd::Next { random, tags } => cmd_next(&conn, random, tags),
        Cmd::Tags => cmd_tags(&conn),
        Cmd::Stats => cmd_stats(&conn),
        Cmd::Export {
            format,
            output,
            statuses,
            tags,
        } => cmd_export(&conn, &format, output, statuses, tags),
        Cmd::Import { file, format } => cmd_import(&conn, &file, format),
        Cmd::Path | Cmd::Completions { .. } | Cmd::Uninstall { .. } => unreachable!(),
    }
}

fn parse_status(s: &str) -> Result<Status> {
    Status::parse(s)
        .ok_or_else(|| anyhow::anyhow!("unknown status '{s}' (to-read, reading, read, dropped)"))
}

fn parse_priority(s: &str) -> Result<Priority> {
    Priority::parse(s).ok_or_else(|| anyhow::anyhow!("unknown priority '{s}' (high, normal, low)"))
}

fn validate_rating(r: i64) -> Result<i64> {
    anyhow::ensure!(
        (1..=5).contains(&r),
        "rating must be 1-5 (0 clears it in edit)"
    );
    Ok(r)
}

fn validate_year(y: i32) -> Result<i32> {
    anyhow::ensure!(
        (1000..=2100).contains(&y),
        "year {y} looks wrong (expected 1000-2100)"
    );
    Ok(y)
}

// ---------------------------------------------------------------------------
// add
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn cmd_add(
    conn: &Connection,
    reference: &str,
    title: Option<String>,
    authors: Option<String>,
    year: Option<i32>,
    venue: Option<String>,
    url: Option<String>,
    tags: Vec<String>,
    priority: Option<String>,
    status: Option<String>,
    rating: Option<i64>,
    note: Option<String>,
    no_fetch: bool,
) -> Result<()> {
    let kind = fetch::classify(reference);

    // Duplicate checks before any network round-trip.
    match &kind {
        fetch::RefKind::Arxiv { id, .. } => {
            if let Some(existing) = db::find_by_arxiv(conn, id)? {
                bail!("already in your list as #{existing} (arXiv {id})");
            }
        }
        fetch::RefKind::Doi(doi) => {
            if let Some(existing) = db::find_by_doi(conn, doi)? {
                bail!("already in your list as #{existing} (doi {doi})");
            }
        }
        _ => {}
    }

    let fetched = match &kind {
        fetch::RefKind::Arxiv { id, fetch_id } if !no_fetch => {
            eprintln!("{}", paint(DIM, &format!("fetching arXiv:{fetch_id} …")));
            Some(fetch::fetch_arxiv(fetch_id, id).map_err(|e| {
                anyhow::anyhow!(
                    "{e:#}\n  you can still add it manually:\n  \
                     rlist add \"<title>\" --url https://arxiv.org/abs/{id}"
                )
            })?)
        }
        fetch::RefKind::Doi(doi) if !no_fetch => {
            eprintln!("{}", paint(DIM, &format!("resolving doi:{doi} …")));
            Some(fetch::fetch_doi(doi).map_err(|e| {
                anyhow::anyhow!(
                    "{e:#}\n  you can still add it manually:\n  \
                     rlist add \"<title>\" --url https://doi.org/{doi}"
                )
            })?)
        }
        _ => None,
    };

    let mut paper = Paper {
        id: 0,
        title: String::new(),
        authors: String::new(),
        year: None,
        venue: None,
        arxiv_id: None,
        doi: None,
        url: None,
        pdf_url: None,
        abstract_: None,
        status: Status::ToRead,
        priority: Priority::Normal,
        rating: None,
        tags: normalize_tags(&tags),
        added_at: db::now(),
        started_at: None,
        finished_at: None,
        notes: vec![],
    };

    if let Some(f) = fetched {
        paper.venue = f
            .venue
            .clone()
            .or_else(|| f.arxiv_id.as_ref().map(|_| "arXiv".to_string()));
        paper.title = f.title;
        paper.authors = f.authors.join("; ");
        paper.year = f.year;
        paper.abstract_ = f.abstract_;
        paper.url = f.url;
        paper.pdf_url = f.pdf_url;
        paper.doi = f.doi;
        paper.arxiv_id = f.arxiv_id;
    } else {
        match &kind {
            fetch::RefKind::Title(t) => paper.title = t.clone(),
            fetch::RefKind::Url(u) => {
                paper.url = Some(u.clone());
                if title.is_none() {
                    bail!(
                        "a plain URL needs --title \"...\" (metadata can only be fetched for arXiv ids and DOIs)"
                    );
                }
            }
            fetch::RefKind::Arxiv { id, .. } => {
                // --no-fetch on an arXiv id: minimal entry.
                paper.arxiv_id = Some(id.clone());
                paper.url = Some(format!("https://arxiv.org/abs/{id}"));
                paper.pdf_url = Some(format!("https://arxiv.org/pdf/{id}"));
                paper.venue = Some("arXiv".into());
                paper.title = format!("arXiv:{id}");
            }
            fetch::RefKind::Doi(d) => {
                paper.doi = Some(d.clone());
                paper.url = Some(format!("https://doi.org/{d}"));
                paper.title = format!("doi:{d}");
            }
        }
    }

    // Manual flags override fetched metadata.
    if let Some(t) = title {
        paper.title = t;
    }
    if let Some(a) = authors {
        paper.authors = a
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("; ");
    }
    if let Some(y) = year {
        paper.year = Some(validate_year(y)?);
    }
    if let Some(v) = venue {
        paper.venue = Some(v);
    }
    if let Some(u) = url {
        paper.url = Some(u);
    }
    if let Some(p) = priority {
        paper.priority = parse_priority(&p)?;
    }
    if let Some(s) = status {
        paper.status = parse_status(&s)?;
    }
    if let Some(r) = rating {
        paper.rating = Some(validate_rating(r)?);
        paper.status = Status::Read;
    }
    match paper.status {
        Status::Reading => paper.started_at = Some(db::now()),
        Status::Read => paper.finished_at = Some(db::now()),
        _ => {}
    }

    paper.title = paper.title.trim().to_string();
    anyhow::ensure!(!paper.title.is_empty(), "paper needs a title");

    // Dedup on identifiers discovered during the fetch (e.g. a DOI fetch that
    // resolves to an arXiv id already in the list), and warn on duplicate titles.
    if let Some(a) = &paper.arxiv_id
        && let Some(existing) = db::find_by_arxiv(conn, a)?
    {
        bail!("already in your list as #{existing} (arXiv {a})");
    }
    if let Some(d) = &paper.doi
        && let Some(existing) = db::find_by_doi(conn, d)?
    {
        bail!("already in your list as #{existing} (doi {d})");
    }
    if let Some(existing) = db::find_by_title(conn, &paper.title)? {
        eprintln!(
            "{} #{existing} has the same title, keeping both",
            paint(YELLOW, "warning:")
        );
    }

    let tx = conn.unchecked_transaction()?;
    let id = db::insert_paper(&tx, &paper)?;
    paper.id = id;
    if let Some(n) = note
        && !n.trim().is_empty()
    {
        db::add_note(&tx, id, n.trim())?;
    }
    tx.commit()?;

    output::confirm_line("added", &paper);
    let mut detail_bits: Vec<String> = Vec::new();
    if !paper.authors.is_empty() {
        detail_bits.push(paper.short_authors());
    }
    if let Some(y) = paper.year {
        detail_bits.push(y.to_string());
    }
    if let Some(v) = &paper.venue {
        detail_bits.push(v.clone());
    }
    if !paper.tags.is_empty() {
        detail_bits.push(format!("[{}]", paper.tags.join(", ")));
    }
    if !detail_bits.is_empty() {
        println!("      {}", paint(DIM, &detail_bits.join(" · ")));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// list / show / search
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn cmd_list(
    conn: &Connection,
    statuses: Vec<String>,
    tags: Vec<String>,
    author: Option<String>,
    year: Option<i32>,
    sort: &str,
    reverse: bool,
    limit: Option<usize>,
    all: bool,
    json: bool,
) -> Result<()> {
    let status_filter: Vec<Status> = if all {
        vec![]
    } else if statuses.is_empty() {
        vec![Status::ToRead, Status::Reading]
    } else {
        statuses
            .iter()
            .map(|s| parse_status(s))
            .collect::<Result<_>>()?
    };
    let tag_filter = normalize_tags(&tags);

    let mut papers: Vec<Paper> = db::all_papers(conn)?
        .into_iter()
        .filter(|p| status_filter.is_empty() || status_filter.contains(&p.status))
        .filter(|p| tag_filter.iter().all(|t| p.tags.contains(t)))
        .filter(|p| {
            author
                .as_ref()
                .is_none_or(|a| p.authors.to_lowercase().contains(&a.to_lowercase()))
        })
        .filter(|p| year.is_none_or(|y| p.year == Some(y)))
        .collect();

    sort_papers(&mut papers, sort)?;
    if reverse {
        papers.reverse();
    }
    let total = papers.len();
    if let Some(n) = limit {
        papers.truncate(n);
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&papers)?);
        return Ok(());
    }
    if papers.is_empty() && total > 0 {
        // Truncated to nothing by -n, where the empty-db onboarding hint would lie.
        println!(
            "{}",
            paint(
                DIM,
                &format!("{total} papers hidden by -n; raise the limit")
            )
        );
        return Ok(());
    }
    output::table(&papers);
    if papers.len() < total {
        println!(
            "{}",
            paint(
                DIM,
                &format!("… and {} more (use -n to adjust)", total - papers.len())
            )
        );
    }
    Ok(())
}

fn sort_papers(papers: &mut [Paper], sort: &str) -> Result<()> {
    match sort {
        "priority" | "prio" | "p" => papers.sort_by(|a, b| {
            (Reverse(a.priority), &a.added_at, a.id).cmp(&(Reverse(b.priority), &b.added_at, b.id))
        }),
        "added" | "a" => papers.sort_by(|a, b| (&b.added_at, b.id).cmp(&(&a.added_at, a.id))),
        "year" | "y" => papers.sort_by(|a, b| {
            b.year
                .unwrap_or(i32::MIN)
                .cmp(&a.year.unwrap_or(i32::MIN))
                .then(a.id.cmp(&b.id))
        }),
        "title" | "t" => papers.sort_by_key(|p| p.title.to_lowercase()),
        "rating" | "r" => papers.sort_by(|a, b| {
            b.rating
                .unwrap_or(0)
                .cmp(&a.rating.unwrap_or(0))
                .then(a.id.cmp(&b.id))
        }),
        other => bail!("unknown sort key '{other}' (priority, added, year, title, rating)"),
    }
    Ok(())
}

fn cmd_show(conn: &Connection, id: i64, json: bool) -> Result<()> {
    let paper = db::get_paper(conn, id)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&paper)?);
    } else {
        output::detail(&paper);
    }
    Ok(())
}

fn cmd_search(conn: &Connection, query: &str, limit: Option<usize>, json: bool) -> Result<()> {
    let mut papers = db::search(conn, query)?;
    if let Some(n) = limit {
        papers.truncate(n);
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&papers)?);
        return Ok(());
    }
    if papers.is_empty() {
        println!("{}", paint(DIM, "no matches"));
        return Ok(());
    }
    output::table(&papers);
    Ok(())
}

// ---------------------------------------------------------------------------
// status transitions
// ---------------------------------------------------------------------------

fn cmd_status_change(
    conn: &Connection,
    ids: &[i64],
    status: Status,
    rating: Option<i64>,
) -> Result<()> {
    let rating = rating.map(validate_rating).transpose()?;
    // Validate every id before touching anything, then commit the whole
    // batch atomically, so `rlist done 3 999 5` cannot half-apply.
    let papers: Vec<_> = ids
        .iter()
        .map(|&id| db::get_paper(conn, id))
        .collect::<Result<_>>()?;
    let tx = conn.unchecked_transaction()?;
    let mut updated = Vec::new();
    for mut p in papers {
        p.status = status;
        match status {
            Status::Reading => {
                p.started_at = Some(db::now());
                p.finished_at = None;
            }
            Status::Read => p.finished_at = Some(db::now()),
            _ => {}
        }
        if let Some(r) = rating {
            p.rating = Some(r);
        }
        db::update_paper(&tx, &p)?;
        updated.push(p);
    }
    tx.commit()?;

    for p in updated {
        let verb = match status {
            Status::Reading => "reading",
            Status::Read => "read ✓",
            Status::Dropped => "dropped",
            Status::ToRead => "queued",
        };
        output::confirm_line(verb, &p);
        if status == Status::Read && p.rating.is_none() {
            println!(
                "      {}",
                paint(DIM, &format!("rate it: rlist edit {} -r 1..5", p.id))
            );
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// edit / note / open / rm
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn cmd_edit(
    conn: &Connection,
    id: i64,
    title: Option<String>,
    authors: Option<String>,
    year: Option<i32>,
    venue: Option<String>,
    url: Option<String>,
    pdf_url: Option<String>,
    doi: Option<String>,
    arxiv: Option<String>,
    add_tags: Vec<String>,
    rm_tags: Vec<String>,
    priority: Option<String>,
    status: Option<String>,
    rating: Option<i64>,
) -> Result<()> {
    let mut p = db::get_paper(conn, id)?;
    let flags_given = title.is_some()
        || authors.is_some()
        || year.is_some()
        || venue.is_some()
        || url.is_some()
        || pdf_url.is_some()
        || doi.is_some()
        || arxiv.is_some()
        || !add_tags.is_empty()
        || !rm_tags.is_empty()
        || priority.is_some()
        || status.is_some()
        || rating.is_some();
    if !flags_given {
        bail!("nothing to change, see `rlist edit --help` for available fields");
    }

    if let Some(t) = title {
        let t = t.trim().to_string();
        anyhow::ensure!(!t.is_empty(), "paper needs a title");
        p.title = t;
    }
    if let Some(a) = authors {
        p.authors = a
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("; ");
    }
    // An empty string (or 0 for year/rating) clears an optional field.
    let opt = |v: String| {
        let v = v.trim().to_string();
        if v.is_empty() { None } else { Some(v) }
    };
    if let Some(y) = year {
        p.year = match y {
            0 => None,
            y => Some(validate_year(y)?),
        };
    }
    if let Some(v) = venue {
        p.venue = opt(v);
    }
    if let Some(u) = url {
        p.url = opt(u);
    }
    if let Some(u) = pdf_url {
        p.pdf_url = opt(u);
    }
    if let Some(d) = doi {
        let d = opt(d);
        if let Some(d) = &d
            && let Some(existing) = db::find_by_doi(conn, d)?
            && existing != id
        {
            bail!("doi {d} is already on #{existing}");
        }
        p.doi = d;
    }
    if let Some(a) = arxiv {
        let a = opt(a).map(|a| fetch::normalize_arxiv_id(&a));
        if let Some(a) = &a
            && let Some(existing) = db::find_by_arxiv(conn, a)?
            && existing != id
        {
            bail!("arXiv {a} is already on #{existing}");
        }
        p.arxiv_id = a;
    }

    for t in normalize_tags(&add_tags) {
        if !p.tags.contains(&t) {
            p.tags.push(t);
        }
    }
    for t in normalize_tags(&rm_tags) {
        if let Some(pos) = p.tags.iter().position(|x| *x == t) {
            p.tags.remove(pos);
        }
    }
    if let Some(pr) = priority {
        p.priority = parse_priority(&pr)?;
    }
    if let Some(s) = status {
        let s = parse_status(&s)?;
        if s != p.status {
            p.status = s;
            // Mirror the start/done transition rules so timestamps can't go
            // stale (e.g. status "reading" with a finished_at left behind).
            match s {
                Status::Reading => {
                    p.started_at = Some(db::now());
                    p.finished_at = None;
                }
                Status::Read => p.finished_at = Some(db::now()),
                Status::ToRead => {
                    p.started_at = None;
                    p.finished_at = None;
                }
                Status::Dropped => {}
            }
        }
    }
    if let Some(r) = rating {
        p.rating = match r {
            0 => None,
            r => Some(validate_rating(r)?),
        };
    }

    db::update_paper(conn, &p)?;
    output::confirm_line("updated", &p);
    Ok(())
}

fn cmd_note(conn: &Connection, id: i64, text: &str) -> Result<()> {
    let p = db::get_paper(conn, id)?;
    let body = if text.trim().is_empty() {
        edit_in_editor(&format!(
            "\n# Note on #{} {}\n# Lines starting with '#' are ignored.\n",
            p.id, p.title
        ))?
    } else {
        text.trim().to_string()
    };
    if body.is_empty() {
        bail!("empty note, nothing saved");
    }
    db::add_note(conn, id, &body)?;
    output::confirm_line("noted", &p);
    Ok(())
}

fn edit_in_editor(template: &str) -> Result<String> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".into());
    let mut file = tempfile::Builder::new()
        .prefix("rlist-note-")
        .suffix(".md")
        .tempfile()
        .context("creating temp file")?;
    file.write_all(template.as_bytes())?;
    file.flush()?;

    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("{editor} \"$1\""))
        .arg("rlist")
        .arg(file.path())
        .status()
        .with_context(|| format!("launching editor '{editor}'"))?;
    anyhow::ensure!(
        status.success(),
        "editor exited with an error; note not saved"
    );

    let content = std::fs::read_to_string(file.path())?;
    Ok(content
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string())
}

fn cmd_open(conn: &Connection, id: i64, pdf: bool) -> Result<()> {
    let p = db::get_paper(conn, id)?;
    let url = p.best_url(pdf).ok_or_else(|| {
        anyhow::anyhow!("#{id} has no URL, set one with `rlist edit {id} --url ...`")
    })?;

    // --pdf downloads the file (once) and opens it locally, so it lands in
    // your PDF viewer instead of a browser tab. Falls back to the browser
    // when no PDF viewer is configured, the download fails, or the link
    // doesn't return a PDF.
    if pdf {
        let open_in_browser = |reason: &str| -> Result<()> {
            eprintln!(
                "{}",
                paint(DIM, &format!("{reason}, opening in the browser instead"))
            );
            println!("opening {}", paint(BOLD, url));
            open::that_detached(url).with_context(|| format!("opening {url}"))
        };

        if !has_pdf_handler() {
            return open_in_browser("no PDF viewer is set");
        }

        let path = cached_pdf_path(id, url);
        if !path.is_file() {
            eprintln!("{}", paint(DIM, &format!("downloading {url} …")));
            match fetch::download_pdf(url) {
                Ok(bytes) => {
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent)
                            .with_context(|| format!("creating {}", parent.display()))?;
                    }
                    let tmp = path.with_extension("part");
                    std::fs::write(&tmp, &bytes)
                        .with_context(|| format!("writing {}", tmp.display()))?;
                    std::fs::rename(&tmp, &path)?;
                }
                Err(e) => {
                    eprintln!("{} {e:#}", paint(YELLOW, "warning:"));
                    return open_in_browser("download failed");
                }
            }
        }
        println!(
            "opening {} in your PDF viewer",
            paint(BOLD, &path.display().to_string())
        );
        if let Err(e) = open::that_detached(&path) {
            eprintln!(
                "{} could not launch a viewer for {}: {e}",
                paint(YELLOW, "warning:"),
                path.display()
            );
            return open_in_browser("viewer failed");
        }
        return Ok(());
    }

    println!("opening {}", paint(BOLD, url));
    open::that_detached(url).with_context(|| format!("opening {url}"))?;
    Ok(())
}

/// Is any application registered to handle PDFs? On Linux we ask xdg-mime,
/// because launching xdg-open with no association either fails or dumps the
/// file on whatever generic fallback exists. macOS and Windows always have
/// a built-in PDF handler.
fn has_pdf_handler() -> bool {
    if !cfg!(target_os = "linux") {
        return true;
    }
    match std::process::Command::new("xdg-mime")
        .args(["query", "default", "application/pdf"])
        .output()
    {
        Ok(out) => out.status.success() && !String::from_utf8_lossy(&out.stdout).trim().is_empty(),
        // xdg-mime itself is missing, let xdg-open try its own fallbacks.
        Err(_) => true,
    }
}

pub fn cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("rlist")
}

/// Cache file for a paper's PDF. The URL hash keeps a stale copy from being
/// reused after the paper's pdf_url changes.
fn cached_pdf_path(id: i64, url: &str) -> PathBuf {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in url.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100000001b3);
    }
    cache_dir().join(format!("{id}-{:08x}.pdf", (h >> 32) as u32 ^ h as u32))
}

fn cmd_rm(conn: &Connection, ids: &[i64], force: bool) -> Result<()> {
    use std::io::IsTerminal;
    // Validate every id first so a typo can't delete half the batch.
    let papers: Vec<_> = ids
        .iter()
        .map(|&id| db::get_paper(conn, id))
        .collect::<Result<_>>()?;
    for p in papers {
        let id = p.id;
        if !force {
            if !std::io::stdin().is_terminal() {
                bail!("refusing to delete without confirmation, use --force");
            }
            eprint!(
                "delete {} {}? [y/N] ",
                paint(BOLD, &format!("#{id}")),
                output::truncate(&p.title, 60)
            );
            std::io::stderr().flush()?;
            let mut answer = String::new();
            std::io::stdin().read_line(&mut answer)?;
            if !matches!(answer.trim(), "y" | "Y" | "yes") {
                println!("kept #{id}");
                continue;
            }
        }
        db::delete_paper(conn, id)?;
        output::confirm_line("removed", &p);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// uninstall
// ---------------------------------------------------------------------------

/// User-level completion files `rlist completions` is documented to create.
fn completion_paths() -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    if let Some(config) = dirs::config_dir() {
        paths.push(config.join("fish/completions/rlist.fish"));
    }
    if let Some(data) = dirs::data_dir() {
        paths.push(data.join("bash-completion/completions/rlist"));
    }
    if let Some(home) = dirs::home_dir() {
        paths.push(home.join(".zfunc/_rlist"));
    }
    paths
}

fn cmd_uninstall(db_path: &std::path::Path, purge: bool, force: bool) -> Result<()> {
    use std::io::IsTerminal;
    let canon = |p: &std::path::Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());

    // Binaries: the one currently running, plus the standard install location
    // (so the dev copy in target/ can also uninstall the installed one).
    let mut binaries: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        binaries.push(exe);
    }
    if let Some(home) = dirs::home_dir() {
        let installed = home.join(".local/bin/rlist");
        if installed.is_file() && !binaries.iter().any(|b| canon(b) == canon(&installed)) {
            binaries.push(installed);
        }
    }

    let completions: Vec<PathBuf> = completion_paths()
        .into_iter()
        .filter(|p| p.is_file())
        .collect();

    // Data targets (hard uninstall only). When the db lives in the default
    // ~/.local/share/rlist/ directory, remove the whole directory. For a
    // custom --db / $RLIST_DB path, remove just the db and its WAL side files.
    let default_dir = dirs::data_dir().map(|d| d.join("rlist"));
    let owns_dir = purge
        && default_dir
            .as_ref()
            .is_some_and(|d| db_path.parent().is_some_and(|p| canon(p) == canon(d)));
    let mut data_files: Vec<PathBuf> = Vec::new();
    if purge && !owns_dir {
        for suffix in ["", "-wal", "-shm"] {
            let mut name = db_path.as_os_str().to_owned();
            name.push(suffix);
            let p = PathBuf::from(name);
            if p.is_file() {
                data_files.push(p);
            }
        }
    }
    // Cached PDF downloads are re-downloadable, so purge removes them too.
    let pdf_cache = cache_dir();
    let purge_cache = purge && pdf_cache.is_dir();

    println!(
        "{} uninstall will remove:",
        if purge {
            paint(RED, "hard")
        } else {
            paint(YELLOW, "soft")
        }
    );
    for p in binaries.iter().chain(&completions).chain(&data_files) {
        println!("  {}", p.display());
    }
    if owns_dir && let Some(d) = &default_dir {
        println!(
            "  {}  {}",
            d.display(),
            paint(RED, "(your entire reading list)")
        );
    }
    if purge_cache {
        println!("  {}  {}", pdf_cache.display(), paint(DIM, "(cached PDFs)"));
    }
    if !purge {
        println!(
            "your reading list at {} will be {}",
            db_path.display(),
            paint(GREEN, "kept")
        );
    }

    if !force {
        if !std::io::stdin().is_terminal() {
            bail!("refusing to uninstall without confirmation, use --force");
        }
        eprint!("proceed? [y/N] ");
        std::io::stderr().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim(), "y" | "Y" | "yes") {
            println!("aborted, nothing removed");
            return Ok(());
        }
    }

    let remove = |p: &std::path::Path| -> Result<()> {
        std::fs::remove_file(p).with_context(|| format!("removing {}", p.display()))?;
        println!("removed {}", p.display());
        Ok(())
    };
    for p in &completions {
        remove(p)?;
    }
    for p in &data_files {
        remove(p)?;
    }
    if owns_dir
        && let Some(d) = &default_dir
        && d.is_dir()
    {
        std::fs::remove_dir_all(d).with_context(|| format!("removing {}", d.display()))?;
        println!("removed {}", d.display());
    }
    if purge_cache && pdf_cache.is_dir() {
        std::fs::remove_dir_all(&pdf_cache)
            .with_context(|| format!("removing {}", pdf_cache.display()))?;
        println!("removed {}", pdf_cache.display());
    }
    // The running binary goes last, since unlinking it while running is fine.
    for p in &binaries {
        remove(p)?;
    }

    if purge {
        println!("{} rlist is fully uninstalled", paint(GREEN, "done:"));
    } else {
        println!(
            "{} rlist removed, your data is still at {}\n      \
             (reinstall to pick it back up, or delete it manually)",
            paint(GREEN, "done:"),
            db_path.display()
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// next / tags / stats
// ---------------------------------------------------------------------------

fn cmd_next(conn: &Connection, random: bool, tags: Vec<String>) -> Result<()> {
    let tag_filter = normalize_tags(&tags);
    let all = db::all_papers(conn)?;
    let mut queue: Vec<Paper> = all
        .iter()
        .filter(|p| p.status == Status::ToRead)
        .filter(|p| tag_filter.iter().all(|t| p.tags.contains(t)))
        .cloned()
        .collect();
    if queue.is_empty() {
        if let Some(current) = all.iter().find(|p| p.status == Status::Reading) {
            println!(
                "{} you're already reading {} {}",
                paint(DIM, "nothing in to-read,"),
                paint(BOLD, &format!("#{}", current.id)),
                output::truncate(&current.title, 60),
            );
        } else {
            println!(
                "{}",
                paint(DIM, "queue is empty, add something with `rlist add`")
            );
        }
        return Ok(());
    }

    let pick = if random {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as usize)
            .unwrap_or(0);
        queue.swap_remove(nanos % queue.len())
    } else {
        queue.sort_by(|a, b| {
            (Reverse(a.priority), &a.added_at, a.id).cmp(&(Reverse(b.priority), &b.added_at, b.id))
        });
        queue.remove(0)
    };

    println!(
        "\n  {} {}",
        paint(GREEN, "up next:"),
        paint(BOLD, &pick.title)
    );
    let mut meta: Vec<String> = Vec::new();
    if !pick.authors.is_empty() {
        meta.push(pick.short_authors());
    }
    if let Some(y) = pick.year {
        meta.push(y.to_string());
    }
    if let Some(v) = &pick.venue {
        meta.push(v.clone());
    }
    if !meta.is_empty() {
        println!("  {}", paint(DIM, &meta.join(" · ")));
    }
    if let Some(abs) = &pick.abstract_ {
        let snippet = output::truncate(abs, 280);
        println!(
            "\n{}",
            output::wrap(&snippet, output::term_width().min(100) - 4, "  ")
        );
    }
    println!(
        "\n  {}",
        paint(
            DIM,
            &format!(
                "rlist start {0}   ·   rlist open {0}   ·   rlist show {0}",
                pick.id
            )
        )
    );
    println!();
    Ok(())
}

fn cmd_tags(conn: &Connection) -> Result<()> {
    let papers = db::all_papers(conn)?;
    let mut counts: Vec<(String, usize, usize)> = Vec::new(); // tag, total, read
    for p in &papers {
        for t in &p.tags {
            match counts.iter_mut().find(|(name, _, _)| name == t) {
                Some((_, total, read)) => {
                    *total += 1;
                    if p.status == Status::Read {
                        *read += 1;
                    }
                }
                None => counts.push((t.clone(), 1, usize::from(p.status == Status::Read))),
            }
        }
    }
    if counts.is_empty() {
        println!(
            "{}",
            paint(DIM, "no tags yet, add one with `rlist add ... -t <tag>`")
        );
        return Ok(());
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    use unicode_width::UnicodeWidthStr;
    let name_w = counts.iter().map(|(n, _, _)| n.width()).max().unwrap_or(4);
    let max = counts[0].1;
    for (name, total, read) in &counts {
        let padded = format!("{name}{}", " ".repeat(name_w.saturating_sub(name.width())));
        println!(
            "  {}  {:>3} {}  {}",
            paint(CYAN, &padded),
            total,
            paint(DIM, &format!("({read} read)")),
            paint(DIM, &output::bar(*total, max, 20)),
        );
    }
    Ok(())
}

fn cmd_stats(conn: &Connection) -> Result<()> {
    let papers = db::all_papers(conn)?;
    if papers.is_empty() {
        println!("{}", paint(DIM, "no papers yet, add one with `rlist add`"));
        return Ok(());
    }

    let count = |s: Status| papers.iter().filter(|p| p.status == s).count();
    let (to_read, reading, read, dropped) = (
        count(Status::ToRead),
        count(Status::Reading),
        count(Status::Read),
        count(Status::Dropped),
    );

    println!();
    println!("  {} papers", paint(BOLD, &papers.len().to_string()));
    println!(
        "  {} to-read   {} reading   {} read   {} dropped",
        paint(YELLOW, &to_read.to_string()),
        paint(output::BLUE, &reading.to_string()),
        paint(GREEN, &read.to_string()),
        paint(DIM, &dropped.to_string()),
    );

    let now = chrono::Local::now();
    let this_month = now.format("%Y-%m").to_string();
    let this_year = now.format("%Y").to_string();
    let finished_starting = |prefix: &str| {
        papers
            .iter()
            .filter(|p| {
                p.status == Status::Read
                    && p.finished_at
                        .as_deref()
                        .is_some_and(|t| t.starts_with(prefix))
            })
            .count()
    };
    println!(
        "  read {} this month, {} this year",
        finished_starting(&this_month),
        finished_starting(&this_year)
    );

    let rated: Vec<i64> = papers.iter().filter_map(|p| p.rating).collect();
    if !rated.is_empty() {
        let avg = rated.iter().sum::<i64>() as f64 / rated.len() as f64;
        println!(
            "  average rating {} ({} rated)",
            paint(YELLOW, &format!("{avg:.1}")),
            rated.len()
        );
    }

    if let Some(oldest) = papers
        .iter()
        .filter(|p| p.status == Status::ToRead)
        .min_by_key(|p| &p.added_at)
        && let Ok(added) =
            chrono::NaiveDate::parse_from_str(output::date_of(&oldest.added_at), "%Y-%m-%d")
    {
        let days = (now.date_naive() - added).num_days();
        if days > 0 {
            println!(
                "  oldest in queue: #{} ({} days) {}",
                oldest.id,
                days,
                paint(DIM, &output::truncate(&oldest.title, 40)),
            );
        }
    }

    // Monthly read histogram, last 12 months.
    use chrono::Datelike;
    let mut months: Vec<(String, usize)> = Vec::new();
    let (mut y, mut m) = (now.year(), now.month());
    for _ in 0..12 {
        let label = format!("{y:04}-{m:02}");
        months.push((label.clone(), finished_starting(&label)));
        if m == 1 {
            m = 12;
            y -= 1;
        } else {
            m -= 1;
        }
    }
    months.reverse();
    let max = months.iter().map(|(_, n)| *n).max().unwrap_or(0);
    if max > 0 {
        println!("\n  {}", paint(DIM, "papers read per month"));
        for (label, n) in &months {
            println!(
                "  {}  {:>2}  {}",
                paint(DIM, label),
                n,
                paint(GREEN, &output::bar(*n, max, 30))
            );
        }
    }
    println!();
    Ok(())
}

// ---------------------------------------------------------------------------
// export / import
// ---------------------------------------------------------------------------

fn cmd_export(
    conn: &Connection,
    format: &str,
    output_file: Option<PathBuf>,
    statuses: Vec<String>,
    tags: Vec<String>,
) -> Result<()> {
    let status_filter: Vec<Status> = statuses
        .iter()
        .map(|s| parse_status(s))
        .collect::<Result<_>>()?;
    let tag_filter = normalize_tags(&tags);
    let mut papers: Vec<Paper> = db::all_papers(conn)?
        .into_iter()
        .filter(|p| status_filter.is_empty() || status_filter.contains(&p.status))
        .filter(|p| tag_filter.iter().all(|t| p.tags.contains(t)))
        .collect();

    let content = match format {
        "bibtex" | "bib" => bibtex::export(&papers),
        "json" => {
            for p in &mut papers {
                p.notes = db::get_notes(conn, p.id)?;
            }
            serde_json::to_string_pretty(&papers)? + "\n"
        }
        "csv" => export_csv(&papers),
        other => bail!("unknown format '{other}' (bibtex, json, csv)"),
    };

    match output_file {
        Some(path) => {
            std::fs::write(&path, &content)
                .with_context(|| format!("writing {}", path.display()))?;
            eprintln!("exported {} papers to {}", papers.len(), path.display());
        }
        None => print!("{content}"),
    }
    Ok(())
}

fn export_csv(papers: &[Paper]) -> String {
    fn esc(s: &str) -> String {
        // RFC 4180: quote on quote, comma, LF, and also CR, which splits
        // records in most readers.
        if s.contains(['"', ',', '\n', '\r']) {
            format!("\"{}\"", s.replace('"', "\"\""))
        } else {
            s.to_string()
        }
    }
    let mut out = String::from(
        "id,title,authors,year,venue,arxiv_id,doi,url,pdf_url,status,priority,rating,tags,added_at,started_at,finished_at\n",
    );
    for p in papers {
        let cols = [
            p.id.to_string(),
            esc(&p.title),
            esc(&p.authors),
            p.year.map(|y| y.to_string()).unwrap_or_default(),
            esc(p.venue.as_deref().unwrap_or("")),
            p.arxiv_id.clone().unwrap_or_default(),
            esc(p.doi.as_deref().unwrap_or("")),
            esc(p.url.as_deref().unwrap_or("")),
            esc(p.pdf_url.as_deref().unwrap_or("")),
            p.status.as_str().to_string(),
            p.priority.as_str().to_string(),
            p.rating.map(|r| r.to_string()).unwrap_or_default(),
            esc(&p.tags.join(",")),
            p.added_at.clone(),
            p.started_at.clone().unwrap_or_default(),
            p.finished_at.clone().unwrap_or_default(),
        ];
        out.push_str(&cols.join(","));
        out.push('\n');
    }
    out
}

fn cmd_import(conn: &Connection, file: &PathBuf, format: Option<String>) -> Result<()> {
    let content =
        std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let format = format.unwrap_or_else(|| {
        match file.extension().and_then(|e| e.to_str()).unwrap_or("") {
            "json" => "json".into(),
            "bib" | "bibtex" => "bibtex".into(),
            // Unknown extension: sniff the content rather than guessing wrong.
            _ if content.trim_start().starts_with(['[', '{']) => "json".into(),
            _ => "bibtex".into(),
        }
    });

    let mut skipped_invalid = 0;
    let papers: Vec<Paper> = match format.as_str() {
        "json" => serde_json::from_str::<Vec<serde_json::Value>>(&content)
            .context("parsing JSON (expected an array of paper objects)")?
            .iter()
            .map(paper_from_json)
            .collect::<Result<_>>()?,
        "bibtex" | "bib" => {
            let entries = bibtex::parse(&content)?;
            let total = entries.len();
            let papers: Vec<Paper> = entries.iter().filter_map(paper_from_bib).collect();
            skipped_invalid = total - papers.len();
            papers
        }
        other => bail!("unknown import format '{other}' (bibtex, json)"),
    };

    if papers.is_empty() {
        bail!(
            "no papers found in {} (parsed as {format}; use -f bibtex|json to force the format)",
            file.display()
        );
    }

    let mut imported = 0;
    let mut skipped_dup = 0;
    let tx = conn.unchecked_transaction()?;
    for mut p in papers {
        let dup = match (&p.arxiv_id, &p.doi) {
            (Some(a), _) if db::find_by_arxiv(&tx, a)?.is_some() => true,
            (_, Some(d)) if db::find_by_doi(&tx, d)?.is_some() => true,
            _ => is_title_twin(&tx, &p)?,
        };
        if dup {
            skipped_dup += 1;
            continue;
        }
        let notes = std::mem::take(&mut p.notes);
        let id = db::insert_paper(&tx, &p)?;
        for n in notes {
            tx.execute(
                "INSERT INTO notes (paper_id, created_at, body) VALUES (?1, ?2, ?3)",
                rusqlite::params![
                    id,
                    if n.created_at.is_empty() {
                        db::now()
                    } else {
                        n.created_at
                    },
                    n.body
                ],
            )?;
        }
        imported += 1;
    }
    tx.commit()?;

    let plural = |n: usize| if n == 1 { "" } else { "s" };
    let mut msg = format!("imported {imported} paper{}", plural(imported));
    if skipped_dup > 0 {
        msg += &format!(", skipped {skipped_dup} duplicate{}", plural(skipped_dup));
    }
    if skipped_invalid > 0 {
        msg += &format!(
            ", skipped {skipped_invalid} entr{} without a title",
            if skipped_invalid == 1 { "y" } else { "ies" }
        );
    }
    println!("{} {msg}", paint(GREEN, "ok:"));
    Ok(())
}

/// Import dedup for papers without identifiers: same title alone isn't
/// enough (different papers share titles), so require matching authors, or a
/// matching year when one side has no authors. A bare title-only entry on
/// both sides still counts as a twin so re-importing a dump stays idempotent.
fn is_title_twin(conn: &Connection, p: &Paper) -> Result<bool> {
    let norm = |s: &str| {
        s.to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    for candidate in db::papers_with_title(conn, p.title.trim())? {
        let twin = if !p.authors.is_empty() && !candidate.authors.is_empty() {
            norm(&p.authors) == norm(&candidate.authors)
        } else if p.year.is_some() && candidate.year.is_some() {
            p.year == candidate.year
        } else {
            // Neither side has anything to distinguish them.
            true
        };
        if twin {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Build a Paper from loose JSON (tolerant of missing fields).
fn paper_from_json(v: &serde_json::Value) -> Result<Paper> {
    let s = |k: &str| v[k].as_str().map(String::from).filter(|x| !x.is_empty());
    let title =
        s("title").ok_or_else(|| anyhow::anyhow!("a JSON paper entry is missing 'title'"))?;
    let tags: Vec<String> = v["tags"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|t| t.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let notes = v["notes"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|n| {
                    n["body"].as_str().map(|b| model::Note {
                        created_at: n["created_at"].as_str().unwrap_or("").to_string(),
                        body: b.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Paper {
        id: 0,
        title,
        authors: s("authors").unwrap_or_default(),
        year: v["year"].as_i64().map(|y| y as i32),
        venue: s("venue"),
        arxiv_id: s("arxiv_id").map(|a| fetch::normalize_arxiv_id(&a)),
        doi: s("doi"),
        url: s("url"),
        pdf_url: s("pdf_url"),
        abstract_: s("abstract"),
        status: s("status")
            .and_then(|x| Status::parse(&x))
            .unwrap_or(Status::ToRead),
        priority: s("priority")
            .and_then(|x| Priority::parse(&x))
            .unwrap_or(Priority::Normal),
        rating: v["rating"].as_i64().filter(|r| (1..=5).contains(r)),
        tags: normalize_tags(&tags),
        added_at: s("added_at").unwrap_or_else(db::now),
        started_at: s("started_at"),
        finished_at: s("finished_at"),
        notes,
    })
}

fn paper_from_bib(e: &bibtex::BibEntry) -> Option<Paper> {
    let f = |k: &str| e.fields.get(k).cloned().filter(|v| !v.is_empty());
    let title = f("title")?;
    let authors = f("author")
        .map(|a| {
            a.split(" and ")
                .map(bibtex::normalize_author)
                .filter(|x| !x.is_empty())
                .collect::<Vec<_>>()
                .join("; ")
        })
        .unwrap_or_default();
    let arxiv_id = f("eprint")
        .filter(|_| {
            e.fields
                .get("archiveprefix")
                .or_else(|| e.fields.get("eprinttype"))
                .is_none_or(|p| p.eq_ignore_ascii_case("arxiv"))
        })
        .map(|a| fetch::normalize_arxiv_id(&a));
    let url = f("url").or_else(|| {
        arxiv_id
            .as_ref()
            .map(|a| format!("https://arxiv.org/abs/{a}"))
    });
    // Real-world exporters separate keywords with ',' or ';'.
    let tags: Vec<String> = f("keywords")
        .map(|k| k.split([',', ';']).map(|t| t.trim().to_string()).collect())
        .unwrap_or_default();
    Some(Paper {
        id: 0,
        title,
        authors,
        year: f("year").and_then(|y| y.parse().ok()),
        venue: f("journal")
            .or_else(|| f("booktitle"))
            .or_else(|| f("howpublished")),
        arxiv_id,
        doi: f("doi"),
        url,
        pdf_url: None,
        abstract_: f("abstract"),
        status: Status::ToRead,
        priority: Priority::Normal,
        rating: None,
        tags: normalize_tags(&tags),
        added_at: db::now(),
        started_at: None,
        finished_at: None,
        notes: vec![],
    })
}
