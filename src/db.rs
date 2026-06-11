use crate::model::{Note, Paper, Priority, Status};
use anyhow::{Context, Result};
use rusqlite::{Connection, Row, params};
use std::path::{Path, PathBuf};

pub fn default_db_path() -> PathBuf {
    if let Ok(p) = std::env::var("RLIST_DB")
        && !p.is_empty()
    {
        return PathBuf::from(p);
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("rlist")
        .join("rlist.db")
}

pub fn open(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating data directory {}", parent.display()))?;
    }
    let conn =
        Connection::open(path).with_context(|| format!("opening database {}", path.display()))?;
    // WAL needs to create side files next to the db; tolerate failure so
    // read-only setups (unwritable directory) can still run read commands.
    let _ = conn.pragma_update(None, "journal_mode", "WAL");
    conn.pragma_update(None, "foreign_keys", "ON")
        .with_context(|| format!("initializing database {}", path.display()))?;
    migrate(&conn).with_context(|| format!("initializing database {}", path.display()))?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version < 1 {
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS papers (
                id          INTEGER PRIMARY KEY,
                title       TEXT NOT NULL,
                authors     TEXT NOT NULL DEFAULT '',
                year        INTEGER,
                venue       TEXT,
                arxiv_id    TEXT UNIQUE,
                doi         TEXT UNIQUE,
                url         TEXT,
                pdf_url     TEXT,
                abstract    TEXT,
                status      TEXT NOT NULL DEFAULT 'to-read',
                priority    INTEGER NOT NULL DEFAULT 1,
                rating      INTEGER,
                tags        TEXT NOT NULL DEFAULT '',
                added_at    TEXT NOT NULL,
                started_at  TEXT,
                finished_at TEXT
            );
            CREATE TABLE IF NOT EXISTS notes (
                id         INTEGER PRIMARY KEY,
                paper_id   INTEGER NOT NULL REFERENCES papers(id) ON DELETE CASCADE,
                created_at TEXT NOT NULL,
                body       TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_notes_paper ON notes(paper_id);
            CREATE VIRTUAL TABLE IF NOT EXISTS papers_fts USING fts5(
                title, authors, abstract, tags,
                content='papers', content_rowid='id'
            );
            CREATE TRIGGER IF NOT EXISTS papers_ai AFTER INSERT ON papers BEGIN
                INSERT INTO papers_fts(rowid, title, authors, abstract, tags)
                VALUES (new.id, new.title, new.authors, coalesce(new.abstract,''), new.tags);
            END;
            CREATE TRIGGER IF NOT EXISTS papers_ad AFTER DELETE ON papers BEGIN
                INSERT INTO papers_fts(papers_fts, rowid, title, authors, abstract, tags)
                VALUES ('delete', old.id, old.title, old.authors, coalesce(old.abstract,''), old.tags);
            END;
            CREATE TRIGGER IF NOT EXISTS papers_au AFTER UPDATE ON papers BEGIN
                INSERT INTO papers_fts(papers_fts, rowid, title, authors, abstract, tags)
                VALUES ('delete', old.id, old.title, old.authors, coalesce(old.abstract,''), old.tags);
                INSERT INTO papers_fts(rowid, title, authors, abstract, tags)
                VALUES (new.id, new.title, new.authors, coalesce(new.abstract,''), new.tags);
            END;
            PRAGMA user_version = 1;
            "#,
        )?;
    }
    Ok(())
}

pub fn now() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

fn paper_from_row(row: &Row) -> rusqlite::Result<Paper> {
    let status: String = row.get("status")?;
    let priority: i64 = row.get("priority")?;
    let tags: String = row.get("tags")?;
    Ok(Paper {
        id: row.get("id")?,
        title: row.get("title")?,
        authors: row.get("authors")?,
        year: row.get("year")?,
        venue: row.get("venue")?,
        arxiv_id: row.get("arxiv_id")?,
        doi: row.get("doi")?,
        url: row.get("url")?,
        pdf_url: row.get("pdf_url")?,
        abstract_: row.get("abstract")?,
        status: Status::parse(&status).unwrap_or(Status::ToRead),
        priority: Priority::from_int(priority),
        rating: row.get("rating")?,
        tags: tags
            .split(',')
            .filter(|t| !t.is_empty())
            .map(String::from)
            .collect(),
        added_at: row.get("added_at")?,
        started_at: row.get("started_at")?,
        finished_at: row.get("finished_at")?,
        notes: Vec::new(),
    })
}

const PAPER_COLS: &str = "id, title, authors, year, venue, arxiv_id, doi, url, pdf_url, \
                          abstract, status, priority, rating, tags, added_at, started_at, finished_at";

