/// Markdown rendering for `CrawlReport`.
///
/// Mirrors `render_markdown()` in reference/diff_crawler/report.py.
/// The Python function operates on live dataclass objects (with `@property`
/// helpers like `identical_schema` and `row_count_delta`); we compute those
/// inline here since they are not stored as struct fields.
use crate::crawler::CrawlReport;

const MAX_EXAMPLES: usize = 25;

/// Render `report` as a Markdown string.
///
/// Output matches the Python `render_markdown(report, max_examples=25)`.
pub fn render_markdown(report: &CrawlReport) -> String {
    let s = &report.summary;
    let mut lines: Vec<String> = Vec::new();

    lines.push("# Diff Report".into());
    lines.push(format!("- **A**: `{}`", report.dir_a));
    lines.push(format!("- **B**: `{}`", report.dir_b));
    lines.push(String::new());

    // ── Tree ─────────────────────────────────────────────────────────────────
    lines.push("## Tree summary".into());
    lines.push(format!("- Files in both: **{}**", s.tree.files_in_both));
    lines.push(format!("- Only in A: **{}**", s.tree.only_in_a));
    lines.push(format!("- Only in B: **{}**", s.tree.only_in_b));
    lines.push(format!("- Tree Jaccard: **{}**", s.tree.tree_jaccard));
    lines.push(String::new());

    // ── Code ──────────────────────────────────────────────────────────────────
    lines.push("## Code".into());
    lines.push(format!(
        "- Compared: {}  |  identical: {}  |  avg similarity: **{}**",
        s.code.compared, s.code.identical, s.code.avg_similarity,
    ));
    lines.push(format!(
        "- Added lines: {}  |  Removed lines: {}",
        s.code.total_added_lines, s.code.total_removed_lines,
    ));

    // Most changed: sort ascending by similarity (most-changed first), show up to max_examples.
    let mut changed_code: Vec<_> =
        report.code_diffs.iter().filter(|d| !d.identical).collect();
    changed_code.sort_by(|a, b| a.similarity.partial_cmp(&b.similarity).unwrap_or(std::cmp::Ordering::Equal));
    if !changed_code.is_empty() {
        lines.push(String::new());
        lines.push("Most changed code files (lowest similarity first):".into());
        for d in changed_code.iter().take(MAX_EXAMPLES) {
            lines.push(format!(
                "  - `{}` — sim {:.3}, +{} / -{}",
                d.path, d.similarity, d.added_lines, d.removed_lines,
            ));
        }
    }
    lines.push(String::new());

    // ── Data ──────────────────────────────────────────────────────────────────
    lines.push("## Data".into());
    lines.push(format!(
        "- Compared: {}  |  schema changed: {}  |  net row delta: {:+}",
        s.data.compared, s.data.schema_changed, s.data.total_row_delta,
    ));
    for d in &report.data_diffs {
        if let Some(err) = &d.error {
            lines.push(format!("  - `{}` — ERROR: {}", d.path, err));
            continue;
        }
        // identical_schema and row_count_delta are @property in Python — compute inline.
        let identical_schema =
            d.columns_added.is_empty() && d.columns_removed.is_empty() && d.type_changes.is_empty();
        let row_count_delta = d.row_count_b - d.row_count_a;
        if identical_schema && row_count_delta == 0 {
            continue;
        }
        let mut bits: Vec<String> = Vec::new();
        bits.push(format!(
            "rows {} -> {} ({:+})",
            d.row_count_a, d.row_count_b, row_count_delta,
        ));
        if !d.columns_added.is_empty() {
            bits.push(format!("+cols {:?}", d.columns_added));
        }
        if !d.columns_removed.is_empty() {
            bits.push(format!("-cols {:?}", d.columns_removed));
        }
        if !d.type_changes.is_empty() {
            bits.push(format!("type changes: {:?}", d.type_changes));
        }
        lines.push(format!("  - `{}` — {}", d.path, bits.join("; ")));
    }
    lines.push(String::new());

    // ── Binary ────────────────────────────────────────────────────────────────
    lines.push("## Binary / model files".into());
    lines.push(format!(
        "- Compared: {}  |  identical: {}",
        s.binary.compared, s.binary.identical,
    ));
    let changed_bin: Vec<_> = report.binary_diffs.iter().filter(|b| !b.identical).collect();
    for x in changed_bin.iter().take(MAX_EXAMPLES) {
        lines.push(format!("  - `{}` — size {} -> {}", x.path, x.size_a, x.size_b));
    }
    lines.push(String::new());

    // ── Renames ───────────────────────────────────────────────────────────────
    lines.push("## Likely renames / moves (probabilistic)".into());
    if report.rename_candidates.is_empty() {
        lines.push("_None detected above threshold._".into());
    } else {
        for r in report.rename_candidates.iter().take(MAX_EXAMPLES) {
            lines.push(format!(
                "  - `{}`  →  `{}`  (content {:.2}, name {:.2}, combined **{:.2}**)",
                r.from_path,
                r.to_path,
                r.content_similarity,
                r.name_similarity,
                r.combined_score,
            ));
        }
    }
    lines.push(String::new());

    // ── Overall ───────────────────────────────────────────────────────────────
    lines.push("## Overall".into());
    lines.push(format!("- Renames detected: {}", s.renames_detected));
    lines.push(format!("- Overall similarity score: **{}**", s.overall_similarity));

    lines.join("\n")
}
