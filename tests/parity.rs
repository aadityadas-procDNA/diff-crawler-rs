/// Structural parity tests: run the Python reference and the Rust binary on
/// the same fixture pair, then compare tree / code / binary outputs.
///
/// Tests self-skip when Python or the reference package is unavailable —
/// they gate CI merges but stay green on machines without a Python environment.
///
/// What we compare (and why):
///   tree.*              — pure set arithmetic, must be exact
///   code_diffs.*        — identical/added/removed exact; similarity within 2%
///                         (Python: Ratcliff-Obershelp via difflib.SequenceMatcher;
///                          Rust: Myers via `similar` — both LCS-based but can
///                          differ by a few matched chars on complex inputs)
///   binary_diffs.*      — SHA-256 + size, byte-exact (same algorithm both sides)
///   summary.tree        — counts exact, jaccard within 1e-6
///   summary.code        — counts exact, avg_similarity within 2%
///   summary.binary      — counts exact
///   overall_similarity  — within 2% (derived from tree_jaccard + avg_code_sim)
///
/// Intentionally skipped:
///   unified_diff        — different library formatting (difflib vs similar)
///   data_diffs          — Python uses DuckDB types; Rust uses simplified inference
///   rename_candidates   — Python returns [] without `datasketch` installed
///   dir_a / dir_b       — different path-separator conventions on Windows
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use tempfile::TempDir;

// ── constants ─────────────────────────────────────────────────────────────────

/// Maximum allowed absolute difference in similarity scores.
/// Python uses Ratcliff-Obershelp (difflib); Rust uses Myers (similar).
/// Empirically verified on the fixture: train.py diff gives python=0.9118,
/// Rust result stays within this band.
const SIM_TOL: f64 = 0.02;

// ── environment discovery ─────────────────────────────────────────────────────

fn rust_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_diff-crawler"))
}

fn reference_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("reference")
}

/// Return a Python interpreter name that can import the reference package,
/// or `None` to signal callers to skip.
fn python_cmd() -> Option<String> {
    let ref_dir = reference_dir().to_string_lossy().into_owned();
    for &py in &["python", "python3"] {
        let ok = Command::new(py)
            .env("PYTHONPATH", &ref_dir)
            .args([
                "-c",
                "from diff_crawler.crawler import DiffCrawler; print('ok')",
            ])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if ok {
            return Some(py.to_string());
        }
    }
    None
}

// ── fixture ───────────────────────────────────────────────────────────────────

