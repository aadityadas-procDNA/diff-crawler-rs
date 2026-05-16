"""Orchestrator: walks both trees, dispatches each common file to the right
differ, runs probabilistic rename detection on the leftovers, and produces a
single CrawlReport.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path

from .classifier import classify
from .tree_diff import walk_tree, diff_trees, TreeDiff
from .code_diff import diff_code, diff_binary, CodeDiffResult, BinaryDiffResult
from .data_diff import diff_data, DataDiffResult
from .matcher import find_renames, RenameCandidate


@dataclass
class CrawlConfig:
    include_full_unified_diff: bool = True
    deep_data_stats: bool = False         # per-column nulls + approx-distinct
    rename_threshold: float = 0.4
    max_renames_reported: int = 200


@dataclass
class CrawlReport:
    dir_a: str
    dir_b: str
    tree: TreeDiff = None  # type: ignore
    code_diffs: list[CodeDiffResult] = field(default_factory=list)
    data_diffs: list[DataDiffResult] = field(default_factory=list)
    binary_diffs: list[BinaryDiffResult] = field(default_factory=list)
    rename_candidates: list[RenameCandidate] = field(default_factory=list)
    summary: dict = field(default_factory=dict)


class DiffCrawler:
    def __init__(self, config: CrawlConfig | None = None):
        self.cfg = config or CrawlConfig()

    def crawl(self, dir_a: Path | str, dir_b: Path | str) -> CrawlReport:
        a_root = Path(dir_a)
        b_root = Path(dir_b)

        idx_a = walk_tree(a_root)
        idx_b = walk_tree(b_root)
        tdiff = diff_trees(idx_a, idx_b)

        report = CrawlReport(dir_a=str(a_root.resolve()),
                             dir_b=str(b_root.resolve()),
                             tree=tdiff)

        # --- common files: dispatch by classification ----------------------
        for rel in sorted(tdiff.in_both):
            abs_a = idx_a.abs_lookup[rel]
            abs_b = idx_b.abs_lookup[rel]
            # Classify on A side; if it disagrees with B we fall back to binary.
            cls_a = classify(abs_a)
            cls_b = classify(abs_b)
            kind = cls_a.kind if cls_a.kind == cls_b.kind else "binary"

            if kind == "code":
                report.code_diffs.append(
                    diff_code(rel, abs_a, abs_b,
                             include_full_diff=self.cfg.include_full_unified_diff))
            elif kind == "data":
                report.data_diffs.append(
                    diff_data(rel, abs_a, abs_b, deep=self.cfg.deep_data_stats))
            elif kind == "binary":
                report.binary_diffs.append(diff_binary(rel, abs_a, abs_b))
            # "ignore" should never reach here since walk_tree filters.

        # --- only-in-A / only-in-B: probabilistic rename detection ---------
        only_a = {rel: idx_a.abs_lookup[rel] for rel in tdiff.only_in_a}
        only_b = {rel: idx_b.abs_lookup[rel] for rel in tdiff.only_in_b}
        renames = find_renames(only_a, only_b, threshold=self.cfg.rename_threshold)
        report.rename_candidates = renames[: self.cfg.max_renames_reported]

        # --- aggregate similarity summary ---------------------------------
        report.summary = self._summarize(report)
        return report

    @staticmethod
    def _summarize(r: CrawlReport) -> dict:
        n_code = len(r.code_diffs)
        n_data = len(r.data_diffs)
        n_bin = len(r.binary_diffs)
        n_total_common = n_code + n_data + n_bin

        avg_code_sim = (
            sum(c.similarity for c in r.code_diffs) / n_code if n_code else 1.0
        )
        identical_code = sum(1 for c in r.code_diffs if c.identical)
        identical_bin = sum(1 for b in r.binary_diffs if b.identical)
        schema_changed = sum(1 for d in r.data_diffs if not d.identical_schema)

        return {
            "tree": {
                "files_in_both": len(r.tree.in_both),
                "only_in_a": len(r.tree.only_in_a),
                "only_in_b": len(r.tree.only_in_b),
                "tree_jaccard": round(r.tree.jaccard, 4),
            },
            "code": {
                "compared": n_code,
                "identical": identical_code,
                "avg_similarity": round(avg_code_sim, 4),
                "total_added_lines": sum(c.added_lines for c in r.code_diffs),
                "total_removed_lines": sum(c.removed_lines for c in r.code_diffs),
            },
            "data": {
                "compared": n_data,
                "schema_changed": schema_changed,
                "total_row_delta": sum(d.row_count_delta for d in r.data_diffs),
            },
            "binary": {
                "compared": n_bin,
                "identical": identical_bin,
            },
            "renames_detected": len(r.rename_candidates),
            "overall_similarity": _overall_similarity(r, avg_code_sim, n_total_common),
        }


def _overall_similarity(r: CrawlReport, avg_code_sim: float, n_common: int) -> float:
    """Crude overall similarity = weighted combo of tree Jaccard and code sim."""
    if n_common == 0 and (r.tree.only_in_a or r.tree.only_in_b):
        return round(r.tree.jaccard, 4)
    return round(0.5 * r.tree.jaccard + 0.5 * avg_code_sim, 4)
