/// Probabilistic rename detection via MinHash LSH.
///
/// Mirrors `find_renames()` in reference/diff_crawler/matcher.py.
///
/// Algorithm:
///   1. Tokenize each file with `\w+|[^\w\s]` (same regex as Python).
///   2. Build k=5 token-level shingles (space-joined 5-grams of tokens).
///   3. Compute a 128-permutation MinHash signature per file.
///   4. Build an LSH index over B-side files using 64 bands × 2 rows.
///   5. For each A-side file query the index; keep the B candidate with the
///      highest combined score = 0.75 × content_sim + 0.25 × name_sim.
///   6. Emit candidates where combined_score ≥ rename_threshold.
///   7. Sort descending by combined_score; truncate at 200.
///
/// # Differences from the Python reference
///
/// - MinHash hash function: Python's `datasketch` uses SHA-1 internally;
///   we use FNV-1a with a permutation-modified seed. Jaccard estimates will
///   differ slightly but rename pairs are still correctly detected.
/// - LSH banding: b=64, r=2 (Python's `datasketch` chooses bands optimally
///   for the requested threshold). At threshold=0.3 our choice gives
///   P(candidate | Jaccard=0.3) ≈ 99.8%, so recall is effectively the same.
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde::Serialize;

use crate::classifier::{classify, FileKind};
use crate::code_diff::rel_to_str;

// ── tunables (match Python reference) ────────────────────────────────────────

const NUM_PERM: usize = 128;
const SHINGLE_K: usize = 5;
/// Minimum Jaccard for LSH candidate selection (pre-filter).
const LSH_THRESHOLD: f64 = 0.3;
/// Skip files larger than this; mirrors Python's MAX_FILE_BYTES.
const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;
/// Truncate results after this many entries; from CLAUDE.md.
const MAX_RENAMES_REPORTED: usize = 200;

/// LSH banding: 64 bands × 2 rows = 128 permutations.
///
/// P(candidate | Jaccard=s) = 1 − (1 − s^ROWS)^BANDS
///   At s=0.3: ≈ 1 − (1 − 0.09)^64 ≈ 99.8%   — high recall, few misses.
///   At s=0.1: ≈ 1 − (1 − 0.01)^64 ≈ 47%     — modest false-positive rate,
///             filtered out by the exact Jaccard check below.
const NUM_BANDS: usize = 64;
const ROWS_PER_BAND: usize = 2; // NUM_PERM / NUM_BANDS

// ── result type ───────────────────────────────────────────────────────────────

/// One rename candidate. JSON shape matches the Python `RenameCandidate` dataclass.
#[derive(Debug, Clone, Serialize)]
pub struct RenameCandidate {
    /// Relative path that exists only in A.
    pub from_path: String,
    /// Relative path that exists only in B.
    pub to_path: String,
    pub content_similarity: f64,
    pub name_similarity: f64,
    pub combined_score: f64,
}

// ── tokenizer ─────────────────────────────────────────────────────────────────

