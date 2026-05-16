# diff_crawler

Compare two ML project directories across multiple layers:

1. **Tree diff** — directory-structure set diff (in both / only-A / only-B), with sensible exclusions (`venv`, `__pycache__`, `mlruns`, `wandb`, model checkpoints, etc.).
2. **Code diff** — unified diff + line counts + character-level similarity for text/code files via `difflib`.
3. **Data diff** — row counts + schema comparison via DuckDB for `.csv`, `.tsv`, `.parquet`, `.jsonl`, large `.json`. Optional per-column null/distinct stats with `--deep`.
4. **Binary/model diff** — SHA-256 hash compare for `.pkl`, `.pt`, `.h5`, `.onnx`, images, etc.
5. **Rename detection** — MinHash LSH (`datasketch`) over shingled tokens of the files unique to each side, blended with filename similarity, to surface likely renames/moves.
6. **Aggregate similarity** — overall % files in common, average code similarity, total row delta, an overall score.

## Install

```bash
pip install -r requirements.txt
```

## Use as a CLI

```bash
# Markdown summary to stdout
python -m diff_crawler /path/to/A /path/to/B

# Both outputs to disk
python -m diff_crawler /path/to/A /path/to/B \
    --json report.json --markdown report.md

# Skip the big unified diffs (still get counts + similarity)
python -m diff_crawler A B --no-diff

# Deeper per-column data stats (slower)
python -m diff_crawler A B --deep
```

## Use as a library

```python
from diff_crawler import DiffCrawler, CrawlConfig, render_markdown
from diff_crawler.report import report_to_dict
import json

report = DiffCrawler(CrawlConfig(deep_data_stats=True)).crawl("A", "B")

print(render_markdown(report))
json.dump(report_to_dict(report), open("report.json", "w"), indent=2)

for c in report.rename_candidates:
    print(c.from_path, "→", c.to_path, c.combined_score)
```

## Layout

```
diff_crawler/
├── __init__.py        public API
├── __main__.py        python -m diff_crawler
├── cli.py             argparse entry point
├── crawler.py         orchestrator (DiffCrawler)
├── tree_diff.py       walk + set diff
├── classifier.py      code / data / binary / ignore
├── code_diff.py       difflib-based code diff
├── data_diff.py       DuckDB row + schema diff
├── matcher.py         MinHash LSH rename detection
└── report.py          JSON + Markdown rendering
```

## Notes / things to tune

- Edit `DEFAULT_EXCLUDED_DIRS` / `DEFAULT_EXCLUDED_SUFFIXES` in `classifier.py` to change what's skipped.
- Rename detection only runs over **text/code** files unique to one side. For data files only on one side we currently just list them; if you need probabilistic matching for those too, the easy extension is to MinHash the column names + first N row hashes.
- DuckDB will refuse some `.xlsx`/`.arrow` files unless the appropriate extension is loaded; those will show up with an `error` field in the report rather than crashing the crawl.
- `--no-diff` keeps the report small for huge codebases; you still get per-file similarity, added/removed counts, and all aggregates.
