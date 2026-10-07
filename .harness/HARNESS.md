# Harness — Bring arxiv-search up to the Ptyktos public-repo governance baseline: fix broken dependabot.yml ecosystem value, add scorecard/supply-chain workflows, deny.toml, and a concrete security contact email

repo: /home/ctown/projects/arxiv-search · phase: Planning

status: **FRESH**

## Constraints (numbered, prioritized — rank 0 is a refusal)

- **FLOOR** (rank 0) [machine floor] Never edit/write anything whose path contains `.env`
- **FLOOR** (rank 0) [machine floor] Never edit/write anything whose path contains `.pem`
- **FLOOR** (rank 0) [machine floor] Never edit/write anything whose path contains `.key`
- **FLOOR** (rank 0) [machine floor] Never edit/write anything whose path contains `.p12`
- **FLOOR** (rank 0) [machine floor] Never edit/write anything whose path contains `.pfx`
- **FLOOR** (rank 0) [machine floor] Never edit/write anything whose path contains `credentials`
- **FLOOR** (rank 0) [machine floor] Never edit/write anything whose path contains `secrets/`
- **FLOOR** (rank 0) [machine floor] Never edit/write anything whose path contains `.ssh/`
- **FLOOR** (rank 0) [machine floor] Never edit/write anything whose path contains `.aws/`
- **FLOOR** (rank 0) [machine floor] Never edit/write anything whose path contains `.gnupg/`
- **FLOOR** (rank 0) [machine floor] Never edit/write anything whose path contains `.git/`
- **FLOOR** (rank 0) [machine floor] Never edit/write anything whose path contains `.claude/`
- **FLOOR** (rank 0) [machine floor] Never edit/write anything whose path contains `.agents/`
- **FLOOR** (rank 0) [machine floor] Never edit/write anything whose path contains `.config/opencode`
- **C3** (rank 0) [/home/ctown/.config/opencode/AGENTS.md] A class of solution that failed once is never re-proposed; three failed
- **C4** (rank 0) [/home/ctown/.config/opencode/AGENTS.md] Never run the heavy path when the cheap path answers the question.
- **C5** (rank 0) [/home/ctown/.config/opencode/AGENTS.md] 5. Secrets are never read, written, or committed
- **C6** (rank 0) [/home/ctown/.claude/CLAUDE.md] step that is red means **REPLAN** — never retry blind, never continue
- **C7** (rank 0) [/home/ctown/.claude/CLAUDE.md] Secrets are never read, written, or committed (`.env*`, keys, credentials,
- **C8** (rank 0) [/home/ctown/.claude/CLAUDE.md] Destructive commands are never run (sudo, shutdown, mkfs, dd, force-push,
- **C9** (rank 0) [/home/ctown/.claude/CLAUDE.md] The anti-loop rule: a class of solution that failed once is never
- **C18** (rank 0) [/home/ctown/projects/arxiv-search/AGENTS.md] Never claim absolute certainty or use definitive "100% solved" rhetoric. Frame code changes, bug fixes, and asynchronous queue management as high-confidence hypotheses supported by immediate empiri...
- **C10** (rank 1) [/home/ctown/projects/arxiv-search/AGENTS.md] 1. **Client Usage (RAG)** — Define the patterns and tools agents must use to query, retrieve, and synthesize scientific literature from the server efficiently and without hallucination.
- **C14** (rank 1) [/home/ctown/projects/arxiv-search/AGENTS.md] MANDATORY READING FOR USERS:** You MUST read the relevant modular documentation in `docs/agents/` before issuing requests to the MCP.
- **C15** (rank 1) [/home/ctown/projects/arxiv-search/AGENTS.md] Agents tasked with developing or maintaining the `arxiv-search` codebase MUST adhere to these tenets:
- **C16** (rank 1) [/home/ctown/projects/arxiv-search/AGENTS.md] Code without citation is hallucination.** Every logic block involving arXiv API limits, semantic scholar graph endpoints, or embedded database schemas must be justifiable.
- **C19** (rank 1) [/home/ctown/projects/arxiv-search/AGENTS.md] The Wave-Based Approach:** Agents are deployed in distinct, sequential Waves (**Alpha**, **Beta**, **Gamma**, **Delta**). A wave must fully resolve and merge before the next wave begins.
- **C20** (rank 1) [/home/ctown/projects/arxiv-search/AGENTS.md] Every agent deployment to modify the server MUST strictly adhere to this format to constrain the problem space:
- **C1** (rank 2) [/home/ctown/.config/opencode/AGENTS.md] If a guard ever passes something it should refuse, fix the guard — do not
- **C2** (rank 2) [/home/ctown/.config/opencode/AGENTS.md] the work. Do not substitute a generic plan, a different checkout, or a
- **C11** (rank 2) [/home/ctown/projects/arxiv-search/AGENTS.md] Whether you are using the MCP tools or developing the Rust codebase, **do not invent patterns or guess parameters.**
- **C12** (rank 2) [/home/ctown/projects/arxiv-search/AGENTS.md] For Users:** Rely on the server's multi-stage retrieval pipelines (`hdrr`) and structural pruning mechanisms. Do not hallucinate API parameters (e.g., `q` vs `query`).
- **C13** (rank 2) [/home/ctown/projects/arxiv-search/AGENTS.md] For Developers:** If the server exhibits bugs (e.g., rate-limit collapses, 429 backoff loops), do not slap generic ML or async band-aids on the code. Step back, trace the physical data flow (the "f...
- **C17** (rank 2) [/home/ctown/projects/arxiv-search/AGENTS.md] The codebase leverages asynchronous Tokio primitives and file-based cross-process locking. Ensure you understand the consequences of blocking operations vs async yields. Do not write code that intr...

## Verification steps

- **V1** [advisory] rustfmt clean — `cargo fmt --all -- --check`
- **V2** [advisory] clippy clean — `cargo clippy --all-targets -- -D warnings`
- **V3** [blocking] tests pass — `cargo test --quiet`
