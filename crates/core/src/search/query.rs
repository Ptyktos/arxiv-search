use serde::{Deserialize, Serialize};

/// arXiv category codes for infrastructure/networking domains
pub mod categories {
    pub const CS_NI: &str = "cs.NI"; // Networking and Internet Architecture
    pub const CS_SY: &str = "cs.SY"; // Systems and Control
    pub const CS_DC: &str = "cs.DC"; // Distributed, Parallel, and Cluster Computing
    pub const CS_CR: &str = "cs.CR"; // Cryptography and Security
    pub const CS_SE: &str = "cs.SE"; // Software Engineering
    pub const CS_OS: &str = "cs.OS"; // Operating Systems
    pub const CS_AI: &str = "cs.AI"; // Artificial Intelligence
    pub const CS_LG: &str = "cs.LG"; // Machine Learning
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQuery {
    pub keywords: Vec<String>,
    pub categories: Vec<String>,
    pub exclude_keywords: Vec<String>,
    pub date_range: Option<(String, String)>, // YYYY-MM format
    pub min_relevance: f32,                    // 0.0-1.0
}

impl Default for SearchQuery {
    fn default() -> Self {
        Self {
            keywords: vec![],
            categories: vec![],
            exclude_keywords: vec![],
            date_range: None,
            min_relevance: 0.5,
        }
    }
}

pub struct QueryBuilder {
    query: SearchQuery,
}

impl QueryBuilder {
    pub fn new() -> Self {
        Self {
            query: SearchQuery::default(),
        }
    }

    /// Add keywords (title/abstract search)
    pub fn keyword(mut self, keyword: &str) -> Self {
        self.query.keywords.push(keyword.to_lowercase());
        self
    }

    /// Add multiple keywords at once
    pub fn keywords(mut self, keywords: &[&str]) -> Self {
        self.query
            .keywords
            .extend(keywords.iter().map(|k| k.to_lowercase()));
        self
    }

    /// Exclude papers matching these keywords
    pub fn exclude(mut self, keyword: &str) -> Self {
        self.query.exclude_keywords.push(keyword.to_lowercase());
        self
    }

    /// Filter by arXiv category
    pub fn category(mut self, category: &str) -> Self {
        self.query.categories.push(category.to_string());
        self
    }

    /// Filter by multiple categories (OR logic)
    pub fn categories(mut self, cats: &[&str]) -> Self {
        self.query
            .categories
            .extend(cats.iter().map(|c| c.to_string()));
        self
    }

    /// Set date range (YYYY-MM format)
    pub fn since(mut self, date: &str) -> Self {
        let until = self.query.date_range.as_ref().map(|(_, u)| u.clone());
        self.query.date_range = Some((date.to_string(), until.unwrap_or_else(|| "2025-12".to_string())));
        self
    }

    /// Set minimum relevance score (0.0-1.0)
    pub fn min_relevance(mut self, score: f32) -> Self {
        self.query.min_relevance = score.clamp(0.0, 1.0);
        self
    }

    pub fn build(self) -> SearchQuery {
        self.query
    }
}

impl Default for QueryBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Pre-built queries for common infrastructure domains
pub mod presets {
    use super::*;

    pub fn networking() -> SearchQuery {
        QueryBuilder::new()
            .keywords(&[
                "networking",
                "network architecture",
                "internet protocol",
                "tcp/ip",
                "routing",
                "bandwidth",
                "latency",
                "throughput",
            ])
            .category(categories::CS_NI)
            .min_relevance(0.6)
            .build()
    }

    pub fn ddos_prevention() -> SearchQuery {
        QueryBuilder::new()
            .keywords(&[
                "ddos",
                "denial of service",
                "attack detection",
                "anomaly detection",
                "rate limiting",
                "traffic filtering",
                "intrusion detection",
                "network security",
            ])
            .categories(&[categories::CS_NI, categories::CS_CR, categories::CS_SY])
            .min_relevance(0.7)
            .build()
    }

    pub fn siem_soar() -> SearchQuery {
        QueryBuilder::new()
            .keywords(&[
                "siem",
                "security information",
                "event management",
                "soar",
                "security orchestration",
                "incident response",
                "threat detection",
                "log analysis",
                "security analytics",
            ])
            .categories(&[categories::CS_CR, categories::CS_SY, categories::CS_AI])
            .min_relevance(0.7)
            .build()
    }

    pub fn virtual_hosting() -> SearchQuery {
        QueryBuilder::new()
            .keywords(&[
                "virtualization",
                "hypervisor",
                "container",
                "kubernetes",
                "resource allocation",
                "vm placement",
                "orchestration",
                "cloud infrastructure",
            ])
            .categories(&[categories::CS_DC, categories::CS_OS, categories::CS_SY])
            .min_relevance(0.6)
            .build()
    }

    pub fn storage_optimization() -> SearchQuery {
        QueryBuilder::new()
            .keywords(&[
                "storage",
                "distributed storage",
                "data replication",
                "caching",
                "deduplication",
                "compression",
                "tiered storage",
                "i/o optimization",
            ])
            .categories(&[categories::CS_DC, categories::CS_SY, categories::CS_OS])
            .min_relevance(0.6)
            .build()
    }

    pub fn infrastructure_optimization() -> SearchQuery {
        QueryBuilder::new()
            .keywords(&[
                "optimization",
                "performance",
                "scalability",
                "resource efficiency",
                "bottleneck",
                "profiling",
                "monitoring",
                "tuning",
            ])
            .categories(&[
                categories::CS_DC,
                categories::CS_SY,
                categories::CS_OS,
                categories::CS_SE,
            ])
            .min_relevance(0.5)
            .build()
    }

    /// Composite: everything relevant to your infrastructure stack
    pub fn your_stack() -> SearchQuery {
        QueryBuilder::new()
            .keywords(&[
                "networking",
                "ddos",
                "security",
                "virtualization",
                "storage",
                "optimization",
                "performance",
                "distributed",
                "cloud",
                "infrastructure",
                "orchestration",
                "monitoring",
            ])
            .categories(&[
                categories::CS_NI,
                categories::CS_CR,
                categories::CS_DC,
                categories::CS_SY,
                categories::CS_OS,
            ])
            .min_relevance(0.55)
            .build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_query_builder() {
        let query = QueryBuilder::new()
            .keywords(&["ddos", "security"])
            .category(categories::CS_CR)
            .min_relevance(0.7)
            .build();

        assert_eq!(query.keywords.len(), 2);
        assert_eq!(query.categories.len(), 1);
        assert_eq!(query.min_relevance, 0.7);
    }

    #[test]
    fn test_presets() {
        let ddos = presets::ddos_prevention();
        assert!(!ddos.keywords.is_empty());
        assert!(!ddos.categories.is_empty());

        let stack = presets::your_stack();
        assert!(stack.keywords.len() > 10);
    }
}
