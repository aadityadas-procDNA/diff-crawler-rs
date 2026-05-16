# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

# diff-crawler (Rust port)

A CLI tool that diffs two ML project directories across multiple layers: directory structure, code, data files, binary/model files, and probabilistic rename detection. Shipped as a single static binary and as a Docker image. No runtime dependencies for the end user.

A working **Python reference implementation** exists at `reference/diff_crawler/` (Python package). It defines the exact semantics and output shape that this Rust port must match. Treat the Python version as the spec; when in doubt about behavior, read its source and replicate.

---

## Development commands

### Python reference (spec/testing)

```bash
# Install deps (from the package dir, which contains pyproject.toml)
cd reference/diff_crawler
pip install -r requirements.txt

# Run against two directories
python -m diff_crawler /path/to/A /path/to/B
python -m diff_crawler /path/to/A /path/to/B --json report.json --markdown report.md
```

### Rust (once Cargo.toml exists)

```bash
cargo build --release          # build optimized binary → target/release/diff-crawler
cargo build                    # debug build
cargo test                     # all tests
cargo test walker              # tests in tests/walker.rs or matching "walker"
cargo test -- --nocapture      # show println! output during tests
cargo clippy -- -D warnings    # lint (must be clean)
cargo fmt                      # format
cargo flamegraph               # profiling (requires cargo-flamegraph)
```

---

## Non-obvious parity details from the Python source

These are behaviors not obvious from reading the spec alone but required for JSON output parity. Verified by reading the Python source directly.

### classifier.py
- Excluded dirs include `.env` and `virtualenv` (in addition to `.venv`/`venv`/`env`).
- `.egg-info` dirs are detected by checking if any path component ends with `.egg-info` (suffix match on the part string, not the dir name alone).
- Unknown extension sniffing order: null byte in first 2048 bytes → binary; UTF-8 decodable → code; otherwise → binary.

### code_diff.py (`diff_code`)
- SHA-256 fast path only fires if file sizes are equal first (size check gates the hash). If sizes differ, proceeds directly to full diff.
- For files >2 MB: returns `CodeDiffResult` with `identical=False`, zero `added_lines`/`removed_lines`, `similarity=1.0` (the dataclass default — not computed), and a note string. No unified diff.
- Similarity is **character-level** `SequenceMatcher` on the full text strings (not line-level).

