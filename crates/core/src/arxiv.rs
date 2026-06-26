use quick_xml::{events::Event, Reader};

use crate::error::ArxivError;
use crate::paper::Paper;

/// Query parameters for searching arXiv.
pub struct QueryParams {
    pub search_query: String,
    pub max_results: u32,
    pub start: u32,
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

/// Escape Lucene special characters to prevent arXiv's search backend from
/// interpreting them as wildcards or operators (e.g. `*` in `C*-algebra`).
fn sanitize_lucene_query(q: &str) -> String {
    let mut out = String::with_capacity(q.len() * 2);
    for ch in q.chars() {
        // Lucene special chars that need escaping.
        // We intentionally do NOT escape `"` or `:` so that arXiv field syntax
        // (ti:, au:, abs:) and quoted phrases still work.
        if matches!(
            ch,
            '+' | '!' | '(' | ')' | '{' | '}' | '[' | ']' | '^' | '~' | '*' | '?' | '\\' | '/'
        ) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Build arXiv query parameters from search criteria.
///
/// # Errors
///
/// Returns `ParseError` if date format is invalid.
pub fn build_query_params(
    query: &str,
    max_results: u32,
    offset: u32,
    date_from: Option<&str>,
    date_to: Option<&str>,
    categories: &[String],
    sort_by: &str,
) -> Result<QueryParams, ArxivError> {
    let mut q = sanitize_lucene_query(query);

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
        start: offset,
        sort_by: sort_by_param.to_string(),
        sort_order: "descending".to_string(),
    })
}

/// A search response from arXiv, including metadata and papers.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct ArxivResponse {
    pub papers: Vec<Paper>,
    pub total_results: u32,
    pub start_index: u32,
}

