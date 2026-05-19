use std::fs;

use tempfile::TempDir;

use diff_crawler::config::CrawlConfig;
use diff_crawler::crawler::{build_summary, crawl};
use diff_crawler::report::render_markdown;

// ── build_summary unit tests ──────────────────────────────────────────────────

#[test]
fn summary_no_code_files_avg_sim_is_one() {
    let s = build_summary(5, 2, 1, 0.8, &[], &[], &[], &[]);
    assert_eq!(
        s.code.avg_similarity, 1.0,
        "avg_similarity must default to 1.0 when no code files"
    );
}

#[test]
fn summary_overall_similarity_blended() {
    use diff_crawler::code_diff::CodeDiffResult;
    let code = CodeDiffResult {
        path: "a.py".into(),
        identical: false,
        added_lines: 1,
        removed_lines: 1,
        similarity: 0.6,
        unified_diff: String::new(),
        note: String::new(),
        error: None,
    };
    // tree_jaccard=0.8, avg_code_sim=0.6 → overall = 0.5*0.8 + 0.5*0.6 = 0.7
    let s = build_summary(5, 0, 0, 0.8, &[code], &[], &[], &[]);
    assert_eq!(s.overall_similarity, 0.7);
}

#[test]
fn summary_overall_similarity_no_common_files_uses_tree_jaccard() {
    // n_common == 0 AND files only on one side → use tree_jaccard alone.
    let s = build_summary(0, 3, 0, 0.0, &[], &[], &[], &[]);
    assert_eq!(s.overall_similarity, 0.0);

    let s2 = build_summary(0, 0, 2, 0.0, &[], &[], &[], &[]);
    assert_eq!(s2.overall_similarity, 0.0);
}

#[test]
fn summary_tree_fields_correct() {
    let s = build_summary(10, 3, 5, 0.5555, &[], &[], &[], &[]);
    assert_eq!(s.tree.files_in_both, 10);
    assert_eq!(s.tree.only_in_a, 3);
    assert_eq!(s.tree.only_in_b, 5);
    // round4(0.5555) = 0.5555
    assert_eq!(s.tree.tree_jaccard, 0.5555);
}

#[test]
fn summary_code_counts_correct() {
    use diff_crawler::code_diff::CodeDiffResult;
    let make = |identical: bool, sim: f64, added: u32, removed: u32| CodeDiffResult {
        path: "x.py".into(),
        identical,
        added_lines: added,
        removed_lines: removed,
        similarity: sim,
        unified_diff: String::new(),
        note: String::new(),
        error: None,
    };
    let diffs = vec![
        make(true, 1.0, 0, 0),
        make(false, 0.5, 10, 5),
        make(false, 0.8, 2, 3),
    ];
    let s = build_summary(3, 0, 0, 1.0, &diffs, &[], &[], &[]);
    assert_eq!(s.code.compared, 3);
    assert_eq!(s.code.identical, 1);
    assert_eq!(s.code.total_added_lines, 12);
    assert_eq!(s.code.total_removed_lines, 8);
}

#[test]
fn summary_data_schema_changed_count() {
    use diff_crawler::data_diff::DataDiffResult;
    use std::collections::BTreeMap;

    let make = |cols_added: Vec<String>, cols_removed: Vec<String>| DataDiffResult {
        path: "x.csv".into(),
        row_count_a: 10,
        row_count_b: 10,
        schema_a: BTreeMap::new(),
        schema_b: BTreeMap::new(),
        columns_added: cols_added,
        columns_removed: cols_removed,
        type_changes: BTreeMap::new(),
        column_stats_a: BTreeMap::new(),
        column_stats_b: BTreeMap::new(),
        note: String::new(),
        error: None,
    };

    let data = vec![
        make(vec!["new_col".into()], vec![]), // schema changed
        make(vec![], vec![]),                 // schema unchanged
        make(vec![], vec!["old_col".into()]), // schema changed
    ];
    let s = build_summary(3, 0, 0, 1.0, &[], &data, &[], &[]);
    assert_eq!(s.data.compared, 3);
    assert_eq!(s.data.schema_changed, 2);
    assert_eq!(s.data.total_row_delta, 0);
}

// ── render_markdown tests ─────────────────────────────────────────────────────