### data_diff.py (`DataDiffResult`)
- `row_count_delta` and `identical_schema` are Python `@property` computed fields — they are **not** stored as dataclass fields and therefore do **not** appear in the JSON output (Python's `asdict()` skips properties).
- `type_changes` is a dict mapping column name → `(old_type, new_type)` tuple; serializes to JSON as `{"col": ["OLD", "NEW"]}`.
- Columns in `type_changes` come from iterating `sorted(a_cols & b_cols)`, so keys are in alphabetical order.

### crawler.py (`DiffCrawler.crawl`)
- Files in `in_both` are sorted before dispatch: `for rel in sorted(tdiff.in_both)`.
- Classification runs on both A and B; if they disagree, the file is treated as `"binary"`.
- `max_renames_reported` defaults to 200; rename list is truncated after scoring.
- Overall similarity formula: if `n_common == 0` AND there are files only on one side, return just `tree_jaccard`; otherwise `0.5 * tree_jaccard + 0.5 * avg_code_sim`.

### matcher.py (`find_renames`)
- Only `"code"`-classified files participate in rename detection; data and binary files are excluded.
- Max file size for MinHash: 5 MB. Files larger than this are skipped silently.
- LSH candidate threshold is **0.3** (for the index query); final report threshold is **0.4** (default `--rename-threshold`).
- If `datasketch` is not installed, returns an empty list (no error).
- Results are sorted descending by `combined_score`.

### report.py (`report_to_dict`)
- Python `set` fields (the `TreeDiff` sets) are converted to **sorted** string lists.
- `tree.in_both`, `tree.only_in_a`, `tree.only_in_b` are explicitly re-sorted after `asdict()` conversion.
- Markdown renderer shows at most 25 examples per section (`max_examples=25`).
- Markdown code diff list is sorted ascending by `similarity` (most-changed first).

---

## Goals (in priority order)

1. **Behavioral parity** with the Python reference for all five diff layers.
2. **Single self-contained binary** — no Python, no Rust toolchain, no system DuckDB required at runtime.
3. **10×+ faster** than the Python version on a realistic repo (target: a repo with ~5k tracked files diffs in under a few seconds on a laptop).
4. **Distributed via Docker** with a thin host-side wrapper script so the UX is `diff-crawler ./A ./B`.
5. **Same CLI surface and report shape** as the Python version, so existing scripts/CI that consume the JSON report keep working.

Non-goals (for now): row-level data diffing with primary keys, GUI, watch mode, network/remote directories, language server integration.

---

## The five layers (what the tool does)

These are what `reference/diff_crawler/` already implements. The Rust port replicates them.

1. **Tree diff** — recursive walk of both roots; compute `in_both`, `only_in_a`, `only_in_b` over relative paths. Exclusions: `venv/.venv/env`, `__pycache__`, `.git`, `node_modules`, `.ipynb_checkpoints`, `.pytest_cache`, `.mypy_cache`, `.ruff_cache`, `.tox`, `dist`, `build`, `*.egg-info`, `mlruns`, `wandb`, `lightning_logs`, `.idea`, `.vscode`. Skip suffixes: `.pyc .pyo .pyd .class .log .tmp .swp .swo`. Skip files: `.DS_Store`, `Thumbs.db`. Skip symlinks.

2. **Code diff** — for text/code files present in both: unified diff (configurable on/off), added/removed line counts, character-level similarity ratio (0–1). Fast path: if SHA-256 matches, mark identical and skip. Skip generating full diff if either side is >2 MB; still produce ratio + counts.

3. **Data diff (DuckDB)** — for `.csv .tsv .parquet .jsonl .ndjson` and large `.json` (>256 KB): row count both sides + delta, schema both sides, columns added/removed, type changes. Optional `--deep` flag adds per-column null counts and approximate distinct counts.

4. **Binary diff** — for `.pkl .pt .pth .ckpt .safetensors .h5 .hdf5 .onnx .pb .tflite .npy .npz .png .jpg .jpeg .gif .bmp .tiff .webp .mp3 .wav .flac .mp4 .mov .avi .zip .tar .gz .bz2 .7z .bin .dat`: SHA-256 hash compare only, plus sizes.

5. **Probabilistic rename detection** — over text/code files unique to one side: MinHash LSH with k-shingled tokens (k=5, num_perm=128, LSH threshold 0.3), combined with filename similarity (SequenceMatcher-equivalent). Combined score = `0.75 * content_jaccard + 0.25 * name_similarity`. Report threshold default 0.4.

Then aggregate into a summary: counts per category, tree Jaccard, average code similarity, total row delta, total added/removed lines, overall similarity score = `0.5 * tree_jaccard + 0.5 * avg_code_sim`.

---

## Recommended crate choices

These are starting recommendations. If you find something materially better while implementing, use it and note why in commit messages.

- **CLI parsing**: `clap` (derive API)
- **Directory walking**: `ignore` (the crate behind `ripgrep`; handles parallel walks and gitignore-style exclusions natively — though we're using our own exclusion list, not `.gitignore`)
- **Parallelism**: `rayon` (`par_iter` for per-file work)
- **Hashing**: `sha2` for SHA-256 parity with the Python version. Consider `blake3` later as an option behind a flag if speed matters more than parity.
- **Code diff**: `similar` crate (Myers/Patience diffs + ratio function, the Rust analog of `difflib`)
- **DuckDB**: `duckdb` crate (official). Statically link.
- **MinHash**: `probminhash` or roll a small one with `ahash`. Either is fine; pick whichever has cleaner LSH support.
- **JSON output**: `serde` + `serde_json`
- **Error handling**: `anyhow` at the binary boundary, `thiserror` for typed errors inside library code
- **Logging**: `tracing` (only emit at `--verbose`; default is silent except for the report)

---

## Suggested project layout

```
diff-crawler/
├── Cargo.toml
├── CLAUDE.md                  (this file)
├── README.md
├── Dockerfile
├── scripts/
│   └── diff-crawler           (host-side bash wrapper)
├── reference/
│   └── diff_crawler/          (Python reference — do not modify)
├── src/
│   ├── main.rs                (CLI entry, arg parsing, dispatch)
│   ├── lib.rs                 (re-exports)
│   ├── config.rs              (CrawlConfig, exclusion lists)
│   ├── classifier.rs          (code/data/binary/ignore decision)
│   ├── walker.rs              (tree walk + TreeIndex + TreeDiff)
│   ├── code_diff.rs           (text diff + binary hash compare)
│   ├── data_diff.rs           (DuckDB row + schema diff)
│   ├── matcher.rs             (MinHash LSH rename detection)
│   ├── crawler.rs             (orchestrator, parallel dispatch)
│   ├── report.rs              (CrawlReport struct, JSON + Markdown render)
│   └── error.rs
├── tests/
│   ├── fixtures/              (small paired sample projects)
│   ├── parity.rs              (compares Rust output to Python reference output)
│   ├── walker.rs
│   ├── code_diff.rs
│   ├── data_diff.rs
│   └── matcher.rs
└── .github/workflows/
    └── ci.yml                 (build, test, multi-arch Docker push, release binaries)
```

---

## CLI contract

Match the Python version. From the Python `cli.py`:

```
diff-crawler DIR_A DIR_B
    [--json PATH]              write full JSON report
    [--markdown PATH]          write Markdown summary
    [--no-diff]                skip full unified diffs, keep counts + similarity
    [--deep]                   per-column null counts + approx distinct
    [--rename-threshold FLOAT] default 0.4
    [-v / --verbose]           tracing output to stderr
```

If neither `--json` nor `--markdown` is given, print Markdown to stdout (same as Python).

Exit codes: `0` on success, `1` on usage error, `2` on I/O or runtime error. The Python version doesn't currently differentiate these; we should.

---

## Report shape (JSON)

Must match the Python `report.report_to_dict()` output structure exactly. Top-level keys:

```
{
  "dir_a": "...",
  "dir_b": "...",
  "tree": { "in_both": [...], "only_in_a": [...], "only_in_b": [...] },
  "code_diffs":   [ { path, identical, added_lines, removed_lines, similarity, unified_diff, note, error } ],
  "data_diffs":   [ { path, row_count_a, row_count_b, schema_a, schema_b, columns_added, columns_removed, type_changes, column_stats_a, column_stats_b, note, error } ],
  "binary_diffs": [ { path, identical, size_a, size_b, sha256_a, sha256_b } ],
  "rename_candidates": [ { from_path, to_path, content_similarity, name_similarity, combined_score } ],
  "summary": { tree, code, data, binary, renames_detected, overall_similarity }
}
```

The `parity.rs` test must deserialize both the Python and Rust JSON outputs for the same fixture pair and compare structurally (allow floating-point tolerance of 1e-6 on similarity scores; allow ordering differences in lists where ordering isn't semantically meaningful).

---

## Implementation phases

Work in this order. Each phase ends with a working binary that does *more*, not a half-broken one that does everything.

### Phase 1 — Scaffolding and walker
- `cargo init`, set up `Cargo.toml` with the deps above.
- Implement `config.rs` (exclusion lists, `CrawlConfig`).
- Implement `classifier.rs` — pure logic, easy to unit test.
- Implement `walker.rs` using the `ignore` crate. Output: `TreeIndex` and `TreeDiff`.
- CLI accepts two paths and prints tree diff as JSON.
- Tests: walker excludes the right things, set arithmetic correct.

### Phase 2 — Code diff and binary diff
- Implement `code_diff.rs` using `similar`.
- SHA-256 fast path before diffing.
- Binary diff path (hash + size only).
- Wire into orchestrator; parallelize per-file work with `rayon`.
- Tests: identical files marked identical without diffing; line counts correct; ratio in [0,1]; large file path takes the no-diff branch.

### Phase 3 — Data diff
- Add `duckdb` crate. Confirm static linking in the release build.
- Implement `data_diff.rs` — replicate the SQL from Python `data_diff.py`:
  - `read_csv_auto / read_parquet / read_json_auto` based on suffix
  - `DESCRIBE`, `COUNT(*)`
  - column adds/removes/type changes
  - `--deep`: `SUM(CASE WHEN col IS NULL THEN 1 ELSE 0 END)` + `APPROX_COUNT_DISTINCT`
- One DuckDB connection per file, processed in parallel via `rayon`. Verify this is safe with the `duckdb` crate (it should be — separate connections are independent).
- Tests: row counts match Python; schema diff matches; `--deep` stats match.

### Phase 4 — Rename detection
- Implement `matcher.rs`: shingle, MinHash, LSH index over B-side, query for each A-side file, blend with filename similarity (use `similar`'s ratio for filenames too — keeps one diff library).
- Tests: known rename pair is detected; coincidental matches below threshold are suppressed; respects `--rename-threshold`.

### Phase 5 — Report rendering and CLI polish
- Implement `report.rs`: `CrawlReport` serializes to the JSON shape above via serde; separate Markdown renderer (porting `render_markdown` from Python `report.py` line by line).
- `--json`, `--markdown`, stdout default all wired.
- Exit codes per the CLI contract.

### Phase 6 — Parity test against Python
- Build a few fixture pairs (small ML-project-shaped directories) under `tests/fixtures/`. Include: identical files, modified files, only-in-A, only-in-B, a rename, a CSV with extra column and more rows, a binary file change.
- `tests/parity.rs`: shells out to the Python reference (assume `python -m diff_crawler` is available *only in the test environment*, e.g. CI sets it up) to produce a reference JSON, then runs the Rust binary on the same fixtures, then diffs the JSONs structurally with float tolerance.
- This test gates merges to main.

### Phase 7 — Docker and distribution
- Write the `Dockerfile` (multi-stage; see below).
- Write `scripts/diff-crawler` host wrapper (mounts inputs read-only, mounts `$PWD` for outputs, runs with `--user $(id -u):$(id -g)`).
- GitHub Actions workflow:
  - Build and test on every PR
  - On tagged release: build multi-arch Docker image (amd64 + arm64) and push to `ghcr.io`
  - On tagged release: build native binaries for Linux x86_64, macOS arm64, macOS x86_64 via `cargo-dist` or hand-rolled; upload to the GitHub Release.

---

## Dockerfile shape

Multi-stage. The runtime stage contains only the binary and a `debian:bookworm-slim` base (default) — no Rust toolchain.

```dockerfile
FROM rust:1.XX AS builder
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/diff-crawler /usr/local/bin/diff-crawler
ENTRYPOINT ["/usr/local/bin/diff-crawler"]
```

DuckDB statically links by default with the `duckdb` crate's `bundled` feature — enable it in `Cargo.toml` so we don't depend on a system `libduckdb`.

Target image size: under 150 MB. If we end up larger, investigate before merging the Dockerfile.

---

## Host wrapper script (`scripts/diff-crawler`)

Goal: user types `diff-crawler ./projA ./projB` and never sees a `docker run` flag. The wrapper:

- Resolves both input paths to absolute paths
- Bind-mounts each as read-only into `/in/a` and `/in/b`
- Bind-mounts `$PWD` as `/out` for any `--json` / `--markdown` outputs
- Passes through `--user $(id -u):$(id -g)` so output files are owned by the user
- Translates host paths in user-supplied `--json` / `--markdown` arguments to container paths

Keep it minimal — bash, ~20–30 lines, POSIX-compatible. Ship it in `scripts/` and document in the README how to install it (copy to `/usr/local/bin/` and `chmod +x`).

---

## Performance targets and how to measure

Build a benchmark fixture: a synthetic repo with ~5,000 files (mix of `.py`, `.csv`, `.parquet`, `.pkl`, a few `node_modules`-shaped excluded directories). Generate it with a script under `tests/bench/`.

Targets on a modern laptop, release build:

- Tree walk + classification: under 200 ms
- Full crawl with code+data diff but no rename detection: under 3 s
- Full crawl with rename detection enabled: under 8 s
- Memory: under 500 MB resident on the above fixture

Use `criterion` for micro-benchmarks of the diff and MinHash inner loops. For end-to-end, time the binary directly.

If any target is missed by more than 2×, profile (`cargo flamegraph`) before adding complexity.

---

## Coding conventions

- **No `unwrap()` or `expect()` in non-test code** except for "this literally cannot fail" cases with a comment explaining why. Use `?` and `anyhow::Result` at the binary boundary, typed errors inside library code.
- **No `unsafe`** without a comment explaining why it's needed and what invariant it upholds. Default to "don't."
- Public structs that cross module boundaries derive `Debug`, `Clone` where cheap, and `Serialize` where they appear in the report.
- Format with `cargo fmt`, lint with `cargo clippy -- -D warnings`. Both run in CI.
- Test naming: `tests/<module>.rs` for integration tests; unit tests inline in `#[cfg(test)] mod tests` blocks.
- Comments explain *why*, not *what*. Don't narrate obvious code.

---

## What to ask vs. decide yourself

Decide yourself, and note in the commit message:
- Choice between equivalent crates (e.g. `probminhash` vs. hand-rolled MinHash)
- Naming of internal modules and types
- How to structure tests
- Specific SQL phrasing, as long as the semantic result matches Python
- Performance tweaks that don't change behavior

Ask before doing:
- Anything that changes the JSON report shape (breaks parity)
- Anything that changes the CLI surface
- Adding a heavy dependency (>500 KB compiled, or with a complex transitive tree)
- Diverging from the five-layer model — adding a sixth layer, removing one, etc.
- Anything that requires changes to the Python reference

---

## Definition of done

- All five layers implemented and passing unit tests.
- `tests/parity.rs` passes against the Python reference on all fixtures.
- `cargo clippy -- -D warnings` clean.
- Dockerfile builds, resulting image under 150 MB, runs the test fixtures end-to-end and produces a report matching the Python output.
- Host wrapper script works on Linux and macOS.
- GitHub Actions CI builds, tests, and on tag, publishes the Docker image and release binaries.
- README has install instructions for both Docker and pre-built binaries, plus a usage section.
- A user with neither Python nor Rust installed can run the tool against two of their directories and get a correct report.
