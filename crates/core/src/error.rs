use thiserror::Error;

#[derive(Debug, Error)]
pub enum ArxivError {
    #[error("failed to parse arXiv response: {0}")]
    ParseError(String),
    #[error("invalid paper ID: {0}")]
    InvalidPaperId(String),
    #[error("no content available for paper: {0}")]
    NoContentAvailable(String),
}
