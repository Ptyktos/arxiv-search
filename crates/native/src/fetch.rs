use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use reqwest::Client;

use crate::persistence::{ArxivCache, DEFAULT_CACHE_TTL};
use crate::rate_limit::TokioRateLimiter;
use arxiv_search_rs_mcp_core::arxiv::QueryParams;
use arxiv_search_rs_mcp_core::RateLimiter;

const ARXIV_API_BASE: &str = "https://export.arxiv.org/api/query";
const ARXIV_HTML_BASE: &str = "https://arxiv.org/html";
const ARXIV_PDF_BASE: &str = "https://arxiv.org/pdf";
const SS_API_BASE: &str = "https://api.semanticscholar.org/graph/v1";
const SS_REC_BASE: &str = "https://api.semanticscholar.org/recommendations/v1";
const ARXIV_RATE_LIMIT: Duration = Duration::from_secs(3);
const MAX_RETRIES: u32 = 3;
const RETRY_BASE_MS: u64 = 3_000;
/// The primary client for making authenticated, rate-limited HTTP requests to arXiv
/// and Semantic Scholar APIs. In the native context, this client is strictly
/// synchronized with a global `TokioRateLimiter` and an asynchronous `ArxivCache`.
#[derive(Clone)]
pub struct FetchClient {
    client: Client,
    rate_limiter: Arc<dyn RateLimiter>,
    ss_api_key: Option<String>,
    cache: ArxivCache,
    #[cfg(feature = "embedded-db")]
    pub db: Option<crate::db::Database>,
}

impl std::fmt::Debug for FetchClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FetchClient")
            .field("client", &self.client)
            .field("ss_api_key", &self.ss_api_key)
            .finish_non_exhaustive()
    }
}

impl FetchClient {
    /// # Errors
    ///
    /// Returns an error if the HTTP client fails to build.
    pub async fn new(ss_api_key: Option<String>) -> Result<Self> {
        #[expect(clippy::duration_suboptimal_units)]
        let client = Client::builder()
            .user_agent(concat!(
                "arxiv-search-rs-mcp/",
                env!("CARGO_PKG_VERSION"),
                " (Rust MCP server)"
            ))
            .timeout(Duration::from_secs(60))
            .build()
            .context("failed to build HTTP client")?;
        let cache = ArxivCache::new(DEFAULT_CACHE_TTL).await?;

        #[cfg(feature = "embedded-db")]
        let db = {
            let db_path = cache.get_cache_dir().join("arxiv_rag.db");
            match crate::db::Database::init(&db_path) {
                Ok(database) => Some(database),
                Err(e) => {
                    tracing::error!("Failed to initialize embedded database: {:?}", e);
                    None
                }
            }
        };

        Ok(Self {
            client,
            rate_limiter: Arc::new(TokioRateLimiter::new(ARXIV_RATE_LIMIT)),
            ss_api_key,
            cache,
            #[cfg(feature = "embedded-db")]
            db,
        })
    }

    /// # Errors
    ///
    /// Returns an error if the HTTP request fails, the server returns an error status, or the
    /// response body cannot be read. Retries transient 429/503 errors with exponential backoff.
    pub async fn fetch_arxiv_query(&self, params: &QueryParams) -> Result<String> {
        for attempt in 0..MAX_RETRIES {
            self.rate_limiter.wait().await;
            let response = self
                .client
                .get(ARXIV_API_BASE)
                .query(&[
                    ("search_query", params.search_query.as_str()),
                    ("max_results", &params.max_results.to_string()),
                    ("start", &params.start.to_string()),
                    ("sortBy", params.sort_by.as_str()),
                    ("sortOrder", params.sort_order.as_str()),
                ])
                .send()
                .await
                .context("arXiv API request failed")?;
            let status = response.status().as_u16();
            if status == 429 || status == 503 {
                let delay = RETRY_BASE_MS * 2u64.pow(attempt);
                tracing::warn!(
                    "arXiv returned {status}, retrying in {delay}ms (attempt {}/{})",
                    attempt + 1,
                    MAX_RETRIES
                );
                tokio::time::sleep(Duration::from_millis(delay)).await;
                continue;
            }
            return response
                .error_for_status()
                .context("arXiv API returned error status")?
                .text()
                .await
                .context("failed to read arXiv response body");
        }
        anyhow::bail!("arXiv API failed after {MAX_RETRIES} retries (429/503)")
    }

