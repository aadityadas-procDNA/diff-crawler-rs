use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rayon::prelude::*;
use serde::Serialize;

use crate::classifier::{classify, FileKind};
use crate::code_diff::{diff_binary, diff_code, BinaryDiffResult, CodeDiffResult};
use crate::config::CrawlConfig;
use crate::data_diff::{diff_data, DataDiffResult};
use crate::matcher::{find_renames, RenameCandidate};
use crate::walker::{diff_trees, walk_tree, TreeDiff};

// ── serialisable tree wrapper ─────────────────────────────────────────────────

/// `TreeDiff` uses `HashSet<PathBuf>` internally; for JSON we need sorted
/// string vecs (mirrors Python's `report_to_dict` which sorts set fields).
#[derive(Debug, Serialize)]
pub struct TreeReportData {
    pub in_both: Vec<String>,
    pub only_in_a: Vec<String>,
    pub only_in_b: Vec<String>,
}

impl TreeReportData {
    pub fn from_diff(diff: &TreeDiff) -> Self {
        Self {
            in_both: sorted_paths(&diff.in_both),
            only_in_a: sorted_paths(&diff.only_in_a),
            only_in_b: sorted_paths(&diff.only_in_b),
        }
    }
}

fn sorted_paths(set: &HashSet<PathBuf>) -> Vec<String> {
    let mut v: Vec<String> = set
        .iter()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();
    v.sort();
    v
}

// ── summary types ─────────────────────────────────────────────────────────────
//
// Mirrors the dict produced by Python's `DiffCrawler._summarize()`.
// Each sub-struct matches one top-level key so the JSON shape is identical.