/// Tokenize `text` using the same logic as Python's `re.compile(r"\w+|[^\w\s]")`:
/// yield contiguous word characters (`[a-zA-Z0-9_]` + Unicode alphanumerics)
/// as single tokens, and each non-whitespace non-word character as its own token.
///
/// Returns byte-range slices into `text` to avoid allocation per token.
fn tokenize(text: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut chars = text.char_indices().peekable();

    while let Some((i, c)) = chars.next() {
        if c.is_alphanumeric() || c == '_' {
            // Collect a contiguous word token.
            let start = i;
            let mut end = i + c.len_utf8();
            while let Some(&(j, nc)) = chars.peek() {
                if nc.is_alphanumeric() || nc == '_' {
                    end = j + nc.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
            tokens.push(&text[start..end]);
        } else if !c.is_whitespace() {
            // Single punctuation / symbol character.
            tokens.push(&text[i..i + c.len_utf8()]);
        }
    }

    tokens
}

/// Build the set of k-shingle strings from `text`.
///
/// Mirrors Python's `_shingles()`: k-grams over the *token* sequence,
/// joined by spaces. E.g. tokens=["def","foo","(","x",")"] → shingles of 5
/// consecutive tokens joined: "def foo ( x )".
fn build_shingles(text: &str, k: usize) -> HashSet<String> {
    let tokens = tokenize(text);
    if tokens.is_empty() {
        return HashSet::new();
    }
    if tokens.len() < k {
        // Python returns {" ".join(tokens)} for small files.
        return std::iter::once(tokens.join(" ")).collect();
    }
    (0..=tokens.len() - k)
        .map(|i| tokens[i..i + k].join(" "))
        .collect()
}

// ── MinHash ───────────────────────────────────────────────────────────────────

/// FNV-1a hash with a permutation-specific seed perturbation.
///
/// Each `perm` in 0..NUM_PERM yields a statistically independent hash function,
/// giving us 128 hash functions without storing 128 separate hash-family values.
#[inline]
fn hash_for_perm(bytes: &[u8], perm: usize) -> u64 {
    const PRIME: u64 = 0x100000001b3;
    // XOR the FNV offset with a scramble of the permutation index so each
    // permutation starts from a different initial value.
    let mut h: u64 =
        0xcbf29ce484222325u64 ^ (perm as u64).wrapping_mul(0x9e3779b97f4a7c15u64);
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(PRIME);
    }
    h
}

/// Compute a MinHash signature of `NUM_PERM` values from a shingle set.
fn compute_minhash(shingles: &HashSet<String>) -> [u64; NUM_PERM] {
    let mut sig = [u64::MAX; NUM_PERM];
    for shingle in shingles {
        let bytes = shingle.as_bytes();
        for (perm, slot) in sig.iter_mut().enumerate() {
            let h = hash_for_perm(bytes, perm);
            if h < *slot {
                *slot = h;
            }
        }
    }
    sig
}

/// Estimate Jaccard similarity from two MinHash signatures.
/// Fraction of matching minimums = unbiased estimator of Jaccard.
#[inline]
fn jaccard_from_sigs(a: &[u64; NUM_PERM], b: &[u64; NUM_PERM]) -> f64 {
    a.iter().zip(b.iter()).filter(|(x, y)| x == y).count() as f64 / NUM_PERM as f64
}

// ── LSH index ─────────────────────────────────────────────────────────────────

/// Mix `band_idx` into a u64 bucket key from `r` consecutive signature values.
#[inline]
fn hash_band(values: &[u64], band_idx: usize) -> u64 {
    let mut h: u64 = (band_idx as u64) ^ 0xdeadbeef_cafe1234;
    for &v in values {
        h = h.wrapping_add(v);
        h = h.wrapping_mul(0x100000001b3u64);
        h ^= h >> 33;
    }
    h
}

/// Build an LSH index over `sigs`.
///
/// Returns `NUM_BANDS` hash maps, each mapping a bucket key to the list of
/// indices (into `sigs`) that fell into that bucket.
fn build_lsh_index(sigs: &[[u64; NUM_PERM]]) -> Vec<HashMap<u64, Vec<usize>>> {
    let mut bands: Vec<HashMap<u64, Vec<usize>>> =
        (0..NUM_BANDS).map(|_| HashMap::new()).collect();

    for (idx, sig) in sigs.iter().enumerate() {
        for (band, map) in bands.iter_mut().enumerate() {
            let start = band * ROWS_PER_BAND;
            let bk = hash_band(&sig[start..start + ROWS_PER_BAND], band);
            map.entry(bk).or_default().push(idx);
        }
    }

    bands
}

/// Query the index for all B-side indices that share at least one band bucket
/// with `query_sig`.
fn query_lsh(
    query_sig: &[u64; NUM_PERM],
    bands: &[HashMap<u64, Vec<usize>>],
) -> HashSet<usize> {
    let mut out = HashSet::new();
    for (band, map) in bands.iter().enumerate() {
        let start = band * ROWS_PER_BAND;
        let bk = hash_band(&query_sig[start..start + ROWS_PER_BAND], band);
        if let Some(bucket) = map.get(&bk) {
            out.extend(bucket);
        }
    }
    out
}

// ── name similarity ───────────────────────────────────────────────────────────

/// Filename-only similarity using `similar`'s character-level ratio.
///
/// Mirrors Python's `_name_similarity()` which uses `difflib.SequenceMatcher`
/// on `path.name` (the filename component only, not the full relative path).
fn name_similarity(a: &Path, b: &Path) -> f64 {
    let na =
        a.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let nb =
        b.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    f64::from(similar::TextDiff::from_chars(&na, &nb).ratio())
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn read_text_lossy(path: &Path) -> String {
    std::fs::read(path)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default()
}

// ── public API ────────────────────────────────────────────────────────────────

/// Detect renames between files unique to each side.
///
/// Only code-classified files participate; data and binary files are excluded.
/// Files larger than 5 MB are silently skipped.
/// Each A-side file is matched to at most one B-side file (the one with the
/// highest combined score), mirroring the Python reference.
///
/// Results are sorted descending by `combined_score` and truncated at 200.
pub fn find_renames(
    only_in_a: &HashSet<PathBuf>,
    only_in_b: &HashSet<PathBuf>,
    lookup_a: &HashMap<PathBuf, PathBuf>,
    lookup_b: &HashMap<PathBuf, PathBuf>,
    rename_threshold: f64,
) -> Vec<RenameCandidate> {
    // Build (rel, abs) lists for code-classified files only.
    let a_files: Vec<(PathBuf, PathBuf)> = only_in_a
        .iter()
        .filter_map(|rel| {
            let abs = lookup_a.get(rel)?.clone();
            (classify(&abs).kind == FileKind::Code).then_some((rel.clone(), abs))
        })
        .collect();

    let b_files: Vec<(PathBuf, PathBuf)> = only_in_b
        .iter()
        .filter_map(|rel| {
            let abs = lookup_b.get(rel)?.clone();
            (classify(&abs).kind == FileKind::Code).then_some((rel.clone(), abs))
        })
        .collect();

    if a_files.is_empty() || b_files.is_empty() {
        return Vec::new();
    }

    // ── Compute MinHash signatures for B-side files (parallel) ────────────────

    // Each entry: (original index into b_files, signature).
    // Files that are too large or unreadable yield None and are excluded.
    let b_raw: Vec<Option<[u64; NUM_PERM]>> = b_files
        .par_iter()
        .map(|(_, abs)| {
            if abs.metadata().map(|m| m.len()).unwrap_or(0) > MAX_FILE_BYTES {
                return None; // silently skip large files
            }
            let text = read_text_lossy(abs);
            Some(compute_minhash(&build_shingles(&text, SHINGLE_K)))
        })
        .collect();

    // Flatten into a compact indexed list.
    let b_entries: Vec<(usize, [u64; NUM_PERM])> = b_raw
        .into_iter()
        .enumerate()
        .filter_map(|(i, opt)| opt.map(|sig| (i, sig)))
        .collect();

    if b_entries.is_empty() {
        return Vec::new();
    }

    // ── Build LSH index over B-side signatures ────────────────────────────────

    let b_sigs_only: Vec<[u64; NUM_PERM]> = b_entries.iter().map(|(_, s)| *s).collect();
    let bands = build_lsh_index(&b_sigs_only);

    // ── Query: for each A-side file find the best matching B-side file ─────────

    // `bands` and `b_entries`/`b_files` are read-only, so sharing across rayon
    // threads is safe (all contained types are Sync).
    let mut results: Vec<RenameCandidate> = a_files
        .par_iter()
        .filter_map(|(rel_a, abs_a)| {
            if abs_a.metadata().map(|m| m.len()).unwrap_or(0) > MAX_FILE_BYTES {
                return None;
            }
            let text_a = read_text_lossy(abs_a);
            let sig_a = compute_minhash(&build_shingles(&text_a, SHINGLE_K));

            let candidates = query_lsh(&sig_a, &bands);

            let mut best: Option<RenameCandidate> = None;
            for compact_idx in candidates {
                let (orig_b_idx, b_sig) = &b_entries[compact_idx];
                let content_sim = jaccard_from_sigs(&sig_a, b_sig);
                // Exact Jaccard check — LSH may surface pairs below LSH_THRESHOLD
                // due to the b=64 r=2 banding having ~47% false-positive rate at
                // Jaccard=0.1.
                if content_sim < LSH_THRESHOLD {
                    continue;
                }
                let (rel_b, _) = &b_files[*orig_b_idx];
                let name_sim = name_similarity(rel_a, rel_b);
                let combined = 0.75 * content_sim + 0.25 * name_sim;

                if best.as_ref().is_none_or(|b: &RenameCandidate| combined > b.combined_score) {
                    best = Some(RenameCandidate {
                        from_path: rel_to_str(rel_a),
                        to_path: rel_to_str(rel_b),
                        content_similarity: content_sim,
                        name_similarity: name_sim,
                        combined_score: combined,
                    });
                }
            }

            best.filter(|b| b.combined_score >= rename_threshold)
        })
        .collect();

    // Sort descending by combined score; truncate at maximum reported.
    results.sort_by(|a, b| {
        b.combined_score
            .partial_cmp(&a.combined_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(MAX_RENAMES_REPORTED);
    results
}
