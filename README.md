# arxiv-search-rs-mcp

![arxiv-search artwork](assets/social-card.png)

Rust MCP server for arXiv search and paper retrieval. The repository exposes several MCP tools and follows a split architecture:

- `crates/native` for the local binary and stdio/SSE MCP transport
- `crates/worker` for a Cloudflare Worker deploy path
- `crates/core` for shared parsing and content-prep logic

The main retrieval flow is designed for LLM ingestion: retrieve paper content directly from arXiv, prune noise, then chunk before handing it downstream.

- **Advanced RAG Engine:** Implements hierarchical text segmentation (arXiv:2507.09935) and Hybrid Document-Routed Retrieval (HDRR, arXiv:2603.26815) for high-precision retrieval.
- **Embedded Database:** Optional `rusqlite` backend for document and chunk indexing, enabling stage-routed searches and cross-document synthesis.
- **Cache TTL:** Automatic pruning of stale cache files to manage disk usage.
- **Native OS Caching:** Caches fetched HTML/PDFs and metadata in the OS cache directory (`~/.cache/mcp/` on Linux, `~/Library/Caches/org.arxiv-search.mcp/` on macOS) and keeps prepared responses in memory, so repeat requests skip HTTP and re-processing entirely.


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
| `offset` | integer | `0` | OpenSearch pagination offset for fetching subsequent pages |
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
| `segmentation_k` | float | - | Optional. If set, uses hierarchical segmentation with the given sensitivity parameter. |
| `full_payload` | bool | `false` | Return the legacy payload (raw markdown, per-chunk text, embeddings) instead of the compact view |

The response is compact JSON, with the paper text appearing exactly once:

- `paper`: metadata (title, authors, abstract, …)
- `source`: `html` or `pdf`. The PDF is used when there's no HTML rendering, or when arXiv's HTML render is degraded.
- `pruned_markdown`: cleaned paper text. The reference list is dropped, appendices are kept, and math appears once as `$TeX$`.
- `chunks`: `[start_char, end_char)` offsets into `pruned_markdown`, plus any hierarchy `context`

### `hdrr`
Hybrid Document-Routed Retrieval. Two-stage retrieval that first routes to relevant documents and then performs scoped chunk searches.

```json
{"q":"transformer scaling laws","limit_docs":5,"limit_chunks":10}
```

### `execute`
Batch fetch abstracts, full text, citations, or recommendations.


## Local usage

```bash
cargo run -p arxiv-search-rs-mcp -- --stdio
```

Or run the SSE server:

```bash
cargo run -p arxiv-search-rs-mcp -- --host 127.0.0.1 --port 3000
```

## Agent install prompt

Paste the block below into any coding-agent context (opencode, Claude Code,
Codex, Cursor, etc.) to have the agent build, install, and wire the server as
an MCP tool. Nothing else is needed — the prompt is self-contained.

```text
Install Ptyktos/arxiv-search as a local MCP server and register it in this
session's agent config so I can search arXiv from future turns.

Steps:
1. Clone into a persistent location (skip if you already have a checkout):
   `gh repo clone Ptyktos/arxiv-search ~/projects/arxiv-search`
   Below, replace `<repo>` with that path.
2. Build the native binary:
   `cargo build --release --manifest-path <repo>/Cargo.toml --bin arxiv-search-mcp`
3. Install it onto PATH:
   `mkdir -p ~/.local/bin && cp <repo>/target/release/arxiv-search-mcp ~/.local/bin/`
4. Register as an MCP server in this project's agent config:
   - opencode: add to `~/.config/opencode/opencode.jsonc` (or project `.opencode/opencode.json`):
       "mcp": {
         "arxiv-search": {
           "type": "local",
           "command": ["/home/USER/.local/bin/arxiv-search-mcp", "--stdio"],
           "enabled": true
         }
       }
     Replace USER with `whoami`. Keep any pre-existing top-level keys; only add `mcp`.
   - Claude Code: append to `.mcp.json`:
       { "mcpServers": { "arxiv-search": { "command": "/home/USER/.local/bin/arxiv-search-mcp", "args": ["--stdio"] } } }
5. Smoke test before telling me to restart. Run the binary with `--host 127.0.0.1 --port 8765`
   in the background, then POST an `initialize` and a `tools/call` to `search`:
     curl -sSi -X POST http://127.0.0.1:8765/mcp \
       -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
       -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"smoke","version":"1"}}}'
   Capture the `mcp-session-id` header, send `notifications/initialized`, then call:
     {"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search","arguments":{"q":"ti:test","n":1}}}
   Expect a `papers` array in the response. Kill the background server.
6. Tell me to restart the agent so the MCP server loads natively. After restart the
   `search` / `retrieve_paper` / `hdrr` / `execute` tools will be first-class.

When invoked natively the tool exposes four MCP tools — `search`, `retrieve_paper`,
`hdrr`, `execute`. Query syntax is arXiv Lucene (`ti:`, `au:`, `abs:`, boolean
`AND`/`OR`/`NOT`, quoted phrases). Cache lives at `~/.cache/mcp/` (Linux).
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
