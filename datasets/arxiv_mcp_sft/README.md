# arxiv-search MCP tool-use SFT dataset

A small, hand-curated supervised fine-tuning (SFT) dataset that teaches a model
to drive the **arxiv-search MCP server** natively - i.e. to reach for
`search` / `retrieve_paper` / `execute` / `hdrr` and call them with the exact
wire format the server expects, without being told how each time.

- **Format:** OpenAI chat fine-tuning JSONL (one JSON object per line, with
  `messages`, `tools`, `parallel_tool_calls`).
- **Interface taught:** the four MCP tools exposed by `crates/native/src/tool.rs`.
- **Curation:** every conversation in `build_dataset.py` is written by hand.
  The script is a *serializer + validator*, not a synthetic generator - it
  exists because the JSON escaping is three levels deep and easy to get wrong.

## Files

| File | Purpose |
|---|---|
| `arxiv_mcp_sft.jsonl` | The dataset. 38 conversations, ready for SFT. |
| `build_dataset.py` | Hand-curated examples + serialization + validation. Re-run to regenerate the JSONL. |
| `README.md` | This file. |

Regenerate / validate:

```bash
python3 datasets/arxiv_mcp_sft/build_dataset.py
```

The script fails loudly if any example produces invalid JSON, an unknown tool,
a malformed `code` envelope, an orphaned tool result, or a conversation that
doesn't end on an assistant turn.

## The one thing the model must learn: the `code` envelope

Every arxiv-search tool takes a **single string argument named `code`** whose
value is itself a JSON document. So a tool call serializes like this (the
`arguments` string is JSON-encoded twice):

```json
{
  "role": "assistant",
  "tool_calls": [{
    "id": "call_1",
    "type": "function",
    "function": {
      "name": "search",
      "arguments": "{\"code\": \"{\\\"q\\\": \\\"ti:attention AND au:vaswani\\\", \\\"n\\\": 5}\"}"
    }
  }]
}
```

That double nesting is the most common thing agents get wrong against this
server, so most examples reinforce it. The dataset also reinforces:

- preferring `q` over the `query` alias, and `paper_id` over `id`;
- arXiv field syntax in `q`: `ti:` `au:` `abs:` `cat:` with `AND` / `OR` / `ANDNOT`;
- arXiv IDs with an `arxiv:` prefix or `vN` version suffix are normalized server-side.

## Tools covered

| Tool | `code` shape | Returns |
|---|---|---|
| `search` | `{q, n<=50, offset, from, to, cats[], sort}` | `{papers[], total_results, start_index}` |
| `retrieve_paper` | `{paper_id, prune_references, chunk_chars, chunk_overlap, segmentation_k}` | a `PreparedPaper` (`paper`, `source`, `raw_markdown`, `pruned_markdown`, `chunks[]`) |
| `execute` | one `Operation` or an **array** of them: `{op, id, limit, ...}` where `op` ∈ `abstract` \| `download` \| `citations` \| `recs` \| `retrieve` | `{id, op, result}` (or an array thereof) |
| `hdrr` | `{q, limit_docs, limit_chunks}` | `{query, routed_documents[], chunks[]}` |

Skills the conversations exercise: field-filtered discovery, category/date
filtering, pagination via `offset`, hierarchical reads with `segmentation_k`,
custom chunking, keeping references, batched `execute` arrays, citation/rec
chains, multi-step `search -> retrieve` chains, multi-paper synthesis with
`hdrr`, and calibration (answering directly when no tool is warranted, and
declining out-of-scope requests).

## Two fidelity notes worth knowing

1. **Tool outputs are illustrative.** The *call* shapes match the server
   exactly, and the examples use **real arXiv IDs and titles** (so the model
   never learns fake identifiers), but the returned abstracts/chunks/citations
   are synthetic stand-ins for what a live server would fetch. This is fine for
   teaching tool-use behavior; it is **not** a factual knowledge source.

2. **`hdrr` requires the `embedded-db` feature.** In `crates/native`, `hdrr`
   only works when the server is built with `--features embedded-db`;
   otherwise it returns `embedded-db feature not enabled`. The dataset includes
   both the happy path *and* a fallback example where the model degrades
   gracefully to `search` + `execute` when the index isn't available, plus an
   empty-routing example. Keep this in mind for how heavily you want the model
   to lean on `hdrr`.

   `retrieve_paper`'s returned `paper` field is intentionally sparse (title ==
   id, no authors/abstract) because the content path doesn't fetch the Atom
   metadata record - the examples reflect that, so the model learns to use
   `search` or `execute op=abstract` when it needs metadata.

## Using it for fine-tuning

The JSONL is in OpenAI's chat tool-calling format, which is consumed directly
by OpenAI fine-tuning and by most open-source SFT stacks (Axolotl,
LLaMA-Factory, Unsloth, Together, etc.).

- Assistant tool-call turns carry a short natural-language `content` note
  alongside `tool_calls` (narration the model learns to emit before acting). A
  few strict validators want `content` to be null/empty when `tool_calls` are
  present; if your trainer is one of them, blank the `content` field on
  tool-call turns before training.
- 38 curated rows is a **seed**, not a finished training set. For a durable
  "native" capability, mix this with a larger corpus and/or hand-author more
  rows in `build_dataset.py` following the same patterns.

## Source of truth

The tool names, parameters, aliases, and response shapes mirror
`crates/native/src/tool.rs` and `crates/core/src/{arxiv,content,paper,semantic_scholar}.rs`.
If those change, update `TOOLS`, the builders, and the examples here to match.
