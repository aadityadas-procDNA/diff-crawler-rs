"""File classification: decide how a given file should be diffed.

Categories
----------
code    -> textual unified diff (.py, .ipynb, .yaml, ...)
data    -> DuckDB row+schema diff (.csv, .parquet, .jsonl, ...)
binary  -> hash-only compare (.pkl, .pt, .h5, model weights, images, ...)
ignore  -> excluded entirely (venv, __pycache__, ...)
"""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

# --- Exclusion rules -------------------------------------------------------

# Directory names that are excluded wherever they appear in the path.
DEFAULT_EXCLUDED_DIRS: frozenset[str] = frozenset({
    "venv", ".venv", "env", ".env", "virtualenv",
    "__pycache__",
    ".git", ".hg", ".svn",
    "node_modules",
    ".ipynb_checkpoints",
    ".pytest_cache", ".mypy_cache", ".ruff_cache", ".tox",
    "dist", "build",
    "mlruns", "wandb", "lightning_logs",
    ".idea", ".vscode",
})

# Exact filenames to skip.
DEFAULT_EXCLUDED_FILES: frozenset[str] = frozenset({
    ".DS_Store", "Thumbs.db", ".gitignore.swp",
})

# File suffixes to skip outright.
DEFAULT_EXCLUDED_SUFFIXES: frozenset[str] = frozenset({
    ".pyc", ".pyo", ".pyd",
    ".class",
    ".log", ".tmp", ".swp", ".swo",
})

# --- Type rules ------------------------------------------------------------

CODE_SUFFIXES: frozenset[str] = frozenset({
    ".py", ".ipynb",
    ".js", ".ts", ".tsx", ".jsx",
    ".java", ".kt", ".scala",
    ".c", ".h", ".cpp", ".hpp", ".cc",
    ".go", ".rs", ".rb", ".php",
    ".sh", ".bash", ".zsh",
    ".r", ".jl",
    ".sql",
    ".yaml", ".yml", ".toml", ".ini", ".cfg",
    ".md", ".rst", ".txt",
    ".dockerfile", ".tf",
    ".html", ".css", ".scss",
})

DATA_SUFFIXES: frozenset[str] = frozenset({
    ".csv", ".tsv",
    ".parquet",
    ".jsonl", ".ndjson",
    ".feather", ".arrow",
    ".xlsx", ".xls",
})

# JSON is ambiguous: small -> treat as code/config, large -> treat as data.
JSON_DATA_THRESHOLD_BYTES = 256 * 1024  # 256 KB

# Binary / model weights — hash-compare only.
BINARY_SUFFIXES: frozenset[str] = frozenset({
    ".pkl", ".pickle", ".joblib",
    ".pt", ".pth", ".ckpt", ".safetensors",
    ".h5", ".hdf5",
    ".onnx", ".pb", ".tflite",
    ".npy", ".npz",
    ".png", ".jpg", ".jpeg", ".gif", ".bmp", ".tiff", ".webp", ".svg",
    ".mp3", ".wav", ".flac", ".mp4", ".mov", ".avi",
    ".zip", ".tar", ".gz", ".bz2", ".7z",
    ".bin", ".dat",
})


@dataclass(frozen=True)
class Classification:
    kind: str          # "code" | "data" | "binary" | "ignore"
    reason: str        # short tag for the report


def is_excluded_path(rel_path: Path,
                     excluded_dirs: frozenset[str] = DEFAULT_EXCLUDED_DIRS,
                     excluded_files: frozenset[str] = DEFAULT_EXCLUDED_FILES,
                     excluded_suffixes: frozenset[str] = DEFAULT_EXCLUDED_SUFFIXES) -> bool:
    """Return True if rel_path should be skipped entirely."""
    parts = rel_path.parts
    if any(p in excluded_dirs for p in parts):
        return True
    if rel_path.name in excluded_files:
        return True
    if rel_path.suffix.lower() in excluded_suffixes:
        return True
    # Egg-info dirs use a suffix on a dir name
    if any(p.endswith(".egg-info") for p in parts):
        return True
    return False


def classify(abs_path: Path) -> Classification:
    """Classify a file by extension (and size for ambiguous types)."""
    suffix = abs_path.suffix.lower()

    if suffix == ".json":
        try:
            size = abs_path.stat().st_size
        except OSError:
            size = 0
        if size >= JSON_DATA_THRESHOLD_BYTES:
            return Classification("data", "json-large")
        return Classification("code", "json-config")

    if suffix in DATA_SUFFIXES:
        return Classification("data", f"data:{suffix.lstrip('.')}")

    if suffix in CODE_SUFFIXES:
        return Classification("code", f"code:{suffix.lstrip('.')}")

    if suffix in BINARY_SUFFIXES:
        return Classification("binary", f"binary:{suffix.lstrip('.')}")

    # Files with no extension or unknown extension: peek a few bytes.
    try:
        with open(abs_path, "rb") as f:
            chunk = f.read(2048)
    except OSError:
        return Classification("binary", "unreadable")

    if b"\x00" in chunk:
        return Classification("binary", "binary-sniffed")
    # Heuristic: mostly printable ASCII/UTF-8 -> treat as code
    try:
        chunk.decode("utf-8")
        return Classification("code", "text-sniffed")
    except UnicodeDecodeError:
        return Classification("binary", "non-utf8")
