use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use diff_crawler::matcher::find_renames;

// ── helpers ───────────────────────────────────────────────────────────────────

fn write(dir: &Path, name: &str, content: &str) -> PathBuf {
    let p = dir.join(name);
    fs::write(&p, content).unwrap();
    p
}

/// Build the (rel, abs) lookup maps used by find_renames.
fn make_lookup(
    _dir: &Path,
    files: &[(&str, PathBuf)],
) -> (HashSet<PathBuf>, HashMap<PathBuf, PathBuf>) {
    let mut set = HashSet::new();
    let mut map = HashMap::new();
    for (rel_str, abs) in files {
        let rel = PathBuf::from(rel_str);
        set.insert(rel.clone());
        map.insert(rel, abs.clone());
    }
    (set, map)
}

// ── core detection ────────────────────────────────────────────────────────────

#[test]
fn identical_file_with_different_name_is_detected() {
    let dir_a = TempDir::new().unwrap();
    let dir_b = TempDir::new().unwrap();

    // Substantial code content so MinHash has enough shingles to work with.
    let content = r#"
def train_model(data, epochs=10):
    model = Sequential()
    model.add(Dense(128, activation='relu'))
    model.add(Dense(64, activation='relu'))
    model.add(Dense(10, activation='softmax'))
    model.compile(optimizer='adam', loss='categorical_crossentropy', metrics=['accuracy'])
    history = model.fit(data, epochs=epochs, validation_split=0.2)
    return model, history

def evaluate_model(model, test_data):
    loss, accuracy = model.evaluate(test_data)
    print(f"Test accuracy: {accuracy:.4f}")
    return accuracy
"#;

    let abs_a = write(dir_a.path(), "model_train.py", content);
    let abs_b = write(dir_b.path(), "train_model.py", content);

    let (only_a, lookup_a) = make_lookup(dir_a.path(), &[("model_train.py", abs_a)]);
    let (only_b, lookup_b) = make_lookup(dir_b.path(), &[("train_model.py", abs_b)]);

    let results = find_renames(&only_a, &only_b, &lookup_a, &lookup_b, 0.4);

    assert_eq!(results.len(), 1, "should detect exactly one rename");
    let r = &results[0];
    assert_eq!(r.from_path, "model_train.py");
    assert_eq!(r.to_path, "train_model.py");
    assert!(
        r.content_similarity > 0.9,
        "identical content should have content_similarity > 0.9, got {}",
        r.content_similarity
    );
    assert!(
        r.combined_score >= 0.4,
        "combined_score must be at threshold"
    );
}

#[test]
fn slightly_modified_file_is_still_detected() {
    let dir_a = TempDir::new().unwrap();
    let dir_b = TempDir::new().unwrap();

    let content_a = r#"
import numpy as np
import pandas as pd

def preprocess(df):
    df = df.dropna()
    df['feature'] = df['value'] * 2
    df['label'] = df['target'].astype(int)
    return df

def split_data(df, test_size=0.2):
    n = len(df)
    split = int(n * (1 - test_size))
    return df[:split], df[split:]
"#;
    // Same structure with one function renamed and a small tweak
    let content_b = r#"
import numpy as np
import pandas as pd

def preprocess_data(df):
    df = df.dropna()
    df['feature'] = df['value'] * 2
    df['label'] = df['target'].astype(int)
    return df

def split_data(df, test_size=0.2):
    n = len(df)
    split = int(n * (1 - test_size))
    return df[:split], df[split:]
"#;

    let abs_a = write(dir_a.path(), "preprocess.py", content_a);
    let abs_b = write(dir_b.path(), "preprocess.py", content_b);

    let (only_a, lookup_a) = make_lookup(dir_a.path(), &[("preprocess.py", abs_a)]);
    let (only_b, lookup_b) = make_lookup(dir_b.path(), &[("preprocess.py", abs_b)]);

    let results = find_renames(&only_a, &only_b, &lookup_a, &lookup_b, 0.4);

    assert_eq!(
        results.len(),
        1,
        "slightly modified file should still be detected"
    );
    assert!(results[0].combined_score >= 0.4);
}

#[test]
fn completely_different_files_are_not_reported() {
    let dir_a = TempDir::new().unwrap();
    let dir_b = TempDir::new().unwrap();

    // Use long, meaningfully different content.
    let abs_a = write(dir_a.path(), "a.py", &"x = 1\n".repeat(200));
    let abs_b = write(
        dir_b.path(),
        "b.py",
        &"y = 'hello world foo bar baz qux quux'\n".repeat(200),
    );

    let (only_a, lookup_a) = make_lookup(dir_a.path(), &[("a.py", abs_a)]);
    let (only_b, lookup_b) = make_lookup(dir_b.path(), &[("b.py", abs_b)]);

    let results = find_renames(&only_a, &only_b, &lookup_a, &lookup_b, 0.4);

    assert!(
        results.is_empty(),
        "completely different files should not produce a rename candidate"
    );
}

