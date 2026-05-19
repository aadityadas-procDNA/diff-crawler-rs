/// Pure-Rust data diff — no DuckDB.
///
/// Mirrors the semantics of `diff_data()` in reference/diff_crawler/data_diff.py
/// using lightweight crates instead of DuckDB.
///
/// # What differs from the Python reference
///
/// - `.feather`, `.arrow`, `.xlsx`, `.xls` are not supported; those paths return
///   an error result (Python used DuckDB extensions that were "rarely available"
///   anyway for Excel).
/// - `approx_distinct` for Parquet deep stats is always `0`; Parquet null counts
///   are read from row-group metadata statistics (present in most writers).
/// - CSV type inference is single-pass with four types only: `BIGINT`, `DOUBLE`,
///   `BOOLEAN`, `VARCHAR`. DuckDB does multi-pass sampling and also detects
///   `DATE`, `TIMESTAMP`, `HUGEINT`, etc.
/// - Parquet logical types are mapped to SQL-like strings that match DuckDB for
///   the common cases but may differ on exotic annotations.
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde::Serialize;

use crate::code_diff::rel_to_str;

// ── result types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct ColumnStat {
    pub nulls: i64,
    pub approx_distinct: i64,
}

/// JSON shape mirrors Python's DataDiffResult dataclass.
///
/// Note: `row_count_delta` and `identical_schema` are `@property` in the Python
/// dataclass — NOT serialized to JSON — so they are absent here too.
///
/// `type_changes` serializes as `{"col": ["OLD_TYPE", "NEW_TYPE"]}` via the
/// `[String; 2]` array, matching Python's tuple-in-dict JSON encoding.
#[derive(Debug, Clone, Serialize)]
pub struct DataDiffResult {
    pub path: String,
    pub row_count_a: i64,
    pub row_count_b: i64,
    /// Column name → SQL-like type string (alphabetical order via BTreeMap).
    pub schema_a: BTreeMap<String, String>,
    pub schema_b: BTreeMap<String, String>,
    pub columns_added: Vec<String>,
    pub columns_removed: Vec<String>,
    /// `{"col": ["OLD_TYPE", "NEW_TYPE"]}` — alphabetical via BTreeMap.
    pub type_changes: BTreeMap<String, [String; 2]>,
    pub column_stats_a: BTreeMap<String, ColumnStat>,
    pub column_stats_b: BTreeMap<String, ColumnStat>,
    pub note: String,
    pub error: Option<String>,
}

impl DataDiffResult {
    fn new(path: String) -> Self {
        Self {
            path,
            row_count_a: 0,
            row_count_b: 0,
            schema_a: BTreeMap::new(),
            schema_b: BTreeMap::new(),
            columns_added: Vec::new(),
            columns_removed: Vec::new(),
            type_changes: BTreeMap::new(),
            column_stats_a: BTreeMap::new(),
            column_stats_b: BTreeMap::new(),
            note: String::new(),
            error: None,
        }
    }

    fn with_error(path: String, msg: impl std::fmt::Display) -> Self {
        let mut r = Self::new(path);
        r.error = Some(msg.to_string());
        r
    }
}

// ── internal intermediate ─────────────────────────────────────────────────────

struct FileStats {
    row_count: i64,
    schema: BTreeMap<String, String>,
    /// Only populated when `deep = true`.
    column_stats: BTreeMap<String, ColumnStat>,
}

// ── type inference for text formats ──────────────────────────────────────────

/// Accumulates values for one CSV/JSONL column to infer its SQL type.
struct TypeTracker {
    any_seen: bool,
    still_bigint: bool,
    still_double: bool,
    still_bool: bool,
}

impl TypeTracker {
    fn new() -> Self {
        Self {
            any_seen: false,
            still_bigint: true,
            still_double: true,
            still_bool: true,
        }
    }

    fn observe(&mut self, s: &str) {
        if s.is_empty() {
            return; // treat empty as null — don't downgrade type
        }
        self.any_seen = true;
        if self.still_bigint && s.parse::<i64>().is_err() {
            self.still_bigint = false;
        }
        if self.still_double && s.parse::<f64>().is_err() {
            self.still_double = false;
        }
        if self.still_bool {
            match s.to_ascii_lowercase().as_str() {
                "true" | "false" | "1" | "0" | "yes" | "no" => {}
                _ => self.still_bool = false,
            }
        }
    }