#[derive(Debug, Clone, Serialize)]
pub struct TreeSummary {
    pub files_in_both: usize,
    pub only_in_a: usize,
    pub only_in_b: usize,
    /// Jaccard index over the file-path union, rounded to 4 decimal places.
    pub tree_jaccard: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CodeSummary {
    pub compared: usize,
    pub identical: usize,
    /// Average character-level similarity across all compared code files,
    /// rounded to 4 decimal places. Defaults to 1.0 when no code files exist.
    pub avg_similarity: f64,
    pub total_added_lines: u64,
    pub total_removed_lines: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DataSummary {
    pub compared: usize,
    pub schema_changed: usize,
    /// Net row delta = sum of (row_count_b − row_count_a) across all data diffs.
    pub total_row_delta: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct BinarySummary {
    pub compared: usize,
    pub identical: usize,
}

/// Aggregate statistics. Top-level `summary` key in the JSON report.
#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub tree: TreeSummary,
    pub code: CodeSummary,
    pub data: DataSummary,
    pub binary: BinarySummary,
    pub renames_detected: usize,
    /// Overall similarity = 0.5 × tree_jaccard + 0.5 × avg_code_sim,
    /// rounded to 4 decimal places. Special case: if no common files exist
    /// but files exist only on one side, returns tree_jaccard alone.
    pub overall_similarity: f64,
}

// ── report ────────────────────────────────────────────────────────────────────

/// Full crawl output. Top-level keys match the Python `report_to_dict()` structure.
#[derive(Debug, Serialize)]
pub struct CrawlReport {
    pub dir_a: String,
    pub dir_b: String,
    pub tree: TreeReportData,
    pub code_diffs: Vec<CodeDiffResult>,
    pub data_diffs: Vec<DataDiffResult>,
    pub binary_diffs: Vec<BinaryDiffResult>,
    pub rename_candidates: Vec<RenameCandidate>,
    pub summary: Summary,
}

// ── internal dispatch enum ────────────────────────────────────────────────────

enum Dispatched {
    Code(CodeDiffResult),
    Binary(BinaryDiffResult),
    Data(DataDiffResult),
}

// ── summary builder ───────────────────────────────────────────────────────────

/// Round to 4 decimal places — mirrors Python's `round(x, 4)`.
fn round4(x: f64) -> f64 {
    (x * 10_000.0).round() / 10_000.0
}

/// Compute the aggregate summary from the crawl components.
///
/// Mirrors `DiffCrawler._summarize()` in reference/diff_crawler/crawler.py.
/// Called with the raw components before `CrawlReport` is assembled so we
/// avoid a chicken-and-egg construction problem.
#[allow(clippy::too_many_arguments)]
pub fn build_summary(
    in_both: usize,
    only_a: usize,
    only_b: usize,
    tree_jaccard: f64,
    code_diffs: &[CodeDiffResult],
    data_diffs: &[DataDiffResult],
    binary_diffs: &[BinaryDiffResult],
    rename_candidates: &[RenameCandidate],
) -> Summary {
    let n_code = code_diffs.len();
    let n_data = data_diffs.len();
    let n_bin = binary_diffs.len();
    let n_common = n_code + n_data + n_bin;

    // avg_code_sim: 1.0 when there are no code files, matching Python's default.
    let avg_code_sim = if n_code == 0 {
        1.0
    } else {
        code_diffs.iter().map(|c| c.similarity).sum::<f64>() / n_code as f64
    };

    // overall_similarity formula from Python's `_overall_similarity()`.
    // If no common files but files exist only on one side, use tree_jaccard alone.
    let overall_similarity = if n_common == 0 && (only_a > 0 || only_b > 0) {
        round4(tree_jaccard)
    } else {
        round4(0.5 * tree_jaccard + 0.5 * avg_code_sim)
    };

    Summary {
        tree: TreeSummary {
            files_in_both: in_both,
            only_in_a: only_a,
            only_in_b: only_b,
            tree_jaccard: round4(tree_jaccard),
        },
        code: CodeSummary {
            compared: n_code,
            identical: code_diffs.iter().filter(|c| c.identical).count(),
            avg_similarity: round4(avg_code_sim),
            total_added_lines: code_diffs.iter().map(|c| c.added_lines as u64).sum(),
            total_removed_lines: code_diffs.iter().map(|c| c.removed_lines as u64).sum(),
        },
        data: DataSummary {
            compared: n_data,
            // schema_changed mirrors Python's `not d.identical_schema` check:
            // identical_schema = (not columns_added and not columns_removed and not type_changes)
            schema_changed: data_diffs
                .iter()
                .filter(|d| {
                    !d.columns_added.is_empty()
                        || !d.columns_removed.is_empty()
                        || !d.type_changes.is_empty()
                })
                .count(),
            // row_count_delta mirrors Python's @property: row_count_b - row_count_a
            total_row_delta: data_diffs
                .iter()
                .map(|d| d.row_count_b - d.row_count_a)
                .sum(),
        },
        binary: BinarySummary {
            compared: n_bin,
            identical: binary_diffs.iter().filter(|b| b.identical).count(),
        },
        renames_detected: rename_candidates.len(),
        overall_similarity,
    }
}

// ── public API ────────────────────────────────────────────────────────────────

/// Walk both roots, classify every common file, diff in parallel with rayon,
/// compute the aggregate summary, and return the assembled report.
///
/// Mirrors the `DiffCrawler.crawl()` method in reference/diff_crawler/crawler.py.
pub fn crawl(dir_a: &Path, dir_b: &Path, config: &CrawlConfig) -> Result<CrawlReport> {
    let idx_a = walk_tree(dir_a).with_context(|| format!("walking {}", dir_a.display()))?;
    let idx_b = walk_tree(dir_b).with_context(|| format!("walking {}", dir_b.display()))?;
    let diff = diff_trees(&idx_a, &idx_b);

    // Capture counts and jaccard before diff is borrowed for rename detection.
    let in_both_count = diff.in_both.len();
    let only_a_count = diff.only_in_a.len();
    let only_b_count = diff.only_in_b.len();
    let tree_jaccard = diff.jaccard();

    // Sort so output order is deterministic, mirroring Python's
    // `for rel in sorted(tdiff.in_both)`.
    let mut common: Vec<PathBuf> = diff.in_both.iter().cloned().collect();
    common.sort();

    let include_diff = config.include_full_unified_diff;
    let deep = config.deep_data_stats;

    // Parallel dispatch — each file is independent so rayon gives us easy speedup.
    let dispatched: Vec<Dispatched> = common
        .par_iter()
        .map(|rel| {
            let abs_a = &idx_a.abs_lookup[rel];
            let abs_b = &idx_b.abs_lookup[rel];

            let cls_a = classify(abs_a);
            let cls_b = classify(abs_b);
            // If both sides classify differently, fall back to binary
            // (mirrors Python: `kind = cls_a.kind if cls_a.kind == cls_b.kind else "binary"`).
            let kind =
                if cls_a.kind == cls_b.kind { cls_a.kind } else { FileKind::Binary };

            match kind {
                FileKind::Code => {
                    Dispatched::Code(diff_code(rel, abs_a, abs_b, include_diff))
                }
                FileKind::Binary => Dispatched::Binary(diff_binary(rel, abs_a, abs_b)),
                FileKind::Data => Dispatched::Data(diff_data(rel, abs_a, abs_b, deep)),
                // Ignored files are filtered out by the walker and never in `in_both`.
                FileKind::Ignore => unreachable!(),
            }
        })
        .collect();

    let mut code_diffs: Vec<CodeDiffResult> = Vec::new();
    let mut data_diffs: Vec<DataDiffResult> = Vec::new();
    let mut binary_diffs: Vec<BinaryDiffResult> = Vec::new();

    for d in dispatched {
        match d {
            Dispatched::Code(r) => code_diffs.push(r),
            Dispatched::Binary(r) => binary_diffs.push(r),
            Dispatched::Data(r) => data_diffs.push(r),
        }
    }

    // Rename detection: only code files unique to each side participate.
    let rename_candidates = find_renames(
        &diff.only_in_a,
        &diff.only_in_b,
        &idx_a.abs_lookup,
        &idx_b.abs_lookup,
        config.rename_threshold,
    );

    // Build aggregate summary from all components before constructing the report.
    let summary = build_summary(
        in_both_count,
        only_a_count,
        only_b_count,
        tree_jaccard,
        &code_diffs,
        &data_diffs,
        &binary_diffs,
        &rename_candidates,
    );

    Ok(CrawlReport {
        dir_a: idx_a.root.to_string_lossy().replace('\\', "/"),
        dir_b: idx_b.root.to_string_lossy().replace('\\', "/"),
        tree: TreeReportData::from_diff(&diff),
        code_diffs,
        data_diffs,
        binary_diffs,
        rename_candidates,
        summary,
    })
}
