use rmcp::{
    model::{CallToolResult, Content, ServerCapabilities, ServerInfo},
    tool, ServerHandler,
};

use arxiv_search_rs_mcp_core::{
    arxiv::{build_query_params, normalize_paper_id, parse_response},
    html::to_markdown,
    pdf::extract_text,
    semantic_scholar::{parse_citations, parse_recommendations},
};

use crate::fetch::FetchClient;

#[derive(Debug, Clone)]
pub struct ArxivServer {
    client: FetchClient,
}

impl ArxivServer {
    #[must_use]
    pub fn new(client: FetchClient) -> Self {
        Self { client }
    }
}

#[tool(tool_box)]
impl ArxivServer {
    #[tool(description = "Search arXiv for papers. Returns metadata, authors, abstracts, and \
        categories. Supports arXiv field syntax: ti: (title), au: (author), abs: (abstract), \
        and boolean AND/OR/ANDNOT operators.")]
    async fn search_papers(
        &self,
        #[tool(param)]
        #[schemars(description = "Search query. Example: \"ti:attention AND au:vaswani\"")]
        query: String,
        #[tool(param)]
        #[schemars(description = "Maximum results to return (1–50, default 10).")]
        max_results: Option<u32>,
        #[tool(param)]
        #[schemars(description = "Start date filter in YYYY-MM-DD format.")]
        date_from: Option<String>,
        #[tool(param)]
        #[schemars(description = "End date filter in YYYY-MM-DD format.")]
        date_to: Option<String>,
        #[tool(param)]
        #[schemars(
            description = "arXiv categories to filter by, e.g. [\"cs.AI\", \"cs.LG\"]."
        )]
        categories: Option<Vec<String>>,
        #[tool(param)]
        #[schemars(description = "Sort by \"relevance\" (default) or \"date\".")]
        sort_by: Option<String>,
    ) -> Result<CallToolResult, rmcp::Error> {
        let params = build_query_params(
            &query,
            max_results.unwrap_or(10),
            date_from.as_deref(),
            date_to.as_deref(),
            &categories.unwrap_or_default(),
            sort_by.as_deref().unwrap_or("relevance"),
        )
        .map_err(|e| rmcp::Error::invalid_params(e.to_string(), None))?;

        let xml = self
            .client
            .fetch_arxiv_query(&params)
            .await
            .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;

        let papers = parse_response(&xml)
            .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;

        let json = serde_json::to_string_pretty(&papers)
            .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Fetch metadata and abstract for a specific arXiv paper by ID. \
        Accepts IDs in formats: \"2103.12345\", \"arxiv:2103.12345\", or \"2103.12345v2\".")]
    async fn get_abstract(
        &self,
        #[tool(param)]
        #[schemars(
            description = "arXiv paper ID, e.g. \"2103.12345\" or \"arxiv:2103.12345\"."
        )]
        paper_id: String,
    ) -> Result<CallToolResult, rmcp::Error> {
        let id = normalize_paper_id(&paper_id)
            .map_err(|e| rmcp::Error::invalid_params(e.to_string(), None))?;

        let xml = self
            .client
            .fetch_arxiv_by_id(&id)
            .await
            .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;

        let papers = parse_response(&xml)
            .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;

        let paper = papers.into_iter().next().ok_or_else(|| {
            rmcp::Error::internal_error(format!("paper {id} not found on arXiv"), None)
        })?;

        let json = serde_json::to_string_pretty(&paper)
            .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Download an arXiv paper and return its content as markdown. \
        Tries the HTML version first (clean, structured output); falls back to PDF text \
        extraction if HTML is unavailable. Content is returned directly — not stored on disk.")]
    async fn download_paper(
        &self,
        #[tool(param)]
        #[schemars(
            description = "arXiv paper ID, e.g. \"2103.12345\" or \"arxiv:2103.12345\"."
        )]
        paper_id: String,
    ) -> Result<CallToolResult, rmcp::Error> {
        let id = normalize_paper_id(&paper_id)
            .map_err(|e| rmcp::Error::invalid_params(e.to_string(), None))?;

        if let Some(html) = self
            .client
            .fetch_html(&id)
            .await
            .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?
        {
            let md = to_markdown(&html)
                .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;
            return Ok(CallToolResult::success(vec![Content::text(md)]));
        }

        let pdf_bytes = self
            .client
            .fetch_pdf(&id)
            .await
            .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;

        let text = extract_text(&pdf_bytes)
            .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(description = "Get papers that cite a given arXiv paper, using the Semantic \
        Scholar API. Set SEMANTIC_SCHOLAR_API_KEY environment variable for higher rate limits.")]
    async fn get_citations(
        &self,
        #[tool(param)]
        #[schemars(description = "arXiv paper ID, e.g. \"2103.12345\".")]
        paper_id: String,
        #[tool(param)]
        #[schemars(description = "Maximum number of citations to return (1–100, default 10).")]
        limit: Option<u32>,
    ) -> Result<CallToolResult, rmcp::Error> {
        let id = normalize_paper_id(&paper_id)
            .map_err(|e| rmcp::Error::invalid_params(e.to_string(), None))?;

        let limit = limit.unwrap_or(10).clamp(1, 100);

        let json = self
            .client
            .fetch_citations(&id, limit)
            .await
            .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;

        let papers = parse_citations(&json)
            .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;

        let out = serde_json::to_string_pretty(&papers)
            .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(out)]))
    }

    #[tool(description = "Get recommended papers similar to a given arXiv paper, using \
        the Semantic Scholar recommendations API. Set SEMANTIC_SCHOLAR_API_KEY for higher \
        rate limits.")]
    async fn get_recommendations(
        &self,
        #[tool(param)]
        #[schemars(description = "arXiv paper ID, e.g. \"2103.12345\".")]
        paper_id: String,
        #[tool(param)]
        #[schemars(
            description = "Maximum number of recommendations to return (1–50, default 10)."
        )]
        limit: Option<u32>,
    ) -> Result<CallToolResult, rmcp::Error> {
        let id = normalize_paper_id(&paper_id)
            .map_err(|e| rmcp::Error::invalid_params(e.to_string(), None))?;

        let limit = limit.unwrap_or(10).clamp(1, 50);

        let json = self
            .client
            .fetch_recommendations(&id, limit)
            .await
            .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;

        let papers = parse_recommendations(&json)
            .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;

        let out = serde_json::to_string_pretty(&papers)
            .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(out)]))
    }
}

#[tool(tool_box)]
impl ServerHandler for ArxivServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: rmcp::model::Implementation {
                name: "arxiv-search-mcp".into(),
                version: env!("CARGO_PKG_VERSION").into(),
            },
            ..Default::default()
        }
    }
}