#[test]
fn threshold_controls_which_candidates_are_reported() {
    let dir_a = TempDir::new().unwrap();
    let dir_b = TempDir::new().unwrap();

    let content = r#"
def process(data):
    result = []
    for item in data:
        if item > 0:
            result.append(item * 2)
    return result
"#;

    let abs_a = write(dir_a.path(), "utils.py", content);
    let abs_b = write(dir_b.path(), "helpers.py", content);

    let (only_a, lookup_a) = make_lookup(dir_a.path(), &[("utils.py", abs_a)]);
    let (only_b, lookup_b) = make_lookup(dir_b.path(), &[("helpers.py", abs_b)]);

    // With a very high threshold (1.0) nothing should be reported.
    let strict = find_renames(&only_a, &only_b, &lookup_a, &lookup_b, 1.0);
    assert!(
        strict.is_empty(),
        "threshold=1.0 should suppress all candidates"
    );

    // With the default threshold the identical-content file should be found.
    let normal = find_renames(&only_a, &only_b, &lookup_a, &lookup_b, 0.4);
    assert_eq!(normal.len(), 1, "threshold=0.4 should find the rename");
}

#[test]
fn results_sorted_descending_by_combined_score() {
    let dir_a = TempDir::new().unwrap();
    let dir_b = TempDir::new().unwrap();

    // Two identical pairs so both show up; scores should be in order.
    let code = |n: u32| {
        format!(
            r#"
def function_{n}(x, y, z):
    result = x + y * z
    intermediate = result ** 2
    output = intermediate / (x + 1)
    return output

def helper_{n}(data):
    return [function_{n}(a, b, c) for a, b, c in data]
"#
        )
    };

    let abs_a1 = write(dir_a.path(), "module_one.py", &code(1));
    let abs_a2 = write(dir_a.path(), "module_two.py", &code(2));
    let abs_b1 = write(dir_b.path(), "module_one_v2.py", &code(1));
    let abs_b2 = write(dir_b.path(), "module_two_v2.py", &code(2));

    let (only_a, lookup_a) = make_lookup(
        dir_a.path(),
        &[("module_one.py", abs_a1), ("module_two.py", abs_a2)],
    );
    let (only_b, lookup_b) = make_lookup(
        dir_b.path(),
        &[("module_one_v2.py", abs_b1), ("module_two_v2.py", abs_b2)],
    );

    let results = find_renames(&only_a, &only_b, &lookup_a, &lookup_b, 0.4);

    // Verify sorted descending.
    for w in results.windows(2) {
        assert!(
            w[0].combined_score >= w[1].combined_score,
            "results must be sorted descending: {} < {}",
            w[0].combined_score,
            w[1].combined_score
        );
    }
}

#[test]
fn empty_sides_return_empty_result() {
    let _dir = TempDir::new().unwrap();
    let only_a: HashSet<PathBuf> = HashSet::new();
    let only_b: HashSet<PathBuf> = HashSet::new();
    let lookup: HashMap<PathBuf, PathBuf> = HashMap::new();

    let results = find_renames(&only_a, &only_b, &lookup, &lookup, 0.4);
    assert!(results.is_empty());
}

// ── crawler integration ───────────────────────────────────────────────────────

#[test]
fn crawl_rename_candidates_populated() {
    use diff_crawler::config::CrawlConfig;
    use diff_crawler::crawler::crawl;

    let dir_a = TempDir::new().unwrap();
    let dir_b = TempDir::new().unwrap();

    let code = r#"
def compute_loss(predictions, labels):
    diff = predictions - labels
    return (diff ** 2).mean()

def compute_accuracy(predictions, labels):
    correct = (predictions.argmax(axis=1) == labels).sum()
    return correct / len(labels)

def evaluate(model, dataset):
    preds = model.predict(dataset.inputs)
    loss = compute_loss(preds, dataset.labels)
    acc = compute_accuracy(preds, dataset.labels)
    return {'loss': loss, 'accuracy': acc}
"#;

    // File exists only in A as metrics.py and only in B as metrics_utils.py
    fs::write(dir_a.path().join("metrics.py"), code).unwrap();
    fs::write(dir_b.path().join("metrics_utils.py"), code).unwrap();

    let config = CrawlConfig {
        rename_threshold: 0.4,
        ..CrawlConfig::default()
    };
    let report = crawl(dir_a.path(), dir_b.path(), &config).unwrap();

    assert!(
        !report.rename_candidates.is_empty(),
        "expected rename_candidates to contain metrics.py → metrics_utils.py"
    );
    let r = &report.rename_candidates[0];
    assert!(r.combined_score >= 0.4);
    assert!(
        !r.from_path.contains('\\'),
        "from_path must use forward slashes"
    );
    assert!(
        !r.to_path.contains('\\'),
        "to_path must use forward slashes"
    );
}

#[test]
fn binary_files_do_not_participate_in_rename_detection() {
    use diff_crawler::config::CrawlConfig;
    use diff_crawler::crawler::crawl;

    let dir_a = TempDir::new().unwrap();
    let dir_b = TempDir::new().unwrap();

    // Same binary content with different names — should NOT produce a rename candidate.
    let binary = vec![0u8; 1024];
    fs::write(dir_a.path().join("model_a.pkl"), &binary).unwrap();
    fs::write(dir_b.path().join("model_b.pkl"), &binary).unwrap();

    let config = CrawlConfig::default();
    let report = crawl(dir_a.path(), dir_b.path(), &config).unwrap();

    assert!(
        report.rename_candidates.is_empty(),
        "binary files must not produce rename candidates"
    );
}
