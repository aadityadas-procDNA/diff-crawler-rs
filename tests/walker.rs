use std::fs;
use std::path::Path;

use tempfile::TempDir;

use diff_crawler::walker::{diff_trees, walk_tree};

// ── helpers ──────────────────────────────────────────────────────────────────

/// Create a file (and any missing parent directories) inside `dir`.
fn touch(dir: &Path, rel: &str) {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&p, b"content").unwrap();
}

// ── set arithmetic and Jaccard ────────────────────────────────────────────────

#[test]
fn identical_trees_have_jaccard_one() {
    let a = TempDir::new().unwrap();
    let b = TempDir::new().unwrap();
    touch(a.path(), "foo.py");
    touch(b.path(), "foo.py");

    let diff = diff_trees(&walk_tree(a.path()).unwrap(), &walk_tree(b.path()).unwrap());

    assert_eq!(diff.in_both.len(), 1);
    assert_eq!(diff.only_in_a.len(), 0);
    assert_eq!(diff.only_in_b.len(), 0);
    assert!((diff.jaccard() - 1.0).abs() < f64::EPSILON);
}

#[test]
fn disjoint_trees_have_jaccard_zero() {
    let a = TempDir::new().unwrap();
    let b = TempDir::new().unwrap();
    touch(a.path(), "only_a.py");
    touch(b.path(), "only_b.py");

    let diff = diff_trees(&walk_tree(a.path()).unwrap(), &walk_tree(b.path()).unwrap());

    assert_eq!(diff.in_both.len(), 0);
    assert_eq!(diff.only_in_a.len(), 1);
    assert_eq!(diff.only_in_b.len(), 1);
    assert!(diff.jaccard() < f64::EPSILON);
}

#[test]
fn partial_overlap_jaccard() {
    let a = TempDir::new().unwrap();
    let b = TempDir::new().unwrap();
    touch(a.path(), "shared.py");
    touch(a.path(), "only_a.py");
    touch(b.path(), "shared.py");
    touch(b.path(), "only_b.py");

    let diff = diff_trees(&walk_tree(a.path()).unwrap(), &walk_tree(b.path()).unwrap());

    assert_eq!(diff.in_both.len(), 1);
    assert_eq!(diff.only_in_a.len(), 1);
    assert_eq!(diff.only_in_b.len(), 1);
    // jaccard = 1 / (1 + 1 + 1)
    assert!((diff.jaccard() - 1.0 / 3.0).abs() < 1e-9);
}

#[test]
fn empty_trees_have_jaccard_one() {
    let a = TempDir::new().unwrap();
    let b = TempDir::new().unwrap();

    let diff = diff_trees(&walk_tree(a.path()).unwrap(), &walk_tree(b.path()).unwrap());

    assert_eq!(diff.jaccard(), 1.0);
}

// ── exclusion: directories ────────────────────────────────────────────────────

#[test]
fn excludes_venv_dir() {
    let dir = TempDir::new().unwrap();
    touch(dir.path(), "main.py");
    touch(dir.path(), "venv/lib/site.py");
    touch(dir.path(), "venv/bin/activate");

    let idx = walk_tree(dir.path()).unwrap();
    assert_eq!(idx.files.len(), 1, "venv/ must be excluded");
}

#[test]
fn excludes_dot_venv_dir() {
    let dir = TempDir::new().unwrap();
    touch(dir.path(), "main.py");
    touch(dir.path(), ".venv/lib/site.py");

    let idx = walk_tree(dir.path()).unwrap();
    assert_eq!(idx.files.len(), 1);
}

#[test]
fn excludes_pycache_dir() {
    let dir = TempDir::new().unwrap();
    touch(dir.path(), "main.py");
    touch(dir.path(), "__pycache__/main.cpython-311.pyc");

    let idx = walk_tree(dir.path()).unwrap();
    assert_eq!(idx.files.len(), 1);
}

#[test]
fn excludes_git_dir() {
    let dir = TempDir::new().unwrap();
    touch(dir.path(), "README.md");
    touch(dir.path(), ".git/config");
    touch(dir.path(), ".git/HEAD");

    let idx = walk_tree(dir.path()).unwrap();
    assert_eq!(idx.files.len(), 1);
}

