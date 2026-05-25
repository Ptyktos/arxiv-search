#!/usr/bin/env python3
"""Serialize the hand-curated arxiv-search MCP tool-use examples to JSONL.

This is NOT a synthetic generator: every conversation below is hand-authored.
The script exists only to (a) guarantee the deeply-nested JSON escaping is
correct and (b) validate the result. The escaping is genuinely tricky here
because every arxiv-search MCP tool takes a *single string argument named
`code`* whose value is itself a JSON document, so a tool call serializes to:

    function.arguments = "{\"code\": \"{\\\"q\\\": \\\"...\\\"}\"}"

Output format: OpenAI chat fine-tuning JSONL. One JSON object per line with
`messages`, `tools`, and `parallel_tool_calls`. Assistant tool-call turns use
`tool_calls`; tool results use a `tool` role message keyed by `tool_call_id`.

NOTE ON FIDELITY: the tool *outputs* in these examples are illustrative
(synthetic), but they are schema-faithful to what crates/native/src/tool.rs
actually returns, and they use real arXiv IDs/titles so the model never learns
fake identifiers. The tool *call* shapes match the server exactly.
"""

from __future__ import annotations

import json
from pathlib import Path

OUT_PATH = Path(__file__).with_name("arxiv_mcp_sft.jsonl")

# --------------------------------------------------------------------------- #
# System prompt (kept identical across every example for training stability).
# --------------------------------------------------------------------------- #

SYSTEM = (
    "You are a research assistant with access to the arxiv-search MCP server, "
    "which retrieves and prepares scientific papers from arXiv. Ground every "
    "claim in real papers returned by the tools; never invent arXiv IDs, "
    "titles, authors, or findings.\n\n"
    "Every tool takes a SINGLE string argument named `code` that contains a "
    "JSON object (or, for `execute`, a JSON object or array). Always put the "
    "parameters inside `code` as JSON text.\n\n"
    "Tools:\n"
    "- search: discover papers. code keys: q (required), n (1-50, default 10), "
    "offset, from, to (YYYY-MM-DD), cats (array like [\"cs.CL\"]), sort "
    "(relevance|date). In q use arXiv field syntax: ti: (title), au: (author), "
    "abs: (abstract), cat: (category), combined with AND/OR/ANDNOT. Prefer the "
    "key `q` (not `query`).\n"
    "- retrieve_paper: fetch ONE paper's content, pruned and chunked for "
    "reading. code keys: paper_id (required), prune_references (default true), "
    "chunk_chars (default 4000), chunk_overlap (default 200), segmentation_k "
    "(optional float, e.g. 1.2, for hierarchical structure on complex papers). "
    "The returned `paper` field carries only the id/url, not metadata.\n"
    "- execute: batch metadata/content ops. code is one Operation or an array "
    "of them. Operation keys: op (abstract|download|citations|recs|retrieve), "
    "id (required), limit (citations<=100, recs<=50). abstract=metadata+"
    "abstract, download=full markdown, citations=papers citing this, "
    "recs=similar papers, retrieve=prepared content.\n"
    "- hdrr: Hybrid Document-Routed Retrieval for multi-paper question "
    "answering over an index. code keys: q (required), limit_docs (default 5), "
    "limit_chunks (default 10).\n\n"
    "Workflow: discover with search; for a cross-paper question prefer hdrr; "
    "for a deep single-paper read use retrieve_paper. arXiv IDs may carry an "
    "`arxiv:` prefix or a version suffix (e.g. 1706.03762v2) and are "
    "normalized server-side. If a search returns nothing, broaden the query "
    "before retrieving."
)

# --------------------------------------------------------------------------- #
# Tool schemas (mirror crates/native/src/tool.rs: each tool has one `code`
# string param described by its #[schemars(description=...)]).
# --------------------------------------------------------------------------- #

TOOLS = [
    {
        "type": "function",
        "function": {
            "name": "search",
            "description": "Search arXiv papers with filters (categories, dates, sorting).",
            "parameters": {
                "type": "object",
                "properties": {
                    "code": {
                        "type": "string",
                        "description": (
                            "JSON object. Keys: q (required arXiv query, e.g. "
                            "'ti:attention AND au:vaswani'; supports ti: au: abs: "
                            "cat: and AND/OR/ANDNOT), n (1-50, default 10), offset "
                            "(default 0), from/to (YYYY-MM-DD), cats (string array "
                            "e.g. [\"cs.AI\",\"cs.LG\"]), sort (relevance|date)."
                        ),
                    }
                },
                "required": ["code"],
            },
        },
    },
    {
        "type": "function",
        "function": {
            "name": "retrieve_paper",
            "description": "Get one paper's content, pruned and chunked for LLM reading. Supports hierarchical segmentation (segmentation_k).",
            "parameters": {
                "type": "object",
                "properties": {
                    "code": {
                        "type": "string",
                        "description": (
                            "JSON object. Keys: paper_id (required arXiv id, e.g. "
                            "'1706.03762'; 'arxiv:' prefix and version suffix are "
                            "normalized), prune_references (default true), "
                            "chunk_chars (default 4000), chunk_overlap (default "
                            "200), segmentation_k (optional float for hierarchical "
                            "chunking, e.g. 1.2)."
                        ),
                    }
                },
                "required": ["code"],
            },
        },
    },
    {
        "type": "function",
        "function": {
            "name": "execute",
            "description": "Batch fetch: abstracts, full text, citations, recommendations, or prepared content.",
            "parameters": {
                "type": "object",
                "properties": {
                    "code": {
                        "type": "string",
                        "description": (
                            "JSON Operation, or a JSON array of Operations for "
                            "batching. Operation keys: op (abstract|download|"
                            "citations|recs|retrieve), id (required arXiv id), "
                            "limit (citations<=100, recs<=50, default 10), "
                            "prune_references, chunk_chars, chunk_overlap, "
                            "segmentation_k."
                        ),
                    }
                },
                "required": ["code"],
            },
        },
    },
    {
        "type": "function",
        "function": {
            "name": "hdrr",
            "description": "Hybrid Document-Routed Retrieval (HDRR): two-stage multi-paper QA. Stage 1 routes documents, stage 2 does scoped chunk search.",
            "parameters": {
                "type": "object",
                "properties": {
                    "code": {
                        "type": "string",
                        "description": (
                            "JSON object. Keys: q (required query), limit_docs "
                            "(default 5, stage-1 documents), limit_chunks (default "
                            "10, stage-2 chunks), segmentation_k (optional)."
                        ),
                    }
                },
                "required": ["code"],
            },
        },
    },
]

# --------------------------------------------------------------------------- #
# A small library of real arXiv papers used to build schema-faithful results.
# --------------------------------------------------------------------------- #

