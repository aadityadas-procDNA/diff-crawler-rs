use std::path::Path;

use crate::config::{EXCLUDED_DIRS, EXCLUDED_FILES, EXCLUDED_SUFFIXES};

/// How a file should be diffed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileKind {
    Code,
    Data,
    Binary,
    Ignore,
}

#[derive(Debug, Clone)]
pub struct Classification {
    pub kind: FileKind,
    /// Short tag for diagnostics/logging.
    pub reason: &'static str,
}

// --- Extension sets (match classifier.py exactly) -------------------------

const CODE_SUFFIXES: &[&str] = &[
    ".py",
    ".ipynb",
    ".js",
    ".ts",
    ".tsx",
    ".jsx",
    ".java",
    ".kt",
    ".scala",
    ".c",
    ".h",
    ".cpp",
    ".hpp",
    ".cc",
    ".go",
    ".rs",
    ".rb",
    ".php",
    ".sh",
    ".bash",
    ".zsh",
    ".r",
    ".jl",
    ".sql",
    ".yaml",
    ".yml",
    ".toml",
    ".ini",
    ".cfg",
    ".md",
    ".rst",
    ".txt",
    ".dockerfile",
    ".tf",
    ".html",
    ".css",
    ".scss",
];

const DATA_SUFFIXES: &[&str] = &[
    ".csv", ".tsv", ".parquet", ".jsonl", ".ndjson", ".feather", ".arrow", ".xlsx", ".xls",
];

const BINARY_SUFFIXES: &[&str] = &[
    ".pkl",
    ".pickle",
    ".joblib",
    ".pt",
    ".pth",
    ".ckpt",
    ".safetensors",
    ".h5",
    ".hdf5",
    ".onnx",
    ".pb",
    ".tflite",
    ".npy",
    ".npz",
    ".png",
    ".jpg",
    ".jpeg",
    ".gif",
    ".bmp",
    ".tiff",
    ".webp",
    ".svg",
    ".mp3",
    ".wav",
    ".flac",
    ".mp4",
    ".mov",
    ".avi",
    ".zip",
    ".tar",
    ".gz",
    ".bz2",
    ".7z",
    ".bin",
    ".dat",
];

/// JSON files below this size are treated as code/config; at or above, as data.
const JSON_DATA_THRESHOLD_BYTES: u64 = 256 * 1024;

// --------------------------------------------------------------------------

/// Returns true if `rel` (a path relative to a tree root) should be skipped
/// entirely — not walked and not diffed.
///
/// Mirrors `is_excluded_path` in classifier.py. Called at walk time; the walker
/// also prevents descending into excluded directories via `filter_entry`, so in
/// practice this function acts as the per-file gate for suffix and filename checks.
pub fn is_excluded_path(rel: &Path) -> bool {
    for component in rel.components() {
        let s = component.as_os_str().to_string_lossy();
        if EXCLUDED_DIRS.contains(&s.as_ref()) {
            return true;
        }
        if s.ends_with(".egg-info") {
            return true;
        }
    }
    if let Some(name) = rel.file_name() {
        let name_str = name.to_string_lossy();
        if EXCLUDED_FILES.contains(&name_str.as_ref()) {
            return true;
        }
    }
    if let Some(ext) = rel.extension() {
        let suffix = format!(".{}", ext.to_string_lossy().to_lowercase());
        if EXCLUDED_SUFFIXES.contains(&suffix.as_str()) {
            return true;
        }
    }
    false
}

/// Classify a file by extension (and size for ambiguous types like `.json`).
///
/// Mirrors `classify()` in classifier.py. The sniffing fallback (null-byte /
/// UTF-8 probe) is used for files with no known extension.
pub fn classify(abs_path: &Path) -> Classification {
    let suffix = abs_path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
        .unwrap_or_default();

    if suffix == ".json" {
        let size = abs_path.metadata().map(|m| m.len()).unwrap_or(0);
        return if size >= JSON_DATA_THRESHOLD_BYTES {
            Classification {
                kind: FileKind::Data,
                reason: "json-large",
            }
        } else {
            Classification {
                kind: FileKind::Code,
                reason: "json-config",
            }
        };
    }

    if DATA_SUFFIXES.contains(&suffix.as_str()) {
        return Classification {
            kind: FileKind::Data,
            reason: "data",
        };
    }
    if CODE_SUFFIXES.contains(&suffix.as_str()) {
        return Classification {
            kind: FileKind::Code,
            reason: "code",
        };
    }
    if BINARY_SUFFIXES.contains(&suffix.as_str()) {
        return Classification {
            kind: FileKind::Binary,
            reason: "binary",
        };
    }

    // Unknown extension: sniff the first 2 KB.
    match sniff(abs_path) {
        Sniff::Binary => Classification {
            kind: FileKind::Binary,
            reason: "binary-sniffed",
        },
        Sniff::Code => Classification {
            kind: FileKind::Code,
            reason: "text-sniffed",
        },
        Sniff::Unreadable => Classification {
            kind: FileKind::Binary,
            reason: "unreadable",
        },
    }
}

enum Sniff {
    Binary,
    Code,
    Unreadable,
}

fn sniff(path: &Path) -> Sniff {
    use std::io::Read;
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Sniff::Unreadable,
    };
    let mut buf = [0u8; 2048];
    let n = match f.read(&mut buf) {
        Ok(n) => n,
        Err(_) => return Sniff::Unreadable,
    };
    let chunk = &buf[..n];
    if chunk.contains(&0u8) {
        return Sniff::Binary;
    }
    match std::str::from_utf8(chunk) {
        Ok(_) => Sniff::Code,
        Err(_) => Sniff::Binary,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn py_is_code() {
        let c = classify(Path::new("foo.py"));
        assert_eq!(c.kind, FileKind::Code);
    }

    #[test]
    fn parquet_is_data() {
        let c = classify(Path::new("data.parquet"));
        assert_eq!(c.kind, FileKind::Data);
    }

    #[test]
    fn pkl_is_binary() {
        let c = classify(Path::new("model.pkl"));
        assert_eq!(c.kind, FileKind::Binary);
    }

    #[test]
    fn exclude_venv_component() {
        assert!(is_excluded_path(Path::new("venv/lib/site.py")));
    }

    #[test]
    fn exclude_egg_info() {
        assert!(is_excluded_path(Path::new("mypackage.egg-info/PKG-INFO")));
    }

    #[test]
    fn exclude_pyc_suffix() {
        assert!(is_excluded_path(Path::new("foo.pyc")));
    }

    #[test]
    fn keep_normal_file() {
        assert!(!is_excluded_path(Path::new("src/main.py")));
    }

    #[test]
    fn keep_dotfile_without_excluded_suffix() {
        // .gitignore has no extension via Path::extension(), so not excluded by suffix rule.
        assert!(!is_excluded_path(Path::new(".gitignore")));
    }
}
