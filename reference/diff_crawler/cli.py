"""Command-line interface.

Usage:
  python -m diff_crawler PATH_A PATH_B [--json out.json] [--markdown out.md]
                                       [--no-diff] [--deep]
                                       [--rename-threshold 0.4]
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from .crawler import DiffCrawler, CrawlConfig
from .report import report_to_dict, render_markdown


def _build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(prog="diff_crawler",
                                description="Diff two ML project directories.")
    p.add_argument("dir_a", type=Path)
    p.add_argument("dir_b", type=Path)
    p.add_argument("--json", type=Path, default=None,
                   help="Write full JSON report to this path.")
    p.add_argument("--markdown", type=Path, default=None,
                   help="Write Markdown summary to this path.")
    p.add_argument("--no-diff", action="store_true",
                   help="Skip full unified diffs (keep counts + similarity).")
    p.add_argument("--deep", action="store_true",
                   help="Run deeper data stats (null counts, approx distinct).")
    p.add_argument("--rename-threshold", type=float, default=0.4,
                   help="Combined-score threshold for rename detection.")
    return p


def main(argv: list[str] | None = None) -> int:
    args = _build_parser().parse_args(argv)
    cfg = CrawlConfig(
        include_full_unified_diff=not args.no_diff,
        deep_data_stats=args.deep,
        rename_threshold=args.rename_threshold,
    )
    crawler = DiffCrawler(cfg)
    report = crawler.crawl(args.dir_a, args.dir_b)

    if args.json:
        args.json.write_text(json.dumps(report_to_dict(report), indent=2))
        print(f"Wrote JSON: {args.json}", file=sys.stderr)
    if args.markdown:
        args.markdown.write_text(render_markdown(report))
        print(f"Wrote Markdown: {args.markdown}", file=sys.stderr)
    if not args.json and not args.markdown:
        # Default: print Markdown to stdout
        print(render_markdown(report))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