PAPERS = {
    "1706.03762": {
        "title": "Attention Is All You Need",
        "authors": ["Ashish Vaswani", "Noam Shazeer", "Niki Parmar"],
        "cats": ["cs.CL", "cs.LG"],
        "published": "2017-06-12T17:57:34Z",
        "abstract": (
            "The dominant sequence transduction models are based on complex "
            "recurrent or convolutional neural networks. We propose the "
            "Transformer, a model architecture based solely on attention "
            "mechanisms, dispensing with recurrence and convolutions entirely. "
            "Experiments show these models are superior in quality while being "
            "more parallelizable and requiring significantly less time to train."
        ),
    },
    "1810.04805": {
        "title": "BERT: Pre-training of Deep Bidirectional Transformers for Language Understanding",
        "authors": ["Jacob Devlin", "Ming-Wei Chang", "Kenton Lee"],
        "cats": ["cs.CL"],
        "published": "2018-10-11T00:50:01Z",
        "abstract": (
            "We introduce BERT, a language representation model that pre-trains "
            "deep bidirectional representations from unlabeled text by jointly "
            "conditioning on both left and right context. BERT obtains new "
            "state-of-the-art results on eleven natural language processing tasks."
        ),
    },
    "2005.14165": {
        "title": "Language Models are Few-Shot Learners",
        "authors": ["Tom B. Brown", "Benjamin Mann", "Nick Ryder"],
        "cats": ["cs.CL"],
        "published": "2020-05-28T17:29:03Z",
        "abstract": (
            "We show that scaling up language models greatly improves "
            "task-agnostic, few-shot performance. We train GPT-3, an "
            "autoregressive language model with 175 billion parameters, and test "
            "its performance in the few-shot setting without any gradient updates."
        ),
    },
    "2203.02155": {
        "title": "Training language models to follow instructions with human feedback",
        "authors": ["Long Ouyang", "Jeff Wu", "Xu Jiang"],
        "cats": ["cs.CL", "cs.LG"],
        "published": "2022-03-04T07:04:42Z",
        "abstract": (
            "We show an avenue for aligning language models with user intent by "
            "fine-tuning with human feedback. We collect demonstrations and use "
            "reinforcement learning from human feedback (RLHF) to fine-tune GPT-3 "
            "into InstructGPT, which is preferred over much larger models."
        ),
    },
    "2201.11903": {
        "title": "Chain-of-Thought Prompting Elicits Reasoning in Large Language Models",
        "authors": ["Jason Wei", "Xuezhi Wang", "Dale Schuurmans"],
        "cats": ["cs.CL"],
        "published": "2022-01-28T16:18:31Z",
        "abstract": (
            "We explore how generating a chain of thought - a series of "
            "intermediate reasoning steps - significantly improves the ability of "
            "large language models to perform complex reasoning, an ability that "
            "emerges naturally at sufficient model scale."
        ),
    },
    "2005.11401": {
        "title": "Retrieval-Augmented Generation for Knowledge-Intensive NLP Tasks",
        "authors": ["Patrick Lewis", "Ethan Perez", "Aleksandra Piktus"],
        "cats": ["cs.CL", "cs.LG"],
        "published": "2020-05-22T21:34:34Z",
        "abstract": (
            "We introduce retrieval-augmented generation (RAG), models that "
            "combine pre-trained parametric memory with non-parametric memory "
            "from a dense vector index of Wikipedia, accessed with a neural "
            "retriever, for knowledge-intensive NLP tasks."
        ),
    },
    "1910.10683": {
        "title": "Exploring the Limits of Transfer Learning with a Unified Text-to-Text Transformer",
        "authors": ["Colin Raffel", "Noam Shazeer", "Adam Roberts"],
        "cats": ["cs.LG", "cs.CL"],
        "published": "2019-10-23T17:37:36Z",
        "abstract": (
            "We explore transfer learning for NLP by introducing a unified "
            "framework that converts every language problem into a text-to-text "
            "format, studying the limits of transfer learning with our "
            "Text-to-Text Transfer Transformer (T5)."
        ),
    },
    "2106.09685": {
        "title": "LoRA: Low-Rank Adaptation of Large Language Models",
        "authors": ["Edward J. Hu", "Yelong Shen", "Phillip Wallis"],
        "cats": ["cs.CL", "cs.LG"],
        "published": "2021-06-17T17:37:18Z",
        "abstract": (
            "We propose Low-Rank Adaptation (LoRA), which freezes the pre-trained "
            "model weights and injects trainable rank decomposition matrices into "
            "each Transformer layer, greatly reducing the number of trainable "
            "parameters for downstream tasks."
        ),
    },
    "2302.13971": {
        "title": "LLaMA: Open and Efficient Foundation Language Models",
        "authors": ["Hugo Touvron", "Thibaut Lavril", "Gautier Izacard"],
        "cats": ["cs.CL"],
        "published": "2023-02-27T17:00:00Z",
        "abstract": (
            "We introduce LLaMA, a collection of foundation language models "
            "ranging from 7B to 65B parameters, trained on trillions of tokens "
            "using publicly available datasets exclusively."
        ),
    },
    "2307.09288": {
        "title": "Llama 2: Open Foundation and Fine-Tuned Chat Models",
        "authors": ["Hugo Touvron", "Louis Martin", "Kevin Stone"],
        "cats": ["cs.CL", "cs.AI"],
        "published": "2023-07-18T17:00:00Z",
        "abstract": (
            "We develop and release Llama 2, a collection of pretrained and "
            "fine-tuned large language models ranging from 7B to 70B parameters, "
            "optimized for dialogue use cases."
        ),
    },
    "1512.03385": {
        "title": "Deep Residual Learning for Image Recognition",
        "authors": ["Kaiming He", "Xiangyu Zhang", "Shaoqing Ren"],
        "cats": ["cs.CV"],
        "published": "2015-12-10T19:51:55Z",
        "abstract": (
            "We present a residual learning framework to ease the training of "
            "networks that are substantially deeper than those used previously, "
            "reformulating layers as learning residual functions with reference "
            "to the layer inputs."
        ),
    },
    "2010.11929": {
        "title": "An Image is Worth 16x16 Words: Transformers for Image Recognition at Scale",
        "authors": ["Alexey Dosovitskiy", "Lucas Beyer", "Alexander Kolesnikov"],
        "cats": ["cs.CV", "cs.LG"],
        "published": "2020-10-22T17:55:01Z",
        "abstract": (
            "We show that a pure transformer applied directly to sequences of "
            "image patches can perform very well on image classification tasks, "
            "attaining excellent results compared to state-of-the-art "
            "convolutional networks while requiring fewer resources to train."
        ),
    },
    "2112.10752": {
        "title": "High-Resolution Image Synthesis with Latent Diffusion Models",
        "authors": ["Robin Rombach", "Andreas Blattmann", "Dominik Lorenz"],
        "cats": ["cs.CV"],
        "published": "2021-12-20T18:55:25Z",
        "abstract": (
            "We apply diffusion models in the latent space of powerful "
            "pretrained autoencoders, enabling high-resolution image synthesis "
            "with greatly reduced computational requirements while retaining "
            "quality and flexibility."
        ),
    },
    "2305.10601": {
        "title": "Tree of Thoughts: Deliberate Problem Solving with Large Language Models",
        "authors": ["Shunyu Yao", "Dian Yu", "Jeffrey Zhao"],
        "cats": ["cs.CL", "cs.AI"],
        "published": "2023-05-17T17:00:00Z",
        "abstract": (
            "We introduce Tree of Thoughts (ToT), a framework that generalizes "
            "chain-of-thought prompting and enables exploration over coherent "
            "units of text (thoughts) as intermediate steps, with lookahead and "
            "backtracking via search."
        ),
    },
    "2104.09864": {
        "title": "RoFormer: Enhanced Transformer with Rotary Position Embedding",
        "authors": ["Jianlin Su", "Yu Lu", "Shengfeng Pan"],
        "cats": ["cs.CL", "cs.LG"],
        "published": "2021-04-20T08:00:00Z",
        "abstract": (
            "We propose Rotary Position Embedding (RoPE) to leverage positional "
            "information in transformer language models, encoding absolute "
            "position with a rotation matrix while naturally incorporating "
            "relative position dependency in self-attention."
        ),
    },
    "1412.6980": {
        "title": "Adam: A Method for Stochastic Optimization",
        "authors": ["Diederik P. Kingma", "Jimmy Ba"],
        "cats": ["cs.LG"],
        "published": "2014-12-22T20:09:24Z",
        "abstract": (
            "We introduce Adam, an algorithm for first-order gradient-based "
            "optimization of stochastic objective functions, based on adaptive "
            "estimates of lower-order moments. The method is computationally "
            "efficient and well suited for problems large in data and parameters."
        ),
    },
    "2401.04088": {
        "title": "Mixtral of Experts",
        "authors": ["Albert Q. Jiang", "Alexandre Sablayrolles", "Antoine Roux"],
        "cats": ["cs.LG", "cs.CL"],
        "published": "2024-01-08T18:00:00Z",
        "abstract": (
            "We introduce Mixtral 8x7B, a Sparse Mixture of Experts (SMoE) "
            "language model where each layer has 8 feedforward experts and a "
            "router selects two per token, giving each token access to 47B "
            "parameters while using only 13B during inference."
        ),
    },
    "2009.06732": {
        "title": "Efficient Transformers: A Survey",
        "authors": ["Yi Tay", "Mostafa Dehghani", "Dara Bahri"],
        "cats": ["cs.LG"],
        "published": "2020-09-14T17:00:00Z",
        "abstract": (
            "Transformer efficiency has become an important research direction. "
            "This survey characterizes a large and thoughtful selection of recent "
            "efficiency-flavored 'X-former' models, providing an organized "
            "taxonomy across the literature."
        ),
    },
}

