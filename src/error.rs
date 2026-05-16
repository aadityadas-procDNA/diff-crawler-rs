use thiserror::Error;

#[derive(Debug, Error)]
pub enum CrawlError {
    #[error("I/O error at {path}: {source}")]
    Io {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("Walk error: {0}")]
    Walk(#[from] ignore::Error),
    #[error("Path strip error: {0}")]
    StripPrefix(#[from] std::path::StripPrefixError),
}
