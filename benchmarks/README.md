# Benchmarks

Reproducible measurements of how quickly, and in how many tokens, `arxiv-search`
turns arXiv papers into text an agent can use. The same measurements run against
named incumbents on the same papers.

- Harness: [`arxiv/bench.py`](arxiv/bench.py), a single self-contained `uv` script
- Corpus and pinned baselines: [`arxiv/corpus.json`](arxiv/corpus.json), [`arxiv/incumbent.lock.txt`](arxiv/incumbent.lock.txt)
- Raw results plus rendered tables: [`arxiv/results/`](arxiv/results/)
- Offline conversion probe: [`crates/core/examples/convert_bench.rs`](../crates/core/examples/convert_bench.rs)

> [!IMPORTANT]
> Read the **claims policy** below before quoting any number from this directory
> in a README, post, or graphic.

---

## Claims policy: what these numbers can and cannot support

| Claim | Supported? | Why |
|---|---|---|
| "Time from asking for paper X to having its text in the agent's context is N ms" | **Yes**: the `retrieve` scenario, with cold and warm reported separately | This is exactly what is measured: from `tools/call` sent to the full response received, over real MCP stdio. |
| "Converting the same HTML/PDF is Nx faster than tool Y" | **Yes**: the `convert` scenario | Byte-identical local inputs, no network, each tool's own converter in its own runtime. |
| "The agent ingests N fewer tokens for the same paper" | **Only per payload, with the caveats listed** | See *Known findings*. The default `retrieve_paper` response currently contains the paper more than once. |
| "Lower peak memory than tool Y" | **Yes**, for the stated setup | Peak RSS of the server process, measured by `getrusage` on process exit. |
| "Improves the model's TTFT" or "improves token throughput" | **No** | Nothing here measures model inference. Faster retrieval shortens the wait *before* generation starts, which is not TTFT. Fewer input tokens can reduce prefill time and cost, but that needs a separate, model-specific measurement that this repo does not contain. |
| "Faster than web fetch" (generic) | **Only against the named `raw-fetch` baseline** | Hosted fetch tools such as an AI provider's built-in WebFetch post-process pages with a model, so they are neither reproducible nor comparable. Name the baseline you actually measured. |
| "Faster than vision / PDF input" | **No numeric claim** | A raw PDF is billed per page by the model provider. We report its bytes, never an invented token count. |

**Rules for any graphic or headline number:**

1. Name the baseline and its exact version (for example `arxiv-mcp-server 0.7.2`), never "existing solutions".
2. Show the setup next to the number: the machine, network note, date, `git_sha`, and the corpus. Every results file records all of these.
3. Report the median with a range, and show `n`. Keep cold and warm separate; don't mix them.
4. Chart ratios only for the scenario they came from. A `convert` speed-up is not an end-to-end speed-up.
5. If a number came from a run with `git_dirty: true`, rerun it from a clean commit first.

---

## What is measured

### Arms (baselines)

