use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use rmcp::{
    handler::server::wrapper::Parameters,
    model::{
        CallToolResult, ContentBlock, ListResourcesResult, PaginatedRequestParams,
        ReadResourceRequestParams, ReadResourceResult, Resource, ResourceContents,
        ServerCapabilities, ServerInfo,
    },
    service::RequestContext,
    tool, tool_handler, tool_router, ErrorData as McpError, RoleServer, ServerHandler,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use arxiv_search_rs_mcp_core::{
    arxiv::{build_query_params, normalize_paper_id, parse_response},
    content::{prepare_paper, PreparationOptions, PreparedPaper},
    html::{extract_metadata, to_markdown, HtmlMetadata},
    pdf::extract_text,
    semantic_scholar::{parse_citations, parse_recommendations},
    Paper,
};

use crate::fetch::FetchClient;

const OPENAPI_SPEC: &str = r#"openapi: "3.0.3"
info:
  title: arXiv MCP Tools
  version: "0.1.0"
paths:
  /search:
    post:
      summary: Search arXiv papers
      requestBody:
        required: true
        content:
          application/json:
            schema:
              type: object
              required: [q]
              properties:
                q:
                  type: string
                  description: "arXiv query. Field syntax: ti: title, au: author, abs: abstract. Booleans: AND OR ANDNOT. Example: ti:attention AND au:vaswani"
                n:
                  type: integer
                  default: 10
                  minimum: 1
                  maximum: 50
                  description: Max results
                offset:
                  type: integer
                  default: 0
                  minimum: 0
                  description: "Starting index for pagination"
                from:
                  type: string
                  format: date
                  description: "Start date YYYY-MM-DD"
                to:
                  type: string
                  format: date
                  description: "End date YYYY-MM-DD"
                cats:
                  type: array
                  items:
                    type: string
                  description: "Category filter e.g. [\"cs.AI\",\"cs.LG\"]"
                sort:
                  type: string
                  enum: [relevance, date]
                  default: relevance
  /execute:
    post:
      summary: Fetch abstract, full text, citations, or recommendations
      description: "Accepts a single Operation or an array of Operations for batching."
      requestBody:
        required: true
        content:
          application/json:
            schema:
              oneOf:
                - $ref: '#/components/schemas/Operation'
                - type: array
                  items:
                    $ref: '#/components/schemas/Operation'
  /retrieve:
    post:
      summary: Retrieve, prune, and chunk a paper for LLM ingestion
      requestBody:
        required: true
        content:
          application/json:
            schema:
              $ref: '#/components/schemas/RetrieveInput'
  /hdrr:
    post:
      summary: Hybrid Document-Routed Retrieval (arXiv:2603.26815)
      description: "Two-stage retrieval: 1. Route documents P(D|q). 2. Scoped chunk search P(c|q, D)."
      requestBody:
        required: true
        content:
          application/json:
            schema:
               $ref: '#/components/schemas/HdrrInput'
  /ingest:
    post:
      summary: Bulk-ingest papers for HDRR
      description: "Ingest one or more arXiv papers into the HDRR database: fetch metadata + full text + TF-IDF embeddings."
      requestBody:
        required: true
        content:
          application/json:
            schema:
              $ref: '#/components/schemas/IngestInput'
components:
  schemas:
    HdrrInput:
      type: object
      required: [q]
      properties:
        q:
          type: string
          description: "Search query"
        limit_docs:
          type: integer
          default: 5
          description: "Stage 1: Max documents to route"
        limit_chunks:
          type: integer
          default: 10
          description: "Stage 2: Max chunks to retrieve"
        segmentation_k:
          type: number
          description: "Sensitivity for hierarchical segmentation"
    IngestInput:
      type: object
      required: [paper_ids]
      properties:
        paper_ids:
          type: array
          items:
            type: string
          description: "List of arXiv IDs to ingest"
        prune_references:
          type: boolean
          default: true
        chunk_chars:
          type: integer
          default: 4000
        chunk_overlap:
          type: integer
          default: 200
    Operation:
      type: object
      required: [op, id]
      properties:
        op:
          type: string
          enum: [abstract, download, citations, recs, retrieve]
          description: "abstract=metadata+abstract, download=full markdown text, citations=papers citing this (SS), recs=similar papers (SS), retrieve=prepared content"
        id:
          type: string
          description: "arXiv ID: \"1706.03762\", \"arxiv:1706.03762\", or \"1706.03762v2\""
        limit:
          type: integer
          default: 10
          description: "citations: max 100. recs: max 50."
        prune_references:
          type: boolean
          default: true
        chunk_chars:
          type: integer
          default: 4000
        chunk_overlap:
          type: integer
          default: 200
    RetrieveInput:
      type: object
      required: [paper_id]
      properties:
        paper_id:
          type: string
          description: "arXiv ID: \"1706.03762\", \"arxiv:1706.03762\", or \"1706.03762v2\""
        prune_references:
          type: boolean
          default: true
        chunk_chars:
          type: integer
          default: 4000
        chunk_overlap:
          type: integer
          default: 200
        segmentation_k:
          type: number
          description: "Sensitivity for hierarchical segmentation (e.g., 1.2 for 512 tokens). If set, enables hierarchical chunking."
"#;

const OPENAPI_URI: &str = "arxiv://openapi";

#[derive(Debug, Deserialize, JsonSchema)]
struct SearchInput {
    #[serde(alias = "query")]
    q: String,
    #[serde(alias = "n", alias = "max_results", default = "default_n")]
    n: u32,
    #[serde(default)]
    offset: u32,
    from: Option<String>,
    to: Option<String>,
    #[serde(alias = "categories", default)]
    cats: Vec<String>,
    #[serde(alias = "sort_by", default = "default_sort")]
    sort: String,
}

const fn default_n() -> u32 {
    10
}

fn default_sort() -> String {
    "relevance".to_string()
}

#[derive(Debug, Deserialize, JsonSchema)]
struct Operation {
    op: String,
    #[serde(alias = "paper_id")]
    id: String,
    limit: Option<u32>,
    #[serde(default = "default_true")]
    prune_references: bool,
    #[serde(default = "default_chunk_chars")]
    chunk_chars: usize,
    #[serde(default = "default_chunk_overlap")]
    chunk_overlap: usize,
    segmentation_k: Option<f32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RetrieveInput {
    #[serde(alias = "id", alias = "arxiv_id")]
    pub paper_id: String,
    #[serde(default = "default_true")]
    pub prune_references: bool,
    #[serde(default = "default_chunk_chars")]
    pub chunk_chars: usize,
    #[serde(default = "default_chunk_overlap")]
    pub chunk_overlap: usize,
    pub segmentation_k: Option<f32>,
    /// Return the legacy full payload (raw markdown, per-chunk text, embeddings)
    /// instead of the compact view that contains the paper text once.
    #[serde(default)]
    pub full_payload: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct HdrrInput {
    #[serde(alias = "query")]
    q: String,
    #[serde(default = "default_limit_docs")]
    limit_docs: usize,
    #[serde(default = "default_limit_chunks")]
    limit_chunks: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct IngestInput {
    #[serde(alias = "ids")]
    paper_ids: Vec<String>,
    #[serde(default = "default_true")]
    prune_references: bool,
    #[serde(default = "default_chunk_chars")]
    chunk_chars: usize,
    #[serde(default = "default_chunk_overlap")]
    chunk_overlap: usize,
}

const fn default_limit_docs() -> usize {
    5
}

const fn default_limit_chunks() -> usize {
    10
}

const fn default_true() -> bool {
    true
}

const fn default_chunk_chars() -> usize {
    4_000
}

const fn default_chunk_overlap() -> usize {
    200
}

/// The main MCP server implementation for native desktop environments.
///
/// Encapsulates the `FetchClient` and routes all incoming tool calls
/// (like `search` and `retrieve_paper`) to the corresponding underlying routines.
#[derive(Debug, Clone)]
pub struct ArxivServer {
    client: FetchClient,
    /// Serialized `retrieve_paper` responses keyed by normalized input, so a
    /// repeat request skips re-conversion and re-indexing entirely.
    responses: Arc<Mutex<HashMap<String, Arc<str>>>>,
}

/// Upper bound on cached `retrieve_paper` responses held in memory.
const RESPONSE_CACHE_ENTRIES: usize = 64;

/// HTML that converts to fewer characters than this is treated as a failed
/// `LaTeXML` render (arXiv serves "Untitled Document" stubs for some papers),
/// and the PDF is used instead.
const MIN_HTML_TEXT_CHARS: usize = 2_000;

/// Full text of a paper plus the metadata printed on its HTML page, if any.
struct FullText {
    source: &'static str,
    markdown: String,
    html_meta: Option<HtmlMetadata>,
}

impl ArxivServer {
    /// Constructs a new `ArxivServer` with the provided HTTP client.
    #[must_use]
    pub fn new(client: FetchClient) -> Self {
        Self {
            client,
            responses: Arc::default(),
        }
    }
}

fn format_hierarchical_chunks(prepared: &mut PreparedPaper) {
    for chunk in &mut prepared.chunks {
        let mut context = Vec::new();
        if let Some(parent) = &chunk.parent_id {
            context.push(parent.as_str());
        }
        if let Some(cluster) = &chunk.cluster_id {
            context.push(cluster.as_str());
        }

        if !context.is_empty() {
            let header = context.join(" -> ");
            chunk.text = format!("Context: {header}\n\n{}", chunk.text);
        }
    }
}

impl ArxivServer {
    /// Fetch paper metadata from the arXiv API (disk-cached by the client).
    async fn fetch_api_metadata(&self, id: &str) -> Option<Paper> {
        let xml = self.client.fetch_arxiv_by_id(id).await.ok()?;
        parse_response(&xml).ok()?.papers.into_iter().next()
    }

    fn paper_from_html(id: &str, meta: &HtmlMetadata) -> Paper {
        Paper {
            title: meta.title.clone(),
            authors: meta
                .authors
                .iter()
                .map(|name| arxiv_search_rs_mcp_core::paper::Author {
                    name: name.clone(),
                    affiliations: Vec::new(),
                })
                .collect(),
            abstract_text: meta.abstract_text.clone(),
            ..Self::fallback_paper(id)
        }
    }

    /// Fetch full text and metadata concurrently.
    ///
    /// The arXiv API lookup runs in the background while the full text is
    /// fetched. If it has not finished by the time the text is ready, the
    /// metadata printed on the HTML page is used instead of waiting, and the
    /// lookup keeps running to warm the metadata cache for next time.
    async fn fetch_paper(&self, id: &str) -> Result<(Paper, FullText), McpError> {
        let this = self.clone();
        let owned = id.to_string();
        let api = tokio::spawn(async move { this.fetch_api_metadata(&owned).await });
        let full = self.fetch_full_text(id).await?;
        let paper = match (&full.html_meta, api.is_finished()) {
            (Some(meta), false) => Self::paper_from_html(id, meta),
            (html_meta, _) => api.await.ok().flatten().unwrap_or_else(|| {
                html_meta.as_ref().map_or_else(
                    || Self::fallback_paper(id),
                    |meta| Self::paper_from_html(id, meta),
                )
            }),
        };
        Ok((paper, full))
    }

    fn fallback_paper(id: &str) -> Paper {
        Paper {
            id: id.to_string(),
            title: id.to_string(),
            authors: Vec::new(),
            abstract_text: String::new(),
            categories: Vec::new(),
            published: String::new(),
            url: format!("https://arxiv.org/abs/{id}"),
            doi: None,
            journal_ref: None,
        }
    }

    /// Try HTML first; fall back to PDF when there is no HTML or it is a
    /// degraded render. Returns a helpful error if neither yields text.
    async fn fetch_full_text(&self, id: &str) -> Result<FullText, McpError> {
        let html = self
            .client
            .fetch_html(id)
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let degraded = match html {
            Some(html) => {
                let markdown = to_markdown(&html)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                let full = FullText {
                    source: "html",
                    html_meta: extract_metadata(&html),
                    markdown,
                };
                if full.markdown.chars().count() >= MIN_HTML_TEXT_CHARS {
                    return Ok(full);
                }
                tracing::warn!(id, "HTML render is nearly empty; trying PDF");
                Some(full)
            }
            None => None,
        };
        let pdf = match self.client.fetch_pdf(id).await {
            Ok(bytes) => extract_text(&bytes).map_err(|e| e.to_string()),
            Err(e) => Err(e.to_string()),
        };
        match (pdf, degraded) {
            (Ok(markdown), degraded) => Ok(FullText {
                source: "pdf",
                markdown,
                html_meta: degraded.and_then(|d| d.html_meta),
            }),
            (Err(_), Some(degraded)) => Ok(degraded),
            (Err(e), None) => Err(McpError::internal_error(
                format!(
                    "No full text available for {id}: HTML not found and PDF fetch failed ({e}). \
                         Use the 'abstract' op via the execute tool to get metadata only."
                ),
                None,
            )),
        }
    }

    /// Store a prepared paper and its chunk embeddings for HDRR, in one
    /// transaction. Skipped when the same chunking is already stored.
    #[cfg(feature = "embedded-db")]
    fn ingest(&self, prepared: &PreparedPaper) {
        let Some(db) = &self.client.db else {
            return;
        };
        let paper = &prepared.paper;
        if db.chunk_count(&paper.id).ok() == Some(prepared.chunks.len()) {
            return;
        }
        let corpus: Vec<&str> = prepared.chunks.iter().map(|c| c.text.as_str()).collect();
        let vectorizer = arxiv_search_rs_mcp_core::tfidf::TfidfVectorizer::new(&corpus);
        let rows: Vec<_> = prepared
            .chunks
            .iter()
            .map(|c| {
                (
                    format!("{}-{}", paper.id, c.index),
                    c.text.as_str(),
                    vectorizer.vectorize(&c.text),
                    c.cluster_id.as_deref(),
                )
            })
            .collect();
        if let Err(e) = db.ingest_paper(&paper.id, &paper.title, &paper.abstract_text, &rows) {
            tracing::error!(?e, "failed to ingest {}", paper.id);
        }
    }

    async fn run_operation(&self, op: Operation) -> Result<Value, McpError> {
        let id = normalize_paper_id(&op.id)
            .map_err(|e| McpError::invalid_params(e.to_string(), None))?;

        match op.op.as_str() {
            "abstract" => {
                let xml = self
                    .client
                    .fetch_arxiv_by_id(&id)
                    .await
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                let response = parse_response(&xml)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                let paper = response.papers
                    .into_iter()
                    .next()
                    .ok_or_else(|| McpError::internal_error(format!("{id} not found"), None))?;
                serde_json::to_value(paper)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))
            }
            "download" => {
                let full = self.fetch_full_text(&id).await?;
                Ok(Value::String(full.markdown))
            }
            "citations" => {
                let limit = op.limit.unwrap_or(10).clamp(1, 100);
                let json = self
                    .client
                    .fetch_citations(&id, limit)
                    .await
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                let papers = parse_citations(&json)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                serde_json::to_value(papers)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))
            }
            "recs" => {
                let limit = op.limit.unwrap_or(10).clamp(1, 50);
                let json = self
                    .client
                    .fetch_recommendations(&id, limit)
                    .await
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                let papers = parse_recommendations(&json)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                serde_json::to_value(papers)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))
            }
            "retrieve" => {
                let (paper, full) = self.fetch_paper(&id).await?;

                let prepared = prepare_paper(
                    paper,
                    full.source,
                    full.markdown,
                    PreparationOptions {
                        prune_references: op.prune_references,
                        chunk_chars: op.chunk_chars,
                        chunk_overlap: op.chunk_overlap,
                        segmentation_k: op.segmentation_k,
                    },
                );
                Ok(prepared.agent_view())
            }
            unknown => Err(McpError::invalid_params(
                format!("unknown op \"{unknown}\"; valid: abstract, download, citations, recs, retrieve"),
                None,
            )),
        }
    }

    /// Retrieve a paper's full text, prune, chunk, and store in the HDRR database.
    /// Fetches real metadata (title, abstract) from arXiv API.
    ///
    /// # Errors
    /// Returns an error if the paper ID is invalid, the arXiv API is unreachable,
    /// or neither HTML nor PDF full text is available.
    pub async fn run_retrieve(&self, input: RetrieveInput) -> Result<Value, McpError> {
        let id = normalize_paper_id(&input.paper_id)
            .map_err(|e| McpError::invalid_params(e.to_string(), None))?;

        let (paper, full) = self.fetch_paper(&id).await?;

        let mut prepared = prepare_paper(
            paper,
            full.source,
            full.markdown,
            PreparationOptions {
                prune_references: input.prune_references,
                chunk_chars: input.chunk_chars,
                chunk_overlap: input.chunk_overlap,
                segmentation_k: input.segmentation_k,
            },
        );

        #[cfg(feature = "embedded-db")]
        self.ingest(&prepared);

        if !input.full_payload {
            return Ok(prepared.agent_view());
        }
        format_hierarchical_chunks(&mut prepared);
        serde_json::to_value(prepared).map_err(|e| McpError::internal_error(e.to_string(), None))
    }

    /// `run_retrieve`, serialized and memoized per normalized input.
    async fn retrieve_cached(&self, input: RetrieveInput) -> Result<Arc<str>, McpError> {
        let id = normalize_paper_id(&input.paper_id)
            .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
        let key = format!(
            "{id}|{}|{}|{}|{:?}|{}",
            input.prune_references,
            input.chunk_chars,
            input.chunk_overlap,
            input.segmentation_k,
            input.full_payload
        );
        if let Some(hit) = self
            .responses
            .lock()
            .ok()
            .and_then(|m| m.get(&key).cloned())
        {
            return Ok(hit);
        }
        let value = self.run_retrieve(input).await?;
        let out: Arc<str> = serde_json::to_string(&value)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?
            .into();
        if let Ok(mut cache) = self.responses.lock() {
            if cache.len() >= RESPONSE_CACHE_ENTRIES {
                cache.clear();
            }
            cache.insert(key, Arc::clone(&out));
        }
        Ok(out)
    }

    fn run_hdrr(&self, input: &HdrrInput) -> Result<Value, McpError> {
        #[cfg(not(feature = "embedded-db"))]
        return Err(McpError::internal_error(
            "embedded-db feature not enabled",
            None,
        ));

        #[cfg(feature = "embedded-db")]
        {
            let db = self
                .client
                .db
                .as_ref()
                .ok_or_else(|| McpError::internal_error("Database not initialized", None))?;

            // Stage 1: Document-level routing P(D|q)
            let routed_docs = db
                .route_documents(&input.q, input.limit_docs)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;

            if routed_docs.is_empty() {
                return Ok(serde_json::json!({
                    "query": input.q,
                    "routed_documents": [],
                    "chunks": [],
                    "message": "No documents routed in Stage 1."
                }));
            }

            // Stage 2: Scoped chunk retrieval P(c|q, D)
            // hierarchical segmentation is applied during ingestion (retrieve_paper),
            // not at query time. The stored embeddings are used for cosine-similarity routing.

            let chunks = db
                .retrieve_chunks_scoped(&input.q, &routed_docs, input.limit_chunks)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;

            Ok(serde_json::json!({
                "query": input.q,
                "routed_documents": routed_docs,
                "chunks": chunks.into_iter().map(|(id, text)| {
                    serde_json::json!({ "id": id, "text": text })
                    }).collect::<Vec<_>>()
            }))
        }
    }

    /// Bulk-ingest: fetch metadata + full text + store in DB for HDRR.
    async fn run_ingest(&self, input: IngestInput) -> Result<Value, McpError> {
        let mut ingested = 0usize;
        let mut errors = Vec::new();

        for raw_id in &input.paper_ids {
            let id = match normalize_paper_id(raw_id) {
                Ok(id) => id,
                Err(e) => {
                    errors.push(serde_json::json!({"id": raw_id, "error": e.to_string()}));
                    continue;
                }
            };

            let (paper, full) = match self.fetch_paper(&id).await {
                Ok(result) => result,
                Err(e) => {
                    errors.push(serde_json::json!({"id": id, "error": e.to_string()}));
                    continue;
                }
            };

            let prepared = prepare_paper(
                paper,
                full.source,
                full.markdown,
                PreparationOptions {
                    prune_references: input.prune_references,
                    chunk_chars: input.chunk_chars,
                    chunk_overlap: input.chunk_overlap,
                    segmentation_k: None,
                },
            );

            #[cfg(feature = "embedded-db")]
            self.ingest(&prepared);

            ingested += 1;
        }

        Ok(serde_json::json!({
            "ingested": ingested,
            "errors": errors,
            "total_requested": input.paper_ids.len(),
        }))
    }
}

