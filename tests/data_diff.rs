use std::fs;
use std::path::Path;

use tempfile::TempDir;

use diff_crawler::data_diff::diff_data;

fn write(dir: &std::path::Path, name: &str, content: &str) -> std::path::PathBuf {
    let p = dir.join(name);
    fs::write(&p, content).unwrap();
    p
}

// ── CSV ───────────────────────────────────────────────────────────────────────

#[test]
fn csv_identical_files_have_same_row_count_and_schema() {
    let dir = TempDir::new().unwrap();
    let csv = "a,b,c\n1,2,3\n4,5,6\n";
    let a = write(dir.path(), "a.csv", csv);
    let b = write(dir.path(), "b.csv", csv);

    let r = diff_data(Path::new("data.csv"), &a, &b, false);

    assert!(r.error.is_none(), "unexpected error: {:?}", r.error);
    assert_eq!(r.row_count_a, 2);
    assert_eq!(r.row_count_b, 2);
    assert!(r.columns_added.is_empty());
    assert!(r.columns_removed.is_empty());
    assert!(r.type_changes.is_empty());
}

#[test]
fn csv_row_counts_differ() {
    let dir = TempDir::new().unwrap();
    let a = write(dir.path(), "a.csv", "x,y\n1,2\n3,4\n5,6\n");
    let b = write(dir.path(), "b.csv", "x,y\n1,2\n");

    let r = diff_data(Path::new("data.csv"), &a, &b, false);

    assert!(r.error.is_none(), "unexpected error: {:?}", r.error);
    assert_eq!(r.row_count_a, 3);
    assert_eq!(r.row_count_b, 1);
}

#[test]
fn csv_column_added() {
    let dir = TempDir::new().unwrap();
    let a = write(dir.path(), "a.csv", "x,y\n1,2\n");
    let b = write(dir.path(), "b.csv", "x,y,z\n1,2,3\n");

    let r = diff_data(Path::new("data.csv"), &a, &b, false);

    assert!(r.error.is_none(), "unexpected error: {:?}", r.error);
    assert_eq!(r.columns_added, vec!["z"]);
    assert!(r.columns_removed.is_empty());
}

#[test]
fn csv_column_removed() {
    let dir = TempDir::new().unwrap();
    let a = write(dir.path(), "a.csv", "x,y,z\n1,2,3\n");
    let b = write(dir.path(), "b.csv", "x,y\n1,2\n");

    let r = diff_data(Path::new("data.csv"), &a, &b, false);

    assert!(r.error.is_none(), "unexpected error: {:?}", r.error);
    assert!(r.columns_added.is_empty());
    assert_eq!(r.columns_removed, vec!["z"]);
}

#[test]
fn csv_type_inferred_bigint_for_integers() {
    let dir = TempDir::new().unwrap();
    let a = write(dir.path(), "a.csv", "id\n1\n2\n3\n");
    let b = write(dir.path(), "b.csv", "id\n4\n5\n");

    let r = diff_data(Path::new("data.csv"), &a, &b, false);

    assert!(r.error.is_none(), "unexpected error: {:?}", r.error);
    assert_eq!(r.schema_a.get("id").map(|s| s.as_str()), Some("BIGINT"));
    assert_eq!(r.schema_b.get("id").map(|s| s.as_str()), Some("BIGINT"));
}

#[test]
fn csv_type_inferred_varchar_for_strings() {
    let dir = TempDir::new().unwrap();
    let a = write(dir.path(), "a.csv", "name\nalice\nbob\n");
    let b = write(dir.path(), "b.csv", "name\ncharlie\n");

    let r = diff_data(Path::new("data.csv"), &a, &b, false);

    assert!(r.error.is_none(), "unexpected error: {:?}", r.error);
    assert_eq!(r.schema_a.get("name").map(|s| s.as_str()), Some("VARCHAR"));
}

#[test]
fn csv_type_change_detected() {
    let dir = TempDir::new().unwrap();
    let a = write(dir.path(), "a.csv", "val\n1\n2\n3\n");
    let b = write(dir.path(), "b.csv", "val\nhello\nworld\n");

    let r = diff_data(Path::new("data.csv"), &a, &b, false);

    assert!(r.error.is_none(), "unexpected error: {:?}", r.error);
    assert!(
        r.type_changes.contains_key("val"),
        "expected type change on 'val'"
    );
    let change = &r.type_changes["val"];
    assert_eq!(change[0], "BIGINT");
    assert_eq!(change[1], "VARCHAR");
}

#[test]
fn csv_schema_keys_are_alphabetical() {
    let dir = TempDir::new().unwrap();
    // Headers intentionally out of alphabetical order.
    let a = write(dir.path(), "a.csv", "z,a,m\n1,2,3\n");
    let b = write(dir.path(), "b.csv", "z,a,m\n4,5,6\n");

    let r = diff_data(Path::new("data.csv"), &a, &b, false);

    assert!(r.error.is_none(), "unexpected error: {:?}", r.error);
    let keys_a: Vec<&String> = r.schema_a.keys().collect();
    let mut sorted = keys_a.clone();
    sorted.sort();
    assert_eq!(
        keys_a, sorted,
        "schema_a keys must be in alphabetical order"
    );
}

