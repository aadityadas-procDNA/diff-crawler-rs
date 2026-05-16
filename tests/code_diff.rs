use std::fs;
use std::path::Path;

use tempfile::TempDir;

use diff_crawler::code_diff::{diff_binary, diff_code};

// ── helpers ───────────────────────────────────────────────────────────────────

fn write(dir: &Path, name: &str, content: &[u8]) -> std::path::PathBuf {
    let p = dir.join(name);
    fs::write(&p, content).unwrap();
    p
}

// ── diff_code ─────────────────────────────────────────────────────────────────

#[test]
fn identical_files_use_fast_path() {
    let dir = TempDir::new().unwrap();
    let a = write(dir.path(), "a.py", b"print('hello')\n");
    let b = write(dir.path(), "b.py", b"print('hello')\n");

    let r = diff_code(Path::new("a.py"), &a, &b, true);

    assert!(r.identical, "identical content must be detected");
    assert_eq!(r.added_lines, 0);
    assert_eq!(r.removed_lines, 0);
    assert_eq!(r.similarity, 1.0);
    assert!(r.unified_diff.is_empty(), "no diff string for identical files");
    assert!(r.error.is_none());
}

#[test]
fn added_line_counted_correctly() {
    let dir = TempDir::new().unwrap();
    let a = write(dir.path(), "a.py", b"line1\nline2\n");
    let b = write(dir.path(), "b.py", b"line1\nline2\nline3\n");

    let r = diff_code(Path::new("a.py"), &a, &b, false);

    assert!(!r.identical);
    assert_eq!(r.added_lines, 1);
    assert_eq!(r.removed_lines, 0);
}

#[test]
fn removed_line_counted_correctly() {
    let dir = TempDir::new().unwrap();
    let a = write(dir.path(), "a.py", b"line1\nline2\nline3\n");
    let b = write(dir.path(), "b.py", b"line1\nline3\n");

    let r = diff_code(Path::new("a.py"), &a, &b, false);

    assert!(!r.identical);
    assert_eq!(r.added_lines, 0);
    assert_eq!(r.removed_lines, 1);
}

#[test]
fn changed_line_counts_one_add_one_remove() {
    let dir = TempDir::new().unwrap();
    let a = write(dir.path(), "a.py", b"line1\nold\nline3\n");
    let b = write(dir.path(), "b.py", b"line1\nnew\nline3\n");

    let r = diff_code(Path::new("a.py"), &a, &b, false);

    assert!(!r.identical);
    assert_eq!(r.added_lines, 1);
    assert_eq!(r.removed_lines, 1);
}

#[test]
fn similarity_is_in_range() {
    let dir = TempDir::new().unwrap();
    let a = write(dir.path(), "a.py", b"hello world\n");
    let b = write(dir.path(), "b.py", b"goodbye world\n");

    let r = diff_code(Path::new("a.py"), &a, &b, false);

    assert!((0.0..=1.0).contains(&r.similarity), "similarity={} out of range", r.similarity);
    assert!(r.similarity < 1.0, "different files must have similarity < 1");
    assert!(r.similarity > 0.0, "files with shared text must have similarity > 0");
}