#[derive(Default)]
struct EntryBuilder {
    id: String,
    title: String,
    summary: String,
    authors: Vec<crate::paper::Author>,
    published: String,
    categories: Vec<String>,
    url: String,
    doi: Option<String>,
    journal_ref: Option<String>,
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
            doi: self.doi,
            journal_ref: self.journal_ref,
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
#[expect(clippy::too_many_lines)]
pub fn parse_response(xml: &str) -> Result<ArxivResponse, ArxivError> {
    #[derive(PartialEq)]
    enum Field {
        Id,
        Title,
        Summary,
        AuthorName,
        Affiliation,
        Published,
        Doi,
        JournalRef,
        TotalResults,
        StartIndex,
    }

    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut papers = Vec::new();
    let mut entry: Option<EntryBuilder> = None;
    let mut current_author: Option<crate::paper::Author> = None;
    let mut field: Option<Field> = None;
    let mut total_results = 0;
    let mut start_index = 0;
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => match e.local_name().as_ref() {
                b"entry" => entry = Some(EntryBuilder::default()),
                b"author" if entry.is_some() => {
                    current_author = Some(crate::paper::Author {
                        name: String::new(),
                        affiliations: Vec::new(),
                    });
                }
                b"id" if entry.is_some() => field = Some(Field::Id),
                b"title" if entry.is_some() => field = Some(Field::Title),
                b"summary" if entry.is_some() => field = Some(Field::Summary),
                b"name" if current_author.is_some() => field = Some(Field::AuthorName),
                b"affiliation" if current_author.is_some() => field = Some(Field::Affiliation),
                b"published" if entry.is_some() => field = Some(Field::Published),
                b"doi" if entry.is_some() => field = Some(Field::Doi),
                b"journal_ref" if entry.is_some() => field = Some(Field::JournalRef),
                b"totalResults" => field = Some(Field::TotalResults),
                b"startIndex" => field = Some(Field::StartIndex),
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
                let text = e
                    .unescape()
                    .map_err(|err| ArxivError::ParseError(err.to_string()))?
                    .into_owned();

                if let Some(f) = &field {
                    match f {
                        Field::TotalResults => {
                            total_results = text.parse().unwrap_or(0);
                        }
                        Field::StartIndex => {
                            start_index = text.parse().unwrap_or(0);
                        }
                        _ => {
                            if let Some(ref mut b) = entry {
                                match f {
                                    Field::Id => b.id = text,
                                    Field::Title => b.title = text.trim().to_string(),
                                    Field::Summary => b.summary = text,
                                    Field::AuthorName => {
                                        if let Some(ref mut a) = current_author {
                                            a.name = text;
                                        }
                                    }
                                    Field::Affiliation => {
                                        if let Some(ref mut a) = current_author {
                                            a.affiliations.push(text);
                                        }
                                    }
                                    Field::Published => b.published = text,
                                    Field::Doi => b.doi = Some(text),
                                    Field::JournalRef => b.journal_ref = Some(text),
                                    _ => {}
                                }
                            }
                        }
                    }
                }
                field = None;
            }
            Ok(Event::End(e)) => {
                field = None;
                if e.local_name().as_ref() == b"author" {
                    if let (Some(ref mut b), Some(a)) = (entry.as_mut(), current_author.take()) {
                        b.authors.push(a);
                    }
                } else if e.local_name().as_ref() == b"entry" {
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

    Ok(ArxivResponse {
        papers,
        total_results,
        start_index,
    })
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
        let p = build_query_params("attention mechanism", 10, 0, None, None, &[], "relevance")
            .expect("valid query params");
        assert_eq!(p.search_query, "attention mechanism");
        assert_eq!(p.max_results, 10);
        assert_eq!(p.start, 0);
        assert_eq!(p.sort_by, "relevance");
        assert_eq!(p.sort_order, "descending");
    }

    #[test]
    fn query_with_categories() {
        let cats = vec!["cs.AI".to_string(), "cs.LG".to_string()];
        let p = build_query_params("transformers", 5, 10, None, None, &cats, "relevance")
            .expect("valid query with categories");
        assert_eq!(p.search_query, "transformers AND (cat:cs.AI OR cat:cs.LG)");
        assert_eq!(p.start, 10);
    }

    #[test]
    fn query_with_dates() {
        let p = build_query_params(
            "bert",
            10,
            0,
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
        let p =
            build_query_params("test", 200, 0, None, None, &[], "relevance").expect("valid query");
        assert_eq!(p.max_results, 50);
    }

    #[test]
    fn max_results_minimum_one() {
        let p =
            build_query_params("test", 0, 0, None, None, &[], "relevance").expect("valid query");
        assert_eq!(p.max_results, 1);
    }

    #[test]
    fn query_with_invalid_date_returns_error() {
        assert!(
            build_query_params("test", 10, 0, Some("2020/01/01"), None, &[], "relevance").is_err()
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
        let response = parse_response(FIXTURE_XML).expect("fixture XML should parse");
        let papers = response.papers;
        assert_eq!(papers.len(), 1);
        let p = &papers[0];
        assert_eq!(p.id, "2103.12345");
        assert_eq!(p.title, "Test Paper Title");
        assert_eq!(p.abstract_text, "This is the abstract.");
        assert_eq!(p.authors.len(), 2);
        assert_eq!(p.authors[0].name, "Alice Smith");
        assert_eq!(p.authors[1].name, "Bob Jones");
        assert_eq!(p.categories, vec!["cs.AI", "cs.LG"]);
        assert_eq!(p.published, "2021-03-23T00:00:00Z");
        assert_eq!(p.url, "https://arxiv.org/abs/2103.12345v1");
    }

    #[test]
    fn parse_empty_feed() {
        let xml = r#"<?xml version="1.0"?><feed xmlns="http://www.w3.org/2005/Atom"></feed>"#;
        let response = parse_response(xml).expect("empty feed should parse");
        assert!(response.papers.is_empty());
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
        let response = parse_response(xml).expect("entry without link should parse");
        let papers = response.papers;
        assert_eq!(papers[0].id, "1901.00001");
        assert_eq!(papers[0].url, "https://arxiv.org/abs/1901.00001");
    }

    #[test]
    fn parse_extended_metadata() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom" xmlns:opensearch="http://a9.com/-/spec/opensearch/1.1/" xmlns:arxiv="http://arxiv.org/schemas/atom">
  <opensearch:totalResults>1000</opensearch:totalResults>
  <opensearch:startIndex>50</opensearch:startIndex>
  <entry>
    <id>http://arxiv.org/abs/2103.12345v1</id>
    <title>Extended Paper</title>
    <summary>Abstract.</summary>
    <author><name>Author</name></author>
    <published>2021-03-23T00:00:00Z</published>
    <arxiv:doi>10.1000/12345</arxiv:doi>
    <arxiv:journal_ref>Nature 2021</arxiv:journal_ref>
  </entry>
</feed>"#;
        let response = parse_response(xml).expect("extended XML should parse");
        assert_eq!(response.total_results, 1000);
        assert_eq!(response.start_index, 50);
        assert_eq!(response.papers.len(), 1);
        let p = &response.papers[0];
        assert_eq!(p.doi, Some("10.1000/12345".to_string()));
        assert_eq!(p.journal_ref, Some("Nature 2021".to_string()));
    }

    #[test]
    fn parse_author_affiliations() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom" xmlns:arxiv="http://arxiv.org/schemas/atom">
  <entry>
    <id>http://arxiv.org/abs/2103.12345v1</id>
    <title>Affiliation Paper</title>
    <summary>Abstract.</summary>
    <author>
      <name>Alice Smith</name>
      <arxiv:affiliation>University of A</arxiv:affiliation>
      <arxiv:affiliation>Institute of B</arxiv:affiliation>
    </author>
    <author>
      <name>Bob Jones</name>
    </author>
    <published>2021-03-23T00:00:00Z</published>
  </entry>
</feed>"#;
        let response = parse_response(xml).expect("XML with affiliations should parse");
        let p = &response.papers[0];
        assert_eq!(p.authors.len(), 2);
        assert_eq!(p.authors[0].name, "Alice Smith");
        assert_eq!(
            p.authors[0].affiliations,
            vec!["University of A", "Institute of B"]
        );
        assert_eq!(p.authors[1].name, "Bob Jones");
        assert!(p.authors[1].affiliations.is_empty());
    }
    #[test]
    fn test_sanitize_lucene_query() {
        assert_eq!(sanitize_lucene_query("attention*"), "attention\\*");
        assert_eq!(sanitize_lucene_query("ti:attention"), "ti:attention");
        assert_eq!(
            sanitize_lucene_query("all:\"deep learning\""),
            "all:\"deep learning\""
        );
        assert_eq!(sanitize_lucene_query("1+1"), "1\\+1");
        assert_eq!(sanitize_lucene_query("(A AND B)"), "\\(A AND B\\)");
    }
}
