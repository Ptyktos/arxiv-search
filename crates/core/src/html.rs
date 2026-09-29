use crate::error::ArxivError;

/// Convert HTML to Markdown, extracting article content when available, and
/// cleaning LaTeX/MathJax rendering artifacts.
///
/// # Errors
///
/// Returns `ArxivError::ParseError` if the HTML cannot be converted to Markdown.
pub fn to_markdown(html: &str) -> Result<String, ArxivError> {
    let content = extract_article(html).unwrap_or(html);
    let content = unwrap_internal_links(content);
    let (content, formulas) = extract_math(&content);
    let md = htmd::convert(&content).map_err(|e| ArxivError::ParseError(e.to_string()))?;
    Ok(restore_math(&clean_latex_artifacts(&md), &formulas))
}

/// Title, authors and abstract as printed on an arXiv HTML (`LaTeXML`) page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HtmlMetadata {
    pub title: String,
    pub authors: Vec<String>,
    pub abstract_text: String,
}

/// Read paper metadata straight from an arXiv HTML page.
///
/// Lets full-text retrieval skip waiting on a separate arXiv API round trip.
/// Returns `None` when the page has no usable title (for example arXiv's
/// "Untitled Document" pages).
#[must_use]
pub fn extract_metadata(html: &str) -> Option<HtmlMetadata> {
    let title = between(html, "<title>", "</title>")
        .map(|t| collapse_ws(&decode_entities(t)))
        .filter(|t| !t.is_empty() && t != "Untitled Document")?;
    let abstract_text = html
        .find("class=\"ltx_abstract\"")
        .and_then(|i| between(&html[i..], ">", "</div>"))
        .map(|a| collapse_ws(&decode_entities(&strip_tags(a))))
        .map(|a| a.strip_prefix("Abstract").unwrap_or(&a).trim().to_string())
        .unwrap_or_default();
    let authors = html
        .match_indices("class=\"ltx_personname\">")
        .filter_map(|(i, m)| {
            let rest = &html[i + m.len()..];
            let name = collapse_ws(&decode_entities(
                &rest[..rest.find('<').unwrap_or(rest.len())],
            ));
            (!name.is_empty()).then_some(name)
        })
        .collect();
    Some(HtmlMetadata {
        title,
        authors,
        abstract_text,
    })
}

fn between<'a>(s: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = s.find(open)? + open.len();
    let len = s[start..].find(close)?;
    Some(&s[start..start + len])
}

fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

const MATH_PLACEHOLDER: &str = "ARXIVMATHPLACEHOLDER";

/// Replace each `<math alttext="...">` element (`LaTeXML` emits `MathML` plus a
/// TeX annotation, which htmd would otherwise render twice) with a plain-text
/// placeholder, returning the TeX sources. Placeholders survive htmd untouched,
/// so the TeX is not markdown-escaped.
fn extract_math(html: &str) -> (String, Vec<String>) {
    let mut out = String::with_capacity(html.len());
    let mut formulas = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find("<math") {
        let after = &rest[start..];
        let (Some(open_end), Some(close)) = (after.find('>'), after.find("</math>")) else {
            break;
        };
        if close < open_end {
            break;
        }
        let open_tag = &after[..open_end];
        let Some(tex) = attr(open_tag, "alttext") else {
            // No TeX source: keep the element as-is.
            out.push_str(&rest[..=start + open_end]);
            rest = &rest[start + open_end + 1..];
            continue;
        };
        let tex = decode_entities(tex);
        let delim = if open_tag.contains("display=\"block\"") {
            "$$"
        } else {
            "$"
        };
        out.push_str(&rest[..start]);
        out.push_str(MATH_PLACEHOLDER);
        out.push_str(&formulas.len().to_string());
        out.push('X');
        formulas.push(format!("{delim}{}{delim}", tex.trim()));
        rest = &after[close + "</math>".len()..];
    }
    out.push_str(rest);
    (out, formulas)
}

