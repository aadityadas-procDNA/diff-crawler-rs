"""DuckDB-based data diff.

For each pair of "data" files we register both as DuckDB views, then compute:
  - row count on each side and the delta
  - schema on each side (column name -> sql type)
  - columns added / removed / type-changed

We deliberately do NOT compute full row-level set differences here — that gets
expensive and requires a primary key. Two optional extras are computed when
cheap: null-count-per-column and approx distinct count on each side, controlled
by `deep` flag.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path

try:
    import duckdb  # type: ignore
except ImportError:  # pragma: no cover
    duckdb = None  # handled lazily


@dataclass
class ColumnStat:
    nulls: int = 0
    approx_distinct: int = 0


@dataclass
class DataDiffResult:
    path: str
    row_count_a: int = 0
    row_count_b: int = 0
    schema_a: dict[str, str] = field(default_factory=dict)
    schema_b: dict[str, str] = field(default_factory=dict)
    columns_added: list[str] = field(default_factory=list)
    columns_removed: list[str] = field(default_factory=list)
    type_changes: dict[str, tuple[str, str]] = field(default_factory=dict)
    column_stats_a: dict[str, ColumnStat] = field(default_factory=dict)
    column_stats_b: dict[str, ColumnStat] = field(default_factory=dict)
    note: str = ""
    error: str | None = None

    @property
    def row_count_delta(self) -> int:
        return self.row_count_b - self.row_count_a

    @property
    def identical_schema(self) -> bool:
        return (not self.columns_added
                and not self.columns_removed
                and not self.type_changes)


def _reader_expr(path: Path) -> str:
    """Return a DuckDB FROM-clause expression for `path` based on its suffix."""
    p = str(path).replace("'", "''")
    suffix = path.suffix.lower()
    if suffix in {".csv", ".tsv"}:
        return f"read_csv_auto('{p}', sample_size=20000, ignore_errors=true)"
    if suffix == ".parquet":
        return f"read_parquet('{p}')"
    if suffix in {".jsonl", ".ndjson"}:
        return f"read_json_auto('{p}', format='newline_delimited')"
    if suffix == ".json":
        return f"read_json_auto('{p}')"
    if suffix in {".feather", ".arrow"}:
        # DuckDB reads Arrow IPC via the arrow extension; fall back as a hint.
        return f"read_parquet('{p}')"  # caller will catch errors
    if suffix in {".xlsx", ".xls"}:
        return f"st_read('{p}')"  # spatial ext, rarely available - will error
    raise ValueError(f"Unsupported data suffix for DuckDB: {suffix}")


def _get_schema(con, view: str) -> dict[str, str]:
    rows = con.execute(f"DESCRIBE {view}").fetchall()
    # DESCRIBE returns columns: column_name, column_type, null, key, default, extra
    return {r[0]: r[1] for r in rows}


def _get_column_stats(con, view: str, columns: list[str]) -> dict[str, ColumnStat]:
    if not columns:
        return {}
    select_parts = []
    for c in columns:
        quoted = '"' + c.replace('"', '""') + '"'
        select_parts.append(f"SUM(CASE WHEN {quoted} IS NULL THEN 1 ELSE 0 END) AS n_{c}")
        select_parts.append(f"APPROX_COUNT_DISTINCT({quoted}) AS d_{c}")
    sql = f"SELECT {', '.join(select_parts)} FROM {view}"
    row = con.execute(sql).fetchone()
    out: dict[str, ColumnStat] = {}
    for i, c in enumerate(columns):
        nulls = int(row[i * 2] or 0)
        distinct = int(row[i * 2 + 1] or 0)
        out[c] = ColumnStat(nulls=nulls, approx_distinct=distinct)
    return out


def diff_data(rel_path: Path, abs_a: Path, abs_b: Path,
              deep: bool = False) -> DataDiffResult:
    """Compute row count + schema diff for one data file present in both trees."""
    if duckdb is None:
        return DataDiffResult(path=str(rel_path), error="duckdb not installed")

    res = DataDiffResult(path=str(rel_path))
    try:
        expr_a = _reader_expr(abs_a)
        expr_b = _reader_expr(abs_b)
    except ValueError as e:
        res.error = str(e)
        return res

    con = duckdb.connect()
    try:
        con.execute(f"CREATE OR REPLACE VIEW va AS SELECT * FROM {expr_a}")
        con.execute(f"CREATE OR REPLACE VIEW vb AS SELECT * FROM {expr_b}")

        res.schema_a = _get_schema(con, "va")
        res.schema_b = _get_schema(con, "vb")

        res.row_count_a = con.execute("SELECT COUNT(*) FROM va").fetchone()[0]
        res.row_count_b = con.execute("SELECT COUNT(*) FROM vb").fetchone()[0]

        a_cols = set(res.schema_a)
        b_cols = set(res.schema_b)
        res.columns_added = sorted(b_cols - a_cols)
        res.columns_removed = sorted(a_cols - b_cols)
        common = sorted(a_cols & b_cols)
        res.type_changes = {
            c: (res.schema_a[c], res.schema_b[c])
            for c in common if res.schema_a[c] != res.schema_b[c]
        }

        if deep and common:
            try:
                res.column_stats_a = _get_column_stats(con, "va", common)
                res.column_stats_b = _get_column_stats(con, "vb", common)
            except Exception as e:
                res.note = f"deep stats failed: {e}"
    except Exception as e:
        res.error = f"{type(e).__name__}: {e}"
    finally:
        con.close()
    return res
