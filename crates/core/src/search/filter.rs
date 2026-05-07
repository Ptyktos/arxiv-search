use serde::{Deserialize, Serialize};
use crate::search::query::SearchQuery;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PaperMetadata {
    pub arxiv_id: String,
    pub title: String,
    pub authors: Vec<String>,
    pub abstract_text: String,
    pub categories: Vec<String>,
    pub published: String, // YYYY-MM-DD
    pub updated: String,
    pub pdf_url: String,
    pub s3_key: Option<String>, // S3 path if available
}

impl PaperMetadata {
    pub fn relevance_score(&self, query: &SearchQuery) -> f32 {
        let mut score = 0.0f32;
        let mut weight_sum = 0.0f32;

        // Title matching (highest weight)
        let title_lower = self.title.to_lowercase();
        let abstract_lower = self.abstract_text.to_lowercase();

        for keyword in &query.keywords {
            let title_match = if title_lower.contains(keyword) { 0.4 } else { 0.0 };
            let abstract_match = if abstract_lower.contains(keyword) { 0.2 } else { 0.0 };
            let fuzzy_match = fuzzy_score(&title_lower, keyword) * 0.3;

            let keyword_score = (title_match + abstract_match + fuzzy_match).min(0.4);
            score += keyword_score;
            weight_sum += 0.4;
        }

        // Category matching
        for category in &query.categories {
            if self.categories.iter().any(|c| c.starts_with(category)) {
                score += 0.3;
                weight_sum += 0.3;
            }
        }

        // Exclude keywords
        for exclude in &query.exclude_keywords {
            if title_lower.contains(exclude) || abstract_lower.contains(exclude) {
                return 0.0; // Instant disqualification
            }
        }

        if weight_sum > 0.0 {
            score / weight_sum
        } else {
            0.0
        }
    }

    pub fn matches(&self, query: &SearchQuery) -> bool {
        self.relevance_score(query) >= query.min_relevance
    }

    /// Generate S3 key from arxiv_id (e.g., 2401.00001v1 -> pdf/2401/2401.00001v1.pdf)
    pub fn s3_key_from_arxiv_id(arxiv_id: &str) -> String {
        // Extract date portion (first 4 chars: YYMM)
        let date_portion = &arxiv_id[..4];
        format!("pdf/{}/{}.pdf", date_portion, arxiv_id)
    }
}

/// Simple fuzzy matching using character sequence similarity
fn fuzzy_score(haystack: &str, needle: &str) -> f32 {
    if needle.is_empty() {
        return 1.0;
    }
    if haystack.is_empty() {
        return 0.0;
    }

    let haystack_chars: Vec<char> = haystack.chars().collect();
    let needle_chars: Vec<char> = needle.chars().collect();

    let mut matched = 0;
    let mut last_match = 0;

    for needle_char in &needle_chars {
        let mut found = false;
        for (i, haystack_char) in haystack_chars.iter().enumerate().skip(last_match) {
            if *haystack_char == *needle_char {
                matched += 1;
                last_match = i + 1;
                found = true;
                break;
            }
        }
        if !found {
            break;
        }
    }

    // Score: matched chars / total needle chars, adjusted for how spread out they are
    let sequence_bonus = if matched == needle_chars.len() {
        // All characters found
        1.0 - ((last_match as f32 - needle_chars.len() as f32) / haystack_chars.len() as f32).abs()
    } else {
        0.0
    };

    (matched as f32 / needle_chars.len() as f32) * 0.7 + sequence_bonus * 0.3
}

pub struct SearchFilter {
    query: SearchQuery,
}

impl SearchFilter {
    pub fn new(query: SearchQuery) -> Self {
        Self { query }
    }

    /// Filter papers and return with relevance scores
    pub fn search(&self, papers: &[PaperMetadata]) -> Vec<(PaperMetadata, f32)> {
        papers
            .iter()
            .map(|p| {
                let score = p.relevance_score(&self.query);
                (p.clone(), score)
            })
            .filter(|(_, score)| *score >= self.query.min_relevance)
            .collect()
    }

    /// Filter papers (simple yes/no)
    pub fn filter(&self, papers: &[PaperMetadata]) -> Vec<PaperMetadata> {
        papers
            .iter()
            .filter(|p| p.matches(&self.query))
            .cloned()
            .collect()
    }

    /// Sort papers by relevance (highest first)
    pub fn rank(&self, mut results: Vec<(PaperMetadata, f32)>) -> Vec<(PaperMetadata, f32)> {
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::query::QueryBuilder;

    #[test]
    fn test_fuzzy_matching() {
        assert!(fuzzy_score("distributed systems", "distributed") > 0.8);
        assert!(fuzzy_score("network security", "netw") > 0.6);
        assert!(fuzzy_score("denial of service", "ddos") < 0.5);
    }

    #[test]
    fn test_paper_relevance() {
        let paper = PaperMetadata {
            arxiv_id: "2401.00001v1".to_string(),
            title: "DDoS Detection in Network Traffic Using ML".to_string(),
            authors: vec!["Alice".to_string()],
            abstract_text: "This paper presents a novel approach to detecting DDoS attacks..."
                .to_string(),
            categories: vec!["cs.NI".to_string(), "cs.CR".to_string()],
            published: "2024-01-01".to_string(),
            updated: "2024-01-01".to_string(),
            pdf_url: "https://arxiv.org/pdf/2401.00001v1".to_string(),
            s3_key: None,
        };

        let query = QueryBuilder::new()
            .keywords(&["ddos", "network"])
            .category("cs.NI")
            .build();

        let score = paper.relevance_score(&query);
        assert!(score > 0.5);
    }

    #[test]
    fn test_s3_key_generation() {
        let key = PaperMetadata::s3_key_from_arxiv_id("2401.00001v1");
        assert_eq!(key, "pdf/2401/2401.00001v1.pdf");
    }

    #[test]
    fn test_search_filter() {
        let papers = vec![
            PaperMetadata {
                arxiv_id: "2401.00001v1".to_string(),
                title: "DDoS Detection".to_string(),
                authors: vec![],
                abstract_text: "About DDoS attacks".to_string(),
                categories: vec!["cs.CR".to_string()],
                published: "2024-01-01".to_string(),
                updated: "2024-01-01".to_string(),
                pdf_url: "https://...".to_string(),
                s3_key: None,
            },
            PaperMetadata {
                arxiv_id: "2401.00002v1".to_string(),
                title: "Cooking Recipes".to_string(),
                authors: vec![],
                abstract_text: "A guide to making pasta".to_string(),
                categories: vec!["other".to_string()],
                published: "2024-01-01".to_string(),
                updated: "2024-01-01".to_string(),
                pdf_url: "https://...".to_string(),
                s3_key: None,
            },
        ];

        let query = QueryBuilder::new()
            .keywords(&["ddos"])
            .min_relevance(0.5)
            .build();

        let filter = SearchFilter::new(query);
        let results = filter.filter(&papers);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].arxiv_id, "2401.00001v1");
    }
}