/// Build the parity fixture pair in two temp dirs.
///
/// All files live at the root (no subdirs) to avoid path-separator divergence
/// between Python's `str(WindowsPath)` (backslashes) and Rust (forward slashes).
///
/// Expected diff outputs:
///   code_diffs:   model.py (identical=true), train.py (identical=false, +3/-1)
///   binary_diffs: weights.pkl (identical=true), checkpoint.pkl (identical=false)
///   tree.only_in_a: [utils.py]
///   tree.only_in_b: [helpers.py]
fn make_fixture() -> (TempDir, TempDir) {
    let dir_a = TempDir::new().unwrap();
    let dir_b = TempDir::new().unwrap();

    let model_code = "\
import numpy as np


class NeuralNetwork:
    def __init__(self, input_size, hidden_size, output_size):
        self.w1 = np.random.randn(input_size, hidden_size) * 0.01
        self.w2 = np.random.randn(hidden_size, output_size) * 0.01
        self.b1 = np.zeros((1, hidden_size))
        self.b2 = np.zeros((1, output_size))

    def forward(self, x):
        self.z1 = np.dot(x, self.w1) + self.b1
        self.a1 = np.tanh(self.z1)
        self.z2 = np.dot(self.a1, self.w2) + self.b2
        return self.z2

    def backward(self, x, y, lr=0.01):
        m = x.shape[0]
        dz2 = self.z2 - y
        dw2 = np.dot(self.a1.T, dz2) / m
        db2 = np.sum(dz2, axis=0, keepdims=True) / m
        da1 = np.dot(dz2, self.w2.T)
        dz1 = da1 * (1 - np.tanh(self.z1) ** 2)
        dw1 = np.dot(x.T, dz1) / m
        db1 = np.sum(dz1, axis=0, keepdims=True) / m
        self.w1 -= lr * dw1
        self.w2 -= lr * dw2
        self.b1 -= lr * db1
        self.b2 -= lr * db2
";

    // Verified with Python: added=3, removed=1, similarity≈0.9118
    let train_a = "\
from model import NeuralNetwork
import numpy as np


def train(model, X, y, epochs=10, lr=0.01):
    losses = []
    for epoch in range(epochs):
        output = model.forward(X)
        loss = np.mean((output - y) ** 2)
        losses.append(loss)
        model.backward(X, y, lr=lr)
    return losses


def evaluate(model, X, y):
    output = model.forward(X)
    mse = np.mean((output - y) ** 2)
    return {\"mse\": float(mse)}
";

    let train_b = "\
from model import NeuralNetwork
import numpy as np


def train(model, X, y, epochs=20, lr=0.001):
    losses = []
    for epoch in range(epochs):
        output = model.forward(X)
        loss = np.mean((output - y) ** 2)
        losses.append(loss)
        model.backward(X, y, lr=lr)
        if epoch % 5 == 0:
            print(f\"Epoch {epoch}: loss={loss:.4f}\")
    return losses


def evaluate(model, X, y):
    output = model.forward(X)
    mse = np.mean((output - y) ** 2)
    return {\"mse\": float(mse)}
";

    let only_a = "# Deprecated utilities\ndef deprecated_normalize(x):\n    return x / x.max()\n";
    let only_b = "# New helper utilities\ndef normalize(x, eps=1e-8):\n    return (x - x.mean()) / (x.std() + eps)\n";

    // .pkl → classified as binary
    let weights: Vec<u8> = (0u8..=127).cycle().take(512).collect();
    let ckpt_a: Vec<u8> = (0u8..=255).cycle().take(512).collect();
    let ckpt_b: Vec<u8> = (0u8..=255).rev().cycle().take(512).collect();

    for (name, content) in [
        ("model.py", model_code.as_bytes()),
        ("train.py", train_a.as_bytes()),
        ("utils.py", only_a.as_bytes()),
    ] {
        std::fs::write(dir_a.path().join(name), content).unwrap();
    }
    std::fs::write(dir_a.path().join("weights.pkl"), &weights).unwrap();
    std::fs::write(dir_a.path().join("checkpoint.pkl"), &ckpt_a).unwrap();

    for (name, content) in [
        ("model.py", model_code.as_bytes()),
        ("train.py", train_b.as_bytes()),
        ("helpers.py", only_b.as_bytes()),
    ] {
        std::fs::write(dir_b.path().join(name), content).unwrap();
    }
    std::fs::write(dir_b.path().join("weights.pkl"), &weights).unwrap();
    std::fs::write(dir_b.path().join("checkpoint.pkl"), &ckpt_b).unwrap();

    (dir_a, dir_b)
}

// ── runners ───────────────────────────────────────────────────────────────────

fn run_rust_json(dir_a: &Path, dir_b: &Path, json_path: &Path) -> Value {
    let out = Command::new(rust_binary())
        .args([
            dir_a.to_str().unwrap(),
            dir_b.to_str().unwrap(),
            "--no-diff",
            "--json",
            json_path.to_str().unwrap(),
        ])
        .output()
        .expect("failed to spawn rust binary");
    assert!(
        out.status.success(),
        "rust binary failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let text = std::fs::read_to_string(json_path)
        .unwrap_or_else(|e| panic!("rust json missing at {}: {e}", json_path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("rust json invalid: {e}"))
}

fn run_python_json(py: &str, dir_a: &Path, dir_b: &Path, json_path: &Path) -> Value {
    let ref_dir = reference_dir().to_string_lossy().into_owned();
    let out = Command::new(py)
        .env("PYTHONPATH", &ref_dir)
        .args([
            "-m",
            "diff_crawler",
            dir_a.to_str().unwrap(),
            dir_b.to_str().unwrap(),
            "--no-diff",
            "--json",
            json_path.to_str().unwrap(),
        ])
        .output()
        .expect("failed to spawn python reference");
    assert!(
        out.status.success(),
        "python reference failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let text = std::fs::read_to_string(json_path)
        .unwrap_or_else(|e| panic!("python json missing at {}: {e}", json_path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("python json invalid: {e}"))
}

/// Build both outputs from a fresh fixture in one call.
/// The fixture temp dirs are live for the duration of this function; the
/// returned Values are owned and independent of the filesystem.
fn both_outputs(py: &str) -> (Value, Value) {
    let (dir_a, dir_b) = make_fixture();
    let tmp = TempDir::new().unwrap();
    let rust_path = tmp.path().join("rust.json");
    let py_path = tmp.path().join("py.json");
    let rust_val = run_rust_json(dir_a.path(), dir_b.path(), &rust_path);
    let py_val = run_python_json(py, dir_a.path(), dir_b.path(), &py_path);
    (rust_val, py_val)
}

// ── comparison utilities ──────────────────────────────────────────────────────

fn norm(s: &str) -> String {
    s.replace('\\', "/")
}

fn sorted_paths(arr: &Value) -> Vec<String> {
    let mut v: Vec<String> = arr
        .as_array()
        .expect("expected JSON array of paths")
        .iter()
        .map(|x| norm(x.as_str().expect("path must be string")))
        .collect();
    v.sort();
    v
}

fn approx_eq(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

fn by_path(arr: &Value) -> HashMap<String, &Value> {
    arr.as_array()
        .expect("expected array")
        .iter()
        .map(|v| (norm(v["path"].as_str().expect("path field missing")), v))
        .collect()
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[test]
fn parity_tree_sets() {
    let py = match python_cmd() {
        Some(p) => p,
        None => {
            eprintln!("SKIP parity_tree_sets: Python unavailable");
            return;
        }
    };
    let (rust, python) = both_outputs(&py);

    for key in &["in_both", "only_in_a", "only_in_b"] {
        let r = sorted_paths(&rust["tree"][key]);
        let p = sorted_paths(&python["tree"][key]);
        assert_eq!(r, p, "tree.{key}\n  rust:   {r:?}\n  python: {p:?}");
    }
}

#[test]
fn parity_code_diffs() {
    let py = match python_cmd() {
        Some(p) => p,
        None => {
            eprintln!("SKIP parity_code_diffs: Python unavailable");
            return;
        }
    };
    let (rust, python) = both_outputs(&py);

    let r_map = by_path(&rust["code_diffs"]);
    let p_map = by_path(&python["code_diffs"]);

    assert_eq!(
        r_map.len(),
        p_map.len(),
        "code_diffs length  rust={}  python={}",
        r_map.len(),
        p_map.len(),
    );

    for (path, r) in &r_map {
        let p = p_map
            .get(path)
            .unwrap_or_else(|| panic!("rust has code diff for '{path}'; python does not"));

        let r_id = r["identical"].as_bool().unwrap();
        let p_id = p["identical"].as_bool().unwrap();
        assert_eq!(r_id, p_id, "{path}: identical  rust={r_id}  python={p_id}");

        let r_add = r["added_lines"].as_u64().unwrap();
        let p_add = p["added_lines"].as_u64().unwrap();
        assert_eq!(
            r_add, p_add,
            "{path}: added_lines  rust={r_add}  python={p_add}"
        );

        let r_rem = r["removed_lines"].as_u64().unwrap();
        let p_rem = p["removed_lines"].as_u64().unwrap();
        assert_eq!(
            r_rem, p_rem,
            "{path}: removed_lines  rust={r_rem}  python={p_rem}"
        );

        let r_sim = r["similarity"].as_f64().unwrap();
        let p_sim = p["similarity"].as_f64().unwrap();
        assert!(
            approx_eq(r_sim, p_sim, SIM_TOL),
            "{path}: similarity  rust={r_sim:.4}  python={p_sim:.4}  tol={SIM_TOL}",
        );
    }
}

#[test]
fn parity_binary_diffs() {
    let py = match python_cmd() {
        Some(p) => p,
        None => {
            eprintln!("SKIP parity_binary_diffs: Python unavailable");
            return;
        }
    };
    let (rust, python) = both_outputs(&py);

    let r_map = by_path(&rust["binary_diffs"]);
    let p_map = by_path(&python["binary_diffs"]);

    assert_eq!(
        r_map.len(),
        p_map.len(),
        "binary_diffs length  rust={}  python={}",
        r_map.len(),
        p_map.len(),
    );

    for (path, r) in &r_map {
        let p = p_map
            .get(path)
            .unwrap_or_else(|| panic!("rust has binary diff for '{path}'; python does not"));

        for field in &["identical", "size_a", "size_b", "sha256_a", "sha256_b"] {
            assert_eq!(
                r[field], p[field],
                "{path}: '{field}'  rust={}  python={}",
                r[field], p[field],
            );
        }
    }
}

#[test]
fn parity_summary_tree() {
    let py = match python_cmd() {
        Some(p) => p,
        None => {
            eprintln!("SKIP parity_summary_tree: Python unavailable");
            return;
        }
    };
    let (rust, python) = both_outputs(&py);

    let r = &rust["summary"]["tree"];
    let p = &python["summary"]["tree"];

    for field in &["files_in_both", "only_in_a", "only_in_b"] {
        assert_eq!(
            r[field], p[field],
            "summary.tree.{field}  rust={}  python={}",
            r[field], p[field]
        );
    }
    let r_jac = r["tree_jaccard"].as_f64().unwrap();
    let p_jac = p["tree_jaccard"].as_f64().unwrap();
    assert!(
        approx_eq(r_jac, p_jac, 1e-6),
        "summary.tree.tree_jaccard  rust={r_jac}  python={p_jac}"
    );
}

#[test]
fn parity_summary_code() {
    let py = match python_cmd() {
        Some(p) => p,
        None => {
            eprintln!("SKIP parity_summary_code: Python unavailable");
            return;
        }
    };
    let (rust, python) = both_outputs(&py);

    let r = &rust["summary"]["code"];
    let p = &python["summary"]["code"];

    for field in &[
        "compared",
        "identical",
        "total_added_lines",
        "total_removed_lines",
    ] {
        assert_eq!(
            r[field], p[field],
            "summary.code.{field}  rust={}  python={}",
            r[field], p[field]
        );
    }
    let r_sim = r["avg_similarity"].as_f64().unwrap();
    let p_sim = p["avg_similarity"].as_f64().unwrap();
    assert!(
        approx_eq(r_sim, p_sim, SIM_TOL),
        "summary.code.avg_similarity  rust={r_sim:.4}  python={p_sim:.4}  tol={SIM_TOL}",
    );
}

#[test]
fn parity_summary_binary() {
    let py = match python_cmd() {
        Some(p) => p,
        None => {
            eprintln!("SKIP parity_summary_binary: Python unavailable");
            return;
        }
    };
    let (rust, python) = both_outputs(&py);

    let r = &rust["summary"]["binary"];
    let p = &python["summary"]["binary"];

    for field in &["compared", "identical"] {
        assert_eq!(
            r[field], p[field],
            "summary.binary.{field}  rust={}  python={}",
            r[field], p[field]
        );
    }
}

#[test]
fn parity_overall_similarity() {
    let py = match python_cmd() {
        Some(p) => p,
        None => {
            eprintln!("SKIP parity_overall_similarity: Python unavailable");
            return;
        }
    };
    let (rust, python) = both_outputs(&py);

    let r = rust["summary"]["overall_similarity"].as_f64().unwrap();
    let p = python["summary"]["overall_similarity"].as_f64().unwrap();
    assert!(
        approx_eq(r, p, SIM_TOL),
        "overall_similarity  rust={r:.4}  python={p:.4}  tol={SIM_TOL}",
    );
}