    /// # Errors
    ///
    /// Returns an error if the HTTP request fails, the server returns an error status, or the
    /// response body cannot be read. Retries transient 429/503 errors with exponential backoff.
    pub async fn fetch_arxiv_by_id(&self, paper_id: &str) -> Result<String> {
        for attempt in 0..MAX_RETRIES {
            self.rate_limiter.wait().await;
            let response = self
                .client
                .get(ARXIV_API_BASE)
                .query(&[("id_list", paper_id)])
                .send()
                .await
                .context("arXiv ID lookup request failed")?;
            let status = response.status().as_u16();
            if status == 429 || status == 503 {
                let delay = RETRY_BASE_MS * 2u64.pow(attempt);
                tracing::warn!("arXiv returned {status} for id {paper_id}, retrying in {delay}ms (attempt {}/{})", attempt + 1, MAX_RETRIES);
                tokio::time::sleep(Duration::from_millis(delay)).await;
                continue;
            }
            return response
                .error_for_status()
                .context("arXiv API returned error status")?
                .text()
                .await
                .context("failed to read arXiv response body");
        }
        anyhow::bail!("arXiv ID lookup failed after {MAX_RETRIES} retries (429/503)")
    }

    /// # Errors
    ///
    /// Returns an error if the HTTP request fails or the server returns a non-404 error status.
    /// Returns `Ok(None)` if the HTML version does not exist (404).
    pub async fn fetch_html(&self, paper_id: &str) -> Result<Option<String>> {
        if let Some(cached) = self.cache.get_html(paper_id).await? {
            tracing::info!("Cache hit for HTML: {}", paper_id);
            return Ok(Some(cached));
        }

        self.rate_limiter.wait().await;
        let url = format!("{ARXIV_HTML_BASE}/{paper_id}");
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context("HTML fetch request failed")?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let text = response
            .error_for_status()
            .context("HTML endpoint returned error status")?
            .text()
            .await
            .context("failed to read HTML response body")?;

        self.cache.set_html(paper_id, &text).await?;
        Ok(Some(text))
    }

    /// # Errors
    ///
    /// Returns an error if the HTTP request fails, the server returns an error status, or the
    /// response body cannot be read.
    pub async fn fetch_pdf(&self, paper_id: &str) -> Result<Vec<u8>> {
        if let Some(cached) = self.cache.get_pdf(paper_id).await? {
            tracing::info!("Cache hit for PDF: {}", paper_id);
            return Ok(cached);
        }

        self.rate_limiter.wait().await;
        let url = format!("{ARXIV_PDF_BASE}/{paper_id}");
        let bytes = self
            .client
            .get(&url)
            .send()
            .await
            .context("PDF fetch request failed")?
            .error_for_status()
            .context("PDF endpoint returned error status")?
            .bytes()
            .await
            .context("failed to read PDF response body")?;
        let bytes_vec = bytes.to_vec();
        self.cache.set_pdf(paper_id, &bytes_vec).await?;
        Ok(bytes_vec)
    }

    /// # Errors
    ///
    /// Returns an error if the HTTP request fails, Semantic Scholar returns an error status, or
    /// the response body cannot be read.
    pub async fn fetch_citations(&self, paper_id: &str, limit: u32) -> Result<String> {
        let url = format!("{SS_API_BASE}/paper/ArXiv:{paper_id}/citations");
        let mut req = self.client.get(&url).query(&[
            ("fields", "title,authors,year,externalIds"),
            ("limit", &limit.to_string()),
        ]);
        if let Some(key) = &self.ss_api_key {
            req = req.header("x-api-key", key);
        }
        req.send()
            .await
            .context("Semantic Scholar citations request failed")?
            .error_for_status()
            .context("Semantic Scholar citations returned error status")?
            .text()
            .await
            .context("failed to read citations response body")
    }