# --------------------------------------------------------------------------- #
# Builders. These only assemble dicts; nothing here invents example content.
# --------------------------------------------------------------------------- #


def sys_msg():
    return {"role": "system", "content": SYSTEM}


def usr(text):
    return {"role": "user", "content": text}


def asst(text):
    return {"role": "assistant", "content": text}


def call(name, code, cid="call_1", note=""):
    """An assistant tool-call turn. `code` is the JSON value the server expects
    inside the `code` string envelope (dict for most tools, dict-or-list for
    execute). `note` is brief narration shown before the call."""
    arguments = json.dumps({"code": json.dumps(code, ensure_ascii=False)}, ensure_ascii=False)
    return {
        "role": "assistant",
        "content": note,
        "tool_calls": [
            {
                "id": cid,
                "type": "function",
                "function": {"name": name, "arguments": arguments},
            }
        ],
    }


def result(cid, payload):
    """A tool-result turn. `payload` may be a dict/list (json-encoded) or a raw
    string (e.g. the markdown returned by op=download)."""
    content = payload if isinstance(payload, str) else json.dumps(payload, ensure_ascii=False)
    return {"role": "tool", "tool_call_id": cid, "content": content}


def full_paper(pid):
    p = PAPERS[pid]
    return {
        "id": pid,
        "title": p["title"],
        "authors": [{"name": n, "affiliations": []} for n in p["authors"]],
        "abstract_text": p["abstract"],
        "categories": p["cats"],
        "published": p["published"],
        "url": f"https://arxiv.org/abs/{pid}",
        "doi": None,
        "journal_ref": None,
    }


def search_response(ids, total, start=0):
    return {
        "papers": [full_paper(i) for i in ids],
        "total_results": total,
        "start_index": start,
    }


def ss_paper(pid, title, authors, year):
    """Semantic-Scholar-derived paper (citations/recs): no abstract/categories."""
    return {
        "id": pid,
        "title": title,
        "authors": [{"name": n, "affiliations": []} for n in authors],
        "abstract_text": "",
        "categories": [],
        "published": str(year),
        "url": f"https://arxiv.org/abs/{pid}" if pid else "",
        "doi": None,
        "journal_ref": None,
    }


def content_paper(pid):
    """The sparse `paper` field that retrieve/op=retrieve returns: title==id,
    no metadata, because the content path does not fetch the Atom record."""
    return {
        "id": pid,
        "title": pid,
        "authors": [],
        "abstract_text": "",
        "categories": [],
        "published": "",
        "url": f"https://arxiv.org/abs/{pid}",
        "doi": None,
        "journal_ref": None,
    }


def prepared(pid, source, chunk_texts, pruned_md, raw_md=None, clusters=None, parents=None):
    chunks = []
    pos = 0
    for i, text in enumerate(chunk_texts):
        cluster_id = clusters[i] if clusters else None
        parent_id = parents[i] if parents else None
        body = text
        ctx = [c for c in (parent_id, cluster_id) if c]
        if ctx:
            body = "Context: " + " -> ".join(ctx) + "\n\n" + text
        chunks.append(
            {
                "index": i,
                "start_char": pos,
                "end_char": pos + len(text),
                "text": body,
                "cluster_id": cluster_id,
                "parent_id": parent_id,
            }
        )
        pos += len(text)
    return {
        "paper": content_paper(pid),
        "source": source,
        "raw_markdown": raw_md if raw_md is not None else pruned_md,
        "pruned_markdown": pruned_md,
        "chunks": chunks,
        "hierarchical_chunks": None,
    }


def exec_one(pid, op, value):
    return {"id": pid, "op": op, "result": value}


# --------------------------------------------------------------------------- #
# The curated conversations. Grouped by the skill each one teaches.
# --------------------------------------------------------------------------- #

EXAMPLES: list[list[dict]] = []
E = EXAMPLES.append

# ---- search: discovery & field filters ----------------------------------- #

# 1. Plain topic search.
E([
    usr("Find me a few foundational papers on the transformer architecture."),
    call("search", {"q": "ti:transformer architecture attention", "n": 5, "sort": "relevance"},
         note="Searching arXiv for foundational transformer papers."),
    result("call_1", search_response(["1706.03762", "1910.10683", "2010.11929"], 4123)),
    asst(
        "Here are the foundational hits:\n"
        "1. **Attention Is All You Need** (1706.03762, Vaswani et al., 2017) - introduces the Transformer, dropping recurrence/convolution for pure attention.\n"
        "2. **Exploring the Limits of Transfer Learning (T5)** (1910.10683, Raffel et al., 2019) - unifies NLP tasks in a text-to-text Transformer.\n"
        "3. **An Image is Worth 16x16 Words (ViT)** (2010.11929, Dosovitskiy et al., 2020) - applies the Transformer to image patches.\n\n"
        "Want me to pull the full text of any of these?"
    ),
])

# 2. Title + author field filters.
E([
    usr("I'm looking for the 'Attention Is All You Need' paper by Vaswani specifically."),
    call("search", {"q": "ti:\"attention is all you need\" AND au:vaswani", "n": 3},
         note="Using title and author field filters to pin it down."),
    result("call_1", search_response(["1706.03762"], 1)),
    asst(
        "Found it: **Attention Is All You Need** (arXiv:1706.03762), Vaswani et al., 2017, "
        "categories cs.CL/cs.LG. https://arxiv.org/abs/1706.03762\n\n"
        "It proposes the Transformer, built entirely on attention. Want the abstract or the full text?"
    ),
])

# 3. Author-only search, sorted by date.
E([
    usr("What are the most recent papers from Hugo Touvron?"),
    call("search", {"q": "au:touvron", "n": 5, "sort": "date"},
         note="Searching by author, newest first."),
    result("call_1", search_response(["2307.09288", "2302.13971"], 12)),
    asst(
        "Most recent first:\n"
        "1. **Llama 2: Open Foundation and Fine-Tuned Chat Models** (2307.09288, 2023)\n"
        "2. **LLaMA: Open and Efficient Foundation Language Models** (2302.13971, 2023)\n\n"
        "Both are from the LLaMA line of open foundation models."
    ),
])

# 4. Category-filtered search.
E([
    usr("Show me retrieval-augmented generation work, but only in NLP categories."),
    call("search",
         {"q": "abs:retrieval augmented generation", "cats": ["cs.CL", "cs.LG"], "n": 5, "sort": "relevance"},
         note="Restricting to cs.CL and cs.LG with a category filter."),
    result("call_1", search_response(["2005.11401"], 287)),
    asst(
        "Top result in cs.CL/cs.LG: **Retrieval-Augmented Generation for Knowledge-Intensive NLP Tasks** "
        "(2005.11401, Lewis et al., 2020). It pairs a parametric seq2seq model with a dense retriever over "
        "Wikipedia. Want me to retrieve the method section?"
    ),
])

