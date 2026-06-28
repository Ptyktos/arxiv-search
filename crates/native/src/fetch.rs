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
    api_base: String,
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
            .user_agent("curl/8.20.0")
            .timeout(Duration::from_secs(15))
            .connect_timeout(Duration::from_secs(5))
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
            api_base: ARXIV_API_BASE.to_string(),
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
            .get(&self.api_base)
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
    pub(crate) async fn scrape_arxiv_search(&self, params: &QueryParams) -> Result<String> {
        let span = tracing::info_span!("arxiv_html_fallback");
        let _enter = span.enter();

        tracing::info!(
            "Scraping arXiv search HTML for query: {}",
            params.search_query
        );

        let search_url = "https://arxiv.org/search/";
        // The web search interface does not understand Lucene field qualifiers or boolean
        // operators — send the original plain-text user query instead of the API's Lucene
        // search_query.  Sort order: web search uses "-relevance" for best-match or
        // "-announced_date_first" for newest-first.
        let order = if params.sort_by == "submittedDate" {
            "-announced_date_first"
        } else {
            "-relevance"
        };
        let response = self.client.get(search_url)
            .query(&[
                ("query", params.raw_query.as_str()),
                ("searchtype", "all"),
                ("abstracts", "show"),
                ("size", "50"), // arXiv web search requires standard sizes (25, 50, 100)
                ("order", order),
            ])
            .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8")
            .header("Accept-Language", "en-US,en;q=0.9")
            .header("Cache-Control", "max-age=0")
            .send()
            .await
            .context("HTML search fallback failed")?;

        let status = response.status();
        let html = response
            .text()
            .await
            .context("failed to read HTML search body")?;

        // Diagnostic 'flagging' of the response for visibility in tests
        if html.contains("Rate exceeded") || html.contains("Access Denied") {
            tracing::warn!(
                "ARXIV BLOCK DETECTED: {}",
                if html.contains("Rate exceeded") {
                    "Rate exceeded"
                } else {
                    "Access Denied"
                }
            );
        }

        if !html.is_empty() {
            let snippet: String = html.chars().take(200).collect();
            tracing::debug!("HTML STATUS: {} | BODY SNIPPET: {}", status, snippet);
        }

        let mut entries = Vec::new();

        let re_block = regex::Regex::new(r"(?is)<li class=.arxiv-result.>(.*?)</li>")?;
        let re_id = regex::Regex::new(r"(?i)arxiv:(\d+\.\d+v?\d*)")?;
        let re_title = regex::Regex::new(r#"(?is)<p class="title is-5 mathjax">\s*(.*?)\s*</p>"#)?;
        let re_authors_block = regex::Regex::new(r"(?is)<p class=.authors.>(.*?)</p>")?;
        let re_author_name = regex::Regex::new(r"(?i)<a [^>]*>(.*?)</a>")?;
        let re_abstract =
            regex::Regex::new(r#"(?is)<span class="abstract-full.*?">\s*(.*?)\s*</span>"#)?;
        let re_categories = regex::Regex::new(r#"(?i)<span class="tag[^>]*>(.*?)</span>"#)?;
        let re_date = regex::Regex::new(r"(?is)Submitted</span>\s*(.*?)\s*;")?;
        let re_strip = regex::Regex::new(r"<[^>]*>")?;

        for cap in re_block.captures_iter(&html) {
            let block = &cap[1];
            let id = re_id.captures(block).map(|c| c[1].to_string());
            let title = re_title
                .captures(block)
                .map(|c| re_strip.replace_all(&c[1], "").trim().to_string());
            let abstract_text = re_abstract
                .captures(block)
                .map(|c| re_strip.replace_all(&c[1], "").trim().to_string())
                .unwrap_or_default();
            let date = re_date
                .captures(block)
                .map(|c| c[1].to_string())
                .unwrap_or_default();

            let mut authors_xml = String::new();
            if let Some(auth_cap) = re_authors_block.captures(block) {
                for auth in re_author_name.captures_iter(&auth_cap[1]) {
                    let _ = std::fmt::write(
                        &mut authors_xml,
                        format_args!("<author><name>{}</name></author>", &auth[1]),
                    );
                }
            }

            let mut categories_xml = String::new();
            for cat in re_categories.captures_iter(block) {
                let _ = std::fmt::write(
                    &mut categories_xml,
                    format_args!(r#"<category term="{}" />"#, &cat[1]),
                );
            }

            if let (Some(id), Some(title)) = (id, title) {
                entries.push(format!(
                    r#"<entry>
                        <id>http://arxiv.org/abs/{id}</id>
                        <title>{title}</title>
                        <summary>{abstract_text}</summary>
                        <published>{date}</published>
                        {authors_xml}
                        {categories_xml}
                        <link href="http://arxiv.org/abs/{id}" rel="alternate" type="text/html"/>
                    </entry>"#
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

        let text = response
            .text()
            .await
            .context("failed to read arXiv response body")?;
        let _ = self.cache.set_metadata(paper_id, &text).await;
        Ok(text)
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
            ("fields", "title,authors,year,externalIds,paperId"),
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
        let mut req = self.client.get(&url).query(&[
            ("limit", &limit.to_string()),
            (
                "fields",
                &"title,authors,year,externalIds,paperId".to_string(),
            ),
        ]);
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
    use arxiv_search_rs_mcp_core::arxiv::{build_query_params, parse_response};
    use tokio::sync::OnceCell;

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
    }

    #[test]
    #[expect(clippy::expect_used)]
    fn test_html_scraping_regex() {
        let html = r#"
    <li class="arxiv-result">
      <p class="list-title is-inline-block"><a href="https://arxiv.org/abs/2605.11861">arXiv:2605.11861</a></p>
      <p class="title is-5 mathjax">
        Observation of sine-Gordon-like solitons in a spinor Bose-<span class="search-hit mathjax">Einstein</span> condensate
      </p>
      <p class="authors">
        <span class="has-text-black-bis has-text-weight-semibold">Authors:</span>
        <a href="/search/cond-mat?searchtype=author&amp;query=Conti%2C+D">Diego Conti</a>, 
        <a href="/search/cond-mat?searchtype=author&amp;query=Rossi%2C+F+A">Federico A. Rossi</a>
      </p>
      <p class="abstract">
        <span class="abstract-full has-text-grey-dark" id="abs-2605.11861"> This is the full abstract text. </span>
      </p>
      <div class="is-marginless">
        <span class="has-text-black-bis has-text-weight-semibold">Submitted</span> 12 May, 2026; 
        <span class="tag is-small is-link is-light">math.DG</span>
      </div>
    </li>
        "#;

        let re_block = regex::Regex::new(r"(?is)<li class=.arxiv-result.>(.*?)</li>")
            .expect("regex: arxiv result block");
        let re_id = regex::Regex::new(r"(?i)arxiv:(\d+\.\d+v?\d*)").expect("regex: arxiv ID");
        let re_title = regex::Regex::new(r#"(?is)<p class="title is-5 mathjax">\s*(.*?)\s*</p>"#)
            .expect("regex: arxiv title");
        let re_authors_block =
            regex::Regex::new(r"(?is)<p class=.authors.>(.*?)</p>").expect("regex: authors block");
        let re_author_name =
            regex::Regex::new(r"(?i)<a [^>]*>(.*?)</a>").expect("regex: author name");
        let re_abstract =
            regex::Regex::new(r#"(?is)<span class="abstract-full.*?">\s*(.*?)\s*</span>"#)
                .expect("regex: abstract");
        let re_categories = regex::Regex::new(r#"(?i)<span class="tag[^>]*>(.*?)</span>"#)
            .expect("regex: category tag");
        let re_date =
            regex::Regex::new(r"(?is)Submitted</span>\s*(.*?)\s*;").expect("regex: submitted date");
        let re_strip = regex::Regex::new(r"<[^>]*>").expect("regex: HTML tag");

        let mut entries = Vec::new();
        for cap in re_block.captures_iter(html) {
            let block = &cap[1];
            let id = re_id.captures(block).map(|c| c[1].to_string());
            let title = re_title
                .captures(block)
                .map(|c| re_strip.replace_all(&c[1], "").trim().to_string());
            let abstract_text = re_abstract
                .captures(block)
                .map(|c| re_strip.replace_all(&c[1], "").trim().to_string())
                .unwrap_or_default();
            let date = re_date
                .captures(block)
                .map(|c| c[1].to_string())
                .unwrap_or_default();

            let mut authors = Vec::new();
            if let Some(auth_cap) = re_authors_block.captures(block) {
                for auth in re_author_name.captures_iter(&auth_cap[1]) {
                    authors.push(auth[1].to_string());
                }
            }

            let mut categories = Vec::new();
            for cat in re_categories.captures_iter(block) {
                categories.push(cat[1].to_string());
            }

            if let (Some(id), Some(title)) = (id, title) {
                entries.push((id, title, abstract_text, date, authors, categories));
            }
        }

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "2605.11861");
        assert_eq!(
            entries[0].1,
            "Observation of sine-Gordon-like solitons in a spinor Bose-Einstein condensate"
        );
        assert_eq!(entries[0].2, "This is the full abstract text.");
        assert_eq!(entries[0].3, "12 May, 2026");
        assert_eq!(entries[0].4, vec!["Diego Conti", "Federico A. Rossi"]);
        assert_eq!(entries[0].5, vec!["math.DG"]);
    }

    #[tokio::test]
    #[ignore = "requires network"]
    #[expect(clippy::expect_used)]
    async fn test_scrape_arxiv_search_real() {
        let client = get_client().await;
        let params = build_query_params("Einstein", 5, 0, None, None, &[], "relevance")
            .expect("valid params");

        let xml = client
            .scrape_arxiv_search(&params)
            .await
            .expect("HTML scraping failed");
        let response = parse_response(&xml).expect("Failed to parse scraped XML");

        assert!(
            !response.papers.is_empty(),
            "Scraper returned no papers for 'Einstein'"
        );
        assert!(
            response
                .papers
                .iter()
                .any(|p| p.title.to_lowercase().contains("einstein")),
            "None of the scraped papers contain 'Einstein' in the title"
        );
    }

    #[tokio::test]
    #[ignore = "requires network"]
    #[expect(clippy::expect_used)]
    async fn test_real_orchestrator_fallback() {
        let mut client = get_client().await;
        // Break the API base to force the orchestrator to fall back to HTML scraping
        client.api_base = "https://broken.example.com/api".to_string();

        let params = build_query_params("Einstein", 3, 0, None, None, &[], "relevance")
            .expect("valid params");

        // This should hit the broken API, log an error, then successfully scrape arXiv.org
        let xml = client
            .fetch_arxiv_query(&params)
            .await
            .expect("Orchestrator fallback failed");
        tracing::debug!("FALLBACK XML: {xml}");
        let response = parse_response(&xml).expect("Failed to parse scraped XML from orchestrator");

        assert!(
            !response.papers.is_empty(),
            "Orchestrator fallback returned no papers for 'Einstein'"
        );
        assert!(
            response
                .papers
                .iter()
                .any(|p| p.title.to_lowercase().contains("einstein")),
            "None of the papers contain 'Einstein'"
        );
    }
}