#[tool_router]
impl ArxivServer {
    #[tool(description = "Search arXiv papers with filters (categories, dates, sorting).")]
    async fn search(
        &self,
        Parameters(input): Parameters<SearchInput>,
    ) -> Result<CallToolResult, McpError> {
        let span = tracing::info_span!("mcp_tool_search");
        let _enter = span.enter();

        tracing::info!("arXiv search: q='{}', n={}", input.q, input.n);

        let params = build_query_params(
            &input.q,
            input.n,
            input.offset,
            input.from.as_deref(),
            input.to.as_deref(),
            &input.cats,
            &input.sort,
        )
        .map_err(|e| McpError::invalid_params(e.to_string(), None))?;

        tracing::debug!("arXiv query params: {:?}", params.search_query);

        let xml = self
            .client
            .fetch_arxiv_query(&params)
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        tracing::info!("arXiv search: received {} bytes", xml.len());

        let response =
            parse_response(&xml).map_err(|e| McpError::internal_error(e.to_string(), None))?;

        tracing::info!("arXiv search: parsed {} papers", response.papers.len());

        let out = serde_json::to_string(&response)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![ContentBlock::text(out)]))
    }

    #[tool(
        description = "Get paper content, pruned and chunked. Supports hierarchical segmentation (segmentation_k)."
    )]
    async fn retrieve_paper(
        &self,
        Parameters(input): Parameters<RetrieveInput>,
    ) -> Result<CallToolResult, McpError> {
        let span = tracing::info_span!("mcp_tool_retrieve_paper");
        let _enter = span.enter();
        let out = self.retrieve_cached(input).await?;
        Ok(CallToolResult::success(vec![ContentBlock::text(
            out.to_string(),
        )]))
    }

    #[tool(
        description = "Hybrid Document-Routed Retrieval (HDRR). Two-stage: 1. Route docs, 2. Scoped chunk search."
    )]
    async fn hdrr(
        &self,
        Parameters(input): Parameters<HdrrInput>,
    ) -> Result<CallToolResult, McpError> {
        let span = tracing::info_span!("mcp_tool_hdrr");
        let _enter = span.enter();
        let out = self.run_hdrr(&input)?;
        let out = serde_json::to_string(&out)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![ContentBlock::text(out)]))
    }

    #[tool(description = "Fetch abstract, full text, citations, or recommendations for a paper.")]
    async fn execute(
        &self,
        Parameters(op): Parameters<Operation>,
    ) -> Result<CallToolResult, McpError> {
        let span = tracing::info_span!("mcp_tool_execute");
        let _enter = span.enter();
        let id = op.id.clone();
        let op_name = op.op.clone();
        let result = self.run_operation(op).await;
        let out = serde_json::to_string(&serde_json::json!({
            "id": id,
            "op": op_name,
            "result": match result {
                Ok(v) => v,
                Err(e) => serde_json::json!({"error": e.to_string()}),
            }
        }))
        .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![ContentBlock::text(out)]))
    }

    #[tool(
        description = "Bulk-ingest papers into the HDRR database. Takes paper_ids, fetches metadata+full text, stores TF-IDF embeddings. Required before hdrr can route to these papers."
    )]
    async fn ingest_corpus(
        &self,
        Parameters(input): Parameters<IngestInput>,
    ) -> Result<CallToolResult, McpError> {
        let span = tracing::info_span!("mcp_tool_ingest_corpus");
        let _enter = span.enter();
        let out_value = self.run_ingest(input).await?;
        let out = serde_json::to_string(&out_value)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![ContentBlock::text(out)]))
    }
}