#[test]
fn completely_different_files_have_low_similarity() {
    let dir = TempDir::new().unwrap();
    let a = write(dir.path(), "a.py", b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n");
    let b = write(dir.path(), "b.py", b"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n");

    let r = diff_code(Path::new("a.py"), &a, &b, false);

    // They share a newline and the trailing \n; similarity should be very low.
    assert!(r.similarity < 0.2, "similarity={} unexpectedly high", r.similarity);
}

#[test]
fn no_diff_flag_omits_unified_diff_but_keeps_counts() {
    let dir = TempDir::new().unwrap();
    let a = write(dir.path(), "a.py", b"x = 1\ny = 2\n");
    let b = write(dir.path(), "b.py", b"x = 1\ny = 99\n");

    let with_diff = diff_code(Path::new("a.py"), &a, &b, true);
    let without_diff = diff_code(Path::new("a.py"), &a, &b, false);

    // Counts and similarity must match regardless of the flag.
    assert_eq!(with_diff.added_lines, without_diff.added_lines);
    assert_eq!(with_diff.removed_lines, without_diff.removed_lines);
    assert!((with_diff.similarity - without_diff.similarity).abs() < 1e-9);

    assert!(!with_diff.unified_diff.is_empty(), "diff string expected when include_full_diff=true");
    assert!(without_diff.unified_diff.is_empty(), "diff string must be empty when include_full_diff=false");
}

#[test]
fn unified_diff_contains_expected_markers() {
    let dir = TempDir::new().unwrap();
    let a = write(dir.path(), "a.py", b"line1\nold_line\nline3\n");
    let b = write(dir.path(), "b.py", b"line1\nnew_line\nline3\n");

    let r = diff_code(Path::new("src/a.py"), &a, &b, true);

    assert!(r.unified_diff.contains("--- a/src/a.py"), "missing --- header");
    assert!(r.unified_diff.contains("+++ b/src/a.py"), "missing +++ header");
    assert!(r.unified_diff.contains("-old_line"), "missing removed line");
    assert!(r.unified_diff.contains("+new_line"), "missing added line");
}

#[test]
fn large_file_skips_diff_and_has_note() {
    let dir = TempDir::new().unwrap();
    // Two different files both > 2 MB; same size so size-equality check fires,
    // hashes differ, then the large-file guard triggers.
    let large: Vec<u8> = std::iter::repeat(b'A').take(2_200_000).collect();
    let large2: Vec<u8> = std::iter::repeat(b'B').take(2_200_000).collect();
    let a = write(dir.path(), "big_a.bin", &large);
    let b = write(dir.path(), "big_b.bin", &large2);

    let r = diff_code(Path::new("big.bin"), &a, &b, true);

    assert!(!r.identical);
    assert!(r.note.contains("too large"), "expected 'too large' note, got: {}", r.note);
    assert!(r.unified_diff.is_empty());
    assert_eq!(r.added_lines, 0);
    assert_eq!(r.removed_lines, 0);
    // Python leaves similarity at the dataclass default (1.0) for the large-file path.
    assert_eq!(r.similarity, 1.0);
    assert!(r.error.is_none());
}

// ── diff_binary ───────────────────────────────────────────────────────────────

#[test]
fn binary_identical_files() {
    let dir = TempDir::new().unwrap();
    let content = b"\x00\x01\x02\x03\xff";
    let a = write(dir.path(), "model_a.pkl", content);
    let b = write(dir.path(), "model_b.pkl", content);

    let r = diff_binary(Path::new("model.pkl"), &a, &b);

    assert!(r.identical);
    assert_eq!(r.size_a, r.size_b);
    assert_eq!(r.sha256_a, r.sha256_b);
    assert!(!r.sha256_a.is_empty());
    assert_eq!(r.sha256_a.len(), 64, "SHA-256 hex string must be 64 chars");
}

#[test]
fn binary_different_files() {
    let dir = TempDir::new().unwrap();
    let a = write(dir.path(), "a.pkl", b"version1\x00\xff");
    let b = write(dir.path(), "b.pkl", b"version2\x00\xff");

    let r = diff_binary(Path::new("model.pkl"), &a, &b);

    assert!(!r.identical);
    assert_ne!(r.sha256_a, r.sha256_b);
    assert_eq!(r.size_a, r.size_b); // same number of bytes
}

#[test]
fn binary_different_sizes() {
    let dir = TempDir::new().unwrap();
    let a = write(dir.path(), "a.pkl", b"short");
    let b = write(dir.path(), "b.pkl", b"much longer content here");

    let r = diff_binary(Path::new("model.pkl"), &a, &b);

    assert!(!r.identical);
    assert!(r.size_a < r.size_b);
}

#[test]
fn binary_sha256_is_lowercase_hex() {
    let dir = TempDir::new().unwrap();
    let a = write(dir.path(), "a.bin", b"test content");
    let b = write(dir.path(), "b.bin", b"other content");

    let r = diff_binary(Path::new("f.bin"), &a, &b);

    for ch in r.sha256_a.chars() {
        assert!(
            ch.is_ascii_hexdigit() && !ch.is_uppercase(),
            "SHA-256 must be lowercase hex, found char: {ch}"
        );
    }
}

// ── crawler integration ───────────────────────────────────────────────────────

#[test]
fn crawl_classifies_code_and_binary_separately() {
    use diff_crawler::config::CrawlConfig;
    use diff_crawler::crawler::crawl;

    let dir_a = TempDir::new().unwrap();
    let dir_b = TempDir::new().unwrap();

    // Code file: same content → identical
    fs::write(dir_a.path().join("train.py"), b"x = 1\n").unwrap();
    fs::write(dir_b.path().join("train.py"), b"x = 1\n").unwrap();

    // Binary file: different content
    fs::write(dir_a.path().join("model.pkl"), b"\x00\x01\x02").unwrap();
    fs::write(dir_b.path().join("model.pkl"), b"\x00\x01\x03").unwrap();

    let report = crawl(dir_a.path(), dir_b.path(), &CrawlConfig::default()).unwrap();

    assert_eq!(report.code_diffs.len(), 1);
    assert_eq!(report.binary_diffs.len(), 1);
    assert!(report.code_diffs[0].identical);
    assert!(!report.binary_diffs[0].identical);
}

#[test]
fn crawl_output_paths_use_forward_slashes() {
    use diff_crawler::config::CrawlConfig;
    use diff_crawler::crawler::crawl;

    let dir_a = TempDir::new().unwrap();
    let dir_b = TempDir::new().unwrap();

    fs::create_dir_all(dir_a.path().join("src")).unwrap();
    fs::create_dir_all(dir_b.path().join("src")).unwrap();
    fs::write(dir_a.path().join("src/lib.py"), b"pass\n").unwrap();
    fs::write(dir_b.path().join("src/lib.py"), b"pass\n").unwrap();

    let report = crawl(dir_a.path(), dir_b.path(), &CrawlConfig::default()).unwrap();

    let path = &report.code_diffs[0].path;
    assert!(!path.contains('\\'), "path must use forward slashes, got: {path}");
    assert_eq!(path, "src/lib.py");
}
