use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};
use similar::{ChangeTag, TextDiff};

/// Skip generating a full unified diff (or computing similarity) if either
/// side exceeds this size. Mirrors MAX_DIFF_BYTES in code_diff.py.
const MAX_DIFF_BYTES: u64 = 2 * 1024 * 1024;

// ── result types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
pub struct CodeDiffResult {
    pub path: String,
    pub identical: bool,
    pub added_lines: u32,
    pub removed_lines: u32,
    /// Character-level ratio matching Python's SequenceMatcher.ratio() (0..1).
    pub similarity: f64,
    pub unified_diff: String,
    pub note: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct BinaryDiffResult {
    pub path: String,
    pub identical: bool,
    pub size_a: u64,
    pub size_b: u64,
    pub sha256_a: String,
    pub sha256_b: String,
}

// ── helpers ───────────────────────────────────────────────────────────────────

pub fn rel_to_str(rel: &Path) -> String {
    rel.to_string_lossy().replace('\\', "/")
}

fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Build an error result. Python's dataclass default for `similarity` is 1.0;
/// we replicate that so the field is never absent in the JSON.
fn make_error(path: String, msg: impl std::fmt::Display) -> CodeDiffResult {
    CodeDiffResult {
        path,
        identical: false,
        added_lines: 0,
        removed_lines: 0,
        similarity: 1.0,
        unified_diff: String::new(),
        note: String::new(),
        error: Some(msg.to_string()),
    }
}

// ── public API ────────────────────────────────────────────────────────────────

/// Compare two text/code files.
///
/// Mirrors `diff_code()` in reference/diff_crawler/code_diff.py.
/// Key parity notes (see CLAUDE.md):
/// - SHA-256 fast path fires only when sizes are equal first.
/// - Files > 2 MB return with similarity=1.0 (the Python dataclass default).
/// - Similarity is character-level SequenceMatcher-equivalent, not line-level.
pub fn diff_code(
    rel: &Path,
    abs_a: &Path,
    abs_b: &Path,
    include_full_diff: bool,
) -> CodeDiffResult {
    let path = rel_to_str(rel);

    // --- Sizes ---
    let (size_a, size_b) = match (abs_a.metadata(), abs_b.metadata()) {
        (Ok(ma), Ok(mb)) => (ma.len(), mb.len()),
        (Err(e), _) | (_, Err(e)) => return make_error(path, format!("stat: {e}")),
    };

    // --- SHA-256 fast path (only when sizes match; mirrors Python) ---
    if size_a == size_b {
        match (hash_file(abs_a), hash_file(abs_b)) {
            (Ok(ha), Ok(hb)) if ha == hb => {
                return CodeDiffResult {
                    path,
                    identical: true,
                    added_lines: 0,
                    removed_lines: 0,
                    similarity: 1.0,
                    unified_diff: String::new(),
                    note: String::new(),
                    error: None,
                };
            }
            (Err(e), _) | (_, Err(e)) => return make_error(path, format!("stat: {e}")),
            _ => {} // same size, different content — fall through to full diff
        }
    }

    // --- Large-file guard ---
    // Python returns the dataclass defaults for similarity (1.0) and line counts (0).
    if size_a > MAX_DIFF_BYTES || size_b > MAX_DIFF_BYTES {
        return CodeDiffResult {
            path,
            identical: false,
            added_lines: 0,
            removed_lines: 0,
            similarity: 1.0,
            unified_diff: String::new(),
            note: format!("skipped full diff: file too large (>{MAX_DIFF_BYTES} bytes)"),
            error: None,
        };
    }

    // --- Read text (UTF-8, replacing invalid bytes — mirrors Python errors="replace") ---
    let text_a = match std::fs::read(abs_a) {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(e) => return make_error(path, e),
    };
    let text_b = match std::fs::read(abs_b) {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(e) => return make_error(path, e),
    };

    // --- Line-level diff for added/removed counts and the unified diff string ---
    let line_diff = TextDiff::from_lines(&text_a, &text_b);

    let mut added = 0u32;
    let mut removed = 0u32;
    for op in line_diff.ops() {
        for change in line_diff.iter_changes(op) {
            match change.tag() {
                ChangeTag::Insert => added += 1,
                ChangeTag::Delete => removed += 1,
                ChangeTag::Equal => {}
            }
        }
    }

    let header_a = format!("a/{path}");
    let header_b = format!("b/{path}");
    let unified = if include_full_diff {
        line_diff
            .unified_diff()
            .context_radius(3)
            .header(&header_a, &header_b)
            .to_string()
    } else {
        String::new()
    };

    // --- Character-level similarity (mirrors Python's SequenceMatcher.ratio()) ---
    // similar::TextDiff::ratio() returns f32; widen to f64 for the report.
    let similarity = f64::from(TextDiff::from_chars(&text_a, &text_b).ratio());

    CodeDiffResult {
        path,
        identical: added == 0 && removed == 0,
        added_lines: added,
        removed_lines: removed,
        similarity,
        unified_diff: unified,
        note: String::new(),
        error: None,
    }
}

/// Hash-only diff for binary and model-weight files.
///
/// Mirrors `diff_binary()` in reference/diff_crawler/code_diff.py.
pub fn diff_binary(rel: &Path, abs_a: &Path, abs_b: &Path) -> BinaryDiffResult {
    let path = rel_to_str(rel);
    let size_a = abs_a.metadata().map(|m| m.len()).unwrap_or(0);
    let size_b = abs_b.metadata().map(|m| m.len()).unwrap_or(0);
    // Python skips hashing zero-length files: `ha = _hash_file(abs_a) if sa else ""`
    let sha256_a = if size_a > 0 {
        hash_file(abs_a).unwrap_or_default()
    } else {
        String::new()
    };
    let sha256_b = if size_b > 0 {
        hash_file(abs_b).unwrap_or_default()
    } else {
        String::new()
    };
    BinaryDiffResult {
        path,
        identical: sha256_a == sha256_b && size_a == size_b,
        size_a,
        size_b,
        sha256_a,
        sha256_b,
    }
}
