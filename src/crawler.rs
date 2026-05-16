use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rayon::prelude::*;
use serde::Serialize;

use crate::classifier::{classify, FileKind};
use crate::code_diff::{diff_binary, diff_code, BinaryDiffResult, CodeDiffResult};
use crate::config::CrawlConfig;
use crate::data_diff::{diff_data, DataDiffResult};
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
    fn from_diff(diff: &TreeDiff) -> Self {
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

// ── report ────────────────────────────────────────────────────────────────────

/// Crawl output. Top-level keys match the Python `report_to_dict()` structure.
/// `rename_candidates` is added in Phase 4.
#[derive(Debug, Serialize)]
pub struct CrawlReport {
    pub dir_a: String,
    pub dir_b: String,
    pub tree: TreeReportData,
    pub code_diffs: Vec<CodeDiffResult>,
    pub data_diffs: Vec<DataDiffResult>,
    pub binary_diffs: Vec<BinaryDiffResult>,
}

// ── internal dispatch enum ────────────────────────────────────────────────────

enum Dispatched {
    Code(CodeDiffResult),
    Binary(BinaryDiffResult),
    Data(DataDiffResult),
}

// ── public API ────────────────────────────────────────────────────────────────

/// Walk both roots, classify every common file, diff in parallel with rayon,
/// and return the assembled report.
///
/// Mirrors the `DiffCrawler.crawl()` method in reference/diff_crawler/crawler.py.
pub fn crawl(dir_a: &Path, dir_b: &Path, config: &CrawlConfig) -> Result<CrawlReport> {
    let idx_a = walk_tree(dir_a).with_context(|| format!("walking {}", dir_a.display()))?;
    let idx_b = walk_tree(dir_b).with_context(|| format!("walking {}", dir_b.display()))?;
    let diff = diff_trees(&idx_a, &idx_b);

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
                FileKind::Binary => {
                    Dispatched::Binary(diff_binary(rel, abs_a, abs_b))
                }
                FileKind::Data => {
                    Dispatched::Data(diff_data(rel, abs_a, abs_b, deep))
                }
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

    Ok(CrawlReport {
        dir_a: idx_a.root.to_string_lossy().replace('\\', "/"),
        dir_b: idx_b.root.to_string_lossy().replace('\\', "/"),
        tree: TreeReportData::from_diff(&diff),
        code_diffs,
        data_diffs,
        binary_diffs,
    })
}