# 5. Date-ranged search.
E([
    usr("Find diffusion model papers submitted in 2021."),
    call("search",
         {"q": "abs:diffusion model image synthesis", "from": "2021-01-01", "to": "2021-12-31",
          "cats": ["cs.CV"], "n": 5, "sort": "date"},
         note="Adding a 2021 submittedDate window and a cs.CV filter."),
    result("call_1", search_response(["2112.10752"], 64)),
    asst(
        "From 2021: **High-Resolution Image Synthesis with Latent Diffusion Models** "
        "(2112.10752, Rombach et al., Dec 2021) - the latent-diffusion approach behind Stable Diffusion. "
        "It runs the diffusion process in a compressed latent space for efficiency."
    ),
])

# 6. Pagination via offset.
E([
    usr("Search for mixture-of-experts language models."),
    call("search", {"q": "abs:mixture of experts language model", "n": 5, "sort": "relevance"},
         note="Initial search for MoE language models."),
    result("call_1", search_response(["2401.04088"], 51)),
    asst("Top hit: **Mixtral of Experts** (2401.04088, Jiang et al., 2024), a sparse MoE with 8 experts per layer. There are 51 results total - say the word for more."),
    usr("Yeah, show me the next page."),
    call("search", {"q": "abs:mixture of experts language model", "n": 5, "offset": 5, "sort": "relevance"},
         cid="call_2", note="Fetching the next page with offset=5."),
    result("call_2", search_response(["2009.06732"], 51, start=5)),
    asst("Next page includes **Efficient Transformers: A Survey** (2009.06732, Tay et al., 2020), which covers sparse and MoE variants among other efficiency techniques."),
])

# 7. Abstract field + ANDNOT boolean.
E([
    usr("I want papers about attention mechanisms but not the ones focused on computer vision."),
    call("search", {"q": "abs:attention mechanism ANDNOT cat:cs.CV", "n": 5, "sort": "relevance"},
         note="Excluding cs.CV with ANDNOT."),
    result("call_1", search_response(["1706.03762", "2104.09864"], 932)),
    asst(
        "Non-vision attention work:\n"
        "1. **Attention Is All You Need** (1706.03762) - the core self-attention Transformer.\n"
        "2. **RoFormer: Enhanced Transformer with Rotary Position Embedding** (2104.09864) - improves how attention encodes position via RoPE."
    ),
])

# ---- retrieve_paper: deep single-paper reads ------------------------------ #

# 8. Straight retrieve by id.
E([
    usr("Pull the full text of arXiv 2106.09685 so I can read how LoRA works."),
    call("retrieve_paper", {"paper_id": "2106.09685"},
         note="Retrieving and chunking the LoRA paper."),
    result("call_1", prepared(
        "2106.09685", "html",
        [
            "# LoRA: Low-Rank Adaptation of Large Language Models\n\nWe propose freezing the pretrained weights W0 and representing the update as a low-rank product BA, so only A and B are trained.",
            "## Method\n\nFor a pretrained weight matrix W0 in R^{d x k}, we constrain its update with a low-rank decomposition W0 + dW = W0 + BA, where B in R^{d x r}, A in R^{r x k}, and the rank r << min(d, k).",
        ],
        pruned_md="# LoRA...\n\n## Method\n\nFor a pretrained weight matrix W0...")),
    asst(
        "Retrieved 2106.09685 (2 chunks). The core idea: freeze the pretrained weight matrix W0 and learn only a "
        "low-rank update dW = BA, with B (d x r) and A (r x k) where rank r is tiny relative to the matrix "
        "dimensions. That cuts trainable parameters by orders of magnitude while matching full fine-tuning quality. "
        "Want me to read further into the experiments?"
    ),
])

# 9. Retrieve with hierarchical segmentation_k for a complex paper.
E([
    usr("I need a deep, structured read of the Attention Is All You Need paper - keep the section structure."),
    call("retrieve_paper", {"paper_id": "1706.03762", "segmentation_k": 1.2},
         note="Enabling hierarchical segmentation (segmentation_k=1.2) to preserve structure."),
    result("call_1", prepared(
        "1706.03762", "html",
        [
            "The Transformer uses stacked self-attention and point-wise fully connected layers for both the encoder and decoder.",
            "Scaled dot-product attention computes softmax(QK^T / sqrt(d_k)) V; multi-head attention runs h of these in parallel.",
        ],
        pruned_md="# Attention Is All You Need\n\n## Model Architecture\n\n...",
        parents=["Model Architecture", "Model Architecture"],
        clusters=["Encoder and Decoder Stacks", "Attention"])),
    asst(
        "Read it with hierarchical structure preserved. Each chunk now carries its structural parent, e.g. "
        "*Model Architecture -> Encoder and Decoder Stacks* and *Model Architecture -> Attention*. Key points: the "
        "Transformer stacks self-attention + feed-forward layers; attention is scaled dot-product, "
        "softmax(QK^T/sqrt(d_k))V, run as h parallel heads. The `Context:` prefix on each chunk tells you where it "
        "sits in the paper's hierarchy."
    ),
])

# 10. Retrieve with custom chunking.
E([
    usr("Retrieve 1512.03385 but use small 1500-character chunks with 100 overlap - I'm feeding it to a small-context model."),
    call("retrieve_paper", {"paper_id": "1512.03385", "chunk_chars": 1500, "chunk_overlap": 100},
         note="Retrieving ResNet with smaller chunks for a small-context model."),
    result("call_1", prepared(
        "1512.03385", "html",
        [
            "Deeper neural networks are more difficult to train. We present a residual learning framework to ease training of networks substantially deeper than before.",
            "We explicitly reformulate the layers as learning residual functions F(x) = H(x) - x with reference to the layer inputs, instead of learning unreferenced functions H(x).",
        ],
        pruned_md="# Deep Residual Learning...\n\n...")),
    asst("Done - 1512.03385 (ResNet) retrieved as 1500-char chunks (overlap 100), HTML source. The thesis: reformulate layers to learn a residual F(x)=H(x)-x rather than the full mapping, which makes very deep nets trainable. Ready for the small-context model."),
])

# 11. Retrieve keeping references.
E([
    usr("Get me 2005.11401 and DON'T strip the references - I want to see what they cited."),
    call("retrieve_paper", {"paper_id": "2005.11401", "prune_references": False},
         note="Setting prune_references=false to keep the bibliography."),
    result("call_1", prepared(
        "2005.11401", "html",
        [
            "We introduce RAG, combining a pretrained seq2seq generator with a dense retriever over a Wikipedia index.",
            "## References\n[1] Karpukhin et al. Dense Passage Retrieval for Open-Domain QA. 2020.\n[2] Vaswani et al. Attention Is All You Need. 2017.",
        ],
        pruned_md="...# References\n[1] Karpukhin et al. ...\n[2] Vaswani et al. ...")),
    asst(
        "Kept the references in. RAG (2005.11401) cites, among others, Dense Passage Retrieval (Karpukhin et al., 2020) "
        "and Attention Is All You Need (Vaswani et al., 2017). With pruning disabled, the bibliography stays in the "
        "chunked output so you can trace its sources."
    ),
])

# 12. ID normalization (arxiv: prefix + version suffix).
E([
    usr("Read arxiv:1706.03762v7 for me."),
    call("retrieve_paper", {"paper_id": "arxiv:1706.03762v7"},
         note="The server normalizes the 'arxiv:' prefix and version suffix."),
    result("call_1", prepared(
        "1706.03762", "html",
        ["The Transformer is the first sequence transduction model based entirely on attention."],
        pruned_md="# Attention Is All You Need\n\n...")),
    asst("Retrieved it - the `arxiv:` prefix and `v7` suffix were normalized to the base id 1706.03762 automatically. It's the Transformer paper; the content is chunked and ready."),
])