#[test]
fn markdown_contains_required_sections() {
    let dir_a = TempDir::new().unwrap();
    let dir_b = TempDir::new().unwrap();
    fs::write(dir_a.path().join("hello.py"), "print('hello')\n").unwrap();
    fs::write(dir_b.path().join("hello.py"), "print('world')\n").unwrap();

    let config = CrawlConfig::default();
    let report = crawl(dir_a.path(), dir_b.path(), &config).unwrap();
    let md = render_markdown(&report);

    assert!(md.contains("# Diff Report"), "must have top-level heading");
    assert!(md.contains("## Tree summary"), "must have tree section");
    assert!(md.contains("## Code"), "must have code section");
    assert!(md.contains("## Data"), "must have data section");
    assert!(
        md.contains("## Binary / model files"),
        "must have binary section"
    );
    assert!(
        md.contains("## Likely renames / moves"),
        "must have renames section"
    );
    assert!(md.contains("## Overall"), "must have overall section");
}

#[test]
fn markdown_dir_paths_present() {
    let dir_a = TempDir::new().unwrap();
    let dir_b = TempDir::new().unwrap();

    let config = CrawlConfig::default();
    let report = crawl(dir_a.path(), dir_b.path(), &config).unwrap();
    let md = render_markdown(&report);

    assert!(
        md.contains(&report.dir_a),
        "markdown must contain dir_a path"
    );
    assert!(
        md.contains(&report.dir_b),
        "markdown must contain dir_b path"
    );
}

#[test]
fn markdown_no_renames_shows_none_message() {
    let dir_a = TempDir::new().unwrap();
    let dir_b = TempDir::new().unwrap();

    let config = CrawlConfig::default();
    let report = crawl(dir_a.path(), dir_b.path(), &config).unwrap();
    let md = render_markdown(&report);

    assert!(
        md.contains("_None detected above threshold._"),
        "empty rename_candidates must show none-message"
    );
}

#[test]
fn markdown_rename_candidate_shown() {
    let dir_a = TempDir::new().unwrap();
    let dir_b = TempDir::new().unwrap();

    let code = r#"
def compute_loss(predictions, labels):
    diff = predictions - labels
    return (diff ** 2).mean()

def compute_accuracy(predictions, labels):
    correct = (predictions.argmax(axis=1) == labels).sum()
    return correct / len(labels)

def evaluate(model, dataset):
    preds = model.predict(dataset.inputs)
    loss = compute_loss(preds, dataset.labels)
    acc = compute_accuracy(preds, dataset.labels)
    return {'loss': loss, 'accuracy': acc}
"#;
    fs::write(dir_a.path().join("metrics.py"), code).unwrap();
    fs::write(dir_b.path().join("metrics_utils.py"), code).unwrap();

    let config = CrawlConfig {
        rename_threshold: 0.4,
        ..CrawlConfig::default()
    };
    let report = crawl(dir_a.path(), dir_b.path(), &config).unwrap();
    let md = render_markdown(&report);

    assert!(
        md.contains("metrics.py"),
        "rename from_path must appear in markdown"
    );
    assert!(
        md.contains("metrics_utils.py"),
        "rename to_path must appear in markdown"
    );
}

#[test]
fn markdown_changed_code_file_listed() {
    let dir_a = TempDir::new().unwrap();
    let dir_b = TempDir::new().unwrap();
    fs::write(dir_a.path().join("algo.py"), "def foo(): return 1\n").unwrap();
    fs::write(
        dir_b.path().join("algo.py"),
        "def bar(): return 99\ndef baz(): pass\n",
    )
    .unwrap();

    let config = CrawlConfig {
        include_full_unified_diff: false,
        ..CrawlConfig::default()
    };
    let report = crawl(dir_a.path(), dir_b.path(), &config).unwrap();
    let md = render_markdown(&report);

    assert!(
        md.contains("Most changed code files"),
        "must list changed code files section when files differ"
    );
    assert!(md.contains("algo.py"), "changed file name must appear");
}

#[test]
fn markdown_overall_similarity_present() {
    let dir_a = TempDir::new().unwrap();
    let dir_b = TempDir::new().unwrap();
    fs::write(dir_a.path().join("a.py"), "x = 1\n").unwrap();
    fs::write(dir_b.path().join("a.py"), "x = 1\n").unwrap();

    let config = CrawlConfig::default();
    let report = crawl(dir_a.path(), dir_b.path(), &config).unwrap();
    let md = render_markdown(&report);

    assert!(
        md.contains("Overall similarity score"),
        "overall similarity must be in markdown"
    );
}
