use std::time::Duration;

use anyhow::{Context as _, Result};
use reqwest::Client;

use crate::persistence::{ArxivCache, DEFAULT_CACHE_TTL};
use arxiv_search_rs_mcp_core::arxiv::QueryParams;

const ARXIV_API_BASE: &str = "https://export.arxiv.org/api/query";
const ARXIV_HTML_BASE: &str = "https://arxiv.org/html";
const ARXIV_PDF_BASE: &str = "https://arxiv.org/pdf";
const SS_API_BASE: &str = "https://api.semanticscholar.org/graph/v1";
const SS_REC_BASE: &str = "https://api.semanticscholar.org/recommendations/v1";

/// Authenticated HTTP client for arXiv and Semantic Scholar.
///
/// This version is KISS: no cross-process locking. It relies on immediate
/// HTML fallback if the API is rate-limited or slow.
#[derive(Clone)]
pub struct FetchClient {
    client: Client,
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
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36")
            .timeout(Duration::from_secs(5))
            .connect_timeout(Duration::from_secs(2))
            .pool_idle_timeout(Duration::from_secs(90))
            .tcp_keepalive(Some(Duration::from_secs(60)))
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
            ss_api_key,
            cache,
            #[cfg(feature = "embedded-db")]
            db,
        })
    }

    /// # Errors
    ///
    /// Returns an error if the HTTP request fails. Falls back to HTML search immediately on API issues.
    pub async fn fetch_arxiv_query(&self, params: &QueryParams) -> Result<String> {
        let span = tracing::info_span!("arxiv_query_orchestrator");
        let _enter = span.enter();

        tracing::info!("Starting arXiv query for: {}", params.search_query);
        
        let response_result = self
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
            .await;

        match response_result {
            Ok(response) => {
                let status = response.status();
                if status.is_success() {
                    return response.text().await.context("failed to read response");
                }
                
                tracing::warn!(%status, "API issue, falling back to HTML immediately");
            }
            Err(e) => {
                tracing::error!(?e, "API request failed, falling back to HTML");
            }
        }
        
        // Immediate fallback to HTML scraping if API fails or is limited
        self.scrape_arxiv_search(params).await
    }

    /// Scrapes the arXiv search page as a fallback for the API.
    async fn scrape_arxiv_search(&self, params: &QueryParams) -> Result<String> {
        let span = tracing::info_span!("arxiv_html_fallback");
        let _enter = span.enter();
        
        tracing::info!("Scraping arXiv search HTML for query: {}", params.search_query);

        let search_url = "https://arxiv.org/search/";
        let response = self.client.get(search_url)
            .query(&[
                ("query", params.search_query.as_str()),
                ("searchtype", "all"),
                ("source", "header"),
                ("size", &params.max_results.to_string()),
                ("start", &params.start.to_string()),
            ])
            .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
            .send()
            .await
            .context("HTML search fallback failed")?;

        let html = response.text().await.context("failed to read HTML search body")?;
        
        let mut entries = Vec::new();
        
        let re_block = regex::Regex::new(r"(?s)<li class=.arxiv-result.>(.*?)</li>").unwrap();
        let re_id = regex::Regex::new(r"arxiv\.org/abs/(\d+\.\d+v?\d*)").unwrap();
        let re_title = regex::Regex::new(r#"(?s)<p class="title is-5 mathjax">\s*(.*?)\s*</p>"#).unwrap();
        let re_strip = regex::Regex::new(r"<[^>]*>").unwrap();

        for cap in re_block.captures_iter(&html) {
            let block = &cap[1];
            let id = re_id.captures(block).map(|c| c[1].to_string());
            let title = re_title.captures(block).map(|c| {
                let t = c[1].to_string();
                re_strip.replace_all(&t, "").trim().to_string()
            });

            if let (Some(id), Some(title)) = (id, title) {
                entries.push(format!(
                    r#"<entry><id>http://arxiv.org/abs/{id}</id><title>{title}</title><link href="http://arxiv.org/abs/{id}" rel="alternate" type="text/html"/></entry>"#
                ));
            }
        }

        if entries.is_empty() && !html.contains("no results") {
            tracing::warn!("HTML fallback found no entries but page doesn't say 'no results'");
        }

        Ok(format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><feed xmlns="http://www.w3.org/2005/Atom">{}</feed>"#,
            entries.join("")
        ))
    }

    /// # Errors
    ///
    /// Returns an error if the HTTP request fails.
    pub async fn fetch_arxiv_by_id(&self, paper_id: &str) -> Result<String> {
        if let Some(cached) = self.cache.get_metadata(paper_id).await? {
            tracing::info!("Cache hit for metadata: {}", paper_id);
            return Ok(cached);
        }

        let response = self
            .client
            .get(ARXIV_API_BASE)
            .query(&[("id_list", paper_id)])
            .send()
            .await
            .context("arXiv ID lookup request failed")?;

        if !response.status().is_success() {
            anyhow::bail!("arXiv API error: status {}", response.status());
        }

        let text = response.text().await.context("failed to read arXiv response body")?;
        let _ = self.cache.set_metadata(paper_id, &text).await;
        Ok(text)
    }

    /// # Errors
    ///
    /// Returns an error if the HTTP request fails or the server returns a non-404 error status.
    pub async fn fetch_html(&self, paper_id: &str) -> Result<Option<String>> {
        if let Some(cached) = self.cache.get_html(paper_id).await? {
            tracing::info!("Cache hit for HTML: {}", paper_id);
            return Ok(Some(cached));
        }

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
    /// Returns an error if the HTTP request fails.
    pub async fn fetch_pdf(&self, paper_id: &str) -> Result<Vec<u8>> {
        if let Some(cached) = self.cache.get_pdf(paper_id).await? {
            tracing::info!("Cache hit for PDF: {}", paper_id);
            return Ok(cached);
        }

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
    /// Returns an error if the HTTP request fails.
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
    /// Returns an error if the HTTP request fails.
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

    #[test]
    fn test_html_scraping_regex() {
        let html = r#"
    <li class="arxiv-result">
      <p class="list-title is-inline-block"><a href="https://arxiv.org/abs/2605.11861">arXiv:2605.11861</a></p>
      <p class="title is-5 mathjax">
        Observation of sine-Gordon-like solitons in a spinor Bose-<span class="search-hit mathjax">Einstein</span> condensate
      </p>
    </li>
        "#;
        
        let mut entries = Vec::new();
        let re_block = regex::Regex::new(r"(?s)<li class=.arxiv-result.>(.*?)</li>").unwrap();
        let re_id = regex::Regex::new(r"arxiv\.org/abs/(\d+\.\d+v?\d*)").unwrap();
        let re_title = regex::Regex::new(r#"(?s)<p class="title is-5 mathjax">\s*(.*?)\s*</p>"#).unwrap();
        let re_strip = regex::Regex::new(r"<[^>]*>").unwrap();

        for cap in re_block.captures_iter(html) {
            let block = &cap[1];
            let id = re_id.captures(block).map(|c| c[1].to_string());
            let title = re_title.captures(block).map(|c| {
                let t = c[1].to_string();
                re_strip.replace_all(&t, "").trim().to_string()
            });

            if let (Some(id), Some(title)) = (id, title) {
                entries.push((id, title));
            }
        }
        
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "2605.11861");
        assert_eq!(entries[0].1, "Observation of sine-Gordon-like solitons in a spinor Bose-Einstein condensate");
    }
}