# 13. Two-step: search to resolve an id, then retrieve.
E([
    usr("Can you get me the full text of the BERT paper?"),
    call("search", {"q": "ti:bert pre-training bidirectional transformers", "n": 1},
         note="I don't have the id memorized - searching first so I don't guess it."),
    result("call_1", search_response(["1810.04805"], 1)),
    call("retrieve_paper", {"paper_id": "1810.04805", "segmentation_k": 1.2}, cid="call_2",
         note="Found id 1810.04805 - now retrieving the full text with structure."),
    result("call_2", prepared(
        "1810.04805", "html",
        [
            "BERT pretrains deep bidirectional representations using a masked language model objective plus next-sentence prediction.",
            "Fine-tuning adds a single output layer on top of the pretrained model to achieve state of the art on many tasks.",
        ],
        pruned_md="# BERT\n\n...",
        parents=["Approach", "Approach"],
        clusters=["Pre-training BERT", "Fine-tuning BERT"])),
    asst(
        "Resolved the title to arXiv:1810.04805 and retrieved it. BERT pretrains with two objectives - a masked "
        "language model and next-sentence prediction - producing deep bidirectional representations; downstream tasks "
        "then just add one output layer for fine-tuning. I searched first rather than guessing the id."
    ),
])

# ---- execute: abstract / download / citations / recs / retrieve / batch --- #

# 14. execute abstract.
E([
    usr("Just give me the abstract of 2201.11903."),
    call("execute", {"op": "abstract", "id": "2201.11903"},
         note="Fetching metadata + abstract only."),
    result("call_1", exec_one("2201.11903", "abstract", full_paper("2201.11903"))),
    asst(
        "**Chain-of-Thought Prompting Elicits Reasoning in Large Language Models** (2201.11903, Wei et al., 2022, cs.CL).\n\n"
        "Abstract: generating a chain of thought - intermediate reasoning steps - markedly improves LLMs' complex-reasoning "
        "ability, and this ability emerges naturally once models are large enough."
    ),
])

# 15. execute download (full markdown).
E([
    usr("Download the full text of 1412.6980 as markdown."),
    call("execute", {"op": "download", "id": "1412.6980"},
         note="op=download returns the full paper as markdown text."),
    result("call_1", exec_one("1412.6980", "download",
        "# Adam: A Method for Stochastic Optimization\n\n## Abstract\nWe introduce Adam, an algorithm for "
        "first-order gradient-based optimization...\n\n## 2. Algorithm\nWe compute biased first/second moment "
        "estimates m_t and v_t, bias-correct them, and update parameters by m_hat / (sqrt(v_hat) + epsilon).")),
    asst("Here's the full markdown for 1412.6980 (Adam). It opens with the abstract, then the algorithm: maintain exponential moving averages of the gradient (m_t) and its square (v_t), bias-correct both, and step by m_hat/(sqrt(v_hat)+epsilon). The complete text is in the tool output above."),
])

# 16. execute citations.
E([
    usr("What papers cite the original GPT-3 paper? Give me the top 5."),
    call("execute", {"op": "citations", "id": "2005.14165", "limit": 5},
         note="Fetching up to 5 citing papers via Semantic Scholar."),
    result("call_1", exec_one("2005.14165", "citations", [
        ss_paper("2203.02155", "Training language models to follow instructions with human feedback", ["Long Ouyang"], 2022),
        ss_paper("2201.11903", "Chain-of-Thought Prompting Elicits Reasoning in Large Language Models", ["Jason Wei"], 2022),
        ss_paper("2302.13971", "LLaMA: Open and Efficient Foundation Language Models", ["Hugo Touvron"], 2023),
    ])),
    asst(
        "Papers citing GPT-3 (2005.14165) include:\n"
        "1. InstructGPT - Training LMs to follow instructions with human feedback (2203.02155, 2022)\n"
        "2. Chain-of-Thought Prompting (2201.11903, 2022)\n"
        "3. LLaMA (2302.13971, 2023)\n\n"
        "These build on or compare against GPT-3's few-shot paradigm."
    ),
])

# 17. execute recs.
E([
    usr("Recommend papers similar to LoRA (2106.09685)."),
    call("execute", {"op": "recs", "id": "2106.09685", "limit": 5},
         note="Fetching similar-paper recommendations."),
    result("call_1", exec_one("2106.09685", "recs", [
        ss_paper("2104.09864", "RoFormer: Enhanced Transformer with Rotary Position Embedding", ["Jianlin Su"], 2021),
        ss_paper("1910.10683", "Exploring the Limits of Transfer Learning with a Unified Text-to-Text Transformer", ["Colin Raffel"], 2019),
    ])),
    asst(
        "Recommendations similar to LoRA:\n"
        "1. **RoFormer / RoPE** (2104.09864) - another lightweight architectural tweak to Transformers.\n"
        "2. **T5** (1910.10683) - the transfer-learning backbone LoRA-style methods are often applied to.\n\n"
        "If you want parameter-efficient-tuning neighbors specifically, I can refine the search."
    ),
])

# 18. execute retrieve op (alternative to retrieve_paper).
E([
    usr("Use execute's retrieve op to prep 2010.11929 for reading."),
    call("execute", {"op": "retrieve", "id": "2010.11929", "chunk_chars": 3000},
         note="op=retrieve returns the same prepared/chunked content as retrieve_paper."),
    result("call_1", exec_one("2010.11929", "retrieve", prepared(
        "2010.11929", "html",
        [
            "We split an image into fixed-size patches, linearly embed each, add position embeddings, and feed the sequence to a standard Transformer encoder.",
            "When pre-trained on large datasets, ViT matches or exceeds state-of-the-art CNNs while using fewer compute resources to train.",
        ],
        pruned_md="# An Image is Worth 16x16 Words\n\n..."))),
    asst("Prepped 2010.11929 (ViT) via execute's retrieve op - 2 chunks, HTML source. ViT cuts an image into fixed patches, embeds them with positions, and runs a vanilla Transformer encoder; with enough pre-training it matches or beats top CNNs at lower training cost."),
])

# 19. execute batch (array) - several abstracts at once.
E([
    usr("Give me the abstracts of the three LLaMA-family papers: 2302.13971, 2307.09288, and the Mixtral one 2401.04088."),
    call("execute", [
        {"op": "abstract", "id": "2302.13971"},
        {"op": "abstract", "id": "2307.09288"},
        {"op": "abstract", "id": "2401.04088"},
    ], note="Batching three abstract ops in a single execute call (array in `code`)."),
    result("call_1", [
        exec_one("2302.13971", "abstract", full_paper("2302.13971")),
        exec_one("2307.09288", "abstract", full_paper("2307.09288")),
        exec_one("2401.04088", "abstract", full_paper("2401.04088")),
    ]),
    asst(
        "Batched all three:\n"
        "- **LLaMA** (2302.13971): 7B-65B foundation models trained only on public data.\n"
        "- **Llama 2** (2307.09288): 7B-70B pretrained + fine-tuned chat models.\n"
        "- **Mixtral of Experts** (2401.04088): sparse MoE, 8 experts/layer, 2 active per token (47B total, ~13B active).\n\n"
        "One execute call with an array handled all three at once."
    ),
])

# 20. execute batch mixed ops on one paper.
E([
    usr("For the Transformer paper 1706.03762, I want both its abstract and the papers that cite it."),
    call("execute", [
        {"op": "abstract", "id": "1706.03762"},
        {"op": "citations", "id": "1706.03762", "limit": 3},
    ], note="Batching an abstract op and a citations op for the same paper."),
    result("call_1", [
        exec_one("1706.03762", "abstract", full_paper("1706.03762")),
        exec_one("1706.03762", "citations", [
            ss_paper("1810.04805", "BERT: Pre-training of Deep Bidirectional Transformers", ["Jacob Devlin"], 2018),
            ss_paper("2005.14165", "Language Models are Few-Shot Learners", ["Tom B. Brown"], 2020),
        ]),
    ]),
    asst(
        "**Attention Is All You Need** (1706.03762, 2017) introduces the attention-only Transformer.\n\n"
        "Highly-cited descendants include BERT (1810.04805, 2018) and GPT-3 (2005.14165, 2020) - both build directly on "
        "the Transformer block. One batched call returned both the abstract and the citation list."
    ),
])

