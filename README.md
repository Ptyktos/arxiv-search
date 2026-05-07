# arxiv-search-rs-mcp

Rust MCP server for arXiv search and paper retrieval. The repo now follows the same split as your YouTube MCP:

- `crates/native` for the local binary and stdio/SSE MCP transport
- `crates/worker` for a Cloudflare Worker deploy path
- `crates/core` for shared parsing and content-prep logic

The main retrieval flow is designed for LLM ingestion: retrieve paper content directly from arXiv, prune noise, then chunk before handing it downstream.

## Tools

### `search`
Search arXiv papers from the API. Pass a JSON object:

```json
{"q":"ti:attention AND au:vaswani","n":5,"sort":"relevance"}
```

| Field | Type | Default | Description |
|---|---|---:|---|
| `q` | string | required | arXiv query syntax with field filters and boolean operators |
| `n` | integer | `10` | Max results, capped at 50 |
| `from` | date | - | Start date `YYYY-MM-DD` |
| `to` | date | - | End date `YYYY-MM-DD` |
| `cats` | string[] | - | Category filter, for example `["cs.AI","cs.LG"]` |
| `sort` | string | `relevance` | `relevance` or `date` |

### `retrieve_paper`
Retrieve a paper directly from arXiv content URLs, prune it, and chunk it for model consumption. Pass:

```json
{"paper_id":"1706.03762","prune_references":true,"chunk_chars":4000,"chunk_overlap":200}
```

| Field | Type | Default | Description |
|---|---|---:|---|
| `paper_id` | string | required | arXiv ID, with or without `arxiv:` prefix and version suffix |
| `prune_references` | bool | `true` | Drops trailing references/bibliography noise |
| `chunk_chars` | integer | `4000` | Target chunk size |
| `chunk_overlap` | integer | `200` | Overlap between chunks |

The response is structured JSON with:

- paper id and content URL
- source used for retrieval
- raw markdown
- pruned markdown
- chunk list

### `execute`
Legacy batch path kept for compatibility. It still supports:

- `abstract`
- `download`
- `citations`
- `recs`
- `retrieve`

## Local usage

```bash
cargo run -p arxiv-search-rs-mcp -- --stdio
```

Or run the SSE server:

```bash
cargo run -p arxiv-search-rs-mcp -- --host 127.0.0.1 --port 3000
```

## Cloudflare Worker

The worker entrypoint lives in `crates/worker`.

```bash
cd crates/worker
cargo check --target wasm32-unknown-unknown
```

`wrangler.toml` is already set up for a `worker-build` deploy flow.

## Architecture

- `crates/core`: arXiv XML parsing, HTML-to-markdown conversion, PDF extraction, and chunk/prune helpers
- `crates/native`: local MCP server with `rmcp`
- `crates/worker`: Cloudflare Worker MCP endpoint with the same tool semantics

## Environment

- `SEMANTIC_SCHOLAR_API_KEY`: raises Semantic Scholar rate limits for `citations` and `recs`
- `HOST`: SSE bind host
- `PORT`: SSE bind port
- `RUST_LOG`: logging filter