    /// # Errors
    ///
    /// Returns an error if the HTTP request fails, Semantic Scholar returns an error status, or
    /// the response body cannot be read.
    pub async fn fetch_recommendations(&self, paper_id: &str, limit: u32) -> Result<String> {
        let url = format!("{SS_REC_BASE}/papers/forpaper/ArXiv:{paper_id}");
        let mut req = self
            .client
            .get(&url)
            .query(&[("limit", &limit.to_string())]);
        if let Some(key) = &self.ss_api_key {
            req = req.header("x-api-key", key);
        }
        req.send()
            .await
            .context("Semantic Scholar recommendations request failed")?
            .error_for_status()
            .context("Semantic Scholar recommendations returned error status")?
            .text()
            .await
            .context("failed to read recommendations response body")
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
    use super::*;
    use arxiv_search_rs_mcp_core::{
        arxiv::{build_query_params, normalize_paper_id, parse_response},
        html::to_markdown,
        semantic_scholar::{parse_citations, parse_recommendations},
    };
    use tokio::sync::OnceCell;

    const ATTENTION_PAPER_ID: &str = "1706.03762";

    static GLOBAL_CLIENT: OnceCell<FetchClient> = OnceCell::const_new();

    async fn get_client() -> FetchClient {
        GLOBAL_CLIENT
            .get_or_init(|| async {
                FetchClient::new(std::env::var("SEMANTIC_SCHOLAR_API_KEY").ok())
                    .await
                    .expect("failed to build test client")
            })
            .await
            .clone()
    }

    #[tokio::test]
    #[ignore = "requires network"]
    async fn search_returns_results() {
        let client = get_client().await;
        let params = build_query_params(
            "attention mechanism transformer",
            5,
            0,
            None,
            None,
            &[],
            "relevance",
        )
        .expect("failed to build query params");
        let xml = client
            .fetch_arxiv_query(&params)
            .await
            .expect("fetch failed");
        let response = parse_response(&xml).expect("parse failed");
        let papers = response.papers;
        assert!(!papers.is_empty(), "search returned no results");
        assert!(!papers[0].title.is_empty());
    }

    #[tokio::test]
    #[ignore = "requires network"]
    async fn get_abstract_known_paper() {
        let client = get_client().await;
        let id = normalize_paper_id(ATTENTION_PAPER_ID).expect("normalize failed");
        let xml = client.fetch_arxiv_by_id(&id).await.expect("fetch failed");
        let response = parse_response(&xml).expect("parse failed");
        let papers = response.papers;
        assert_eq!(papers.len(), 1);
        assert!(
            papers[0].title.to_lowercase().contains("attention"),
            "unexpected title: {}",
            papers[0].title
        );
    }

    #[tokio::test]
    #[ignore = "requires network"]
    async fn download_paper_html_path() {
        let client = get_client().await;
        let html = client.fetch_html("2303.08774").await.expect("fetch failed");
        assert!(
            html.is_some(),
            "expected HTML to be available for this paper"
        );
        let md = to_markdown(html.as_deref().expect("html was None")).expect("markdown failed");
        assert!(md.len() > 100, "markdown output was suspiciously short");
    }

    #[tokio::test]
    #[ignore = "requires network"]
    async fn citations_returns_results() {
        let client = get_client().await;
        let json = client
            .fetch_citations(ATTENTION_PAPER_ID, 5)
            .await
            .expect("fetch failed");
        let _papers = parse_citations(&json).expect("parse failed");
    }

    #[tokio::test]
    #[ignore = "requires network"]
    async fn recommendations_returns_results() {
        let client = get_client().await;
        let json = client
            .fetch_recommendations(ATTENTION_PAPER_ID, 5)
            .await
            .expect("fetch failed");
        let _papers = parse_recommendations(&json).expect("parse failed");
    }
}
