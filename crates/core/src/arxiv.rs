use quick_xml::{events::Event, Reader};

use crate::error::ArxivError;
use crate::paper::Paper;

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
        return Err(ArxivError::ParseError(format!(
            "invalid date format: {date}"
        )));
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

#[derive(Default)]
struct EntryBuilder {
    id: String,
    title: String,
    summary: String,
    authors: Vec<String>,
    published: String,
    categories: Vec<String>,
    url: String,
}

impl EntryBuilder {
    fn into_paper(self) -> Result<Paper, ArxivError> {
        if self.id.is_empty() {
            return Err(ArxivError::ParseError("entry missing <id>".to_string()));
        }
        let id = extract_id_from_url(&self.id);
        let url = if self.url.is_empty() {
            format!("https://arxiv.org/abs/{id}")
        } else {
            self.url
        };
        Ok(Paper {
            id,
            title: self.title,
            authors: self.authors,
            abstract_text: self.summary.trim().to_string(),
            categories: self.categories,
            published: self.published,
            url,
        })
    }
}

fn extract_id_from_url(url: &str) -> String {
    url.rsplit('/')
        .next()
        .unwrap_or(url)
        .split('v')
        .next()
        .unwrap_or(url)
        .to_string()
}

/// Parse arXiv Atom XML response into paper structs.
///
/// # Errors
///
/// Returns `ParseError` if XML is malformed or required fields are missing.
pub fn parse_response(xml: &str) -> Result<Vec<Paper>, ArxivError> {
    #[derive(PartialEq)]
    enum Field {
        Id,
        Title,
        Summary,
        AuthorName,
        Published,
    }

    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut papers = Vec::new();
    let mut entry: Option<EntryBuilder> = None;
    let mut field: Option<Field> = None;
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => match e.local_name().as_ref() {
                b"entry" => entry = Some(EntryBuilder::default()),
                b"id" if entry.is_some() => field = Some(Field::Id),
                b"title" if entry.is_some() => field = Some(Field::Title),
                b"summary" if entry.is_some() => field = Some(Field::Summary),
                b"name" if entry.is_some() => field = Some(Field::AuthorName),
                b"published" if entry.is_some() => field = Some(Field::Published),
                _ => {}
            },
            Ok(Event::Empty(e)) if entry.is_some() => {
                let name = e.local_name();
                if name.as_ref() == b"category" {
                    if let Some(ref mut b) = entry {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"term" {
                                let term = attr
                                    .unescape_value()
                                    .map_err(|err| ArxivError::ParseError(err.to_string()))?
                                    .into_owned();
                                b.categories.push(term);
                            }
                        }
                    }
                } else if name.as_ref() == b"link" {
                    if let Some(ref mut b) = entry {
                        let mut is_html = false;
                        let mut href = String::new();
                        for attr in e.attributes().flatten() {
                            match attr.key.as_ref() {
                                b"type" if attr.value.as_ref() == b"text/html" => {
                                    is_html = true;
                                }
                                b"href" => {
                                    href = attr
                                        .unescape_value()
                                        .map_err(|err| ArxivError::ParseError(err.to_string()))?
                                        .into_owned();
                                }
                                _ => {}
                            }
                        }
                        if is_html && !href.is_empty() {
                            b.url = href;
                        }
                    }
                }
            }
            Ok(Event::Text(e)) => {
                if let (Some(f), Some(ref mut b)) = (&field, &mut entry) {
                    let text = e
                        .unescape()
                        .map_err(|err| ArxivError::ParseError(err.to_string()))?
                        .into_owned();
                    match f {
                        Field::Id => b.id = text,
                        Field::Title => b.title = text.trim().to_string(),
                        Field::Summary => b.summary = text,
                        Field::AuthorName => b.authors.push(text),
                        Field::Published => b.published = text,
                    }
                }
                field = None;
            }
            Ok(Event::End(e)) => {
                field = None;
                if e.local_name().as_ref() == b"entry" {
                    if let Some(builder) = entry.take() {
                        papers.push(builder.into_paper()?);
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(ArxivError::ParseError(e.to_string())),
            _ => {}
        }
        buf.clear();
    }

    Ok(papers)
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
        let p = build_query_params(
            "bert",
            10,
            Some("2020-01-01"),
            Some("2020-12-31"),
            &[],
            "date",
        )
        .expect("valid query with dates");
        assert_eq!(
            p.search_query,
            "bert AND submittedDate:[202001010000 TO 202012310000]"
        );
        assert_eq!(p.sort_by, "submittedDate");
    }

    #[test]
    fn max_results_capped_at_50() {
        let p = build_query_params("test", 200, None, None, &[], "relevance").expect("valid query");
        assert_eq!(p.max_results, 50);
    }

    #[test]
    fn max_results_minimum_one() {
        let p = build_query_params("test", 0, None, None, &[], "relevance").expect("valid query");
        assert_eq!(p.max_results, 1);
    }

    #[test]
    fn query_with_invalid_date_returns_error() {
        assert!(
            build_query_params("test", 10, Some("2020/01/01"), None, &[], "relevance").is_err()
        );
    }

    const FIXTURE_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>ArXiv Query</title>
  <entry>
    <id>http://arxiv.org/abs/2103.12345v1</id>
    <title>Test Paper Title</title>
    <summary>  This is the abstract.  </summary>
    <author><name>Alice Smith</name></author>
    <author><name>Bob Jones</name></author>
    <published>2021-03-23T00:00:00Z</published>
    <category term="cs.AI" scheme="http://arxiv.org/schemas/atom"/>
    <category term="cs.LG" scheme="http://arxiv.org/schemas/atom"/>
    <link href="https://arxiv.org/abs/2103.12345v1" rel="alternate" type="text/html"/>
  </entry>
</feed>"#;

    #[test]
    fn parse_single_entry() {
        let papers = parse_response(FIXTURE_XML).expect("fixture XML should parse");
        assert_eq!(papers.len(), 1);
        let p = &papers[0];
        assert_eq!(p.id, "2103.12345");
        assert_eq!(p.title, "Test Paper Title");
        assert_eq!(p.abstract_text, "This is the abstract.");
        assert_eq!(p.authors, vec!["Alice Smith", "Bob Jones"]);
        assert_eq!(p.categories, vec!["cs.AI", "cs.LG"]);
        assert_eq!(p.published, "2021-03-23T00:00:00Z");
        assert_eq!(p.url, "https://arxiv.org/abs/2103.12345v1");
    }

    #[test]
    fn parse_empty_feed() {
        let xml = r#"<?xml version="1.0"?><feed xmlns="http://www.w3.org/2005/Atom"></feed>"#;
        let papers = parse_response(xml).expect("empty feed should parse");
        assert!(papers.is_empty());
    }

    #[test]
    fn parse_missing_html_link_derives_url_from_id() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <entry>
    <id>http://arxiv.org/abs/1901.00001v1</id>
    <title>Old Paper</title>
    <summary>Abstract.</summary>
    <published>2019-01-01T00:00:00Z</published>
  </entry>
</feed>"#;
        let papers = parse_response(xml).expect("entry without link should parse");
        assert_eq!(papers[0].id, "1901.00001");
        assert_eq!(papers[0].url, "https://arxiv.org/abs/1901.00001");
    }
}
