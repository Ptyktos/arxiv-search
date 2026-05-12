use crate::error::ArxivError;
use crate::paper::Paper;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct CitationsResponse {
    pub data: Vec<CitationEntry>,
}

#[derive(Debug, Deserialize)]
pub struct CitationEntry {
    #[serde(rename = "citingPaper")]
    pub citing_paper: SsPaper,
}

#[derive(Debug, Deserialize)]
pub struct RecommendationsResponse {
    #[serde(rename = "recommendedPapers")]
    pub recommended_papers: Vec<SsPaper>,
}

#[derive(Debug, Deserialize)]
pub struct SsPaper {
    pub title: String,
    #[serde(default)]
    pub authors: Vec<SsAuthor>,
    pub year: Option<u32>,
    #[serde(rename = "externalIds", default)]
    pub external_ids: SsExternalIds,
}

#[derive(Debug, Deserialize, Default)]
pub struct SsAuthor {
    pub name: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct SsExternalIds {
    #[serde(rename = "ArXiv")]
    pub arxiv: Option<String>,
}

fn ss_paper_to_paper(p: SsPaper) -> Paper {
    let id = p.external_ids.arxiv.clone().unwrap_or_default();
    let url = if id.is_empty() {
        String::new()
    } else {
        format!("https://arxiv.org/abs/{id}")
    };
    Paper {
        id,
        title: p.title,
        authors: p.authors.into_iter().map(|a| a.name).collect(),
        abstract_text: String::new(),
        categories: Vec::new(),
        published: p.year.map(|y| y.to_string()).unwrap_or_default(),
        url,
        doi: None,
        journal_ref: None,
    }
}

/// Parse a Semantic Scholar citations response JSON into a vector of papers.
///
/// # Errors
///
/// Returns `ArxivError::ParseError` if the JSON is invalid or missing required fields.
pub fn parse_citations(json: &str) -> Result<Vec<Paper>, ArxivError> {
    let response: CitationsResponse =
        serde_json::from_str(json).map_err(|e| ArxivError::ParseError(e.to_string()))?;
    Ok(response
        .data
        .into_iter()
        .map(|e| ss_paper_to_paper(e.citing_paper))
        .collect())
}

/// Parse a Semantic Scholar recommendations response JSON into a vector of papers.
///
/// # Errors
///
/// Returns `ArxivError::ParseError` if the JSON is invalid or missing required fields.
pub fn parse_recommendations(json: &str) -> Result<Vec<Paper>, ArxivError> {
    let response: RecommendationsResponse =
        serde_json::from_str(json).map_err(|e| ArxivError::ParseError(e.to_string()))?;
    Ok(response
        .recommended_papers
        .into_iter()
        .map(ss_paper_to_paper)
        .collect())
}

#[cfg(test)]
#[expect(clippy::expect_used)]
mod tests {
    use super::*;

    const CITATIONS_JSON: &str = r#"{
  "data": [
    {
      "citingPaper": {
        "title": "Citing Paper One",
        "authors": [{"name": "Carol White"}],
        "year": 2022,
        "externalIds": {"ArXiv": "2201.99999"}
      }
    }
  ]
}"#;

    const RECS_JSON: &str = r#"{
  "recommendedPapers": [
    {
      "title": "Recommended Paper",
      "authors": [{"name": "Dave Black"}],
      "year": 2023,
      "externalIds": {"ArXiv": "2301.88888"}
    }
  ]
}"#;

    const EMPTY_CITATIONS: &str = r#"{"data": []}"#;

    #[test]
    fn parse_single_citation() {
        let papers = parse_citations(CITATIONS_JSON).expect("citations JSON should parse");
        assert_eq!(papers.len(), 1);
        assert_eq!(papers[0].title, "Citing Paper One");
        assert_eq!(papers[0].authors, vec!["Carol White"]);
        assert_eq!(papers[0].id, "2201.99999");
        assert_eq!(papers[0].published, "2022");
        assert_eq!(papers[0].url, "https://arxiv.org/abs/2201.99999");
    }

    #[test]
    fn parse_single_recommendation() {
        let papers = parse_recommendations(RECS_JSON).expect("recommendations JSON should parse");
        assert_eq!(papers.len(), 1);
        assert_eq!(papers[0].title, "Recommended Paper");
        assert_eq!(papers[0].id, "2301.88888");
        assert_eq!(papers[0].published, "2023");
    }

    #[test]
    fn parse_empty_citations() {
        let papers = parse_citations(EMPTY_CITATIONS).expect("empty citations should parse");
        assert!(papers.is_empty());
    }

    #[test]
    fn paper_without_arxiv_id_has_empty_url() {
        let json = r#"{
  "data": [{
    "citingPaper": {
      "title": "No ArXiv",
      "authors": [],
      "year": 2021,
      "externalIds": {}
    }
  }]
}"#;
        let papers = parse_citations(json).expect("paper without arxiv id should parse");
        assert_eq!(papers[0].id, "");
        assert_eq!(papers[0].url, "");
    }
}
