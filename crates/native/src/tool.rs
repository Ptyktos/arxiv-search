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
    content::{prepare_paper, PreparationOptions},
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
components:
  schemas:
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

fn default_n() -> u32 {
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
}

fn default_true() -> bool {
    true
}

fn default_chunk_chars() -> usize {
    4_000
}

fn default_chunk_overlap() -> usize {
    200
}

#[derive(Debug, Clone)]
pub struct ArxivServer {
    client: FetchClient,
}

impl ArxivServer {
    #[must_use]
    pub fn new(client: FetchClient) -> Self {
        Self { client }
    }

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

                let prepared = prepare_paper(
                    paper,
                    source,
                    text,
                    PreparationOptions {
                        prune_references: op.prune_references,
                        chunk_chars: op.chunk_chars,
                        chunk_overlap: op.chunk_overlap,
                    },
                );

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

        let prepared = prepare_paper(
            paper,
            source_and_text.0,
            source_and_text.1,
            PreparationOptions {
                prune_references: input.prune_references,
                chunk_chars: input.chunk_chars,
                chunk_overlap: input.chunk_overlap,
            },
        );

        serde_json::to_value(prepared).map_err(|e| rmcp::Error::internal_error(e.to_string(), None))
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
