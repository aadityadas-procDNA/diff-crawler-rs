use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::EnvFilter;

use diff_crawler::config::CrawlConfig;
use diff_crawler::crawler::crawl;

#[derive(Parser)]
#[command(name = "diff-crawler", about = "Diff two ML project directories")]
struct Cli {
    dir_a: PathBuf,
    dir_b: PathBuf,

    /// Write full JSON report to this path.
    #[arg(long)]
    json: Option<PathBuf>,

    /// Write Markdown summary to this path. (Rendered in Phase 5.)
    #[arg(long)]
    markdown: Option<PathBuf>,

    /// Skip full unified diffs (keep counts + similarity).
    #[arg(long)]
    no_diff: bool,

    /// Run deeper data stats (null counts, approx distinct). (Phase 3.)
    #[arg(long)]
    deep: bool,

    /// Combined-score threshold for rename detection.
    #[arg(long, default_value_t = 0.4)]
    rename_threshold: f64,

    /// Emit tracing output to stderr.
    #[arg(short, long)]
    verbose: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let filter = if cli.verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"))
    };
    tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).init();

    let config = CrawlConfig {
        include_full_unified_diff: !cli.no_diff,
        deep_data_stats: cli.deep,
        rename_threshold: cli.rename_threshold,
        ..CrawlConfig::default()
    };

    let report = crawl(&cli.dir_a, &cli.dir_b, &config)?;
    let json_text = serde_json::to_string_pretty(&report)?;

    if let Some(json_path) = &cli.json {
        std::fs::write(json_path, &json_text)?;
        eprintln!("Wrote JSON: {}", json_path.display());
    }

    // Markdown renderer is Phase 5; until then always emit JSON to stdout.
    println!("{json_text}");

    Ok(())
}
