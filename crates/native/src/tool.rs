use std::future::Future;

use rmcp::{
    model::{
        Annotated, CallToolResult, Content, ListResourcesResult, PaginatedRequestParam,
        RawResource, ReadResourceRequestParam, ReadResourceResult, ResourceContents,
        ServerCapabilities, ServerInfo,
    },
    service::RequestContext,
    tool, Error as McpError, RoleServer, ServerHandler,
};
use serde::Deserialize;
use serde_json::Value;

use arxiv_search_rs_mcp_core::{
    arxiv::{build_query_params, normalize_paper_id, parse_response},
    content::{prepare_paper, PreparationOptions, PreparedPaper},
    html::to_markdown,
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

#[derive(Debug, Deserialize)]
struct SearchInput {
    q: String,
    #[serde(default = "default_n")]
    n: u32,
    #[serde(default)]
    offset: u32,
    from: Option<String>,
    to: Option<String>,
    #[serde(default)]
    cats: Vec<String>,
    #[serde(default = "default_sort")]
    sort: String,
}

const fn default_n() -> u32 {
    10
}

fn default_sort() -> String {
    "relevance".to_string()
}

#[derive(Debug, Deserialize)]
struct Operation {
    op: String,
    id: String,
    limit: Option<u32>,
    #[serde(default = "default_true")]
    prune_references: bool,
    #[serde(default = "default_chunk_chars")]
    chunk_chars: usize,
    #[serde(default = "default_chunk_overlap")]
    chunk_overlap: usize,
    pub segmentation_k: Option<f32>,
}

#[derive(Debug, Deserialize)]
struct RetrieveInput {
    paper_id: String,
    #[serde(default = "default_true")]
    prune_references: bool,
    #[serde(default = "default_chunk_chars")]
    chunk_chars: usize,
    #[serde(default = "default_chunk_overlap")]
    chunk_overlap: usize,
    pub segmentation_k: Option<f32>,
}

#[derive(Debug, Deserialize)]
struct HdrrInput {
    q: String,
    #[serde(default = "default_limit_docs")]
    limit_docs: usize,
    #[serde(default = "default_limit_chunks")]
    limit_chunks: usize,
    pub segmentation_k: Option<f32>,
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
}

impl ArxivServer {
    /// Constructs a new `ArxivServer` with the provided HTTP client.
    #[must_use]
    pub const fn new(client: FetchClient) -> Self {
        Self { client }
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
    #[expect(clippy::too_many_lines)]
    async fn run_operation(&self, op: Operation) -> Result<Value, rmcp::Error> {
        let id = normalize_paper_id(&op.id)
            .map_err(|e| rmcp::Error::invalid_params(e.to_string(), None))?;

        match op.op.as_str() {
            "abstract" => {
                let xml = self
                    .client
                    .fetch_arxiv_by_id(&id)
                    .await
                    .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;
                let response = parse_response(&xml)
                    .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;
                let paper = response.papers
                    .into_iter()
                    .next()
                    .ok_or_else(|| rmcp::Error::internal_error(format!("{id} not found"), None))?;
                serde_json::to_value(paper)
                    .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))
            }
            "download" => {
                if let Some(html) = self
                    .client
                    .fetch_html(&id)
                    .await
                    .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?
                {
                    let md = to_markdown(&html)
                        .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;
                    return Ok(Value::String(md));
                }
                let bytes = self
                    .client
                    .fetch_pdf(&id)
                    .await
                    .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;
                let text = extract_text(&bytes)
                    .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;
                Ok(Value::String(text))
            }
            "citations" => {
                let limit = op.limit.unwrap_or(10).clamp(1, 100);
                let json = self
                    .client
                    .fetch_citations(&id, limit)
                    .await
                    .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;
                let papers = parse_citations(&json)
                    .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;
                serde_json::to_value(papers)
                    .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))
            }
            "recs" => {
                let limit = op.limit.unwrap_or(10).clamp(1, 50);
                let json = self
                    .client
                    .fetch_recommendations(&id, limit)
                    .await
                    .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;
                let papers = parse_recommendations(&json)
                    .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;
                serde_json::to_value(papers)
                    .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))
            }
            "retrieve" => {
                let (source, text) = if let Some(html) = self
                    .client
                    .fetch_html(&id)
                    .await
                    .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?
                {
                    let md = to_markdown(&html)
                        .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;
                    ("html", md)
                } else {
                    let bytes = self
                        .client
                        .fetch_pdf(&id)
                        .await
                        .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;
                    let text = extract_text(&bytes)
                        .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;
                    ("pdf", text)
                };

                let paper = Paper {
                    id: id.clone(),
                    title: id.clone(),
                    authors: Vec::new(),
                    abstract_text: String::new(),
                    categories: Vec::new(),
                    published: String::new(),
                    url: format!("https://arxiv.org/abs/{id}"),
                    doi: None,
                    journal_ref: None,
                };

                let mut prepared = prepare_paper(
                    paper,
                    source,
                    text,
                    PreparationOptions {
                        prune_references: op.prune_references,
                        chunk_chars: op.chunk_chars,
                        chunk_overlap: op.chunk_overlap,
                        segmentation_k: op.segmentation_k,
                    },
                );
                format_hierarchical_chunks(&mut prepared);
 
                serde_json::to_value(prepared)
                    .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))
            }
            unknown => Err(rmcp::Error::invalid_params(
                format!("unknown op \"{unknown}\"; valid: abstract, download, citations, recs, retrieve"),
                None,
            )),
        }
    }

    async fn run_retrieve(&self, input: RetrieveInput) -> Result<Value, rmcp::Error> {
        let id = normalize_paper_id(&input.paper_id)
            .map_err(|e| rmcp::Error::invalid_params(e.to_string(), None))?;

        let source_and_text = if let Some(html) = self
            .client
            .fetch_html(&id)
            .await
            .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?
        {
            let md =
                to_markdown(&html).map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;
            ("html", md)
        } else {
            let bytes = self
                .client
                .fetch_pdf(&id)
                .await
                .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;
            let text = extract_text(&bytes)
                .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;
            ("pdf", text)
        };

        let paper = Paper {
            id: id.clone(),
            title: id.clone(),
            authors: Vec::new(),
            abstract_text: String::new(),
            categories: Vec::new(),
            published: String::new(),
            url: format!("https://arxiv.org/abs/{id}"),
            doi: None,
            journal_ref: None,
        };

        let mut prepared = prepare_paper(
            paper.clone(),
            source_and_text.0,
            source_and_text.1,
            PreparationOptions {
                prune_references: input.prune_references,
                chunk_chars: input.chunk_chars,
                chunk_overlap: input.chunk_overlap,
                segmentation_k: input.segmentation_k,
            },
        );

        #[cfg(feature = "embedded-db")]
        if let Some(db) = &self.client.db {
            let _ = db.store_paper(&paper.id, &paper.title, &paper.abstract_text);
            for chunk in &prepared.chunks {
                let id = format!("{}-{}", paper.id, chunk.index);
                let _ = db.store_chunk(&id, &paper.id, &chunk.text, None, chunk.cluster_id.as_deref());
            }
        }

        format_hierarchical_chunks(&mut prepared);
 
        serde_json::to_value(prepared).map_err(|e| rmcp::Error::internal_error(e.to_string(), None))
    }

    fn run_hdrr(&self, input: &HdrrInput) -> Result<Value, rmcp::Error> {
        #[cfg(not(feature = "embedded-db"))]
        return Err(rmcp::Error::internal_error("embedded-db feature not enabled", None));

        #[cfg(feature = "embedded-db")]
        {
            let db = self.client.db.as_ref()
                .ok_or_else(|| rmcp::Error::internal_error("Database not initialized", None))?;

            // Stage 1: Document-level routing P(D|q)
            let routed_docs = db.route_documents(&input.q, input.limit_docs)
                .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;

            if routed_docs.is_empty() {
                return Ok(serde_json::json!({
                    "query": input.q,
                    "routed_documents": [],
                    "chunks": [],
                    "message": "No documents routed in Stage 1."
                }));
            }

            // Stage 2: Scoped chunk retrieval P(c|q, D)
            // Note: hierarchical segmentation (segmentation_k) is handled during ingestion.
            let _ = input.segmentation_k; 

            let chunks = db.retrieve_chunks_scoped(&input.q, &routed_docs, input.limit_chunks)
                .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;

            Ok(serde_json::json!({
                "query": input.q,
                "routed_documents": routed_docs,
                "chunks": chunks.into_iter().map(|(id, text)| {
                    serde_json::json!({ "id": id, "text": text })
                }).collect::<Vec<_>>()
            }))
        }
    }
}

