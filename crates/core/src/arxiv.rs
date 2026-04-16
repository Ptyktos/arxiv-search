use crate::error::ArxivError;

/// Query parameters for searching arXiv.
pub struct QueryParams {
    pub search_query: String,
    pub max_results: u32,
    pub sort_by: String,
    pub sort_order: String,
}

/// Normalize a paper ID by removing prefixes and version numbers.
///
/// # Errors
///
/// Returns `InvalidPaperId` if the input is empty or contains no digits.
pub fn normalize_paper_id(raw: &str) -> Result<String, ArxivError> {
    let s = raw.trim();
    let s = if s.to_lowercase().starts_with("arxiv:") {
        &s[6..]
    } else {
        s
    };
    let s = s.split('v').next().unwrap_or(s);
    if s.is_empty() || !s.chars().any(|c| c.is_ascii_digit()) {
        return Err(ArxivError::InvalidPaperId(raw.to_string()));
    }
    Ok(s.to_string())
}

fn format_arxiv_date(date: &str) -> Result<String, ArxivError> {
    let parts: Vec<&str> = date.split('-').collect();
    if parts.len() != 3
        || parts[0].len() != 4
        || parts[1].len() != 2
        || parts[2].len() != 2
        || !parts.iter().all(|p| p.chars().all(|c| c.is_ascii_digit()))
    {
        return Err(ArxivError::ParseError(format!("invalid date format: {date}")));
    }
    Ok(format!("{}{}{}0000", parts[0], parts[1], parts[2]))
}

/// Build arXiv query parameters from search criteria.
///
/// # Errors
///
/// Returns `ParseError` if date format is invalid.
pub fn build_query_params(
    query: &str,
    max_results: u32,
    date_from: Option<&str>,
    date_to: Option<&str>,
    categories: &[String],
    sort_by: &str,
) -> Result<QueryParams, ArxivError> {
    let mut q = query.to_string();

    if !categories.is_empty() {
        let cat_filter = categories
            .iter()
            .map(|c| format!("cat:{c}"))
            .collect::<Vec<_>>()
            .join(" OR ");
        q = format!("{q} AND ({cat_filter})");
    }

    if date_from.is_some() || date_to.is_some() {
        let from = date_from
            .map(format_arxiv_date)
            .transpose()?
            .unwrap_or_else(|| "*".to_string());
        let to = date_to
            .map(format_arxiv_date)
            .transpose()?
            .unwrap_or_else(|| "*".to_string());
        q = format!("{q} AND submittedDate:[{from} TO {to}]");
    }

    let sort_by_param = if sort_by == "date" {
        "submittedDate"
    } else {
        "relevance"
    };

    Ok(QueryParams {
        search_query: q,
        max_results: max_results.clamp(1, 50),
        sort_by: sort_by_param.to_string(),
        sort_order: "descending".to_string(),
    })
}

/// Parse arXiv Atom XML response into paper structs.
///
/// # Errors
///
/// Returns `ParseError` if XML is malformed or required fields are missing.
pub fn parse_response(_xml: &str) -> Result<Vec<crate::paper::Paper>, ArxivError> {
    // Intentional placeholder - XML parsing will be implemented in Task 3
    Err(ArxivError::ParseError(
        "parse_response not yet implemented".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
    use super::*;

    #[test]
    fn normalize_bare_id() {
        assert_eq!(
            normalize_paper_id("2103.12345").expect("valid bare ID"),
            "2103.12345"
        );
    }

    #[test]
    fn normalize_arxiv_prefix() {
        assert_eq!(
            normalize_paper_id("arxiv:2103.12345").expect("valid arxiv prefix ID"),
            "2103.12345"
        );
        assert_eq!(
            normalize_paper_id("ArXiv:2103.12345").expect("valid ArXiv prefix ID"),
            "2103.12345"
        );
    }

    #[test]
    fn normalize_strips_version() {
        assert_eq!(
            normalize_paper_id("2103.12345v2").expect("valid ID with version"),
            "2103.12345"
        );
    }

    #[test]
    fn normalize_rejects_empty() {
        assert!(normalize_paper_id("").is_err());
    }

    #[test]
    fn basic_query() {
        let p = build_query_params("attention mechanism", 10, None, None, &[], "relevance")
            .expect("valid query params");
        assert_eq!(p.search_query, "attention mechanism");
        assert_eq!(p.max_results, 10);
        assert_eq!(p.sort_by, "relevance");
        assert_eq!(p.sort_order, "descending");
    }

    #[test]
    fn query_with_categories() {
        let cats = vec!["cs.AI".to_string(), "cs.LG".to_string()];
        let p = build_query_params("transformers", 5, None, None, &cats, "relevance")
            .expect("valid query with categories");
        assert_eq!(p.search_query, "transformers AND (cat:cs.AI OR cat:cs.LG)");
    }

    #[test]
    fn query_with_dates() {
        let p = build_query_params("bert", 10, Some("2020-01-01"), Some("2020-12-31"), &[], "date")
            .expect("valid query with dates");
        assert_eq!(p.search_query, "bert AND submittedDate:[202001010000 TO 202012310000]");
        assert_eq!(p.sort_by, "submittedDate");
    }

    #[test]
    fn max_results_capped_at_50() {
        let p = build_query_params("test", 200, None, None, &[], "relevance")
            .expect("valid query");
        assert_eq!(p.max_results, 50);
    }

    #[test]
    fn max_results_minimum_one() {
        let p = build_query_params("test", 0, None, None, &[], "relevance")
            .expect("valid query");
        assert_eq!(p.max_results, 1);
    }

    #[test]
    fn query_with_invalid_date_returns_error() {
        assert!(
            build_query_params("test", 10, Some("2020/01/01"), None, &[], "relevance").is_err()
        );
    }
}
