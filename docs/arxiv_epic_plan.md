# Epic: ArXiv MCP Server Metadata Pipeline Hardening

**Objective:** Establish a robust, scalable architecture for ingesting and processing arXiv metadata within the `arxiv-search` MCP server. The pipeline must gracefully handle arXiv API quirks, enforce strict rate-limiting, and prepare metadata for high-fidelity LLM context injection.

## Sprint 1: API Resiliency & OpenSearch Pagination

**Status:** Completed
**Sprint Goal:** Enhance the core `arxiv.rs` parsing and fetching logic to support robust OpenSearch pagination and strict rate-limiting (3-second delay) to comply with arXiv Terms of Service.

### Task 1.1: Rate Limiter Middleware
- **Status:** Completed. In-memory and trait-based rate-limiter applied.
- **Subtask 1.1.1:** Implement a token-bucket or mutex-based rate limiter in `crates/core` that ensures a 3-second delay between outgoing arXiv API HTTP requests.
- **Subtask 1.1.2:** Ensure the rate limiter works across both `crates/native` (reqwest/stdio) and `crates/worker` (Cloudflare fetch).

### Task 1.2: OpenSearch Pagination
- **Status:** Completed.
- **Subtask 1.2.1:** Update `parse_response` in `crates/core/src/arxiv.rs` to extract `<opensearch:totalResults>` and `<opensearch:startIndex>`.
- **Subtask 1.2.2:** Add pagination support to the `search` tool, allowing the MCP client to fetch results beyond the top 50 (e.g., using an `offset` parameter).

## Sprint 2: Extended Metadata & Content Caching

**Status:** Active
**Sprint Goal:** Extend the `Paper` DTO to capture richer semantic context and implement a local caching layer to prevent redundant API calls.

### Task 2.1: Rich DTOs
- **Status:** Partially Completed (DOIs and Journal Refs added in Sprint 1).
- **Subtask 2.1.1:** Extend `crates/core/src/paper.rs` to upgrade the `authors` list into a dedicated `Author` struct that tracks both the author's name and their institutional `affiliations`.
- **Subtask 2.1.2:** Update the `quick_xml` parser loop to track `<author>` blocks and capture `<arxiv:affiliation>` gracefully without failing on missing data.

### Task 2.2: Local Persistence (Native Only)
- **Subtask 2.2.1:** Implement a lightweight caching mechanism (e.g., SQLite or local filesystem JSON/RON files) in `crates/native` for retrieved papers.
- **Subtask 2.2.2:** Ensure the `retrieve_paper` tool checks the local cache before hitting the arXiv or Semantic Scholar APIs.
