# Agentic Reference: Tool Selection & Pitfalls

## 1. Tool Selection Strategy
- **`search`**: Use for high-level discovery. Prefer specific field filters (`ti:`, `au:`, `cat:`) to reduce noise.
- **`retrieve_paper`**: Use for deep reading. **Always** set `segmentation_k` (recommended: `1.2`) for complex papers to get hierarchical context.
- **`hdrr`**: **The primary RAG tool.** Use this for question-answering over a set of papers. It performs two-stage document-level routing and scoped chunk retrieval.
- **`execute`**: Use for metadata-only operations (citations, recommendations).

## 2. Common Pitfalls
- **`q` vs `query`**: The `search` and `hdrr` tools expect a `q` field. Many agents mistakenly use `query`. While the server now supports `query` as an alias, always prefer `q` for consistency with the OpenAPI spec.
- **`paper_id` vs `id`**: The `retrieve_paper` tool expects `paper_id`. The server supports `id` as an alias, but `paper_id` is the primary key.
- **Op selection**: In `execute`, ensure `op` is one of the allowed strings (e.g., `citations`, `recs`).
- **Malformed JSON**: All tools take a single `code` string which must contain a valid JSON object. Do not pass parameters directly to the tool call; wrap them in the `code` string.