# ---- hdrr: multi-paper synthesis (and graceful fallback) ------------------ #

# 21. hdrr success across an index.
E([
    usr("Across the indexed reasoning papers, how does tree-of-thoughts differ from chain-of-thought?"),
    call("hdrr", {"q": "tree of thoughts vs chain of thought reasoning strategy", "limit_docs": 4, "limit_chunks": 8},
         note="Multi-paper question - using HDRR for document routing + scoped chunk retrieval."),
    result("call_1", {
        "query": "tree of thoughts vs chain of thought reasoning strategy",
        "routed_documents": ["2201.11903", "2305.10601"],
        "chunks": [
            {"id": "2201.11903-2", "text": "Chain-of-thought prompting elicits a single linear sequence of intermediate reasoning steps before the final answer."},
            {"id": "2305.10601-4", "text": "Tree of Thoughts maintains a tree of partial solutions (thoughts) and explores it with search, enabling lookahead and backtracking rather than one linear path."},
        ],
    }),
    asst(
        "HDRR routed to the two relevant papers and pulled the key chunks:\n\n"
        "- **Chain-of-Thought** (2201.11903) produces a *single linear* sequence of reasoning steps, then answers.\n"
        "- **Tree of Thoughts** (2305.10601) generalizes that into a *tree* of partial solutions explored with search, "
        "adding lookahead and backtracking instead of committing to one path.\n\n"
        "So ToT trades extra compute for the ability to explore and recover from bad intermediate steps."
    ),
])

# 22. hdrr with tuned limits.
E([
    usr("Summarize how the indexed papers approach parameter-efficient fine-tuning. Keep it tight - just route to the 3 best docs."),
    call("hdrr", {"q": "parameter efficient fine-tuning low-rank adaptation", "limit_docs": 3, "limit_chunks": 6},
         note="Tightening limit_docs to 3 for a focused synthesis."),
    result("call_1", {
        "query": "parameter efficient fine-tuning low-rank adaptation",
        "routed_documents": ["2106.09685"],
        "chunks": [
            {"id": "2106.09685-1", "text": "LoRA freezes pretrained weights and learns low-rank update matrices, cutting trainable parameters by up to 10,000x with no added inference latency."},
        ],
    }),
    asst("With limit_docs=3, HDRR routed to a single strongly-relevant doc: **LoRA** (2106.09685). Its approach to parameter-efficient fine-tuning is to freeze the base weights and train only small low-rank update matrices - drastically fewer trainable parameters and no extra inference latency, since the update can be merged back into the weights."),
])

# 23. hdrr fallback when embedded-db isn't enabled.
E([
    usr("Across the literature, how do RAG and T5 differ in how they bring in external knowledge?"),
    call("hdrr", {"q": "retrieval augmented generation vs T5 external knowledge", "limit_docs": 5, "limit_chunks": 10},
         note="Trying HDRR first for the cross-paper question."),
    result("call_1", {"error": "embedded-db feature not enabled"}),
    call("search", {"q": "abs:retrieval augmented generation OR ti:text-to-text transfer transformer", "n": 4, "sort": "relevance"},
         cid="call_2", note="HDRR needs the embedded-db index, which isn't built here. Falling back to search + targeted abstracts."),
    result("call_2", search_response(["2005.11401", "1910.10683"], 410)),
    call("execute", [
        {"op": "abstract", "id": "2005.11401"},
        {"op": "abstract", "id": "1910.10683"},
    ], cid="call_3", note="Pulling both abstracts to compare directly."),
    result("call_3", [
        exec_one("2005.11401", "abstract", full_paper("2005.11401")),
        exec_one("1910.10683", "abstract", full_paper("1910.10683")),
    ]),
    asst(
        "HDRR wasn't available (the server's embedded-db index isn't enabled in this build), so I fell back to "
        "search + abstracts:\n\n"
        "- **RAG** (2005.11401) brings in knowledge *non-parametrically*: a neural retriever fetches passages from an "
        "external Wikipedia index at inference time and conditions generation on them.\n"
        "- **T5** (1910.10683) stores everything *parametrically* - all knowledge lives in the model weights learned "
        "during text-to-text pre-training; there's no external lookup.\n\n"
        "So the core difference is external retrieval vs. baked-in parameters. If you build the index, HDRR would do "
        "this routing in one call."
    ),
])

# 24. hdrr empty routing -> guide to ingest first.
E([
    usr("Use HDRR to summarize the indexed papers on protein structure prediction."),
    call("hdrr", {"q": "protein structure prediction folding", "limit_docs": 5},
         note="Querying the index for protein-structure papers."),
    result("call_1", {
        "query": "protein structure prediction folding",
        "routed_documents": [],
        "chunks": [],
        "message": "No documents routed in Stage 1.",
    }),
    asst(
        "HDRR routed zero documents - the index doesn't contain anything on protein structure prediction yet "
        "(HDRR only searches papers that have already been retrieved/ingested). Two options:\n"
        "1. I can `search` arXiv (e.g. cat:q-bio.BM) to find candidate papers, then `retrieve_paper` them to populate "
        "the index, after which HDRR will work.\n"
        "2. Or, if you have specific arXiv IDs, give them to me and I'll ingest those directly.\n\n"
        "Want me to start with a search?"
    ),
])

# ---- multi-step agentic chains -------------------------------------------- #

# 25. Full chain: search -> retrieve (with segmentation) -> synthesize.
E([
    usr("I keep hearing about 'rotary position embeddings'. Find the paper and explain the core idea from the actual text."),
    call("search", {"q": "ti:rotary position embedding", "n": 2, "sort": "relevance"},
         note="First, locate the RoPE paper."),
    result("call_1", search_response(["2104.09864"], 3)),
    call("retrieve_paper", {"paper_id": "2104.09864", "segmentation_k": 1.2}, cid="call_2",
         note="Retrieving RoFormer with structure to read the method."),
    result("call_2", prepared(
        "2104.09864", "html",
        [
            "RoPE encodes absolute position by multiplying token embeddings with a rotation matrix whose angle depends on position.",
            "Because rotations compose additively, the dot product between two rotated vectors depends only on their relative offset, injecting relative-position information directly into self-attention.",
        ],
        pruned_md="# RoFormer\n\n## Rotary Position Embedding\n\n...",
        parents=["Proposed Approach", "Proposed Approach"],
        clusters=["Rotary Position Embedding", "Properties"])),
    asst(
        "The paper is **RoFormer** (arXiv:2104.09864). From its method text: RoPE multiplies each token's embedding by "
        "a *rotation matrix* whose angle is a function of the token's absolute position. The trick is that rotations "
        "compose, so the attention dot product between two tokens ends up depending only on their *relative* distance. "
        "That gives you absolute-position encoding that automatically expresses relative position inside self-attention "
        "- no extra learned position table, and it extrapolates better to longer sequences."
    ),
])

