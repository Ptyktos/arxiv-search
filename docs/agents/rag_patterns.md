# Agentic Reference: Advanced RAG & Efficiency

## 1. Token Efficiency
- The server automatically prunes references and boilerplate.
- Use `hdrr` with `limit_docs` (Stage 1) and `limit_chunks` (Stage 2) to prevent context window overflow.
- Chunks carry **Hierarchical Metadata**. Pay attention to the `Context:` prefix; it represents the structural parent (e.g., Section Header) of the chunk.

## 2. Advanced RAG Patterns
- **Multi-Paper Synthesis**: If answering across multiple papers, use `hdrr`. The stage-routed search ensures that the retrieval space is confined only to relevant documents identified in Stage 1.
- **Context-Aware Retrieval**: When using `retrieve_paper`, the `cluster_id` and `parent_id` allow you to reconstruct the paper's logical structure if needed.

## 3. Best Practices (Based on arXiv:2508.14704)
- **Precise Parameterization**: Don't guess. Use the OpenAPI schema.
- **Error Handling**: If a search returns no results, try broader terms before attempting `retrieve_paper`.
- **Reasoning First**: Before calling a tool, think about whether you need a broad search or a scoped retrieval.
