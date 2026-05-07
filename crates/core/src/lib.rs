pub mod arxiv;
pub mod content;
pub mod error;
pub mod html;
pub mod paper;
pub mod semantic_scholar;

#[cfg(not(target_arch = "wasm32"))]
pub mod pdf;

#[cfg(target_arch = "wasm32")]
pub mod pdf {
    use crate::error::ArxivError;

    #[must_use]
    pub fn extract_text(_bytes: &[u8]) -> Result<String, ArxivError> {
        Err(ArxivError::NoContentAvailable(
            "PDF extraction is unavailable in the worker runtime".to_string(),
        ))
    }
}

pub use content::{PaperChunk, PreparationOptions, PreparedPaper};
pub use error::ArxivError;
pub use paper::Paper;
