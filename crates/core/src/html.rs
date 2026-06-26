use crate::error::ArxivError;

/// Convert HTML to Markdown, extracting article content when available, and
/// cleaning LaTeX/MathJax rendering artifacts.
///
/// # Errors
///
/// Returns `ArxivError::ParseError` if the HTML cannot be converted to Markdown.
pub fn to_markdown(html: &str) -> Result<String, ArxivError> {
    let content = extract_article(html).unwrap_or(html);
    let md = htmd::convert(content).map_err(|e| ArxivError::ParseError(e.to_string()))?;
    Ok(clean_latex_artifacts(&md))
}

/// Clean common LaTeX/MathJax rendering artifacts from converted markdown.
/// These appear when htmd processes arXiv's MathJax-laden HTML.
fn clean_latex_artifacts(text: &str) -> String {
    text.replace("\\\\", "\\")
        .replace("italic\\_", "_")
        .replace("italic_", "_")
        .replace("POSTSUBSCRIPT", "")
        .replace("POSTSUPERSCRIPT", "")
        .replace("startsubscript", "")
        .replace("endsubscript", "")
        .replace("start_SUP", "")
        .replace("end_SUP", "")
        .replace("start_POST", "")
        .replace("end_POST", "")
        .replace("start^{", "{")
        .replace("end^{", "}")
        .replace("start_{", "{")
        .replace("end_{", "}")
        .replace("\\ \\ ", " ")
        .replace("  ", " ")
}

fn extract_article(html: &str) -> Option<&str> {
    let start = html.find("<article")?;
    let end = html.rfind("</article>")?;
    Some(&html[start..end + "</article>".len()])
}

#[cfg(test)]
#[expect(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn converts_simple_html() {
        let html = "<p>Hello world.</p>";
        let md = to_markdown(html).expect("simple HTML should convert");
        assert!(md.contains("Hello world"));
    }

    #[test]
    fn converts_article_element() {
        let html = "<article><h1>Title</h1><p>First paragraph.</p></article>";
        let md = to_markdown(html).expect("article HTML should convert");
        assert!(md.contains("Title"));
        assert!(md.contains("First paragraph"));
    }

    #[test]
    fn extracts_article_from_full_page() {
        let html = "<html><head></head><body><nav>Nav noise</nav>\
            <article><p>Paper content here.</p></article><footer>Footer</footer></body></html>";
        let md = to_markdown(html).expect("full page should convert");
        assert!(md.contains("Paper content here"));
        assert!(!md.contains("Nav noise"));
    }

    #[test]
    fn falls_back_to_full_html_when_no_article() {
        let html = "<html><body><p>No article tag.</p></body></html>";
        let md = to_markdown(html).expect("page without article should convert");
        assert!(md.contains("No article tag"));
    }

    #[test]
    fn extract_article_helper() {
        let html = "<nav>x</nav><article><p>y</p></article><footer>z</footer>";
        let extracted = extract_article(html).expect("article should be found");
        assert!(extracted.starts_with("<article"));
        assert!(extracted.ends_with("</article>"));
        assert!(!extracted.contains("footer"));
    }

    #[test]
    fn cleans_latex_escape_artifacts() {
        let input = "The wavefunction \\\\psi satisfies \\\\nabla^2 \\\\psi = 0";
        let cleaned = clean_latex_artifacts(input);
        assert!(
            !cleaned.contains("\\\\\\\\"),
            "should collapse double-escapes"
        );
    }

    #[test]
    fn cleans_mathjax_post_markers() {
        let input = "E = mc{POSTSUPERSCRIPT}2{end_POST}";
        let cleaned = clean_latex_artifacts(input);
        assert!(!cleaned.contains("POSTSUPERSCRIPT"));
        assert!(!cleaned.contains("POST"));
    }
}