    fn infer(&self) -> &'static str {
        if !self.any_seen {
            return "VARCHAR";
        }
        if self.still_bool {
            return "BOOLEAN";
        }
        if self.still_bigint {
            return "BIGINT";
        }
        if self.still_double {
            return "DOUBLE";
        }
        "VARCHAR"
    }
}

// ── CSV / TSV ─────────────────────────────────────────────────────────────────

fn read_csv(path: &Path, deep: bool) -> Result<FileStats, String> {
    let mut rdr = csv::ReaderBuilder::new()
        .flexible(true)
        .from_path(path)
        .map_err(|e| e.to_string())?;

    let headers = rdr.headers().map_err(|e| e.to_string())?.clone();
    let col_names: Vec<String> = headers.iter().map(String::from).collect();
    let ncols = col_names.len();

    let mut trackers: Vec<TypeTracker> = (0..ncols).map(|_| TypeTracker::new()).collect();
    let mut nulls: Vec<i64> = vec![0; ncols];
    // Cap per-column distinct sets at 100_000 entries to bound memory.
    let mut distinct: Vec<HashSet<String>> = if deep {
        (0..ncols).map(|_| HashSet::new()).collect()
    } else {
        Vec::new()
    };
    let mut row_count = 0i64;

    for result in rdr.records() {
        let record = result.map_err(|e| e.to_string())?;
        row_count += 1;
        for (i, field) in record.iter().enumerate() {
            if i >= ncols {
                continue;
            }
            if field.is_empty() {
                nulls[i] += 1;
            } else {
                trackers[i].observe(field);
                if deep && distinct[i].len() < 100_000 {
                    distinct[i].insert(field.to_string());
                }
            }
        }
    }

    let schema: BTreeMap<String, String> = col_names
        .iter()
        .enumerate()
        .map(|(i, name)| (name.clone(), trackers[i].infer().to_string()))
        .collect();

    let column_stats = if deep {
        col_names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                (
                    name.clone(),
                    ColumnStat {
                        nulls: nulls[i],
                        approx_distinct: distinct[i].len() as i64,
                    },
                )
            })
            .collect()
    } else {
        BTreeMap::new()
    };

    Ok(FileStats {
        row_count,
        schema,
        column_stats,
    })
}

// ── JSONL / NDJSON ────────────────────────────────────────────────────────────

fn json_type_str(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Bool(_) => "BOOLEAN",
        serde_json::Value::Number(n) => {
            if n.is_f64() {
                "DOUBLE"
            } else {
                "BIGINT"
            }
        }
        serde_json::Value::String(_) => "VARCHAR",
        serde_json::Value::Array(_) => "JSON",
        serde_json::Value::Object(_) => "JSON",
        serde_json::Value::Null => "VARCHAR", // unknown until a non-null seen
    }
}

fn read_jsonl(path: &Path, deep: bool) -> Result<FileStats, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let reader = BufReader::new(file);

    // col → (type_so_far, null_count, distinct_set)
    let mut col_info: BTreeMap<String, (String, i64, HashSet<String>)> = BTreeMap::new();
    let mut row_count = 0i64;

    for line in reader.lines() {
        let line = line.map_err(|e| e.to_string())?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let v: serde_json::Value =
            serde_json::from_str(trimmed).map_err(|e| format!("JSON parse: {e}"))?;
        let Some(obj) = v.as_object() else {
            continue; // skip non-object lines
        };
        row_count += 1;

        for (k, val) in obj {
            let entry = col_info
                .entry(k.clone())
                .or_insert_with(|| (String::from("VARCHAR"), 0, HashSet::new()));

            if val.is_null() {
                entry.1 += 1;
            } else {
                // Upgrade type: VARCHAR → more specific, but never downgrade
                let observed = json_type_str(val);
                if entry.0 == "VARCHAR" && observed != "VARCHAR" {
                    entry.0 = observed.to_string();
                }
                if deep && entry.2.len() < 100_000 {
                    entry.2.insert(val.to_string());
                }
            }
        }
    }

    let schema: BTreeMap<String, String> = col_info
        .iter()
        .map(|(k, (t, _, _))| (k.clone(), t.clone()))
        .collect();

    let column_stats = if deep {
        col_info
            .iter()
            .map(|(k, (_, nulls, distinct))| {
                (
                    k.clone(),
                    ColumnStat {
                        nulls: *nulls,
                        approx_distinct: distinct.len() as i64,
                    },
                )
            })
            .collect()
    } else {
        BTreeMap::new()
    };

    Ok(FileStats {
        row_count,
        schema,
        column_stats,
    })
}

// ── large JSON (top-level array of objects) ───────────────────────────────────