#[test]
fn excludes_node_modules_dir() {
    let dir = TempDir::new().unwrap();
    touch(dir.path(), "index.js");
    touch(dir.path(), "node_modules/lodash/index.js");
    touch(dir.path(), "node_modules/.bin/jest");

    let idx = walk_tree(dir.path()).unwrap();
    assert_eq!(idx.files.len(), 1);
}

#[test]
fn excludes_egg_info_dir() {
    let dir = TempDir::new().unwrap();
    touch(dir.path(), "setup.py");
    touch(dir.path(), "mypackage.egg-info/PKG-INFO");
    touch(dir.path(), "mypackage.egg-info/SOURCES.txt");

    let idx = walk_tree(dir.path()).unwrap();
    assert_eq!(idx.files.len(), 1, "*.egg-info/ must be excluded");
}

#[test]
fn excludes_nested_excluded_dir() {
    // An excluded dir nested inside a normal dir.
    let dir = TempDir::new().unwrap();
    touch(dir.path(), "src/train.py");
    touch(dir.path(), "src/__pycache__/train.cpython-311.pyc");

    let idx = walk_tree(dir.path()).unwrap();
    assert_eq!(idx.files.len(), 1);
}

// ── exclusion: files ─────────────────────────────────────────────────────────

#[test]
fn excludes_ds_store() {
    let dir = TempDir::new().unwrap();
    touch(dir.path(), "code.py");
    touch(dir.path(), ".DS_Store");

    let idx = walk_tree(dir.path()).unwrap();
    assert_eq!(idx.files.len(), 1);
}

#[test]
fn excludes_thumbs_db() {
    let dir = TempDir::new().unwrap();
    touch(dir.path(), "code.py");
    touch(dir.path(), "Thumbs.db");

    let idx = walk_tree(dir.path()).unwrap();
    assert_eq!(idx.files.len(), 1);
}

// ── exclusion: suffixes ───────────────────────────────────────────────────────

#[test]
fn excludes_pyc_suffix() {
    let dir = TempDir::new().unwrap();
    touch(dir.path(), "code.py");
    touch(dir.path(), "code.pyc");

    let idx = walk_tree(dir.path()).unwrap();
    assert_eq!(idx.files.len(), 1);
}

#[test]
fn excludes_log_suffix() {
    let dir = TempDir::new().unwrap();
    touch(dir.path(), "train.py");
    touch(dir.path(), "run.log");

    let idx = walk_tree(dir.path()).unwrap();
    assert_eq!(idx.files.len(), 1);
}

#[test]
fn suffix_check_is_case_insensitive() {
    let dir = TempDir::new().unwrap();
    touch(dir.path(), "module.py");
    touch(dir.path(), "module.PYC");

    let idx = walk_tree(dir.path()).unwrap();
    assert_eq!(idx.files.len(), 1, ".PYC (uppercase) must be excluded");
}

#[test]
fn dotfile_without_extension_is_not_excluded_by_suffix() {
    // A file literally named ".log" has no extension per Path::extension() (it's
    // treated as a dotfile stem), so it must NOT be excluded by the suffix rule.
    let dir = TempDir::new().unwrap();
    touch(dir.path(), "code.py");
    touch(dir.path(), ".log"); // dotfile — NOT a ".log" extension

    let idx = walk_tree(dir.path()).unwrap();
    assert_eq!(idx.files.len(), 2, ".log dotfile must be kept");
}

// ── abs_lookup ────────────────────────────────────────────────────────────────

#[test]
fn abs_lookup_resolves_to_existing_file() {
    let dir = TempDir::new().unwrap();
    touch(dir.path(), "src/model.py");

    let idx = walk_tree(dir.path()).unwrap();
    let rel = std::path::PathBuf::from("src/model.py");
    let abs = idx
        .abs_lookup
        .get(&rel)
        .expect("rel path must be in abs_lookup");
    assert!(abs.exists());
}