#[test]
fn csv_deep_stats_populated() {
    let dir = TempDir::new().unwrap();
    let a = write(dir.path(), "a.csv", "x,y\n1,2\n,4\n5,6\n"); // empty field → null in x
    let b = write(dir.path(), "b.csv", "x,y\n10,20\n30,40\n");

    let r = diff_data(Path::new("data.csv"), &a, &b, true);

    assert!(r.error.is_none(), "unexpected error: {:?}", r.error);
    assert!(
        !r.column_stats_a.is_empty(),
        "column_stats_a should be populated"
    );
    assert!(
        !r.column_stats_b.is_empty(),
        "column_stats_b should be populated"
    );
    // x has one null in file a
    assert_eq!(r.column_stats_a["x"].nulls, 1);
    // y has no nulls in file a
    assert_eq!(r.column_stats_a["y"].nulls, 0);
}

#[test]
fn csv_deep_stats_empty_without_flag() {
    let dir = TempDir::new().unwrap();
    let a = write(dir.path(), "a.csv", "x,y\n1,2\n");
    let b = write(dir.path(), "b.csv", "x,y\n3,4\n");

    let r = diff_data(Path::new("data.csv"), &a, &b, false);

    assert!(r.error.is_none(), "unexpected error: {:?}", r.error);
    assert!(r.column_stats_a.is_empty());
    assert!(r.column_stats_b.is_empty());
}

#[test]
fn csv_distinct_counts_populated_with_deep() {
    let dir = TempDir::new().unwrap();
    let a = write(dir.path(), "a.csv", "x\n1\n2\n2\n3\n");
    let b = write(dir.path(), "b.csv", "x\n1\n1\n");

    let r = diff_data(Path::new("data.csv"), &a, &b, true);

    assert!(r.error.is_none(), "unexpected error: {:?}", r.error);
    // 3 distinct values in a (1, 2, 3), 1 in b (1)
    assert_eq!(r.column_stats_a["x"].approx_distinct, 3);
    assert_eq!(r.column_stats_b["x"].approx_distinct, 1);
}

// ── JSONL ─────────────────────────────────────────────────────────────────────

#[test]
fn jsonl_row_counts() {
    let dir = TempDir::new().unwrap();
    let a = write(dir.path(), "a.jsonl", "{\"x\":1}\n{\"x\":2}\n{\"x\":3}\n");
    let b = write(dir.path(), "b.jsonl", "{\"x\":4}\n");

    let r = diff_data(Path::new("data.jsonl"), &a, &b, false);

    assert!(r.error.is_none(), "unexpected error: {:?}", r.error);
    assert_eq!(r.row_count_a, 3);
    assert_eq!(r.row_count_b, 1);
}

#[test]
fn jsonl_schema_detected() {
    let dir = TempDir::new().unwrap();
    let a = write(dir.path(), "a.jsonl", "{\"name\":\"alice\",\"score\":95}\n");
    let b = write(dir.path(), "b.jsonl", "{\"name\":\"bob\",\"score\":80}\n");

    let r = diff_data(Path::new("data.jsonl"), &a, &b, false);

    assert!(r.error.is_none(), "unexpected error: {:?}", r.error);
    assert!(r.schema_a.contains_key("name"));
    assert!(r.schema_a.contains_key("score"));
}

#[test]
fn ndjson_treated_same_as_jsonl() {
    let dir = TempDir::new().unwrap();
    let a = write(dir.path(), "a.ndjson", "{\"v\":1}\n{\"v\":2}\n");
    let b = write(dir.path(), "b.ndjson", "{\"v\":3}\n");

    let r = diff_data(Path::new("data.ndjson"), &a, &b, false);

    assert!(r.error.is_none(), "unexpected error: {:?}", r.error);
    assert_eq!(r.row_count_a, 2);
    assert_eq!(r.row_count_b, 1);
}

// ── path handling ─────────────────────────────────────────────────────────────

#[test]
fn path_uses_forward_slashes() {
    let dir = TempDir::new().unwrap();
    let a = write(dir.path(), "a.csv", "x\n1\n");
    let b = write(dir.path(), "b.csv", "x\n2\n");

    let r = diff_data(Path::new("sub/data.csv"), &a, &b, false);

    assert!(
        !r.path.contains('\\'),
        "path must use forward slashes, got: {}",
        r.path
    );
    assert_eq!(r.path, "sub/data.csv");
}

// ── unsupported format returns a skip note (not an error) ────────────────────

#[test]
fn unsupported_format_returns_skip_note() {
    let dir = TempDir::new().unwrap();
    let a = write(dir.path(), "a.xlsx", "fake");
    let b = write(dir.path(), "b.xlsx", "fake");

    let r = diff_data(Path::new("data.xlsx"), &a, &b, false);

    assert!(
        r.error.is_none(),
        "unsupported format should not produce an error"
    );
    assert!(
        r.note.contains("skipped"),
        "expected 'skipped' in note, got: {}",
        r.note
    );
}

// ── crawler integration ───────────────────────────────────────────────────────

#[test]
fn crawl_includes_data_diffs_for_csv() {
    use diff_crawler::config::CrawlConfig;
    use diff_crawler::crawler::crawl;

    let dir_a = TempDir::new().unwrap();
    let dir_b = TempDir::new().unwrap();

    fs::write(dir_a.path().join("train.csv"), b"a,b\n1,2\n3,4\n").unwrap();
    fs::write(dir_b.path().join("train.csv"), b"a,b\n1,2\n3,4\n5,6\n").unwrap();

    let report = crawl(dir_a.path(), dir_b.path(), &CrawlConfig::default()).unwrap();

    assert_eq!(report.data_diffs.len(), 1, "expected one data diff");
    let d = &report.data_diffs[0];
    assert!(d.error.is_none(), "unexpected error: {:?}", d.error);
    assert_eq!(d.row_count_a, 2);
    assert_eq!(d.row_count_b, 3);
}
