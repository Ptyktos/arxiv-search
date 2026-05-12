# Wave Alpha: API Resiliency & OpenSearch Pagination

**Status:** Ready to Assign
**Wave Objective:** Harden the core `arxiv-search` pipeline against rate-limit violations and implement robust pagination for the MCP tools.

**CRITICAL CONSTRAINTS:**
1. **DO NOT violate arXiv Rate Limits.** Ensure a strict 3-second delay between sequential API requests.
2. **Cross-Platform Compatibility:** The core logic must remain compatible with both `crates/native` (tokio/reqwest) and `crates/worker` (Cloudflare WASM).

## Agent Alpha-1 (The Paginator)
**Task:** Implement OpenSearch Pagination and richer DTOs.
**Prompt:**
> You are Sanguine Agent Alpha-1. You are operating as part of the "ArXiv MCP Server Metadata Pipeline Hardening" Epic.
> 
> **MANDATORY FIRST STEPS:**
> 1. Read `docs/arxiv_epic_plan.md` to understand your specific domain.
> 2. Create your isolated worktree: `git worktree add ../arxiv-search-alpha-1 -b feature/alpha-1-pagination`
> 3. Change directory into your worktree before beginning any work.
>
> **YOUR OBJECTIVE:**
> In `crates/core/src/arxiv.rs`, update the `parse_response` function to extract the `<opensearch:totalResults>` and `<opensearch:startIndex>` elements from the arXiv Atom feed.
> Modify the `search` tool parameters (and the `QueryParams` struct) to accept an optional `offset` or `start_index` to allow fetching beyond the first 50 results.
> Additionally, update `crates/core/src/paper.rs` and the XML parser to capture the `doi` and `journal_ref` if present in the feed.
> 
> **VALIDATION:**
> Write tests in `crates/core/src/arxiv.rs` demonstrating that your parser correctly extracts OpenSearch metadata and that the `Paper` struct safely parses feeds with missing DOIs. Run `cargo test -p core` within your worktree. Commit your changes (excluding `target/`) before ending your task.

## Agent Alpha-2 (The Sentinel)
**Task:** Implement Cross-Platform Rate Limiting.
**Prompt:**
> You are Sanguine Agent Alpha-2. You are operating as part of the "ArXiv MCP Server Metadata Pipeline Hardening" Epic.
> 
> **MANDATORY FIRST STEPS:**
> 1. Read `docs/arxiv_epic_plan.md` to understand your specific domain.
> 2. Create your isolated worktree: `git worktree add ../arxiv-search-alpha-2 -b feature/alpha-2-ratelimit`
> 3. Change directory into your worktree before beginning any work.
>
> **YOUR OBJECTIVE:**
> Implement a rigorous 3-second rate limiter to respect arXiv's Terms of Service.
> Because `arxiv-search` runs both natively (`crates/native`) and as a Cloudflare Worker (`crates/worker`), you cannot simply use `std::thread::sleep` or native Mutexes universally in `crates/core`.
> Architect a trait-based `RateLimiter` interface in `crates/core`. Provide a native tokio-based implementation in `crates/native` and an appropriate stub/fetch-wrapper in `crates/worker`.
> Integrate this into the search and retrieve flows so that no two arXiv HTTP requests occur within 3 seconds of each other.
> 
> **VALIDATION:**
> Ensure your changes compile for both targets. Run `cargo check -p native` and `cargo check -p worker --target wasm32-unknown-unknown`. Commit your changes (excluding `target/`) before ending your task.
