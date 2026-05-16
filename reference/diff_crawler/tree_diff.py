"""Walk a directory tree and produce a set of relative paths (excluding noise).

The tree diff layer is just set arithmetic on those relative paths:
  in_both       = A ∩ B
  only_in_a     = A − B
  only_in_b     = B − A
"""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path

from .classifier import (
    DEFAULT_EXCLUDED_DIRS, DEFAULT_EXCLUDED_FILES, DEFAULT_EXCLUDED_SUFFIXES,
    is_excluded_path,
)


@dataclass
class TreeIndex:
    """The set of relative file paths under a root, plus per-path absolute paths."""
    root: Path
    files: set[Path] = field(default_factory=set)             # relative Paths
    abs_lookup: dict[Path, Path] = field(default_factory=dict)  # rel -> abs


def walk_tree(root: Path,
              excluded_dirs: frozenset[str] = DEFAULT_EXCLUDED_DIRS,
              excluded_files: frozenset[str] = DEFAULT_EXCLUDED_FILES,
              excluded_suffixes: frozenset[str] = DEFAULT_EXCLUDED_SUFFIXES) -> TreeIndex:
    """Walk `root` recursively and return a TreeIndex of non-excluded files."""
    root = root.resolve()
    if not root.is_dir():
        raise ValueError(f"Not a directory: {root}")

    idx = TreeIndex(root=root)
    for path in root.rglob("*"):
        # Skip symlinks (avoid infinite loops / cross-tree leakage)
        if path.is_symlink():
            continue
        if not path.is_file():
            continue
        rel = path.relative_to(root)
        if is_excluded_path(rel, excluded_dirs, excluded_files, excluded_suffixes):
            continue
        idx.files.add(rel)
        idx.abs_lookup[rel] = path
    return idx


@dataclass
class TreeDiff:
    in_both: set[Path]
    only_in_a: set[Path]
    only_in_b: set[Path]

    @property
    def jaccard(self) -> float:
        union = self.in_both | self.only_in_a | self.only_in_b
        if not union:
            return 1.0
        return len(self.in_both) / len(union)


def diff_trees(a: TreeIndex, b: TreeIndex) -> TreeDiff:
    return TreeDiff(
        in_both=a.files & b.files,
        only_in_a=a.files - b.files,
        only_in_b=b.files - a.files,
    )
