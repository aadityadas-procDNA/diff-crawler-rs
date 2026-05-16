"""Unified-diff + line stats + similarity for text/code files."""

from __future__ import annotations

import difflib
import hashlib
from dataclasses import dataclass, field
from pathlib import Path

# Skip producing a full unified diff for very large files (still keep ratio).
MAX_DIFF_BYTES = 2 * 1024 * 1024  # 2 MB per side


@dataclass
class CodeDiffResult:
    path: str
    identical: bool
    added_lines: int = 0
    removed_lines: int = 0
    similarity: float = 1.0           # 0..1, character-level SequenceMatcher ratio
    unified_diff: str = ""            # truncated or empty when too large
    note: str = ""                    # e.g. "skipped: too large"
    error: str | None = None


def _hash_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()


def diff_code(rel_path: Path, abs_a: Path, abs_b: Path,
              include_full_diff: bool = True) -> CodeDiffResult:
    """Compare two code/text files."""
    # Cheap path: identical bytes -> done.
    try:
        if abs_a.stat().st_size == abs_b.stat().st_size:
            if _hash_file(abs_a) == _hash_file(abs_b):
                return CodeDiffResult(path=str(rel_path), identical=True)
    except OSError as e:
        return CodeDiffResult(path=str(rel_path), identical=False, error=f"stat: {e}")

    try:
        size_a = abs_a.stat().st_size
        size_b = abs_b.stat().st_size
    except OSError as e:
        return CodeDiffResult(path=str(rel_path), identical=False, error=str(e))

    if size_a > MAX_DIFF_BYTES or size_b > MAX_DIFF_BYTES:
        return CodeDiffResult(
            path=str(rel_path),
            identical=False,
            note=f"skipped full diff: file too large (>{MAX_DIFF_BYTES} bytes)",
        )

    try:
        text_a = abs_a.read_text(encoding="utf-8", errors="replace")
        text_b = abs_b.read_text(encoding="utf-8", errors="replace")
    except OSError as e:
        return CodeDiffResult(path=str(rel_path), identical=False, error=str(e))

    lines_a = text_a.splitlines(keepends=True)
    lines_b = text_b.splitlines(keepends=True)

    added = 0
    removed = 0
    diff_lines: list[str] = []
    udiff = difflib.unified_diff(
        lines_a, lines_b,
        fromfile=f"a/{rel_path}",
        tofile=f"b/{rel_path}",
        n=3,
    )
    for line in udiff:
        if include_full_diff:
            diff_lines.append(line)
        if line.startswith("+") and not line.startswith("+++"):
            added += 1
        elif line.startswith("-") and not line.startswith("---"):
            removed += 1

    ratio = difflib.SequenceMatcher(a=text_a, b=text_b, autojunk=False).ratio()

    return CodeDiffResult(
        path=str(rel_path),
        identical=(added == 0 and removed == 0),
        added_lines=added,
        removed_lines=removed,
        similarity=ratio,
        unified_diff="".join(diff_lines) if include_full_diff else "",
    )


@dataclass
class BinaryDiffResult:
    path: str
    identical: bool
    size_a: int
    size_b: int
    sha256_a: str
    sha256_b: str


def diff_binary(rel_path: Path, abs_a: Path, abs_b: Path) -> BinaryDiffResult:
    """Hash-only diff for binary / model-weight files."""
    sa = abs_a.stat().st_size
    sb = abs_b.stat().st_size
    ha = _hash_file(abs_a) if sa else ""
    hb = _hash_file(abs_b) if sb else ""
    return BinaryDiffResult(
        path=str(rel_path),
        identical=(ha == hb and sa == sb),
        size_a=sa, size_b=sb,
        sha256_a=ha, sha256_b=hb,
    )
