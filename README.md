# arxiv-search-rs-mcp

Rust MCP server for arXiv paper search and retrieval. Exposes two tools — `search` and `execute` — each accepting a JSON string. Full schema is served as an MCP resource at `arxiv://openapi`.

## Tools

### `search`
Search arXiv papers. Pass a JSON object:

```json
{"q": "ti:attention AND au:vaswani", "n": 5, "sort": "relevance"}
```

| Field  | Type     | Default      | Description |
|--------|----------|--------------|-------------|
| `q`    | string   | **required** | arXiv query — field syntax (`ti:`, `au:`, `abs:`) and booleans (`AND`, `OR`, `ANDNOT`) |
| `n`    | integer  | `10`         | Max results (1–50) |
| `from` | date     | —            | Start date `YYYY-MM-DD` |
| `to`   | date     | —            | End date `YYYY-MM-DD` |
| `cats` | string[] | —            | Category filter e.g. `["cs.AI","cs.LG"]` |
| `sort` | string   | `relevance`  | `relevance` or `date` |

### `execute`
Fetch abstract, full text, citations, or recommendations. Pass a single operation or an array for batching:

```json
{"op": "abstract", "id": "1706.03762"}
```

```json
[
  {"op": "abstract",  "id": "1706.03762"},
  {"op": "citations", "id": "1706.03762", "limit": 20}
]
```

| Field   | Type    | Description |
|---------|---------|-------------|
| `op`    | string  | `abstract` · `download` · `citations` · `recs` |
| `id`    | string  | arXiv ID: `1706.03762`, `arxiv:1706.03762`, or `1706.03762v2` |
| `limit` | integer | `citations` max 100, `recs` max 50 (default 10) |

- **`abstract`** — metadata + abstract text
- **`download`** — full paper as markdown (HTML first, PDF fallback)
- **`citations`** — papers citing this paper (Semantic Scholar)
- **`recs`** — similar papers (Semantic Scholar)

### Resource: `arxiv://openapi`

OpenAPI 3.0 schema for both tool inputs. Fetch it when you need the full spec.

## Usage

### Claude Desktop (stdio)

```json
{
  "mcpServers": {
    "arxiv": {
      "command": "/path/to/arxiv-search-mcp",
      "args": ["--stdio"],
      "env": {
        "SEMANTIC_SCHOLAR_API_KEY": "optional-key-for-higher-rate-limits"
      }
    }
  }
}
```

### HTTP / SSE server

```bash
arxiv-search-mcp --host 127.0.0.1 --port 3000
# SSE endpoint:  http://127.0.0.1:3000/sse
# Post endpoint: http://127.0.0.1:3000/message
```

## Building

```bash
cargo build --release
# binary: target/release/arxiv-search-mcp
```

## Environment variables

| Variable                    | Description |
|-----------------------------|-------------|
| `SEMANTIC_SCHOLAR_API_KEY`  | Raises Semantic Scholar rate limits |
| `HOST`                      | Bind host for SSE mode (default `127.0.0.1`) |
| `PORT`                      | Bind port for SSE mode (default `3000`) |
| `RUST_LOG`                  | Log filter (default `arxiv_search_mcp=info,rmcp=warn`) |

## Architecture

Two-crate workspace:

- **`crates/core`** — pure logic, no I/O: arXiv XML parser, HTML→markdown, PDF extraction, Semantic Scholar types
- **`crates/native`** — binary: HTTP client, MCP server, CLI

arXiv requests are rate-limited to one per 3 seconds per the API terms of service.
