"""Detect renames / moves with probabilistic content matching.

We MinHash a shingled bag-of-tokens for every text-like file in only_in_a and
only_in_b, push the B-side hashes into an LSH index, then for each A-side file
query the index and pick the highest Jaccard similarity above `threshold`.

Filename similarity (SequenceMatcher) is used as a tie-breaker / boost — a file
renamed `train.py` -> `training.py` with the same content should match more
strongly than a coincidental content overlap with a totally differently named
file.
"""

from __future__ import annotations

import difflib
import re
from dataclasses import dataclass
from pathlib import Path

try:
    from datasketch import MinHash, MinHashLSH  # type: ignore
except ImportError:  # pragma: no cover
    MinHash = None
    MinHashLSH = None

from .classifier import classify

# tunables
NUM_PERM = 128
SHINGLE_SIZE = 5             # k for k-shingles over whitespace-tokenized text
LSH_THRESHOLD = 0.3          # candidate threshold; final score is exact Jaccard
MIN_REPORT_SIMILARITY = 0.4  # don't emit weak matches
MAX_FILE_BYTES = 5 * 1024 * 1024
_TOKEN_RE = re.compile(r"\w+|[^\w\s]")


@dataclass
class RenameCandidate:
    from_path: str          # path that exists only in A
    to_path: str            # path that exists only in B
    content_similarity: float
    name_similarity: float
    combined_score: float


def _shingles(text: str, k: int = SHINGLE_SIZE) -> set[str]:
    tokens = _TOKEN_RE.findall(text)
    if len(tokens) < k:
        return {" ".join(tokens)} if tokens else set()
    return {" ".join(tokens[i:i + k]) for i in range(len(tokens) - k + 1)}


def _minhash_text(text: str) -> "MinHash":
    m = MinHash(num_perm=NUM_PERM)
    for sh in _shingles(text):
        m.update(sh.encode("utf-8"))
    return m


def _read_text_safe(path: Path) -> str | None:
    try:
        if path.stat().st_size > MAX_FILE_BYTES:
            return None
        return path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return None


def _name_similarity(a: str, b: str) -> float:
    return difflib.SequenceMatcher(a=a, b=b, autojunk=False).ratio()


def find_renames(only_in_a: dict[Path, Path],
                 only_in_b: dict[Path, Path],
                 threshold: float = MIN_REPORT_SIMILARITY) -> list[RenameCandidate]:
    """
    Parameters
    ----------
    only_in_a, only_in_b
        Mappings rel_path -> abs_path for files unique to each side.
    threshold
        Minimum combined score for a match to be reported.

    Returns sorted list (highest score first).
    """
    if MinHash is None or MinHashLSH is None:
        return []

    # Build MinHashes for every text-like file on both sides
    def build_hashes(items: dict[Path, Path]) -> dict[Path, "MinHash"]:
        out: dict[Path, MinHash] = {}
        for rel, abs_p in items.items():
            cls = classify(abs_p)
            if cls.kind != "code":
                continue
            text = _read_text_safe(abs_p)
            if not text:
                continue
            out[rel] = _minhash_text(text)
        return out

    a_hashes = build_hashes(only_in_a)
    b_hashes = build_hashes(only_in_b)
    if not a_hashes or not b_hashes:
        return []

    lsh = MinHashLSH(threshold=LSH_THRESHOLD, num_perm=NUM_PERM)
    for rel, mh in b_hashes.items():
        lsh.insert(str(rel), mh)

    rel_b_by_key = {str(rel): rel for rel in b_hashes}
    results: list[RenameCandidate] = []
    for rel_a, mh_a in a_hashes.items():
        best: RenameCandidate | None = None
        for key in lsh.query(mh_a):
            rel_b = rel_b_by_key[key]
            content_sim = mh_a.jaccard(b_hashes[rel_b])
            name_sim = _name_similarity(rel_a.name, rel_b.name)
            # Weighted: content dominates, name nudges
            combined = 0.75 * content_sim + 0.25 * name_sim
            if best is None or combined > best.combined_score:
                best = RenameCandidate(
                    from_path=str(rel_a),
                    to_path=str(rel_b),
                    content_similarity=content_sim,
                    name_similarity=name_sim,
                    combined_score=combined,
                )
        if best and best.combined_score >= threshold:
            results.append(best)

    results.sort(key=lambda r: r.combined_score, reverse=True)
    return results
