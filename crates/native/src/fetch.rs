use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use reqwest::Client;
use tokio::sync::Mutex;

use arxiv_search_rs_mcp_core::arxiv::QueryParams;

const ARXIV_API_BASE: &str = "https://export.arxiv.org/api/query";
const ARXIV_HTML_BASE: &str = "https://arxiv.org/html";
const ARXIV_PDF_BASE: &str = "https://arxiv.org/pdf";
const SS_API_BASE: &str = "https://api.semanticscholar.org/graph/v1";
const SS_REC_BASE: &str = "https://api.semanticscholar.org/recommendations/v1";
const ARXIV_RATE_LIMIT: Duration = Duration::from_secs(3);

#[derive(Clone)]
pub struct FetchClient {
    client: Client,
    last_arxiv_request: Arc<Mutex<Option<Instant>>>,
    ss_api_key: Option<String>,
}

impl FetchClient {
    /// # Errors
    ///
    /// Returns an error if the HTTP client fails to build.
    pub fn new(ss_api_key: Option<String>) -> Result<Self> {
        let client = Client::builder()
            .user_agent(concat!(
                "arxiv-search-rs-mcp/",
                env!("CARGO_PKG_VERSION"),
                " (Rust MCP server)"
            ))
            .timeout(Duration::from_secs(60))
            .build()
            .context("failed to build HTTP client")?;
        Ok(Self {
            client,
            last_arxiv_request: Arc::new(Mutex::new(None)),
            ss_api_key,
        })
    }

    async fn arxiv_rate_limit(&self) {
        let sleep_duration = {
            let mut last = self.last_arxiv_request.lock().await;
            let elapsed = last.map(|t| t.elapsed());
            *last = Some(Instant::now());
            elapsed.and_then(|e| ARXIV_RATE_LIMIT.checked_sub(e))
        };
        if let Some(d) = sleep_duration {
            tokio::time::sleep(d).await;
        }
    }

    /// # Errors
    ///
    /// Returns an error if the HTTP request fails, the server returns an error status, or the
    /// response body cannot be read.
    pub async fn fetch_arxiv_query(&self, params: &QueryParams) -> Result<String> {
        self.arxiv_rate_limit().await;
        self.client
            .get(ARXIV_API_BASE)
            .query(&[
                ("search_query", params.search_query.as_str()),
                ("max_results", &params.max_results.to_string()),
                ("sortBy", params.sort_by.as_str()),
                ("sortOrder", params.sort_order.as_str()),
            ])
            .send()
            .await
            .context("arXiv API request failed")?
            .error_for_status()
            .context("arXiv API returned error status")?
            .text()
            .await
            .context("failed to read arXiv response body")
    }

    /// # Errors
    ///
    /// Returns an error if the HTTP request fails, the server returns an error status, or the
    /// response body cannot be read.
    pub async fn fetch_arxiv_by_id(&self, paper_id: &str) -> Result<String> {
        self.arxiv_rate_limit().await;
        self.client
            .get(ARXIV_API_BASE)
            .query(&[("id_list", paper_id)])
            .send()
            .await
            .context("arXiv ID lookup request failed")?
            .error_for_status()
            .context("arXiv API returned error status")?
            .text()
            .await
            .context("failed to read arXiv response body")
    }

    /// # Errors
    ///
    /// Returns an error if the HTTP request fails or the server returns a non-404 error status.
    /// Returns `Ok(None)` if the HTML version does not exist (404).
    pub async fn fetch_html(&self, paper_id: &str) -> Result<Option<String>> {
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
        Ok(Some(text))
    }

    /// # Errors
    ///
    /// Returns an error if the HTTP request fails, the server returns an error status, or the
    /// response body cannot be read.
    pub async fn fetch_pdf(&self, paper_id: &str) -> Result<Vec<u8>> {
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
        Ok(bytes.to_vec())
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
