# diff-crawler

[![CI](https://github.com/AadityaVardhanDas/diff-crawler-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/AadityaVardhanDas/diff-crawler-rs/actions/workflows/ci.yml)

A fast CLI tool that diffs two ML project directories across five layers:
tree structure, code changes, data file schemas, binary/model hashes, and
probabilistic rename detection.  Written in Rust; ships as a single static
binary with no runtime dependencies.

---

## Quick start

```bash
# Compare two project checkpoints
diff-crawler ./project-v1 ./project-v2

# Write a JSON report and a Markdown summary
diff-crawler ./project-v1 ./project-v2 --json report.json --markdown report.md

# Skip full unified diffs (faster; still shows counts + similarity)
diff-crawler ./project-v1 ./project-v2 --no-diff

# Deep data stats (null counts + approx distinct per column)
diff-crawler ./project-v1 ./project-v2 --deep
```

Output goes to stdout as Markdown when neither `--json` nor `--markdown` is given.

---

## Installation

### Option 1: Pre-built binary (recommended)

Download the archive for your platform from the
[latest GitHub Release](https://github.com/AadityaVardhanDas/diff-crawler-rs/releases/latest):

| Platform | Archive |
|---|---|
| Linux x86\_64 | `diff-crawler-linux-x86_64.tar.gz` |
| macOS Apple Silicon | `diff-crawler-macos-arm64.tar.gz` |
| macOS Intel | `diff-crawler-macos-x86_64.tar.gz` |

Extract and install:

```bash
# Linux / macOS — adjust the archive name for your platform
tar xzf diff-crawler-linux-x86_64.tar.gz
sudo mv diff-crawler /usr/local/bin/
diff-crawler --help
```

> **Linux glibc requirement**: the Linux binary is compiled on Ubuntu 22.04
> (glibc 2.35).  It runs on Debian 11+, Ubuntu 20.04+, Fedora 36+, and any
> other distro with glibc ≥ 2.35.  Older distros should use the Docker option.

### Option 2: Docker + wrapper script (no install, any OS)

Pull the image:

```bash
docker pull ghcr.io/aadityavardhandardas/diff-crawler:latest
```

Install the host-side wrapper script so you can call it like a native binary:

```bash
sudo curl -fsSL \
  https://raw.githubusercontent.com/AadityaVardhanDas/diff-crawler-rs/main/scripts/diff-crawler \
  -o /usr/local/bin/diff-crawler
sudo chmod +x /usr/local/bin/diff-crawler
```

Then use it exactly as you would the native binary:

```bash
diff-crawler ./project-v1 ./project-v2 --json report.json
```

The wrapper script automatically:
- Resolves both directory paths to absolute paths
- Bind-mounts them read-only as `/in/a` and `/in/b` inside the container
- Mounts the current working directory as `/out` so `--json`/`--markdown`
  outputs land in `$PWD`
- Passes `--user $(id -u):$(id -g)` so output files are owned by you, not root

Override the image with `DIFF_CRAWLER_IMAGE`:

```bash
DIFF_CRAWLER_IMAGE=ghcr.io/aadityavardhandardas/diff-crawler:1.0.0 \
  diff-crawler ./v1 ./v2
```

### Option 3: Build from source

Requirements: [Rust stable](https://rustup.rs/) (1.81+)

```bash
git clone https://github.com/AadityaVardhanDas/diff-crawler-rs
cd diff-crawler-rs
cargo build --release
# Binary is at: target/release/diff-crawler
sudo cp target/release/diff-crawler /usr/local/bin/
```

---

## Usage

```
diff-crawler DIR_A DIR_B [OPTIONS]

Arguments:
  DIR_A   First directory (the "before" snapshot)
  DIR_B   Second directory (the "after" snapshot)

Options:
  --json <PATH>               Write the full JSON report to this file
  --markdown <PATH>           Write a Markdown summary to this file
  --no-diff                   Skip full unified diffs (keep line counts + similarity)
  --deep                      Add per-column null counts and approx distinct counts
                              to data diffs (slower)
  --rename-threshold <FLOAT>  Minimum combined score to report a rename [default: 0.4]
  -v, --verbose               Emit debug tracing to stderr
  -h, --help                  Print help
```

### Examples

```bash
# Minimal: print Markdown to stdout
diff-crawler ./checkpoint-epoch-10 ./checkpoint-epoch-20

# Save both formats
diff-crawler ./v1 ./v2 --json diff.json --markdown diff.md

# Faster scan (no line-by-line diffs, just counts)
diff-crawler ./v1 ./v2 --no-diff --json diff.json

# Rename detection with a tighter threshold
diff-crawler ./v1 ./v2 --rename-threshold 0.6

# Verbose mode to see what's being processed
diff-crawler ./v1 ./v2 --verbose 2>debug.log
```

### Exit codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Usage error (bad arguments) |
| 2 | I/O or runtime error |

---

## Output formats

### Markdown (default)

When neither `--json` nor `--markdown` is given, or when only `--json` is
given, the Markdown report is printed to stdout.  It contains:

- Overall similarity score
- Tree diff summary (files added / removed / shared)
- Code changes (sorted by similarity ascending — most-changed first)
- Data file changes (row counts, schema changes)
- Binary file changes (SHA-256 comparison)
- Rename candidates

### JSON report

Pass `--json report.json` to write a machine-readable report.  The schema:

```jsonc
{
  "dir_a": "/abs/path/to/A",
  "dir_b": "/abs/path/to/B",

  "tree": {
    "in_both":   ["model.py", "train.py"],   // sorted
    "only_in_a": ["old_utils.py"],
    "only_in_b": ["new_helpers.py"]
  },

  "code_diffs": [
    {
      "path":          "train.py",
      "identical":     false,
      "added_lines":   3,
      "removed_lines": 1,
      "similarity":    0.9118,
      "unified_diff":  "--- a/train.py\n+++ b/train.py\n...",  // omitted with --no-diff
      "note":          null,
      "error":         null
    }
  ],

  "data_diffs": [
    {
      "path":           "metrics.csv",
      "row_count_a":    1000,
      "row_count_b":    1200,
      "schema_a":       { "loss": "DOUBLE", "epoch": "BIGINT" },
      "schema_b":       { "loss": "DOUBLE", "epoch": "BIGINT", "lr": "DOUBLE" },
      "columns_added":  ["lr"],
      "columns_removed":[],
      "type_changes":   {},           // {"col": ["OLD_TYPE", "NEW_TYPE"]}
      "column_stats_a": null,         // populated with --deep
      "column_stats_b": null,
      "note":           null,
      "error":          null
    }
  ],

  "binary_diffs": [
    {
      "path":      "weights.pkl",
      "identical": false,
      "size_a":    204800,
      "size_b":    209920,
      "sha256_a":  "abc123...",
      "sha256_b":  "def456..."
    }
  ],

  "rename_candidates": [
    {
      "from_path":          "old_trainer.py",
      "to_path":            "trainer_v2.py",
      "content_similarity": 0.87,
      "name_similarity":    0.62,
      "combined_score":     0.81
    }
  ],

  "summary": {
    "tree": {
      "files_in_both": 15,
      "only_in_a":     2,
      "only_in_b":     3,
      "tree_jaccard":  0.75
    },
    "code": {
      "compared":           10,
      "identical":           7,
      "total_added_lines":  42,
      "total_removed_lines":18,
      "avg_similarity":     0.94
    },
    "data": {
      "compared":        3,
      "schema_changed":  1,
      "total_row_delta": 200
    },
    "binary": {
      "compared":  4,
      "identical": 3
    },
    "renames_detected": 1,
    "overall_similarity": 0.85
  }
}
```

---

## The five diff layers

### 1. Tree diff

Walks both directory trees recursively, collecting relative file paths.
Computes `in_both`, `only_in_a`, `only_in_b`, and tree Jaccard similarity
(`|in_both| / |union|`).

**Excluded directories**: `.venv`, `venv`, `env`, `.env`, `virtualenv`,
`__pycache__`, `.git`, `node_modules`, `.ipynb_checkpoints`, `.pytest_cache`,
`.mypy_cache`, `.ruff_cache`, `.tox`, `dist`, `build`, `*.egg-info`, `mlruns`,
`wandb`, `lightning_logs`, `.idea`, `.vscode`.

**Skipped file suffixes**: `.pyc`, `.pyo`, `.pyd`, `.class`, `.log`, `.tmp`,
`.swp`, `.swo`.

**Skipped files**: `.DS_Store`, `Thumbs.db`.  Symlinks are skipped.

### 2. Code diff

For text/code files present in both directories:
- **Fast path**: if file sizes are equal and SHA-256 hashes match, mark
  identical without reading the content
- **Line counts**: added and removed line counts from a Myers diff
- **Similarity**: character-level ratio in [0, 1] (0 = completely different,
  1 = identical)
- **Unified diff**: optional (omitted with `--no-diff` or for files > 2 MB)

Files > 2 MB still get a similarity score of 1.0 and a note explaining the
skip.

### 3. Data diff

For CSV, TSV, Parquet, JSONL, and NDJSON files:
- Row counts for both sides
- Column schemas (inferred types)
- Columns added, removed, or with type changes

With `--deep`: per-column null count and approximate distinct count.

### 4. Binary diff

For `.pkl`, `.pt`, `.pth`, `.ckpt`, `.safetensors`, `.h5`, `.hdf5`, `.onnx`,
`.pb`, `.tflite`, `.npy`, `.npz`, images, audio, video, and archives:
- SHA-256 hash comparison
- File sizes
- `identical: true/false`

No content is decoded — binary files are treated as opaque blobs.

### 5. Rename detection

For text/code files unique to one side:
- k=5 character shingles, MinHash with 128 permutations, LSH threshold 0.3
- Combined score = `0.75 × content_jaccard + 0.25 × filename_similarity`
- Reported if combined score ≥ `--rename-threshold` (default 0.4)
- Results sorted descending by combined score

### Overall similarity

```
if files_in_both == 0 and there are files only on one side:
    overall = tree_jaccard
else:
    overall = 0.5 × tree_jaccard + 0.5 × avg_code_similarity
```

---

## CI/CD integration

### GitHub Actions — diff on PR

Add a diff report as a PR comment to track how a training run changed:

```yaml
- name: Diff model directories
  run: |
    diff-crawler ./baseline ./candidate --json diff.json --no-diff
  
- name: Post diff summary
  uses: actions/github-script@v7
  with:
    script: |
      const fs = require('fs');
      const report = JSON.parse(fs.readFileSync('diff.json'));
      const sim = (report.summary.overall_similarity * 100).toFixed(1);
      github.rest.issues.createComment({
        ...context.repo,
        issue_number: context.issue.number,
        body: `### Model diff\nOverall similarity: **${sim}%**`
      });
```

### Makefile target

```makefile
.PHONY: diff
diff:
	diff-crawler ./baseline ./$(VERSION) --json diff-$(VERSION).json --markdown diff-$(VERSION).md
```

---

## Development

### Prerequisites

- Rust stable (install via [rustup](https://rustup.rs/))
- Python 3.11+ with `duckdb` and `datasketch` (for parity tests only)

### Building

```bash
cargo build           # debug build (fast, no optimizations)
cargo build --release # release build → target/release/diff-crawler
```

### Testing

```bash
cargo test                        # all tests
cargo test walker                 # tests matching "walker"
cargo test -- --nocapture         # show println! output
cargo test parity -- --nocapture  # parity tests against Python reference
```

**Parity tests** (`tests/parity.rs`) require the Python reference to be
importable.  They will skip gracefully if Python or `duckdb`/`datasketch` are
not installed.  To set them up:

```bash
pip install duckdb datasketch
# Then run:
cargo test parity
```

### Linting

```bash
cargo clippy -- -D warnings   # must be clean (same check as CI)
cargo fmt                     # auto-format
cargo fmt --check             # check only (same as CI)
```

### Running the Python reference

The Python reference implementation lives in `reference/diff_crawler/` and
defines the exact semantics the Rust port must match.

```bash
cd reference/diff_crawler
pip install duckdb datasketch
python -m diff_crawler /path/to/A /path/to/B --json ref.json
```

---

## Architecture

```
src/
├── main.rs        CLI entry — arg parsing, tracing init, dispatch, exit codes
├── lib.rs         re-exports
├── config.rs      CrawlConfig — exclusion lists, thresholds, feature flags
├── classifier.rs  classifies files as code / data / binary / ignore
├── walker.rs      recursive tree walk → TreeIndex + TreeDiff
├── code_diff.rs   SHA-256 fast path + Myers diff + character similarity
├── data_diff.rs   CSV/Parquet/JSONL schema + row-count diff
├── matcher.rs     MinHash LSH rename detection
├── crawler.rs     orchestrator — parallel rayon dispatch over all files
├── report.rs      CrawlReport → JSON (serde) + Markdown renderer
└── error.rs       typed error types
```

Key crates:
- `similar` — Myers diff + character-level ratio (Rust analog of Python's `difflib`)
- `rayon` — parallel per-file processing
- `sha2` — SHA-256 parity with the Python reference
- `parquet` + `csv` — pure-Rust data file parsing (no DuckDB at runtime)
- `clap` — CLI argument parsing (derive API)
- `serde` + `serde_json` — JSON serialization

---

## Performance

Targets on a modern laptop, release build, ~5 000-file repo:

| Operation | Target |
|-----------|--------|
| Tree walk + classification | < 200 ms |
| Full crawl (code + data diff, no rename) | < 3 s |
| Full crawl with rename detection | < 8 s |
| Peak memory | < 500 MB |

Profile with:
```bash
cargo flamegraph -- ./A ./B --no-diff
```

---

## Contributing

1. Fork and clone
2. Make changes; ensure `cargo test` and `cargo clippy -- -D warnings` pass
3. Open a pull request — CI runs automatically

Please match the coding conventions in `CLAUDE.md`.

---

## License

MIT — see [LICENSE](LICENSE) for details.
