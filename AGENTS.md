# 🧠 AGENTS.md: arXiv-search MCP

This server implements **Advanced RAG** (arXiv:2507.09935, arXiv:2603.26815). Follow these patterns for maximum efficacy.

## 1. Tool Selection Strategy
- **`search`**: Use for high-level discovery. Prefer specific field filters (`ti:`, `au:`, `cat:`) to reduce noise.
- **`retrieve_paper`**: Use for deep reading. **Always** set `segmentation_k` (recommended: `1.2`) for complex papers to get hierarchical context.
- **`hdrr`**: **The primary RAG tool.** Use this for question-answering over a set of papers. It performs two-stage document-level routing and scoped chunk retrieval.
- **`execute`**: Use for metadata-only operations (citations, recommendations).

## 2. Token Efficiency
- The server automatically prunes references and boilerplate.
- Use `hdrr` with `limit_docs` (Stage 1) and `limit_chunks` (Stage 2) to prevent context window overflow.
- Chunks carry **Hierarchical Metadata**. Pay attention to the `Context:` prefix; it represents the structural parent (e.g., Section Header) of the chunk.

## 3. Advanced RAG Patterns
- **Multi-Paper Synthesis**: If answering across multiple papers, use `hdrr`. The stage-routed search ensures that the retrieval space is confined only to relevant documents identified in Stage 1.
- **Context-Aware Retrieval**: When using `retrieve_paper`, the `cluster_id` and `parent_id` allow you to reconstruct the paper's logical structure if needed.

## 4. Best Practices (Based on arXiv:2508.14704)
- **Precise Parameterization**: Don't guess. Use the OpenAPI schema.
- **Error Handling**: If a search returns no results, try broader terms before attempting `retrieve_paper`.
- **Reasoning First**: Before calling a tool, think about whether you need a broad search or a scoped retrieval.

## 5. Common Pitfalls
- **`q` vs `query`**: The `search` and `hdrr` tools expect a `q` field. Many agents mistakenly use `query`. While the server now supports `query` as an alias, always prefer `q` for consistency with the OpenAPI spec.
- **`paper_id` vs `id`**: The `retrieve_paper` tool expects `paper_id`. The server supports `id` as an alias, but `paper_id` is the primary key.
- **Op selection**: In `execute`, ensure `op` is one of the allowed strings (e.g., `citations`, `recs`).
- **Malformed JSON**: All tools take a single `code` string which must contain a valid JSON object. Do not pass parameters directly to the tool call; wrap them in the `code` string.
