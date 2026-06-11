# RList

A fast, featureful command-line reading list for academic papers.

Add a paper using arXiv id, DOI, or url and rlist will fetch its metadata automatically.
Track what you plan to read, are reading, and have read.
Tag, prioritize, rate and annotate papers.
Export to BibTeX when it's time to cite.

## Building & installing

### Prerequisites

- **Rust 1.88 or newer**, install via [rustup](https://rustup.rs/):
  `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
  (or update an existing toolchain with `rustup update`)
- **A C compiler** (`gcc` or `clang`), used once at build time to compile
  the bundled SQLite

### Build and install

```sh
git clone <repo-url> && cd RList
cargo build --release
cp target/release/rlist ~/.local/bin/   # or anywhere on your PATH
```

Or let cargo do the copying (installs to `~/.cargo/bin`):

```sh
cargo install --path
```

### Shell completions (optional)

```sh
rlist completions fish > ~/.config/fish/completions/rlist.fish
rlist completions bash > ~/.local/share/bash-completion/completions/rlist
rlist completions zsh  > ~/.zfunc/_rlist
```

## Uninstall

Uninstalling is built in, with two modes:

```sh
rlist uninstall           # soft: removes the binary and shell completions,
                          # but keeps your reading list and notes
rlist uninstall --purge   # hard: removes EVERYTHING, including the database
```

Both ask for confirmation first (`--force` skips it). A soft uninstall leaves
the database in place, so reinstalling later picks your list right back up.

## Quick start

```sh
# Add papers metadata is fetched for you
rlist add 1706.03762 -t transformers -p high      # arXiv id
rlist add 10.1038/nature14539 -t deep-learning    # DOI
rlist add https://arxiv.org/abs/2005.14165        # arXiv URL
rlist add "Some Obscure Tech Report" --authors "Jane Doe; Bob Roe" --year 2024

# Your queue (to-read + reading, high priority first)
rlist

# What should I read next?
rlist next

# Reading lifecycle
rlist start 3                  # mark as reading
rlist done 3 -r 5              # finished, rated 5/5
rlist drop 7                   # decided not to read it

# Notes & details
rlist note 3 "key idea: scaled dot-product attention"
rlist note 3                   # no text -> opens $EDITOR
rlist show 3                   # full details: abstract, links, notes

# Find things, full-text over titles, authors, abstracts, tags, notes
rlist search attention transfor     # last term matches as a prefix

# Open in the browser
rlist open 3                   # paper page
rlist open 3 --pdf             # PDF link

# Slice your list
rlist list -s read --sort rating         # best papers you've read
rlist list -t transformers -A            # everything tagged transformers
rlist list --author hinton --sort year   # by author, newest first
rlist list --json                        # machine-readable

# Export / import
rlist export -f bibtex -o refs.bib       # also: json, csv
rlist export -t transformers             # filter what you export
rlist import refs.bib                    # BibTeX or JSON, duplicates skipped

# Overview
rlist stats                    # counts, monthly histogram, oldest in queue
rlist tags                     # tags with counts
```

## Reference

| Command | What it does |
|---|---|
| `add <ref>` | Add by arXiv id, DOI, URL, or plain title. `-t` tag, `-p` priority, `--status`, `-r` rating, `--note`, `--no-fetch` |
| `list` (`ls`) | List papers. Default shows your queue; `-A` all, `-s` status, `-t` tag, `-a` author, `-y` year, `--sort`, `-R` reverse, `-n` limit, `--json` |
| `show <id>` | Full details incl. abstract and notes. `--json` |
| `search <terms>` | FTS5 full-text search; also matches notes. `find` is an alias |
| `next` | Suggest what to read (priority, then oldest). `--random`, `-t` tag |
| `start / done / drop <ids>` | Status transitions with timestamps. `done -r 1..5` rates |
| `edit <id>` | Change any field; `-t`/`--rm-tag` manage tags |
| `note <id> [text]` | Append a timestamped note; no text opens `$EDITOR` |
| `open <id>` | Open page (or `--pdf`) in your browser |
| `rm <ids>` | Delete (asks unless `--force`) |
| `tags` / `stats` | Tag counts / reading statistics |
| `export` | BibTeX, JSON, or CSV; filterable; `-o` file |
| `import <file>` | BibTeX or JSON; skips duplicates |
| `path` | Print the database location |
| `completions <shell>` | Shell completion script |

Statuses: `to-read` ○, `reading` ◐, `read` ●, `dropped`.

Priorities: `high` ↑, `normal`, `low` ↓.

## Data

Everything lives in one SQLite file: `~/.local/share/rlist/rlist.db`
(override with `--db` or `$RLIST_DB`). Back it up by copying the file;
`rlist export -f json` is a portable full dump including notes.

Metadata sources: the [arXiv API](https://info.arxiv.org/help/api/) for arXiv
ids and [Crossref](https://api.crossref.org) (with doi.org content negotiation
as a fallback) for DOIs.

## AI Notice

This entire repo is almost entirely vibe-coded.
Use at your own risk.
