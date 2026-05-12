# 🧠 AGENTS.md: arXiv-search MCP — Architectural Blueprint & Operations Manual

This server implements **Advanced RAG** (arXiv:2507.09935, arXiv:2603.26815). Follow these patterns for maximum efficacy.

**This document follows the rigor standard established across SanguineHost platforms (e.g., Lyrium Engine, Sanguine Scribe).**

---

## 0. The Prime Directive: Precise Retrieval

The `arxiv-search` MCP is designed to extract maximum signal from scientific literature while minimizing token usage. **Do not hallucinate parameters or use brute-force search strategies.** Rely on the server's multi-stage retrieval pipelines (`hdrr`) and structural pruning mechanisms.

**Key Architectural Difference from Basic RAG:**
- **Hierarchical Metadata:** The server returns chunks with structural awareness (`cluster_id`, `parent_id`).
- **Two-Stage Routing:** `hdrr` performs document-level routing before chunk retrieval to stay within context limits.

---

## 1. Blueprint Modules & The Swarm Mandate

> [!IMPORTANT]
> **MANDATORY READING:** AI agents are FORBIDDEN from guessing tool parameters or usage patterns. You MUST read the relevant modular documentation in `docs/agents/` before issuing requests to the MCP.

- [**Tool Selection & Pitfalls**](docs/agents/tools.md) — The specific tools available (`search`, `retrieve_paper`, `hdrr`, `execute`), their appropriate use cases, and common parameter errors to avoid (e.g., `q` vs `query`).
- [**Advanced RAG & Efficiency**](docs/agents/rag_patterns.md) — Token pruning, multi-paper synthesis strategies, context-aware retrieval, and empirical best practices for scientific literature extraction.

---

## 2. The Agentic Chain Workflow

When interacting with the `arxiv-search` MCP, follow the established agentic chain:
1. **Discover:** Use `search` with precise field filters (`ti:`, `au:`) to locate relevant document IDs.
2. **Synthesize:** If answering a question across multiple papers, prioritize `hdrr` to prevent context overflow.
3. **Deep Dive:** Only use `retrieve_paper` with `segmentation_k` when deep, hierarchical analysis of a *single* paper is required.