#[tool(tool_box)]
impl ArxivServer {
    #[tool(description = "Search arXiv papers. JSON input — schema at arxiv://openapi.")]
    async fn search(
        &self,
        #[tool(param)]
        #[schemars(description = "JSON object per arxiv://openapi /search schema.")]
        code: String,
    ) -> Result<CallToolResult, rmcp::Error> {
        let input: SearchInput = serde_json::from_str(&code)
            .map_err(|e| rmcp::Error::invalid_params(format!("invalid JSON: {e}"), None))?;

        let params = build_query_params(
            &input.q,
            input.n,
            input.offset,
            input.from.as_deref(),
            input.to.as_deref(),
            &input.cats,
            &input.sort,
        )
        .map_err(|e| rmcp::Error::invalid_params(e.to_string(), None))?;

        let xml = self
            .client
            .fetch_arxiv_query(&params)
            .await
            .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;

        let response =
            parse_response(&xml).map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;

        let out = serde_json::to_string_pretty(&response)
            .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(out)]))
    }

    #[tool(
        description = "Retrieve, prune, and chunk a paper into LLM-ready content. \
        JSON input — schema at arxiv://openapi."
    )]
    async fn retrieve_paper(
        &self,
        #[tool(param)]
        #[schemars(
            description = "JSON object with paper_id, prune_references, chunk_chars, and chunk_overlap."
        )]
        code: String,
    ) -> Result<CallToolResult, rmcp::Error> {
        let input: RetrieveInput = serde_json::from_str(&code)
            .map_err(|e| rmcp::Error::invalid_params(format!("invalid JSON: {e}"), None))?;
        let out = self.run_retrieve(input).await?;
        let out = serde_json::to_string_pretty(&out)
            .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(out)]))
    }

    #[tool(
        description = "Hybrid Document-Routed Retrieval (arXiv:2603.26815). \
        Two-stage: 1. Route docs, 2. Scoped chunk search."
    )]
    async fn hdrr(
        &self,
        #[tool(param)]
        #[schemars(description = "JSON object with q, limit_docs, limit_chunks.")]
        code: String,
    ) -> Result<CallToolResult, rmcp::Error> {
        let input: HdrrInput = serde_json::from_str(&code)
            .map_err(|e| rmcp::Error::invalid_params(format!("invalid JSON: {e}"), None))?;
        let out = self.run_hdrr(&input)?;
        let out = serde_json::to_string_pretty(&out)
            .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;
        
        // Satisfy clippy lints
        tokio::task::yield_now().await;
        drop(code);

        Ok(CallToolResult::success(vec![Content::text(out)]))
    }

    #[tool(
        description = "Fetch abstract, full text, citations, or recommendations. \
        JSON input — schema at arxiv://openapi. Pass an array for batching."
    )]
    async fn execute(
        &self,
        #[tool(param)]
        #[schemars(
            description = "JSON Operation or array of Operations per arxiv://openapi /execute schema."
        )]
        code: String,
    ) -> Result<CallToolResult, rmcp::Error> {
        let raw: Value = serde_json::from_str(&code)
            .map_err(|e| rmcp::Error::invalid_params(format!("invalid JSON: {e}"), None))?;

        let ops: Vec<Operation> = if raw.is_array() {
            serde_json::from_value(raw)
                .map_err(|e| rmcp::Error::invalid_params(format!("invalid operation: {e}"), None))?
        } else {
            vec![serde_json::from_value(raw).map_err(|e| {
                rmcp::Error::invalid_params(format!("invalid operation: {e}"), None)
            })?]
        };

        let mut results = Vec::with_capacity(ops.len());
        for op in ops {
            let id = op.id.clone();
            let op_name = op.op.clone();
            let result = self.run_operation(op).await;
            results.push(serde_json::json!({
                "id": id,
                "op": op_name,
                "result": match result {
                    Ok(v) => v,
                    Err(e) => serde_json::json!({"error": e.to_string()}),
                }
            }));
        }

        let out = if results.len() == 1 {
            serde_json::to_string_pretty(&results[0])
        } else {
            serde_json::to_string_pretty(&results)
        };
        let out = out.map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(out)]))
    }
}

