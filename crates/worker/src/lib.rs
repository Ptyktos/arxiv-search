use serde::{Deserialize, Serialize};
use worker::*;

use arxiv_search_rs_mcp_core::{
    arxiv::{build_query_params, normalize_paper_id, parse_response},
    content::{prepare_paper, PreparationOptions},
    html::to_markdown,
    Paper,
};

const ARXIV_API_BASE: &str = "https://export.arxiv.org/api/query";
const ARXIV_HTML_BASE: &str = "https://arxiv.org/html";

#[derive(Deserialize)]
struct RpcRequest {
    id: Option<serde_json::Value>,
    method: String,
    params: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct RpcResponse {
    jsonrpc: &'static str,
    id: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<RpcError>,
}

#[derive(Serialize)]
struct RpcError {
    code: i32,
    message: String,
}

#[derive(Debug, Deserialize)]
struct SearchInput {
    q: String,
    #[serde(default = "default_n")]
    n: u32,
    from: Option<String>,
    to: Option<String>,
    #[serde(default)]
    cats: Vec<String>,
    #[serde(default = "default_sort")]
    sort: String,
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

impl RpcResponse {
    fn ok(id: Option<serde_json::Value>, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    fn err(id: Option<serde_json::Value>, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(RpcError {
                code,
                message: message.into(),
            }),
        }
    }
}

fn default_n() -> u32 {
    10
}

fn default_sort() -> String {
    "relevance".to_string()
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

fn cors_headers() -> Headers {
    let mut h = Headers::new();
    h.set("Access-Control-Allow-Origin", "*")
        .expect("valid header name");
    h.set("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
        .expect("valid header name");
    h.set("Access-Control-Allow-Headers", "Content-Type, Accept")
        .expect("valid header name");
    h
}

fn json_response(body: &impl Serialize) -> Result<Response> {
    let json = serde_json::to_string(body).map_err(|e| Error::RustError(e.to_string()))?;
    let mut resp = Response::from_body(ResponseBody::Body(json.into_bytes()))?;
    let headers = resp.headers_mut();
    headers.set("Content-Type", "application/json")?;
    headers.set("Access-Control-Allow-Origin", "*")?;
    Ok(resp)
}

fn sse_response(body: &impl Serialize) -> Result<Response> {
    let json = serde_json::to_string(body).map_err(|e| Error::RustError(e.to_string()))?;
    let data = format!("data: {json}\n\n");
    let mut resp = Response::from_body(ResponseBody::Body(data.into_bytes()))?;
    let headers = resp.headers_mut();
    headers.set("Content-Type", "text/event-stream")?;
    headers.set("Cache-Control", "no-cache")?;
    headers.set("Access-Control-Allow-Origin", "*")?;
    Ok(resp)
}

async fn fetch_text(url: &str) -> std::result::Result<String, String> {
    let request = Request::new(url, Method::Get).map_err(|e| e.to_string())?;
    let mut response = Fetch::Request(request)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if response.status_code() >= 400 {
        return Err(format!("HTTP {} fetching {url}", response.status_code()));
    }
    response.text().await.map_err(|e| e.to_string())
}

async fn fetch_arxiv_query(
    params: &arxiv_search_rs_mcp_core::arxiv::QueryParams,
) -> std::result::Result<String, String> {
    let url = format!(
        "{ARXIV_API_BASE}?search_query={}&max_results={}&sortBy={}&sortOrder={}",
        urlencoding::encode(&params.search_query),
        params.max_results,
        params.sort_by,
        params.sort_order,
    );
    fetch_text(&url).await
}

async fn fetch_html(paper_id: &str) -> std::result::Result<Option<String>, String> {
    let url = format!("{ARXIV_HTML_BASE}/{paper_id}");
    let request = Request::new(&url, Method::Get).map_err(|e| e.to_string())?;
    let mut response = Fetch::Request(request)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if response.status_code() == 404 {
        return Ok(None);
    }
    if response.status_code() >= 400 {
        return Err(format!("HTTP {} fetching {url}", response.status_code()));
    }
    response.text().await.map(Some).map_err(|e| e.to_string())
}

async fn handle_search_papers(args: &serde_json::Value) -> std::result::Result<String, String> {
    let input: SearchInput = serde_json::from_value(args.clone()).map_err(|e| e.to_string())?;
    let params = build_query_params(
        &input.q,
        input.n,
        input.from.as_deref(),
        input.to.as_deref(),
        &input.cats,
        &input.sort,
    )
    .map_err(|e| e.to_string())?;
    let xml = fetch_arxiv_query(&params).await?;
    let papers = parse_response(&xml).map_err(|e| e.to_string())?;
    serde_json::to_string_pretty(&papers).map_err(|e| e.to_string())
}

async fn handle_retrieve_paper(args: &serde_json::Value) -> std::result::Result<String, String> {
    let input: RetrieveInput = serde_json::from_value(args.clone()).map_err(|e| e.to_string())?;
    let paper_id = normalize_paper_id(&input.paper_id).map_err(|e| e.to_string())?;

    let (source, text) = if let Some(html) = fetch_html(&paper_id).await? {
        ("html", to_markdown(&html).map_err(|e| e.to_string())?)
    } else {
        ("abstract", paper_id.clone())
    };

    let paper = Paper {
        id: paper_id.clone(),
        title: paper_id.clone(),
        authors: Vec::new(),
        abstract_text: String::new(),
        categories: Vec::new(),
        published: String::new(),
        url: format!("https://arxiv.org/abs/{paper_id}"),
    };

    let prepared = prepare_paper(
        paper,
        source,
        text,
        PreparationOptions {
            prune_references: input.prune_references,
            chunk_chars: input.chunk_chars,
            chunk_overlap: input.chunk_overlap,
        },
    );
    serde_json::to_string_pretty(&prepared).map_err(|e| e.to_string())
}

async fn dispatch(rpc: RpcRequest) -> RpcResponse {
    let id = rpc.id;
    match rpc.method.as_str() {
        "initialize" => RpcResponse::ok(
            id,
            serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": {
                    "name": "arxiv-search-rs-mcp",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
        ),
        "tools/list" => RpcResponse::ok(
            id,
            serde_json::json!({
                "tools": [
                    {
                        "name": "search_papers",
                        "description": "Search arXiv papers",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "q": { "type": "string" },
                                "n": { "type": "integer", "default": 10 },
                                "from": { "type": "string", "description": "YYYY-MM-DD" },
                                "to": { "type": "string", "description": "YYYY-MM-DD" },
                                "cats": { "type": "array", "items": { "type": "string" } },
                                "sort": { "type": "string", "enum": ["relevance", "date"], "default": "relevance" }
                            },
                            "required": ["q"]
                        }
                    },
                    {
                        "name": "retrieve_paper",
                        "description": "Retrieve, prune, and chunk a paper into LLM-ready content",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "paper_id": { "type": "string" },
                                "prune_references": { "type": "boolean", "default": true },
                                "chunk_chars": { "type": "integer", "default": 4000 },
                                "chunk_overlap": { "type": "integer", "default": 200 }
                            },
                            "required": ["paper_id"]
                        }
                    }
                ]
            }),
        ),
        "tools/call" => {
            let params = rpc.params.as_ref().and_then(|p| p.as_object());
            let tool_name = params
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("");
            let empty = serde_json::Value::Null;
            let args = params.and_then(|p| p.get("arguments")).unwrap_or(&empty);

            let result = match tool_name {
                "search_papers" => handle_search_papers(args)
                    .await
                    .map(serde_json::Value::String),
                "retrieve_paper" => handle_retrieve_paper(args)
                    .await
                    .map(serde_json::Value::String),
                _ => return RpcResponse::err(id, -32601, format!("Unknown tool: {tool_name}")),
            };

            match result {
                Ok(text) => RpcResponse::ok(
                    id,
                    serde_json::json!({ "content": [{ "type": "text", "text": text }] }),
                ),
                Err(e) => RpcResponse::err(id, -1, e),
            }
        }
        other => RpcResponse::err(id, -32601, format!("Method not found: {other}")),
    }
}

#[event(fetch)]
pub async fn main(mut req: Request, _env: Env, _ctx: Context) -> Result<Response> {
    if req.method() == Method::Options {
        return Ok(Response::empty()?
            .with_headers(cors_headers())
            .with_status(204));
    }

    let path = req.path();
    match (req.method(), path.as_str()) {
        (_, "/") => json_response(&serde_json::json!({
            "name": "arxiv-search-rs-mcp",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Remote MCP server for arXiv search and paper retrieval",
            "endpoints": { "sse": "/sse", "mcp": "/mcp" },
            "tools": ["search_papers", "retrieve_paper"]
        })),
        (Method::Post, "/mcp") => {
            let rpc: RpcRequest = req
                .json()
                .await
                .map_err(|_| Error::RustError("Invalid JSON-RPC body".into()))?;
            json_response(&dispatch(rpc).await)
        }
        (Method::Post, "/sse") => {
            let rpc: RpcRequest = req
                .json()
                .await
                .map_err(|_| Error::RustError("Invalid JSON-RPC body".into()))?;
            sse_response(&dispatch(rpc).await)
        }
        (Method::Get, "/sse") => sse_response(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        })),
        _ => Response::error("Not Found", 404),
    }
}
