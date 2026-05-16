use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ignore::WalkBuilder;

use crate::config::{EXCLUDED_DIRS, EXCLUDED_FILES, EXCLUDED_SUFFIXES};

/// All non-excluded file paths under a root directory.
#[derive(Debug, Clone)]
pub struct TreeIndex {
    pub root: PathBuf,
    /// Relative paths (relative to `root`).
    pub files: HashSet<PathBuf>,
    /// rel → abs lookup for dispatch in later phases.
    pub abs_lookup: HashMap<PathBuf, PathBuf>,
}

impl TreeIndex {
    fn new(root: PathBuf) -> Self {
        Self { root, files: HashSet::new(), abs_lookup: HashMap::new() }
    }

    fn insert(&mut self, rel: PathBuf, abs: PathBuf) {
        self.abs_lookup.insert(rel.clone(), abs);
        self.files.insert(rel);
    }
}

/// Set-arithmetic result of comparing two `TreeIndex` values.
#[derive(Debug, Clone)]
pub struct TreeDiff {
    pub in_both: HashSet<PathBuf>,
    pub only_in_a: HashSet<PathBuf>,
    pub only_in_b: HashSet<PathBuf>,
}

impl TreeDiff {
    /// Jaccard similarity over the file-path union. Returns 1.0 for two empty trees.
    pub fn jaccard(&self) -> f64 {
        let union_size =
            self.in_both.len() + self.only_in_a.len() + self.only_in_b.len();
        if union_size == 0 {
            return 1.0;
        }
        self.in_both.len() as f64 / union_size as f64
    }
}

pub fn diff_trees(a: &TreeIndex, b: &TreeIndex) -> TreeDiff {
    TreeDiff {
        in_both: a.files.intersection(&b.files).cloned().collect(),
        only_in_a: a.files.difference(&b.files).cloned().collect(),
        only_in_b: b.files.difference(&a.files).cloned().collect(),
    }
}

/// Returns `true` if this walker entry should be skipped (and, for directories,
/// not descended into). Called from the `filter_entry` closure.
fn should_skip(entry: &ignore::DirEntry) -> bool {
    // Never filter the root itself (depth 0).
    if entry.depth() == 0 {
        return false;
    }

    let name = entry.file_name().to_string_lossy();

    // Exclude by directory/file name and .egg-info suffix (both files and dirs).
    if EXCLUDED_DIRS.contains(&name.as_ref()) || name.ends_with(".egg-info") {
        return true;
    }

    // File-only checks.
    if entry.file_type().is_some_and(|ft| ft.is_file()) {
        if EXCLUDED_FILES.contains(&name.as_ref()) {
            return true;
        }
        // Use Path::extension() which, like Python's Path.suffix, treats dotfiles
        // (e.g. `.log`) as having no extension, avoiding false-positive exclusions.
        if let Some(ext) = entry.path().extension() {
            let suffix = format!(".{}", ext.to_string_lossy().to_lowercase());
            if EXCLUDED_SUFFIXES.contains(&suffix.as_str()) {
                return true;
            }
        }
    }

    false
}

/// Walk `root` and return a `TreeIndex` of all non-excluded, non-symlink files.
///
/// Mirrors `walk_tree()` in reference/diff_crawler/tree_diff.py.
pub fn walk_tree(root: &Path) -> Result<TreeIndex> {
    let root = root
        .canonicalize()
        .with_context(|| format!("cannot resolve path: {}", root.display()))?;

    let mut index = TreeIndex::new(root.clone());

    let walker = WalkBuilder::new(&root)
        // Disable all gitignore-style filtering; we use our own exclusion list.
        .hidden(false)
        .ignore(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        // Don't follow symlinks; they are yielded as entries but ft.is_file()
        // returns false for them (symlink_metadata is used when follow_links=false).
        .follow_links(false)
        .filter_entry(|e| !should_skip(e))
        .build();

    for result in walker {
        let entry = result?;

        let ft = match entry.file_type() {
            Some(ft) => ft,
            None => continue, // only None for stdin pseudo-entries
        };

        // Only regular files; directories are traversal nodes, symlinks are skipped.
        if !ft.is_file() {
            continue;
        }

        // Belt-and-suspenders: also skip anything the OS reports as a symlink.
        if entry.path_is_symlink() {
            continue;
        }

        let abs_path = entry.into_path();
        let rel_path = abs_path
            .strip_prefix(&root)
            .context("entry path is outside of the walk root")?
            .to_path_buf();

        index.insert(rel_path, abs_path);
    }

    Ok(index)
}