fn read_json(path: &Path, deep: bool) -> Result<FileStats, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let v: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("JSON parse: {e}"))?;
    let arr = match &v {
        serde_json::Value::Array(a) => a,
        _ => return Err("large JSON is not a top-level array; skipping data diff".to_string()),
    };

    let mut col_info: BTreeMap<String, (String, i64, HashSet<String>)> = BTreeMap::new();

    for item in arr {
        let Some(obj) = item.as_object() else {
            continue;
        };
        for (k, val) in obj {
            let entry = col_info
                .entry(k.clone())
                .or_insert_with(|| (String::from("VARCHAR"), 0, HashSet::new()));
            if val.is_null() {
                entry.1 += 1;
            } else {
                let observed = json_type_str(val);
                if entry.0 == "VARCHAR" && observed != "VARCHAR" {
                    entry.0 = observed.to_string();
                }
                if deep && entry.2.len() < 100_000 {
                    entry.2.insert(val.to_string());
                }
            }
        }
    }

    let row_count = arr.len() as i64;
    let schema: BTreeMap<String, String> = col_info
        .iter()
        .map(|(k, (t, _, _))| (k.clone(), t.clone()))
        .collect();
    let column_stats = if deep {
        col_info
            .iter()
            .map(|(k, (_, nulls, distinct))| {
                (
                    k.clone(),
                    ColumnStat {
                        nulls: *nulls,
                        approx_distinct: distinct.len() as i64,
                    },
                )
            })
            .collect()
    } else {
        BTreeMap::new()
    };

    Ok(FileStats {
        row_count,
        schema,
        column_stats,
    })
}

// ── Parquet ───────────────────────────────────────────────────────────────────

fn parquet_physical_type_str(col: &parquet::schema::types::ColumnDescriptor) -> String {
    use parquet::basic::{ConvertedType, Type as PhysicalType};

    match col.physical_type() {
        PhysicalType::BOOLEAN => "BOOLEAN".to_string(),
        PhysicalType::INT32 => match col.converted_type() {
            ConvertedType::DATE => "DATE".to_string(),
            ConvertedType::TIME_MILLIS => "TIME".to_string(),
            _ => "INTEGER".to_string(),
        },
        PhysicalType::INT64 => match col.converted_type() {
            ConvertedType::TIMESTAMP_MILLIS | ConvertedType::TIMESTAMP_MICROS => {
                "TIMESTAMP".to_string()
            }
            _ => "BIGINT".to_string(),
        },
        PhysicalType::INT96 => "TIMESTAMP".to_string(), // legacy timestamp
        PhysicalType::FLOAT => "FLOAT".to_string(),
        PhysicalType::DOUBLE => "DOUBLE".to_string(),
        PhysicalType::BYTE_ARRAY => match col.converted_type() {
            ConvertedType::UTF8 | ConvertedType::ENUM | ConvertedType::JSON => {
                "VARCHAR".to_string()
            }
            _ => "BLOB".to_string(),
        },
        PhysicalType::FIXED_LEN_BYTE_ARRAY => match col.converted_type() {
            ConvertedType::DECIMAL => "DECIMAL".to_string(),
            _ => "BLOB".to_string(),
        },
    }
}

fn read_parquet(path: &Path, deep: bool) -> Result<FileStats, String> {
    use parquet::file::reader::{FileReader, SerializedFileReader};

    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let reader = SerializedFileReader::new(file).map_err(|e| e.to_string())?;
    let meta = reader.metadata();

    // Row count from row-group metadata — no data pages needed.
    let row_count: i64 = meta.row_groups().iter().map(|rg| rg.num_rows()).sum();

    // Schema
    let schema_descr = meta.file_metadata().schema_descr();
    let mut schema = BTreeMap::new();
    for i in 0..schema_descr.num_columns() {
        let col = schema_descr.column(i);
        schema.insert(col.name().to_string(), parquet_physical_type_str(&col));
    }

    // Deep stats: null counts from row-group column statistics.
    // approx_distinct is 0 — computing it requires reading data pages.
    let column_stats = if deep {
        let mut nulls_per_col: BTreeMap<String, i64> =
            schema.keys().map(|k| (k.clone(), 0i64)).collect();

        for rg in meta.row_groups() {
            for col_idx in 0..rg.num_columns() {
                let col_chunk = rg.column(col_idx);
                let col_name = schema_descr.column(col_idx).name().to_string();
                if let Some(stats) = col_chunk.statistics() {
                    use parquet::file::statistics::Statistics;
                    let null_count = match stats {
                        Statistics::Boolean(s) => s.null_count_opt().unwrap_or(0) as i64,
                        Statistics::Int32(s) => s.null_count_opt().unwrap_or(0) as i64,
                        Statistics::Int64(s) => s.null_count_opt().unwrap_or(0) as i64,
                        Statistics::Int96(s) => s.null_count_opt().unwrap_or(0) as i64,
                        Statistics::Float(s) => s.null_count_opt().unwrap_or(0) as i64,
                        Statistics::Double(s) => s.null_count_opt().unwrap_or(0) as i64,
                        Statistics::ByteArray(s) => s.null_count_opt().unwrap_or(0) as i64,
                        Statistics::FixedLenByteArray(s) => s.null_count_opt().unwrap_or(0) as i64,
                    };
                    *nulls_per_col.entry(col_name).or_insert(0) += null_count;
                }
            }
        }

        nulls_per_col
            .into_iter()
            .map(|(k, nulls)| {
                (
                    k,
                    ColumnStat {
                        nulls,
                        approx_distinct: 0,
                    },
                )
            })
            .collect()
    } else {
        BTreeMap::new()
    };

    Ok(FileStats {
        row_count,
        schema,
        column_stats,
    })
}