# 26. Compare two papers via batched abstracts.
E([
    usr("Compare how ResNet and ViT approach image recognition."),
    call("execute", [
        {"op": "abstract", "id": "1512.03385"},
        {"op": "abstract", "id": "2010.11929"},
    ], note="Grabbing both abstracts in one batched call to compare."),
    result("call_1", [
        exec_one("1512.03385", "abstract", full_paper("1512.03385")),
        exec_one("2010.11929", "abstract", full_paper("2010.11929")),
    ]),
    asst(
        "- **ResNet** (1512.03385) stays convolutional but adds *residual connections* (learning F(x)=H(x)-x), which "
        "lets you train very deep CNNs without degradation.\n"
        "- **ViT** (2010.11929) drops convolution: it splits the image into patches and feeds them to a standard "
        "*Transformer* encoder, beating CNNs once pre-trained on enough data.\n\n"
        "In short: ResNet makes depth trainable within the CNN paradigm; ViT replaces the paradigm with attention."
    ),
])

# 27. Literature-landscape: broad search -> batch abstracts -> summarize.
E([
    usr("Give me a quick lay of the land on instruction tuning / RLHF for LLMs."),
    call("search", {"q": "abs:instruction tuning human feedback RLHF", "cats": ["cs.CL"], "n": 3, "sort": "relevance"},
         note="Broad discovery search in cs.CL."),
    result("call_1", search_response(["2203.02155", "2005.14165"], 156)),
    call("execute", [{"op": "abstract", "id": "2203.02155"}], cid="call_2",
         note="Pulling the InstructGPT abstract as the anchor reference."),
    result("call_2", exec_one("2203.02155", "abstract", full_paper("2203.02155"))),
    asst(
        "Quick landscape on instruction tuning / RLHF:\n"
        "- The anchor is **InstructGPT** (2203.02155, Ouyang et al., 2022): collect human demonstrations + preference "
        "comparisons, then fine-tune with RLHF so the model follows intent; a 1.3B InstructGPT was preferred over 175B GPT-3.\n"
        "- Context: it builds on **GPT-3** (2005.14165), whose few-shot prompting motivated the need for better alignment.\n\n"
        "Want me to retrieve InstructGPT's method section for the RLHF training loop details?"
    ),
])

# 28. Find citing work, then deep-read one of them.
E([
    usr("Find a notable paper that cites the Transformer paper, then summarize that citing paper's contribution."),
    call("execute", {"op": "citations", "id": "1706.03762", "limit": 3},
         note="Step 1: who cites the Transformer paper?"),
    result("call_1", exec_one("1706.03762", "citations", [
        ss_paper("1810.04805", "BERT: Pre-training of Deep Bidirectional Transformers", ["Jacob Devlin"], 2018),
        ss_paper("2005.14165", "Language Models are Few-Shot Learners", ["Tom B. Brown"], 2020),
    ])),
    call("execute", {"op": "abstract", "id": "1810.04805"}, cid="call_2",
         note="Step 2: read BERT's abstract to summarize its contribution."),
    result("call_2", exec_one("1810.04805", "abstract", full_paper("1810.04805"))),
    asst(
        "A notable citing paper is **BERT** (1810.04805, Devlin et al., 2018). Its contribution: take the Transformer "
        "encoder and pre-train it *bidirectionally* with a masked-language-model objective, so each token sees both "
        "left and right context. Fine-tuned with a single extra layer, BERT set new state of the art on eleven NLP "
        "tasks - establishing the pretrain-then-fine-tune recipe that dominated NLP afterward."
    ),
])

# 29. Recommendations -> drill into one rec.
E([
    usr("Find me something similar to the chain-of-thought paper, then give me that recommendation's abstract."),
    call("execute", {"op": "recs", "id": "2201.11903", "limit": 3},
         note="Step 1: get recommendations similar to CoT."),
    result("call_1", exec_one("2201.11903", "recs", [
        ss_paper("2305.10601", "Tree of Thoughts: Deliberate Problem Solving with Large Language Models", ["Shunyu Yao"], 2023),
    ])),
    call("execute", {"op": "abstract", "id": "2305.10601"}, cid="call_2",
         note="Step 2: fetch the abstract of the top recommendation."),
    result("call_2", exec_one("2305.10601", "abstract", full_paper("2305.10601"))),
    asst(
        "The top recommendation is **Tree of Thoughts** (2305.10601, Yao et al., 2023). Abstract in brief: ToT "
        "generalizes chain-of-thought into a searchable *tree* of intermediate \"thoughts\", letting the model do "
        "lookahead and backtracking to deliberately solve problems where a single linear chain would fail."
    ),
])

# 30. Decide hdrr vs retrieve based on scope.
E([
    usr("I have three papers indexed and want to know what they collectively say about scaling laws. One call, please."),
    call("hdrr", {"q": "scaling laws model size data compute", "limit_docs": 3, "limit_chunks": 9},
         note="Collective/cross-paper question over the index - HDRR is the right single call (not three retrieve_paper calls)."),
    result("call_1", {
        "query": "scaling laws model size data compute",
        "routed_documents": ["2005.14165", "2302.13971", "1910.10683"],
        "chunks": [
            {"id": "2005.14165-7", "text": "Performance improves smoothly and predictably as we scale parameters, data, and compute; few-shot ability strengthens with size."},
            {"id": "2302.13971-3", "text": "Smaller models trained on more tokens can outperform larger models trained on fewer, motivating compute-optimal training."},
        ],
    }),
    asst(
        "One HDRR call routed across all three indexed papers. Collectively on scaling:\n"
        "- **GPT-3** (2005.14165): capability improves smoothly and predictably with more parameters/data/compute, and "
        "few-shot ability strengthens with scale.\n"
        "- **LLaMA** (2302.13971): pushes the compute-optimal angle - a smaller model trained on *more tokens* can beat a "
        "bigger one, so token budget matters as much as parameter count.\n\n"
        "Net: scale helps predictably, but the parameter-vs-token allocation is itself a lever."
    ),
])

# ---- edge cases & robustness ---------------------------------------------- #

# 31. Empty results -> broaden the query.
E([
    usr("Search for 'quantum transformer attention for protein folding mixture of experts'."),
    call("search", {"q": "quantum transformer attention protein folding mixture of experts", "n": 5, "sort": "relevance"},
         note="Trying the full, very specific phrase first."),
    result("call_1", search_response([], 0)),
    call("search", {"q": "abs:transformer protein structure", "cats": ["q-bio.BM", "cs.LG"], "n": 5, "sort": "relevance"},
         cid="call_2", note="Zero results - broadening to the core concepts and adding category filters."),
    result("call_2", search_response([], 0)),
    asst(
        "The very specific phrase returned 0 results, and even a broadened `abs:transformer protein structure` search "
        "(in q-bio.BM/cs.LG) came back empty in this index. That combination of terms may be too niche or the wording "
        "doesn't match how authors phrase it. Could you tell me which single concept matters most - e.g. \"protein "
        "structure prediction\" vs \"mixture of experts\" - so I can target a productive search?"
    ),
])

# 32. User supplies a title only -> must search, never guess the id.
E([
    usr("Get the citations for the paper titled 'Deep Residual Learning for Image Recognition'."),
    call("search", {"q": "ti:\"deep residual learning for image recognition\"", "n": 1},
         note="I won't guess the arXiv id from a title - searching to resolve it first."),
    result("call_1", search_response(["1512.03385"], 1)),
    call("execute", {"op": "citations", "id": "1512.03385", "limit": 5}, cid="call_2",
         note="Resolved to 1512.03385 - now fetching its citations."),
    result("call_2", exec_one("1512.03385", "citations", [
        ss_paper("2010.11929", "An Image is Worth 16x16 Words", ["Alexey Dosovitskiy"], 2020),
        ss_paper("2112.10752", "High-Resolution Image Synthesis with Latent Diffusion Models", ["Robin Rombach"], 2021),
    ])),
    asst(
        "I resolved the title to **arXiv:1512.03385** (ResNet) via search rather than guessing the id, then fetched its "
        "citations. Notable citing work: ViT (2010.11929) and Latent Diffusion Models (2112.10752) - both lean on "
        "ResNet-style backbones or its deep-network insights."
    ),
])