#[expect(clippy::unused_async_trait_impl)]
#[tool_handler]
impl ServerHandler for ArxivServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(rmcp::model::Implementation::new(
            "arxiv-search-mcp",
            env!("CARGO_PKG_VERSION"),
        ))
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourcesResult, McpError>> + Send + '_ {
        std::future::ready(Ok(ListResourcesResult {
            meta: None,
            resources: vec![Resource::new(OPENAPI_URI, "arXiv MCP OpenAPI Schema")
                .with_description(
                    "OpenAPI 3.0 schema for search, retrieve, and legacy execute inputs.",
                )
                .with_mime_type("application/yaml")
                .with_size(OPENAPI_SPEC.len() as u64)],
            next_cursor: None,
        }))
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ReadResourceResult, McpError>> + Send + '_ {
        let result = if request.uri == OPENAPI_URI {
            Ok(ReadResourceResult::new(vec![
                ResourceContents::TextResourceContents {
                    uri: OPENAPI_URI.to_string(),
                    mime_type: Some("application/yaml".to_string()),
                    text: OPENAPI_SPEC.to_string(),
                    meta: None,
                },
            ]))
        } else {
            Err(McpError::resource_not_found(
                format!("unknown resource: {}", request.uri),
                None,
            ))
        };
        std::future::ready(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arxiv_search_rs_mcp_core::content::PaperChunk;

    #[test]
    fn test_hierarchical_formatting() {
        let mut prepared = PreparedPaper {
            paper: Paper {
                id: "test".into(),
                title: "test".into(),
                authors: vec![],
                abstract_text: String::new(),
                categories: vec![],
                published: String::new(),
                url: String::new(),
                doi: None,
                journal_ref: None,
            },
            source: "test".into(),
            raw_markdown: String::new(),
            pruned_markdown: String::new(),
            chunks: vec![
                PaperChunk {
                    index: 0,
                    start_char: 0,
                    end_char: 4,
                    text: "body".into(),
                    cluster_id: Some("Cluster A".into()),
                    parent_id: Some("Header 1".into()),
                },
                PaperChunk {
                    index: 1,
                    start_char: 5,
                    end_char: 9,
                    text: "body2".into(),
                    cluster_id: None,
                    parent_id: Some("Header 2".into()),
                },
                PaperChunk {
                    index: 2,
                    start_char: 10,
                    end_char: 14,
                    text: "body3".into(),
                    cluster_id: None,
                    parent_id: None,
                },
            ],
            hierarchical_chunks: None,
        };

        format_hierarchical_chunks(&mut prepared);

        assert_eq!(
            prepared.chunks[0].text,
            "Context: Header 1 -> Cluster A\n\nbody"
        );
        assert_eq!(prepared.chunks[1].text, "Context: Header 2\n\nbody2");
        assert_eq!(prepared.chunks[2].text, "body3");
    }

    #[test]
    #[expect(clippy::expect_used)]
    fn test_serialization_includes_metadata() {
        let chunk = PaperChunk {
            index: 0,
            start_char: 0,
            end_char: 4,
            text: "body".into(),
            cluster_id: Some("Cluster A".into()),
            parent_id: Some("Header 1".into()),
        };
        let json = serde_json::to_value(&chunk).expect("should serialize");
        assert_eq!(json["cluster_id"], "Cluster A");
        assert_eq!(json["parent_id"], "Header 1");

        let flat_chunk = PaperChunk {
            index: 1,
            start_char: 5,
            end_char: 9,
            text: "body2".into(),
            cluster_id: None,
            parent_id: None,
        };
        let json_flat = serde_json::to_value(&flat_chunk).expect("should serialize flat");
        assert!(json_flat["cluster_id"].is_null());
        assert!(json_flat["parent_id"].is_null());
    }
}
