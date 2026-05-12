use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Paper {
    pub id: String,
    pub title: String,
    pub authors: Vec<String>,
    pub abstract_text: String,
    pub categories: Vec<String>,
    pub published: String,
    pub url: String,
    pub doi: Option<String>,
    pub journal_ref: Option<String>,
}
