use std::path::PathBuf;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use diff_crawler::config::CrawlConfig;
use diff_crawler::crawler::crawl;
use diff_crawler::report::render_markdown;

#[derive(Parser)]
#[command(name = "diff-crawler", about = "Diff two ML project directories")]
struct Cli {
    dir_a: PathBuf,
    dir_b: PathBuf,

    /// Write full JSON report to this path.
    #[arg(long)]
    json: Option<PathBuf>,

    /// Write Markdown summary to this path.
    #[arg(long)]
    markdown: Option<PathBuf>,

    /// Skip full unified diffs (keep counts + similarity).
    #[arg(long)]
    no_diff: bool,

    /// Run deeper data stats (null counts, approx distinct).
    #[arg(long)]
    deep: bool,

    /// Combined-score threshold for rename detection.
    #[arg(long, default_value_t = 0.4)]
    rename_threshold: f64,

    /// Emit tracing output to stderr.
    #[arg(short, long)]
    verbose: bool,
}

fn main() {
    let cli = Cli::parse();

    let filter = if cli.verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"))
    };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();

    let config = CrawlConfig {
        include_full_unified_diff: !cli.no_diff,
        deep_data_stats: cli.deep,
        rename_threshold: cli.rename_threshold,
        ..CrawlConfig::default()
    };

    let report = match crawl(&cli.dir_a, &cli.dir_b, &config) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e:#}");
            std::process::exit(2);
        }
    };

    // ── JSON output ───────────────────────────────────────────────────────────
    if let Some(json_path) = &cli.json {
        let text = match serde_json::to_string_pretty(&report) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("error serializing JSON: {e}");
                std::process::exit(2);
            }
        };
        if let Err(e) = std::fs::write(json_path, &text) {
            eprintln!("error writing {}: {e}", json_path.display());
            std::process::exit(2);
        }
        eprintln!("Wrote JSON: {}", json_path.display());
    }

    // ── Markdown output ───────────────────────────────────────────────────────
    let md = render_markdown(&report);

    if let Some(md_path) = &cli.markdown {
        if let Err(e) = std::fs::write(md_path, &md) {
            eprintln!("error writing {}: {e}", md_path.display());
            std::process::exit(2);
        }
        eprintln!("Wrote Markdown: {}", md_path.display());
    }

    // If neither --json nor --markdown was given, print Markdown to stdout.
    // If only --json was given, still print Markdown to stdout so the user
    // gets human-readable output by default (matches Python CLI behaviour).
    if cli.markdown.is_none() {
        println!("{md}");
    }
}