// ── dispatch ──────────────────────────────────────────────────────────────────

fn read_file(path: &Path, deep: bool) -> Result<FileStats, String> {
    let suffix = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
        .unwrap_or_default();

    match suffix.as_str() {
        ".csv" | ".tsv" => read_csv(path, deep),
        ".jsonl" | ".ndjson" => read_jsonl(path, deep),
        ".json" => read_json(path, deep),
        ".parquet" => read_parquet(path, deep),
        other => Err(format!("unsupported: {other}")),
    }
}

// ── public API ────────────────────────────────────────────────────────────────

const UNSUPPORTED_SUFFIXES: &[&str] = &[".feather", ".arrow", ".xlsx", ".xls"];

/// Compute row count + schema diff for one data file present in both trees.
///
/// Mirrors `diff_data()` in reference/diff_crawler/data_diff.py, with the
/// limitations noted at the top of this module.
pub fn diff_data(rel: &Path, abs_a: &Path, abs_b: &Path, deep: bool) -> DataDiffResult {
    let path = rel_to_str(rel);

    // Formats we cannot parse without a heavier dependency — return a note so
    // the user knows the file exists and should be checked manually.
    let suffix = abs_a
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
        .unwrap_or_default();
    if UNSUPPORTED_SUFFIXES.contains(&suffix.as_str()) {
        let mut r = DataDiffResult::new(path);
        r.note = format!("skipped: {suffix} format is not supported; check this file manually");
        return r;
    }

    let stats_a = match read_file(abs_a, deep) {
        Ok(s) => s,
        Err(e) => return DataDiffResult::with_error(path, e),
    };
    let stats_b = match read_file(abs_b, deep) {
        Ok(s) => s,
        Err(e) => return DataDiffResult::with_error(path, e),
    };

    let a_cols: BTreeSet<&str> = stats_a.schema.keys().map(|s| s.as_str()).collect();
    let b_cols: BTreeSet<&str> = stats_b.schema.keys().map(|s| s.as_str()).collect();

    let columns_added: Vec<String> = b_cols.difference(&a_cols).map(|s| s.to_string()).collect();
    let columns_removed: Vec<String> = a_cols.difference(&b_cols).map(|s| s.to_string()).collect();

    // BTreeSet intersection is sorted alphabetically.
    let type_changes: BTreeMap<String, [String; 2]> = a_cols
        .intersection(&b_cols)
        .filter_map(|c| {
            let ta = stats_a.schema[*c].clone();
            let tb = stats_b.schema[*c].clone();
            if ta != tb {
                Some((c.to_string(), [ta, tb]))
            } else {
                None
            }
        })
        .collect();

    // Parquet deep stats note: approx_distinct is always 0.
    let note = if deep
        && (abs_a
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("parquet"))
            || abs_b
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("parquet")))
    {
        "approx_distinct is 0 for Parquet (requires reading data pages)".to_string()
    } else {
        String::new()
    };

    DataDiffResult {
        path,
        row_count_a: stats_a.row_count,
        row_count_b: stats_b.row_count,
        schema_a: stats_a.schema,
        schema_b: stats_b.schema,
        columns_added,
        columns_removed,
        type_changes,
        column_stats_a: stats_a.column_stats,
        column_stats_b: stats_b.column_stats,
        note,
        error: None,
    }
}
