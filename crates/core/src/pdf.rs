use crate::error::ArxivError;

/// Extract text from a PDF as markdown.
///
/// # Errors
///
/// Returns an error if the PDF cannot be parsed or contains no extractable text.
pub fn extract_text(bytes: &[u8]) -> Result<String, ArxivError> {
    let result = pdf_inspector::process_pdf_mem(bytes)
        .map_err(|e| ArxivError::ParseError(e.to_string()))?;
    result
        .markdown
        .ok_or_else(|| {
            ArxivError::NoContentAvailable(
                "PDF produced no markdown output".to_string(),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_bytes_returns_error() {
        assert!(extract_text(b"").is_err());
    }

    #[test]
    fn non_pdf_bytes_returns_error() {
        assert!(extract_text(b"this is not a pdf").is_err());
    }
}
