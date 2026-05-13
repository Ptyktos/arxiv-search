# 🧠 AGENTS.md: arXiv-search MCP — Architectural Blueprint & Operations Manual

This document is the definitive AI-native architectural blueprint and operations manual for the **arXiv-search** server. It serves a **dual mandate**:

1. **Client Usage (RAG)** — Define the patterns and tools agents must use to query, retrieve, and synthesize scientific literature from the server efficiently and without hallucination.
2. **Core Development** — Define the strict engineering protocols, Swarm Methodology, and empirical rigor required for AI agents modifying the `arxiv-search` codebase itself.

**This document follows the strict rigor standard established by `~/Workspace/sanguine/AGENTS.md` and `~/Workspace/lyrium/AGENTS.md`.**

---

## 1. The Prime Directive: Stop Guessing (Audit Over Patching)

Whether you are using the MCP tools or developing the Rust codebase, **do not invent patterns or guess parameters.** 
- **For Users:** Rely on the server's multi-stage retrieval pipelines (`hdrr`) and structural pruning mechanisms. Do not hallucinate API parameters (e.g., `q` vs `query`).
- **For Developers:** If the server exhibits bugs (e.g., rate-limit collapses, 429 backoff loops), do not slap generic ML or async band-aids on the code. Step back, trace the physical data flow (the "funnel"), and audit the empirical reality of the locks and queues before patching.

---

## 2. Blueprint Modules & Capabilities (The RAG Mandate)

> [!IMPORTANT]
> **MANDATORY READING FOR USERS:** You MUST read the relevant modular documentation in `docs/agents/` before issuing requests to the MCP.

When interacting with the `arxiv-search` MCP, follow the established agentic chain:
1. **Discover:** Use `search` with precise field filters (`ti:`, `au:`) to locate relevant document IDs.
2. **Synthesize:** If answering a question across multiple papers, prioritize `hdrr` (Hybrid Document-Routed Retrieval) to stay within context limits.
3. **Deep Dive:** Only use `retrieve_paper` with `segmentation_k` when deep, hierarchical analysis of a *single* paper is required.

---

## 3. Engineering Constitution (The Sanguine Standard)

Agents tasked with developing or maintaining the `arxiv-search` codebase MUST adhere to these tenets:

### I. Absolute Traceability
- **Code without citation is hallucination.** Every logic block involving arXiv API limits, semantic scholar graph endpoints, or embedded database schemas must be justifiable.

### II. The Cartesian Constraint
- The codebase leverages asynchronous Tokio primitives and file-based cross-process locking. Ensure you understand the consequences of blocking operations vs async yields. Do not write code that introduces deadlocks or silent queue collapses under high concurrent load.

### III. Epistemic Humility (The Probabilistic Agent)
- Never claim absolute certainty or use definitive "100% solved" rhetoric. Frame code changes, bug fixes, and asynchronous queue management as high-confidence hypotheses supported by immediate empirical evidence.

---

## 4. The Swarm Methodology (Epics, Waves, and Agents)

`arxiv-search` development revolves around structured parallelism to prevent merge conflicts and ensure strict architectural compliance:

- **The Wave-Based Approach:** Agents are deployed in distinct, sequential Waves (**Alpha**, **Beta**, **Gamma**, **Delta**). A wave must fully resolve and merge before the next wave begins.
- **Parallel Agent Syntax:** Agents within the same wave (e.g., `Alpha-1`, `Alpha-2`) operate **in parallel** on isolated Git Worktrees, explicitly constrained to touch disjoint files to prevent merge conflicts.
- **Testing Hygiene:** Run tests specifically for your crate (`cargo test -p arxiv-search-native`) rather than blindly running massive workspace tests.

### The Standard Agent Prompt Template
Every agent deployment to modify the server MUST strictly adhere to this format to constrain the problem space:

```markdown
#### Agent [Wave]-[Num]: [Agent Title]
*   **Mandatory Pre-flight**: [Exact documents, RFCs, or code the agent MUST read before acting.]
*   **Target Files**: [Exact paths to the files the agent is allowed to edit. Explicit constraint.]
*   **Task**: [Actionable summary of the engineering work. Audit the ground truth first.]
*   **Mandatory Output**: [Specific test suite that must pass, or specific markdown artifact document that must be generated.]
*   **Git Isolation**: `git worktree add ../[repo]-[wave]-[num] -b feat/[wave]-[num]-[topic]`
```