| Arm | What it represents | Pinned as |
|---|---|---|
| `arxiv-search` | This repo's `retrieve_paper` / `search` tools with their documented defaults | the `git_sha` in each results file |
| `arxiv-mcp-server` | The most widely used arXiv MCP server ([blazickjp/arxiv-mcp-server](https://github.com/blazickjp/arxiv-mcp-server)). Like us it is HTML-first with PDF fallback, via `pymupdf4llm`. | `arxiv-mcp-server[pdf]==0.7.2`, every transitive dependency frozen in `incumbent.lock.txt` |
| `raw-fetch` | What a naive "fetch this URL" tool hands a model: the raw arXiv HTML page, with no extraction | plain HTTP GET |
| `jina-reader` (opt-in, `--jina`) | A hosted URL-to-markdown service (`r.jina.ai`) | **Not reproducible**: a remote service whose output changed between identical requests during development. Shown for context only; never chart it. |

Fairness settings that differ from each tool's defaults:

- **`arxiv-mcp-server`** caps a response at 12,000 chars unless `return_full_text: true` is passed. The harness passes it, so both arms return the full paper in one call.
- **`arxiv-search`** prunes the reference list by default and the incumbent does not. The `convert` table reports our output both with and without pruning, so the effect can be attributed.
- **`search`**: the incumbent defaults to ~280-char abstract snippets, while we return full abstracts. Response tokens for `search` therefore compare defaults, not identical content.

### Scenarios

**`retrieve`: paper ID to full text, end to end.** For each paper, arm, and rep:

1. Spawn a fresh server process with an empty, throwaway cache directory. `HOME` and `XDG_CACHE_HOME` point to a temp dir for our server; the incumbent gets `--storage-path`.
2. `initialize`, recorded as **startup ms**.
3. One `tools/call`, recorded as **cold ms**. This includes arXiv network time.
4. `--warm-reps` repeat calls in the same process, recorded as **warm ms** (served from the tool's own disk cache).
5. Close stdin. A wrapper records the server's **peak RSS** as it exits.
6. **Response tokens** is everything the tool returned into context. **Content tokens** is the paper text inside it: `pruned_markdown` for us, `content` for the incumbent.

**`search`**: the same Lucene queries against both servers in one warm process each. The results are dominated by the arXiv API.

**`convert`: offline, no network.** HTML and PDF fixtures are downloaded once by `setup`. Then:

- `arxiv-search`: `convert_bench` runs `html::to_markdown` / `pdf::extract_text`, then `prepare_paper` (prune plus chunk).
- `arxiv-mcp-server`: its own `_html_to_text` for HTML, and `pymupdf4llm.to_markdown` for PDF, exactly as its `download_paper` calls them.
- Timing is taken inside each process, so interpreter and process startup are excluded.

### Controls

- **Politeness and limiter neutrality.** A `--pause` (default 3.5 s) separates network calls. This follows arXiv's ≥3 s guidance and also keeps the incumbent's built-in 3 s rate limiter from adding sleep to its measured time.
- **Order effects.** Arm order alternates each rep so neither arm consistently benefits from a just-warmed arXiv edge cache.
- **Tokens** use `tiktoken` `o200k_base` as a documented **proxy**. It is not any specific model's tokenizer, so compare ratios and don't quote absolute counts for a particular model.
- **Abstract recall** is the fraction of the paper's abstract words (4+ letters) found in the output. It is a *sanity floor* that catches "fast because it returned nothing", not a quality score.
- **Pinned versions.** Paper IDs are pinned (`v7`, and so on). The results record the incumbent version, `rustc`, the git SHA, and a dirty flag.

---

## Reproduce

Prerequisites: a Rust toolchain, [`uv`](https://docs.astral.sh/uv/), and network access to `arxiv.org`.

```bash
# 1. Build release binaries, install the frozen incumbent, download fixtures (~2-5 min)
uv run benchmarks/arxiv/bench.py setup

# 2. Run everything (~10 min, mostly polite pauses between arXiv requests)
BENCH_NETWORK_NOTE="home fibre, EU" uv run benchmarks/arxiv/bench.py run --label my-laptop

# Offline-only, no network needed after setup (~1 min)
uv run benchmarks/arxiv/bench.py run --scenarios convert --label my-laptop

# Re-render a results table
uv run benchmarks/arxiv/bench.py report benchmarks/arxiv/results/<file>.json
```

Each run writes `results/<UTC stamp>-<label>.json` (every raw sample, the environment, and the config) and a rendered `.md`.
Commit both when publishing numbers. Useful flags: `--cold-reps`, `--warm-reps`, `--convert-iters`, `--pause`, `--papers <ids>` (subset, for smoke tests), and `--jina`.

To bump the incumbent, edit `corpus.json`, reinstall, then regenerate the lock with
`uv pip freeze --python benchmarks/.cache/incumbent-venv/bin/python` and commit the new results next to the old ones.

<!-- RESULTS -->