pub fn insert_paper(conn: &Connection, p: &Paper) -> Result<i64> {
    conn.execute(
        "INSERT INTO papers (title, authors, year, venue, arxiv_id, doi, url, pdf_url, abstract,
                             status, priority, rating, tags, added_at, started_at, finished_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
        params![
            p.title,
            p.authors,
            p.year,
            p.venue,
            p.arxiv_id,
            p.doi,
            p.url,
            p.pdf_url,
            p.abstract_,
            p.status.as_str(),
            p.priority as i64,
            p.rating,
            p.tags.join(","),
            p.added_at,
            p.started_at,
            p.finished_at,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn update_paper(conn: &Connection, p: &Paper) -> Result<()> {
    let n = conn.execute(
        "UPDATE papers SET title=?1, authors=?2, year=?3, venue=?4, arxiv_id=?5, doi=?6, url=?7,
                           pdf_url=?8, abstract=?9, status=?10, priority=?11, rating=?12, tags=?13,
                           added_at=?14, started_at=?15, finished_at=?16
         WHERE id=?17",
        params![
            p.title,
            p.authors,
            p.year,
            p.venue,
            p.arxiv_id,
            p.doi,
            p.url,
            p.pdf_url,
            p.abstract_,
            p.status.as_str(),
            p.priority as i64,
            p.rating,
            p.tags.join(","),
            p.added_at,
            p.started_at,
            p.finished_at,
            p.id,
        ],
    )?;
    anyhow::ensure!(n == 1, "no paper with id {}", p.id);
    Ok(())
}

pub fn get_paper(conn: &Connection, id: i64) -> Result<Paper> {
    let mut p = conn
        .query_row(
            &format!("SELECT {PAPER_COLS} FROM papers WHERE id = ?1"),
            [id],
            paper_from_row,
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => {
                anyhow::anyhow!("no paper with id {id} (try `rlist list --all`)")
            }
            other => anyhow::Error::from(other),
        })?;
    p.notes = get_notes(conn, id)?;
    Ok(p)
}

pub fn delete_paper(conn: &Connection, id: i64) -> Result<bool> {
    let n = conn.execute("DELETE FROM papers WHERE id = ?1", [id])?;
    Ok(n > 0)
}

pub fn all_papers(conn: &Connection) -> Result<Vec<Paper>> {
    let mut stmt = conn.prepare(&format!("SELECT {PAPER_COLS} FROM papers ORDER BY id"))?;
    let rows = stmt.query_map([], paper_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn find_by_arxiv(conn: &Connection, arxiv_id: &str) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT id FROM papers WHERE arxiv_id = ?1 COLLATE NOCASE",
            [arxiv_id],
            |r| r.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?)
}

pub fn find_by_doi(conn: &Connection, doi: &str) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT id FROM papers WHERE doi = ?1 COLLATE NOCASE",
            [doi],
            |r| r.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?)
}

pub fn find_by_title(conn: &Connection, title: &str) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT id FROM papers WHERE title = ?1 COLLATE NOCASE",
            [title],
            |r| r.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?)
}

/// All papers whose title matches case-insensitively. Used by import dedup,
/// which must distinguish "same paper again" from "different paper that
/// happens to share a title".
pub fn papers_with_title(conn: &Connection, title: &str) -> Result<Vec<Paper>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {PAPER_COLS} FROM papers WHERE title = ?1 COLLATE NOCASE"
    ))?;
    let rows = stmt.query_map([title], paper_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn add_note(conn: &Connection, paper_id: i64, body: &str) -> Result<()> {
    // Ensure the paper exists so we fail with a friendly error.
    get_paper(conn, paper_id)?;
    conn.execute(
        "INSERT INTO notes (paper_id, created_at, body) VALUES (?1, ?2, ?3)",
        params![paper_id, now(), body],
    )?;
    Ok(())
}

pub fn get_notes(conn: &Connection, paper_id: i64) -> Result<Vec<Note>> {
    let mut stmt =
        conn.prepare("SELECT created_at, body FROM notes WHERE paper_id = ?1 ORDER BY id")?;
    let rows = stmt.query_map([paper_id], |r| {
        Ok(Note {
            created_at: r.get(0)?,
            body: r.get(1)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Full-text search across title/authors/abstract/tags (FTS5) plus note
/// bodies (LIKE). Returns papers ordered by FTS rank, then id.
pub fn search(conn: &Connection, query: &str) -> Result<Vec<Paper>> {
    let terms: Vec<&str> = query.split_whitespace().collect();
    if terms.is_empty() {
        return Ok(Vec::new());
    }

    // Quote each term so user input can't break FTS5 query syntax; the last
    // term matches as a prefix so `rlist search transfor` finds transformers.
    let fts_query = terms
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let quoted = format!("\"{}\"", t.replace('"', "\"\""));
            if i == terms.len() - 1 {
                format!("{quoted}*")
            } else {
                quoted
            }
        })
        .collect::<Vec<_>>()
        .join(" ");

    let mut ids: Vec<i64> = Vec::new();
    let mut stmt =
        conn.prepare("SELECT rowid FROM papers_fts WHERE papers_fts MATCH ?1 ORDER BY rank")?;
    let rows = stmt.query_map([&fts_query], |r| r.get::<_, i64>(0))?;
    for id in rows {
        ids.push(id?);
    }

    // Also match papers whose notes contain every term.
    let like_clauses = vec!["body LIKE ? ESCAPE '\\'"; terms.len()].join(" AND ");
    let sql = format!("SELECT DISTINCT paper_id FROM notes WHERE {like_clauses}");
    let mut stmt = conn.prepare(&sql)?;
    let like_params: Vec<String> = terms
        .iter()
        .map(|t| {
            // The escape character must be escaped first, or backslashes in
            // the user's term get consumed as LIKE escapes.
            let t = t
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            format!("%{t}%")
        })
        .collect();
    let rows = stmt.query_map(rusqlite::params_from_iter(like_params.iter()), |r| {
        r.get::<_, i64>(0)
    })?;
    for id in rows {
        let id = id?;
        if !ids.contains(&id) {
            ids.push(id);
        }
    }

    ids.iter().map(|&id| get_paper(conn, id)).collect()
}
