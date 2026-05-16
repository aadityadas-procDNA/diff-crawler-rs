/// Runtime configuration for a crawl run.
#[derive(Debug, Clone)]
pub struct CrawlConfig {
    pub include_full_unified_diff: bool,
    pub deep_data_stats: bool,
    pub rename_threshold: f64,
    pub max_renames_reported: usize,
}

impl Default for CrawlConfig {
    fn default() -> Self {
        Self {
            include_full_unified_diff: true,
            deep_data_stats: false,
            rename_threshold: 0.4,
            max_renames_reported: 200,
        }
    }
}

// Exclusion sets — match reference/diff_crawler/classifier.py exactly.

pub const EXCLUDED_DIRS: &[&str] = &[
    "venv",
    ".venv",
    "env",
    ".env",
    "virtualenv",
    "__pycache__",
    ".git",
    ".hg",
    ".svn",
    "node_modules",
    ".ipynb_checkpoints",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".tox",
    "dist",
    "build",
    "mlruns",
    "wandb",
    "lightning_logs",
    ".idea",
    ".vscode",
];

pub const EXCLUDED_FILES: &[&str] = &[".DS_Store", "Thumbs.db", ".gitignore.swp"];

/// Suffixes include the leading dot and are matched case-insensitively.
pub const EXCLUDED_SUFFIXES: &[&str] =
    &[".pyc", ".pyo", ".pyd", ".class", ".log", ".tmp", ".swp", ".swo"];
