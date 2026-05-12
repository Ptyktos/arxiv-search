use serde::{Deserialize, Serialize};

/// Represents an author of an arXiv paper, capturing both their name and
/// any institutional affiliations extracted from the Atom metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Author {
    /// The full name of the author.
    pub name: String,
    /// A list of associated institutional affiliations, if any.
    pub affiliations: Vec<String>,
}

/// A Data Transfer Object representing an individual arXiv paper.
///
/// Contains core metadata extracted from either the arXiv API or Semantic Scholar
/// API endpoints, ensuring a standardized schema for LLM context injection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Paper {
    /// The unique arXiv ID (e.g. `1706.03762`).
    pub id: String,
    /// The title of the paper.
    pub title: String,
    /// A list of authors and their institutional affiliations.
    pub authors: Vec<Author>,
    /// The abstract text.
    pub abstract_text: String,
    /// `ArXiv` subject categories (e.g., `cs.AI`, `cs.LG`).
    pub categories: Vec<String>,
    /// The publication timestamp (e.g., `2021-03-23T00:00:00Z`).
    pub published: String,
    /// The canonical URL to the paper on arxiv.org.
    pub url: String,
    /// Digital Object Identifier, if provided.
    pub doi: Option<String>,
    /// Journal reference string, if published outside arXiv.
    pub journal_ref: Option<String>,
}