fn restore_math(md: &str, formulas: &[String]) -> String {
    if formulas.is_empty() {
        return md.to_string();
    }
    let mut out = String::with_capacity(md.len());
    let mut rest = md;
    while let Some(pos) = rest.find(MATH_PLACEHOLDER) {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + MATH_PLACEHOLDER.len()..];
        let digits = after.bytes().take_while(u8::is_ascii_digit).count();
        let formula = after[..digits]
            .parse::<usize>()
            .ok()
            .and_then(|i| formulas.get(i))
            .filter(|_| after[digits..].starts_with('X'));
        if let Some(formula) = formula {
            out.push_str(formula);
            rest = &after[digits + 1..];
        } else {
            out.push_str(MATH_PLACEHOLDER);
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// Unwrap `<a href="#...">text</a>` (citations, figure/section cross-refs) to
/// just `text`: the in-page anchors are meaningless outside the page and cost
/// the reader ~10% of a paper's tokens as `[text](#bib.bib1 "")` residue.
fn unwrap_internal_links(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find("<a ") {
        let after = &rest[start..];
        let Some(open_end) = after.find('>') else {
            break;
        };
        let open_tag = &after[..open_end];
        let internal = attr(open_tag, "href").is_some_and(|h| h.starts_with('#'));
        out.push_str(&rest[..start]);
        if internal {
            let body = &after[open_end + 1..];
            if let Some(close) = body.find("</a>") {
                out.push_str(&body[..close]);
                rest = &body[close + "</a>".len()..];
                continue;
            }
        }
        out.push_str(&after[..=open_end]);
        rest = &after[open_end + 1..];
    }
    out.push_str(rest);
    out
}

/// Value of a double-quoted attribute in an opening tag.
fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!(" {name}=\"");
    let start = tag.find(&needle)? + needle.len();
    let len = tag[start..].find('"')?;
    Some(&tag[start..start + len])
}

fn decode_entities(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
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
    fn math_is_emitted_once_as_unescaped_tex() {
        let html = r#"<p>Let <math alttext="W_{0}\in\mathbb{R}^{d\times k}" display="inline"><semantics><mi>W</mi><annotation encoding="application/x-tex">W_{0}\in\mathbb{R}^{d\times k}</annotation></semantics></math> hold.</p><math alttext="a&lt;b" display="block"><mi>a</mi></math>"#;
        let md = to_markdown(html).expect("math HTML should convert");
        assert!(md.contains(r"$W_{0}\in\mathbb{R}^{d\times k}$"), "{md}");
        assert!(md.contains("$$a<b$$"), "{md}");
        assert_eq!(md.matches("mathbb").count(), 1, "{md}");
    }

    #[test]
    fn internal_links_are_unwrapped_external_kept() {
        let html = r##"<p>See <a href="#bib.bib1" title="">Smith (2020)</a> and <a href="https://example.com">site</a>.</p>"##;
        let md = to_markdown(html).expect("links should convert");
        assert!(md.contains("See Smith (2020) and"), "{md}");
        assert!(!md.contains("#bib"), "{md}");
        assert!(md.contains("(https://example.com)"), "{md}");
    }

    #[test]
    fn extracts_metadata_from_latexml_page() {
        let html = r#"<html><head><title>Attention Is All You Need</title></head><body>
            <span class="ltx_personname">Ashish Vaswani<br></span><span class="ltx_personname">Noam &amp; Co<sup>1</sup></span>
            <div class="ltx_abstract"><h6 class="ltx_title">Abstract</h6>
            <p class="ltx_p">The dominant   sequence <em>transduction</em> models.</p></div></body></html>"#;
        let meta = extract_metadata(html).expect("metadata");
        assert_eq!(meta.title, "Attention Is All You Need");
        assert_eq!(meta.authors, vec!["Ashish Vaswani", "Noam & Co"]);
        assert_eq!(
            meta.abstract_text,
            "The dominant sequence transduction models."
        );
        assert!(extract_metadata("<title>Untitled Document</title>").is_none());
    }

    #[test]
    fn stray_placeholder_text_is_left_alone() {
        assert_eq!(
            restore_math("ARXIVMATHPLACEHOLDER9X", &["$x$".into()]),
            "ARXIVMATHPLACEHOLDER9X"
        );
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