# 33. Messy id with prefix + version inside execute.
E([
    usr("What's the abstract of ArXiv:2005.14165v4?"),
    call("execute", {"op": "abstract", "id": "ArXiv:2005.14165v4"},
         note="The 'ArXiv:' prefix and 'v4' suffix are normalized to 2005.14165 server-side."),
    result("call_1", exec_one("2005.14165", "abstract", full_paper("2005.14165"))),
    asst(
        "**Language Models are Few-Shot Learners** (2005.14165, Brown et al., 2020). The id normalized from "
        "`ArXiv:2005.14165v4` to the base `2005.14165`. Abstract: scaling to a 175B-parameter autoregressive model "
        "(GPT-3) yields strong task-agnostic few-shot performance with no gradient updates - just in-context examples."
    ),
])

# 34. Capping n at 50.
E([
    usr("Give me 200 papers on neural machine translation."),
    call("search", {"q": "abs:neural machine translation", "n": 50, "sort": "relevance"},
         note="arXiv search caps results at 50 per call, so requesting the max and noting pagination."),
    result("call_1", search_response(["1706.03762", "1910.10683"], 8800)),
    asst(
        "A single call is capped at 50 results, so I requested the maximum (there are ~8,800 matches total). Top of the "
        "list includes Attention Is All You Need (1706.03762) and T5 (1910.10683). To go past 50, I can paginate with "
        "`offset` (50, 100, ...). How many pages do you want?"
    ),
])

# ---- calibration: when NOT to call a tool --------------------------------- #

# 35. Conceptual question - answer directly, no tool.
E([
    usr("Quick conceptual question: what's the difference between precision and recall?"),
    asst(
        "Precision = TP / (TP + FP): of the items you flagged positive, how many actually are. Recall = TP / (TP + FN): "
        "of all the real positives, how many you caught. Precision punishes false alarms; recall punishes misses. You "
        "usually trade one for the other (the F1 score is their harmonic mean).\n\n"
        "That's general ML knowledge, so no paper lookup needed - but if you want the source of a specific metric or a "
        "paper that analyzes the trade-off, I can search arXiv."
    ),
])

# 36. Out-of-scope request - explain the tool's boundary, no tool call.
E([
    usr("Can you summarize the PDF sitting in my Downloads folder?"),
    asst(
        "I can't reach your local filesystem - the arxiv-search tools only fetch papers *from arXiv*, either by arXiv "
        "ID (e.g. 2106.09685) or by a search query. If that PDF is an arXiv paper, give me its ID or title and I'll "
        "retrieve and summarize it for you. Otherwise this server can't open it."
    ),
])

# 37. Reinforce the `code` envelope on a complex multi-field query.
E([
    usr("Find at most 8 papers, sorted newest-first, that mention 'long context' in the abstract, in cs.CL, submitted between 2023-06-01 and 2024-06-01."),
    call("search", {
        "q": "abs:long context",
        "cats": ["cs.CL"],
        "from": "2023-06-01",
        "to": "2024-06-01",
        "n": 8,
        "sort": "date",
    }, note="Packing every constraint into the single `code` JSON object: q, cats, from/to, n, sort."),
    result("call_1", search_response(["2401.04088"], 73)),
    asst(
        "All constraints went into one `code` object (q=abs:long context, cats=[cs.CL], from/to date window, n=8, "
        "sort=date). Within that 2023-06 to 2024-06 window the top date-sorted hit is **Mixtral of Experts** "
        "(2401.04088, Jan 2024). There are 73 matches total if you want me to page through more."
    ),
])

# 38. Disambiguate a vague reference using search results.
E([
    usr("Pull up 'the GPT paper'."),
    call("search", {"q": "ti:language models AND au:brown", "n": 3, "sort": "relevance"},
         note="\"The GPT paper\" is ambiguous - searching to surface the most likely candidate rather than guessing an id."),
    result("call_1", search_response(["2005.14165"], 9)),
    asst(
        "\"The GPT paper\" is ambiguous (GPT-1/2/3/4 all exist), but the most-cited single match is **Language Models "
        "are Few-Shot Learners** - the GPT-3 paper (arXiv:2005.14165, Brown et al., 2020). Is that the one you meant, "
        "or did you want GPT-2 / InstructGPT / GPT-4? I'll retrieve whichever you confirm."
    ),
])

# --------------------------------------------------------------------------- #
# Assembly + validation.
# --------------------------------------------------------------------------- #

VALID_TOOL_NAMES = {t["function"]["name"] for t in TOOLS}


def to_record(messages):
    return {"messages": [sys_msg(), *messages], "tools": TOOLS, "parallel_tool_calls": False}


def validate(record):
    msgs = record["messages"]
    assert msgs[0]["role"] == "system", "first message must be system"
    open_calls: set[str] = set()
    seen_ids: set[str] = set()
    for m in msgs:
        role = m["role"]
        if role == "assistant" and m.get("tool_calls"):
            for tc in m["tool_calls"]:
                cid = tc["id"]
                assert cid not in seen_ids, f"duplicate tool_call id {cid}"
                seen_ids.add(cid)
                name = tc["function"]["name"]
                assert name in VALID_TOOL_NAMES, f"unknown tool {name}"
                # arguments must be a JSON string holding {"code": <json string>}
                args = json.loads(tc["function"]["arguments"])
                assert set(args.keys()) == {"code"}, f"args must be just 'code', got {args.keys()}"
                code_val = json.loads(args["code"])  # code must itself be valid JSON
                if name == "execute":
                    assert isinstance(code_val, (dict, list)), "execute code must be object or array"
                    ops = code_val if isinstance(code_val, list) else [code_val]
                    for o in ops:
                        assert o.get("op") in {"abstract", "download", "citations", "recs", "retrieve"}, o
                        assert "id" in o, "execute op needs id"
                else:
                    assert isinstance(code_val, dict), f"{name} code must be an object"
                    if name in {"search", "hdrr"}:
                        assert "q" in code_val, f"{name} needs q"
                    if name == "retrieve_paper":
                        assert "paper_id" in code_val, "retrieve_paper needs paper_id"
                open_calls.add(cid)
        elif role == "tool":
            cid = m["tool_call_id"]
            assert cid in open_calls, f"tool result {cid} has no matching call"
            open_calls.discard(cid)
            json_or_str = m["content"]
            # content is either raw text (download) or JSON; if it looks like JSON, it must parse
            stripped = json_or_str.lstrip()
            if stripped[:1] in "{[":
                json.loads(json_or_str)
    assert not open_calls, f"tool calls without results: {open_calls}"
    # the conversation must end on an assistant message
    assert msgs[-1]["role"] == "assistant", "conversation must end with assistant"


def main():
    records = [to_record(conv) for conv in EXAMPLES]
    for i, rec in enumerate(records):
        try:
            validate(rec)
        except AssertionError as exc:  # pragma: no cover - surfaced at build time
            raise SystemExit(f"example {i} failed validation: {exc}") from exc

    lines = [json.dumps(rec, ensure_ascii=False) for rec in records]
    # Every line must independently round-trip as JSON.
    for i, line in enumerate(lines):
        json.loads(line)
        assert "\n" not in line, f"example {i} contains a newline in the serialized line"

    OUT_PATH.write_text("\n".join(lines) + "\n", encoding="utf-8")

    tool_counts: dict[str, int] = {}
    turn_total = 0
    for conv in EXAMPLES:
        turn_total += len(conv)
        for m in conv:
            for tc in m.get("tool_calls", []):
                name = tc["function"]["name"]
                tool_counts[name] = tool_counts.get(name, 0) + 1

    print(f"wrote {len(records)} examples -> {OUT_PATH}")
    print(f"  user/assistant/tool turns (excl. system): {turn_total}")
    print(f"  tool-call distribution: {dict(sorted(tool_counts.items()))}")


if __name__ == "__main__":
    main()
