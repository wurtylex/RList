#!/usr/bin/env bash
# Reproducible benchmark for rlist. See the Benchmarks section of the README.
#
# Requires: hyperfine (https://github.com/sharkdp/hyperfine), python3
# Usage:    scripts/bench.sh [paper-count]   (default 10000)

set -euo pipefail
cd "$(dirname "$0")/.."

COUNT="${1:-10000}"
DIR="$(mktemp -d /tmp/rlist-bench.XXXXXX)"
trap 'rm -rf "$DIR"' EXIT
BIN="target/release/rlist"
DB="$DIR/bench.db"

echo "== building release binary"
cargo build --release --locked

echo "== generating $COUNT synthetic papers"
python3 - "$COUNT" "$DIR/papers.json" <<'EOF'
import json, random, sys

count, out = int(sys.argv[1]), sys.argv[2]
random.seed(42)  # deterministic dataset

topics = ["neural", "transformer", "graph", "quantum", "bayesian", "sparse",
          "causal", "federated", "adversarial", "multimodal", "symbolic",
          "convex", "stochastic", "geometric", "probabilistic"]
objects = ["networks", "optimization", "inference", "representations",
           "attention", "embeddings", "regularization", "generalization",
           "pretraining", "distillation", "compression", "alignment"]
domains = ["language modeling", "computer vision", "robotics", "genomics",
           "recommendation", "speech recognition", "drug discovery",
           "time series forecasting", "program synthesis", "reinforcement learning"]
first = ["Ada", "Grace", "Alan", "Edsger", "Barbara", "Donald", "John",
         "Claude", "Yoshua", "Fei-Fei", "Geoffrey", "Jürgen", "Daphne", "Judea"]
last = ["Lovelace", "Hopper", "Turing", "Dijkstra", "Liskov", "Knuth",
        "McCarthy", "Shannon", "Bengio", "Li", "Hinton", "Schmidhuber",
        "Koller", "Pearl", "Vapnik", "Hochreiter"]
statuses = ["to-read"] * 60 + ["read"] * 30 + ["reading"] * 5 + ["dropped"] * 5
words = topics + objects + ["method", "model", "bound", "theorem", "dataset",
        "baseline", "ablation", "benchmark", "convergence", "scaling", "robust",
        "efficient", "framework", "analysis", "empirical", "novel", "approach"]

papers = []
for i in range(count):
    t, o, d = random.choice(topics), random.choice(objects), random.choice(domains)
    authors = "; ".join(f"{random.choice(first)} {random.choice(last)}"
                        for _ in range(random.randint(1, 6)))
    abstract = " ".join(random.choice(words) for _ in range(100)).capitalize() + "."
    papers.append({
        "title": f"{t.capitalize()} {o} for {d} (study {i})",
        "authors": authors,
        "year": random.randint(1995, 2026),
        "venue": random.choice(["NeurIPS", "ICML", "ICLR", "Nature", "JMLR", "arXiv"]),
        "arxiv_id": f"{random.randint(1501, 2612)}.{i:05d}",
        "abstract": abstract,
        "status": random.choice(statuses),
        "priority": random.choice(["low", "normal", "normal", "high"]),
        "rating": random.choice([None, 1, 2, 3, 4, 5]),
        "tags": random.sample(topics, k=random.randint(0, 3)),
    })
json.dump(papers, open(out, "w"))
EOF

echo "== importing into a fresh database"
time "$BIN" --db "$DB" import "$DIR/papers.json"
cp "$DB" "$DB.pristine"
DBSIZE=$(du -h "$DB" | cut -f1)
echo "== database size: $DBSIZE"

echo "== running hyperfine"
hyperfine --warmup 3 --runs 20 \
  --export-markdown "$DIR/results.md" --export-json "$DIR/results.json" \
  --command-name "path (startup only)"   "$BIN --db $DB path" \
  --command-name "list (queue view)"     "$BIN --db $DB list" \
  --command-name "list -A ($COUNT rows)" "$BIN --db $DB list -A" \
  --command-name "search (FTS5)"         "$BIN --db $DB search neural attention" \
  --command-name "show + notes"          "$BIN --db $DB show 5000" \
  --command-name "next"                  "$BIN --db $DB next" \
  --command-name "stats"                 "$BIN --db $DB stats" \
  --command-name "export bibtex"         "$BIN --db $DB export -f bibtex"

hyperfine --warmup 3 --runs 20 \
  --export-json "$DIR/results-write.json" \
  --prepare "cp $DB.pristine $DB" \
  --command-name "add (write path)" \
  "$BIN --db $DB add 'A Benchmark Paper' --authors 'Bench Mark' --year 2026 --no-fetch"

echo "== rendering chart"
mkdir -p docs
python3 scripts/bench_chart.py docs/benchmark.svg "$DIR/results.json" "$DIR/results-write.json"

echo
echo "== markdown results at $DIR/results.md =="
cat "$DIR/results.md"
cp "$DIR/results.md" /tmp/rlist-bench-results.md
