# arxiv-search-rs-mcp Design Spec

**Date:** 2026-04-15
**Status:** Approved

## Overview

A Rust rewrite of [blazickjp/arxiv-mcp-server](https://github.com/blazickjp/arxiv-mcp-server),
following the same two-crate pattern as `youtube-transcript-mcp-rust`. Exposes five MCP tools
for searching arXiv, fetching paper content as markdown, and querying Semantic Scholar for
citations and recommendations. Fully stateless — no local paper storage.

---

## Crate Structure

```
arxiv-search-rs-mcp/
├── Cargo.toml                       # workspace, shared lints/profile
├── crates/
│   ├── core/                        # pure logic, no async I/O
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── error.rs             # ArxivError (thiserror)
│   │       ├── paper.rs             # Paper struct (shared type)
│   │       ├── arxiv.rs             # query building + Atom XML parsing
│   │       ├── html.rs              # HTML → markdown (htmd crate)
│   │       ├── pdf.rs               # PDF text extraction (pdf-extract)
│   │       └── semantic_scholar.rs  # SS response types + deserialization
│   └── native/                      # binary
│       └── src/
│           ├── main.rs              # CLI (clap) + stdio/SSE transport setup
│           ├── tool.rs              # rmcp #[tool] impls + ServerHandler
│           └── fetch.rs             # all reqwest HTTP calls
```

`core` is I/O-free and fully unit-testable without a network or async runtime. `native` owns
all HTTP I/O and wires it to `rmcp` stdio and SSE transports.

---

## Tools

### `search_papers`

Queries `https://export.arxiv.org/api/query`, parses Atom XML, returns an array of paper objects.

**Parameters:**

| param | type | required | notes |
|---|---|---|---|
| `query` | string | yes | supports arXiv field syntax: `ti:`, `au:`, `abs:`, boolean AND/OR/ANDNOT |
| `max_results` | u32 | no | default 10, capped at 50 |
| `date_from` | string | no | YYYY-MM-DD; injected as `submittedDate:[YYYYMMDD0000 TO ...]` |
| `date_to` | string | no | YYYY-MM-DD |
| `categories` | `Vec<String>` | no | e.g. `["cs.AI", "cs.LG"]`, ANDed into query |
| `sort_by` | string | no | `"relevance"` (default) \| `"date"` |

**Returns:** `Vec<Paper>` serialised as JSON.

---

### `get_abstract`

Same API endpoint with `id_list={paper_id}`. Returns a single paper object.

Normalises paper IDs: accepts `2103.12345`, `arxiv:2103.12345`, or versioned `2103.12345v2`
(strips version suffix for the API call).

**Parameters:**

| param | type | required |
|---|---|---|
| `paper_id` | string | yes |

**Returns:** Single `Paper` as JSON.

---

### `download_paper`

Fetches paper content and returns it as a markdown string. No local storage.

**Strategy:**
1. Try `https://arxiv.org/html/{paper_id}` — parse main content, convert to markdown via `htmd`
2. On 404 or parse failure — fetch `https://arxiv.org/pdf/{paper_id}`, extract text via
   `pdf-extract`, wrap as plain markdown

Note: HTML path (available for most post-2020 papers) produces significantly cleaner output
than PDF extraction. PDF extraction is plain text formatted as markdown paragraphs — acceptable
for LLM consumption but without structural headers/sections.

**Parameters:**

| param | type | required |
|---|---|---|
| `paper_id` | string | yes |

**Returns:** Markdown string of paper content.

---

### `get_citations`

`GET https://api.semanticscholar.org/graph/v1/paper/ArXiv:{paper_id}/citations`

Returns papers that cite this paper.

**Parameters:**

| param | type | required | notes |
|---|---|---|---|
| `paper_id` | string | yes | |
| `limit` | u32 | no | default 10, max 100 |

**Returns:** Array of citing papers (title, authors, year, arxiv id if available) as JSON.

---

### `get_recommendations`

`GET https://api.semanticscholar.org/recommendations/v1/papers/forpaper/ArXiv:{paper_id}`

Returns papers recommended by Semantic Scholar based on a seed paper.

**Parameters:**

| param | type | required | notes |
|---|---|---|---|
| `paper_id` | string | yes | |
| `limit` | u32 | no | default 10, max 50 |

**Returns:** Array of recommended papers as JSON.

Both Semantic Scholar tools send the `x-api-key` header when `SEMANTIC_SCHOLAR_API_KEY` is set
in the environment. Falls back to unauthenticated (100 req/5 min) if not set.

---

## Data Flow

```
search_papers / get_abstract:
  tool.rs → fetch.rs (rate-limit gate → reqwest GET arXiv API)
           → raw Atom XML
           → core::arxiv::parse_response() → Vec<Paper>
           → serialise to JSON → CallToolResult

download_paper:
  tool.rs → fetch.rs::fetch_html() → Option<String>
           → Some(html): core::html::to_markdown()
           → None:        fetch.rs::fetch_pdf() → bytes
                          → core::pdf::extract_text()
           → CallToolResult (markdown string)

get_citations / get_recommendations:
  tool.rs → fetch.rs (optional x-api-key header → reqwest GET)
           → core::semantic_scholar::parse_citations() / parse_recommendations()
           → Vec<Paper> → serialise → CallToolResult
```

---

## Rate Limiting

arXiv asks for ≥3 seconds between requests. `fetch.rs` holds an
`Arc<Mutex<Instant>>` tracking the last arXiv request time. Before every arXiv
call it checks elapsed time and sleeps the remainder if needed.

Semantic Scholar has no stated minimum interval. The optional API key raises the
rate ceiling; without it, the unauthenticated limit is ~100 requests/5 min.
HTTP 429 responses from Semantic Scholar surface as descriptive tool errors with
the `Retry-After` hint if present in the response headers.

---

## Error Handling

- `core`: `thiserror`-based `ArxivError` enum — variants: `ParseError`, `InvalidPaperId`,
  `NoContentAvailable`
- `native`: `anyhow` for internal plumbing
- Tool handlers map to `rmcp::Error::invalid_params` (bad paper ID) or
  `rmcp::Error::internal_error` (network/parse failures)
- Same pattern as `youtube-transcript-mcp-rust`

---

## Dependencies

### `crates/core`

| crate | purpose |
|---|---|
| `thiserror` | error types |
| `serde` + `serde_json` | serialisation |
| `quick-xml` | Atom XML parsing |
| `htmd` | HTML → markdown |
| `pdf-extract` | PDF text extraction |
| `chrono` | date parsing/formatting for query construction |

### `crates/native`

| crate | purpose |
|---|---|
| `arxiv-search-rs-mcp-core` | core logic |
| `rmcp` | MCP server (stdio + SSE transports) |
| `tokio` | async runtime |
| `reqwest` | HTTP client (rustls-tls) |
| `clap` | CLI (`--stdio`, `--host`, `--port`) |
| `tracing` + `tracing-subscriber` | logging to stderr |
| `anyhow` | error handling |
| `schemars` | JSON schema generation for tool params |
| `axum` + `tower-http` | SSE server |
| `serde` + `serde_json` | serialisation |

---

## Testing

### `crates/core` (unit, no network)

- `arxiv.rs`: parse fixture Atom XML → assert correct `Paper` fields, author list, category
  extraction, ID normalisation (`arxiv:2103.12345v2` → `2103.12345`)
- `html.rs`: feed arXiv HTML snippet → assert markdown output structure
- `pdf.rs`: smoke test with embedded minimal test PDF bytes
- `semantic_scholar.rs`: parse fixture JSON for citations and recommendations

### `crates/native` (integration, `#[ignore]`, require network)

- `search_papers` with a real query returns ≥1 result
- `get_abstract` for a known paper ID returns expected title
- `download_paper` for a known post-2020 paper hits HTML path; older paper falls back to PDF

---

## CLI Interface

Mirrors `youtube-transcript-mcp-rust`:

```
arxiv-search-mcp [--stdio] [--host <HOST>] [--port <PORT>]
```

- `--stdio`: stdio transport for Claude Desktop and local MCP clients
- Default: HTTP/SSE server at `127.0.0.1:3000`
- `SEMANTIC_SCHOLAR_API_KEY`: optional env var for higher Semantic Scholar rate limits