#[tool(tool_box)]
impl ServerHandler for ArxivServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
            server_info: rmcp::model::Implementation {
                name: "arxiv-search-mcp".into(),
                version: env!("CARGO_PKG_VERSION").into(),
            },
            ..Default::default()
        }
    }

    fn list_resources(
        &self,
        _request: PaginatedRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourcesResult, McpError>> + Send + '_ {
        std::future::ready(Ok(ListResourcesResult {
            resources: vec![Annotated {
                raw: RawResource {
                    uri: OPENAPI_URI.to_string(),
                    name: "arXiv MCP OpenAPI Schema".to_string(),
                    description: Some(
                        "OpenAPI 3.0 schema for search, retrieve, and legacy execute inputs."
                            .to_string(),
                    ),
                    mime_type: Some("application/yaml".to_string()),
                    size: u32::try_from(OPENAPI_SPEC.len()).ok(),
                },
                annotations: None,
            }],
            next_cursor: None,
        }))
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ReadResourceResult, McpError>> + Send + '_ {
        let result = if request.uri == OPENAPI_URI {
            Ok(ReadResourceResult {
                contents: vec![ResourceContents::TextResourceContents {
                    uri: OPENAPI_URI.to_string(),
                    mime_type: Some("application/yaml".to_string()),
                    text: OPENAPI_SPEC.to_string(),
                }],
            })
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
                abstract_text: "".into(),
                categories: vec![],
                published: "".into(),
                url: "".into(),
                doi: None,
                journal_ref: None,
            },
            source: "test".into(),
            raw_markdown: "".into(),
            pruned_markdown: "".into(),
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

        assert_eq!(prepared.chunks[0].text, "Context: Header 1 -> Cluster A\n\nbody");
        assert_eq!(prepared.chunks[1].text, "Context: Header 2\n\nbody2");
        assert_eq!(prepared.chunks[2].text, "body3");
    }

    #[test]
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
