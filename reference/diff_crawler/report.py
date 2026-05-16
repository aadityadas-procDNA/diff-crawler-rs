"""Render a CrawlReport to JSON (a plain dict) and to readable Markdown."""

from __future__ import annotations

from dataclasses import asdict, is_dataclass
from pathlib import Path
from typing import Any


def report_to_dict(report) -> dict[str, Any]:
    """Convert dataclasses + sets + Paths into a JSON-serializable dict."""
    def _convert(x):
        if is_dataclass(x):
            return {k: _convert(v) for k, v in asdict(x).items()}
        if isinstance(x, dict):
            return {str(k): _convert(v) for k, v in x.items()}
        if isinstance(x, (list, tuple)):
            return [_convert(v) for v in x]
        if isinstance(x, set):
            return sorted(_convert(v) for v in x)
        if isinstance(x, Path):
            return str(x)
        return x

    d = _convert(report)
    # tree's sets need stable ordering and string-ification
    if "tree" in d and d["tree"]:
        for k in ("in_both", "only_in_a", "only_in_b"):
            d["tree"][k] = sorted(d["tree"].get(k, []))
    return d


def render_markdown(report, max_examples: int = 25) -> str:
    """Render a Markdown summary suitable for a PR comment or README."""
    s = report.summary
    lines: list[str] = []
    lines.append(f"# Diff Report")
    lines.append(f"- **A**: `{report.dir_a}`")
    lines.append(f"- **B**: `{report.dir_b}`")
    lines.append("")

    lines.append("## Tree summary")
    t = s["tree"]
    lines.append(f"- Files in both: **{t['files_in_both']}**")
    lines.append(f"- Only in A: **{t['only_in_a']}**")
    lines.append(f"- Only in B: **{t['only_in_b']}**")
    lines.append(f"- Tree Jaccard: **{t['tree_jaccard']}**")
    lines.append("")

    lines.append("## Code")
    c = s["code"]
    lines.append(f"- Compared: {c['compared']}  |  identical: {c['identical']}  "
                 f"|  avg similarity: **{c['avg_similarity']}**")
    lines.append(f"- Added lines: {c['total_added_lines']}  |  "
                 f"Removed lines: {c['total_removed_lines']}")
    changed = [d for d in report.code_diffs if not d.identical]
    changed.sort(key=lambda d: d.similarity)
    if changed:
        lines.append("")
        lines.append("Most changed code files (lowest similarity first):")
        for d in changed[:max_examples]:
            lines.append(f"  - `{d.path}` — sim {d.similarity:.3f}, "
                         f"+{d.added_lines} / -{d.removed_lines}")
    lines.append("")

    lines.append("## Data")
    dsum = s["data"]
    lines.append(f"- Compared: {dsum['compared']}  |  "
                 f"schema changed: {dsum['schema_changed']}  |  "
                 f"net row delta: {dsum['total_row_delta']:+d}")
    for d in report.data_diffs:
        if d.error:
            lines.append(f"  - `{d.path}` — ERROR: {d.error}")
            continue
        if d.identical_schema and d.row_count_delta == 0:
            continue
        bits = [f"rows {d.row_count_a} -> {d.row_count_b} ({d.row_count_delta:+d})"]
        if d.columns_added:
            bits.append(f"+cols {d.columns_added}")
        if d.columns_removed:
            bits.append(f"-cols {d.columns_removed}")
        if d.type_changes:
            bits.append(f"type changes: {d.type_changes}")
        lines.append(f"  - `{d.path}` — " + "; ".join(bits))
    lines.append("")

    lines.append("## Binary / model files")
    b = s["binary"]
    lines.append(f"- Compared: {b['compared']}  |  identical: {b['identical']}")
    changed_bin = [x for x in report.binary_diffs if not x.identical]
    for x in changed_bin[:max_examples]:
        lines.append(f"  - `{x.path}` — size {x.size_a} -> {x.size_b}")
    lines.append("")

    lines.append("## Likely renames / moves (probabilistic)")
    if not report.rename_candidates:
        lines.append("_None detected above threshold._")
    else:
        for r in report.rename_candidates[:max_examples]:
            lines.append(f"  - `{r.from_path}`  →  `{r.to_path}`  "
                         f"(content {r.content_similarity:.2f}, "
                         f"name {r.name_similarity:.2f}, "
                         f"combined **{r.combined_score:.2f}**)")
    lines.append("")

    lines.append("## Overall")
    lines.append(f"- Renames detected: {s['renames_detected']}")
    lines.append(f"- Overall similarity score: **{s['overall_similarity']}**")
    return "\n".join(lines)
