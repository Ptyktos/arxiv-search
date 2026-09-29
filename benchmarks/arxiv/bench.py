#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = [
#   "mcp==1.30.0",
#   "httpx==0.28.1",
#   "tiktoken==0.14.0",
# ]
# ///
"""Reproducible benchmark: arxiv-search vs. incumbents for "paper -> text an agent can use".

Scenarios (see benchmarks/README.md for methodology and the claims policy):

  retrieve  End-to-end over MCP stdio: time from `tools/call` sent to the full
            response received, cold (empty cache, fresh process) and warm
            (same process, second call). Also records server startup, peak RSS,
            and the size of what lands in the agent's context.
  search    `search` latency for the same queries against both MCP servers.
  convert   Offline: HTML/PDF -> text on byte-identical local inputs, no network.

Usage:
  uv run benchmarks/arxiv/bench.py setup            # build + install pinned incumbent + fetch fixtures
  uv run benchmarks/arxiv/bench.py run              # all scenarios -> results/<stamp>.{json,md}
  uv run benchmarks/arxiv/bench.py run --scenarios convert
  uv run benchmarks/arxiv/bench.py report results/<file>.json
"""

from __future__ import annotations

import argparse
import asyncio
import datetime as dt
import json
import os
import platform
import re
import shutil
import signal
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
CACHE = HERE.parent / ".cache"
FIXTURES = CACHE / "fixtures"
INCUMBENT_VENV = CACHE / "incumbent-venv"
RESULTS = HERE / "results"
CORPUS = json.loads((HERE / "corpus.json").read_text())

OUR_BIN = REPO / "target" / "release" / "arxiv-search-mcp"
OUR_CONVERT = REPO / "target" / "release" / "examples" / "convert_bench"
UA = "arxiv-search-benchmark/1 (+https://github.com/Ptyktos/arxiv-search)"


def log(msg: str) -> None:
    print(msg, file=sys.stderr, flush=True)


def incumbent_python() -> Path:
    return INCUMBENT_VENV / ("Scripts/python.exe" if os.name == "nt" else "bin/python")


# --------------------------------------------------------------------------- metrics


_ENC = None


def tokens(text: str) -> int:
    """Token count with tiktoken o200k_base: a documented *proxy*, not any model's exact tokenizer."""
    global _ENC
    if _ENC is None:
        import tiktoken

        _ENC = tiktoken.get_encoding(CORPUS["tokenizer"])
    return len(_ENC.encode(text, disallowed_special=()))


_WORD = re.compile(r"[a-z]{4,}")


def abstract_recall(text: str, abstract: str) -> float | None:
    """Fraction of distinct 4+ letter abstract words present in `text`.

    A sanity floor that catches "fast because it returned nothing"; it is not a
    quality score for the whole paper.
    """
    want = set(_WORD.findall(abstract.lower()))
    if not want:
        return None
    have = set(_WORD.findall(text.lower()))
    return round(len(want & have) / len(want), 3)


def summarize(xs: list[float]) -> dict[str, float] | None:
    xs = [x for x in xs if x is not None]
    if not xs:
        return None
    return {
        "median": round(statistics.median(xs), 1),
        "min": round(min(xs), 1),
        "max": round(max(xs), 1),
        "n": len(xs),
    }


# --------------------------------------------------------------------------- rss wrapper


def rsswrap(out: str, cmd: list[str]) -> int:
    """Run `cmd` with inherited stdio; on exit, record its peak RSS to `out`.

    Uses getrusage(RUSAGE_CHILDREN).ru_maxrss, so it measures the server process
    itself (and any children it waited on), not this wrapper.
    """
    import resource

    proc = subprocess.Popen(cmd)
    for sig in (signal.SIGTERM, signal.SIGINT):
        signal.signal(sig, lambda s, _f: proc.send_signal(s))
    rc = proc.wait()
    ru = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
    kib = ru / 1024 if sys.platform == "darwin" else ru  # macOS reports bytes
    Path(out).write_text(json.dumps({"max_rss_mib": round(kib / 1024, 1), "rc": rc}))
    return 0


# --------------------------------------------------------------------------- arms


class McpArm:
    """One MCP server under test, launched fresh per cold run."""

    name: str
    retrieve_tool: str
    search_tool: str

    def command(self, state_dir: Path) -> tuple[list[str], dict[str, str]]:
        raise NotImplementedError

    def retrieve_args(self, paper_id: str) -> dict[str, Any]:
        raise NotImplementedError

    def search_args(self, q: str, n: int) -> dict[str, Any]:
        raise NotImplementedError

    def content(self, response: str) -> str:
        """The paper text inside the response (vs. the whole response payload)."""
        raise NotImplementedError

    def source(self, response: str) -> str | None:
        try:
            return json.loads(response).get("source")
        except (json.JSONDecodeError, AttributeError):
            return None


class ArxivSearchArm(McpArm):
    name = "arxiv-search"
    retrieve_tool = "retrieve_paper"
    search_tool = "search"

    def command(self, state_dir):
        # The cache/DB live under the OS cache dir; point HOME/XDG at a throwaway
        # dir so every cold run starts empty and never touches the user's cache.
        env = {"HOME": str(state_dir), "XDG_CACHE_HOME": str(state_dir / "cache"), "RUST_LOG": "error"}
        return [str(OUR_BIN), "--stdio"], env

    def retrieve_args(self, paper_id):
        return {"paper_id": paper_id}  # documented defaults

    def search_args(self, q, n):
        return {"q": q, "n": n}

    def content(self, response):
        try:
            return json.loads(response).get("pruned_markdown", "")
        except json.JSONDecodeError:
            return ""


class IncumbentArm(McpArm):
    name = "arxiv-mcp-server"
    retrieve_tool = "download_paper"
    search_tool = "search_papers"

    def command(self, state_dir):
        env = {"HOME": str(state_dir)}
        return [str(incumbent_python()), "-m", "arxiv_mcp_server", "--storage-path", str(state_dir / "papers")], env

    def retrieve_args(self, paper_id):
        # Without return_full_text the incumbent caps a response at 12k chars and
        # expects pagination; request the whole paper so both arms return full text.
        return {"paper_id": paper_id, "return_full_text": True}

    def search_args(self, q, n):
        # Default is ~280-char abstract snippets; request full abstracts so both
        # servers return the same content and response tokens are comparable.
        return {"query": q, "max_results": n, "abstract_mode": "full"}

    def content(self, response):
        try:
            return json.loads(response).get("content", "")
        except json.JSONDecodeError:
            return ""


ARMS: dict[str, McpArm] = {a.name: a for a in (ArxivSearchArm(), IncumbentArm())}


def server_params(arm: McpArm, state_dir: Path):
    """Launch parameters for a fresh server wrapped by `_rsswrap`, plus its RSS output file."""
    from mcp import StdioServerParameters

    cmd, extra_env = arm.command(state_dir)
    rss_file = state_dir / "rss.json"
    params = StdioServerParameters(
        command=sys.executable,
        args=[str(Path(__file__).resolve()), "_rsswrap", str(rss_file), *cmd],
        # Inherit the caller env (proxy / CA settings) and override only state paths.
        env={**os.environ, **extra_env},
    )
    return params, rss_file


def response_text(result) -> str:
    return "".join(getattr(block, "text", "") for block in result.content)


async def run_mcp_retrieve(arm: McpArm, paper: dict, warm_reps: int) -> dict:
    from mcp import ClientSession
    from mcp.client.stdio import stdio_client

    state_dir = Path(tempfile.mkdtemp(prefix=f"bench-{arm.name}-"))
    params, rss_file = server_params(arm, state_dir)
    rec: dict[str, Any] = {"arm": arm.name, "paper": paper["id"]}
    errlog = open(state_dir / "stderr.log", "w")
    try:
        t0 = time.perf_counter()
        async with stdio_client(params, errlog=errlog) as (r, w), ClientSession(r, w) as session:
            await session.initialize()
            rec["startup_ms"] = (time.perf_counter() - t0) * 1e3

            t = time.perf_counter()
            res = await session.call_tool(arm.retrieve_tool, arm.retrieve_args(paper["id"]))
            rec["cold_ms"] = (time.perf_counter() - t) * 1e3
            text = response_text(res)
            rec["is_error"] = bool(res.isError)

            warm = []
            for _ in range(warm_reps):
                t = time.perf_counter()
                await session.call_tool(arm.retrieve_tool, arm.retrieve_args(paper["id"]))
                warm.append((time.perf_counter() - t) * 1e3)
            rec["warm_ms"] = warm
    except Exception as e:  # record, keep going
        rec["error"] = f"{type(e).__name__}: {e} (stderr: {(state_dir / 'stderr.log').read_text()[-300:]})"
        errlog.close()
        shutil.rmtree(state_dir, ignore_errors=True)
        return rec
    finally:
        errlog.close()

    for _ in range(50):  # wrapper writes after the server exits
        if rss_file.exists():
            break
        await asyncio.sleep(0.1)
    if rss_file.exists():
        rec["peak_rss_mib"] = json.loads(rss_file.read_text())["max_rss_mib"]

    content = arm.content(text)
    rec |= {
        "source": arm.source(text),
        "response_chars": len(text),
        "response_tokens": tokens(text),
        "content_chars": len(content),
        "content_tokens": tokens(content),
        "abstract_recall": abstract_recall(content, paper_abstract(paper["id"])),
    }
    if rec["is_error"] or not content:
        rec["error"] = rec.get("error") or text[:300]
    shutil.rmtree(state_dir, ignore_errors=True)
    return rec


def run_raw_fetch(paper: dict) -> dict:
    """Baseline: what a generic "fetch this URL" tool hands the model, unprocessed."""
    import httpx

    rec: dict[str, Any] = {"arm": "raw-fetch", "paper": paper["id"]}
    with httpx.Client(follow_redirects=True, timeout=60, headers={"User-Agent": UA}) as c:
        t = time.perf_counter()
        r = c.get(f"https://arxiv.org/html/{paper['id']}")
        source = "html"
        if r.status_code == 404:
            r = c.get(f"https://arxiv.org/pdf/{paper['id']}")
            source = "pdf"
        rec["cold_ms"] = (time.perf_counter() - t) * 1e3
    rec["source"] = source
    rec["response_bytes"] = len(r.content)
    if source == "html":
        rec["response_chars"] = len(r.text)
        rec["response_tokens"] = tokens(r.text)
        rec["abstract_recall"] = abstract_recall(r.text, paper_abstract(paper["id"]))
    # A raw PDF is not text: reading it requires a model with PDF/vision input,
    # billed per page by the provider. We report bytes only and do not invent a token count.
    return rec


def run_jina(paper: dict) -> dict:
    """Optional baseline: Jina Reader (r.jina.ai), a hosted URL -> markdown service."""
    import httpx

    src = "pdf" if paper.get("pdf_only") else "html"
    url = f"https://r.jina.ai/https://arxiv.org/{src}/{paper['id']}"
    headers = {"User-Agent": UA}  # service defaults; its output varies between runs
    if key := os.environ.get("JINA_API_KEY"):
        headers["Authorization"] = f"Bearer {key}"
    rec: dict[str, Any] = {"arm": "jina-reader", "paper": paper["id"], "source": src}
    with httpx.Client(timeout=120, headers=headers) as c:
        t = time.perf_counter()
        r = c.get(url)
        rec["cold_ms"] = (time.perf_counter() - t) * 1e3
    if r.status_code != 200:
        rec["error"] = f"HTTP {r.status_code}: {r.text[:200]}"
        return rec
    rec |= {
        "response_chars": len(r.text),
        "response_tokens": tokens(r.text),
        "content_tokens": tokens(r.text),
        "abstract_recall": abstract_recall(r.text, paper_abstract(paper["id"])),
    }
    return rec


async def scenario_retrieve(args) -> list[dict]:
    rows = []
    papers = CORPUS["papers"]
    for rep in range(args.cold_reps):
        for paper in papers:
            # Alternate arm order each rep so neither consistently goes first
            # against a just-warmed arXiv edge cache.
            order = list(ARMS.values())
            if rep % 2:
                order.reverse()
            for arm in order:
                log(f"[retrieve] rep {rep + 1}/{args.cold_reps} {paper['id']} {arm.name}")
                row = await run_mcp_retrieve(arm, paper, args.warm_reps)
                row["rep"] = rep
                rows.append(row)
                time.sleep(args.pause)
            log(f"[retrieve] rep {rep + 1}/{args.cold_reps} {paper['id']} raw-fetch")
            rows.append(run_raw_fetch(paper) | {"rep": rep})
            time.sleep(args.pause)
            if args.jina and rep == 0:
                log(f"[retrieve] {paper['id']} jina-reader")
                rows.append(run_jina(paper) | {"rep": rep})
                time.sleep(args.pause)
    return rows


async def scenario_search(args) -> list[dict]:
    from mcp import ClientSession
    from mcp.client.stdio import stdio_client

    rows = []
    for arm in ARMS.values():
        state_dir = Path(tempfile.mkdtemp(prefix=f"bench-search-{arm.name}-"))
        params, _rss = server_params(arm, state_dir)
        errlog = open(state_dir / "stderr.log", "w")
        async with stdio_client(params, errlog=errlog) as (r, w), ClientSession(r, w) as session:
            await session.initialize()
            for rep in range(args.search_reps):
                for q in CORPUS["queries"]:
                    log(f"[search] {arm.name} rep {rep + 1} {q['q']!r}")
                    t = time.perf_counter()
                    res = await session.call_tool(arm.search_tool, arm.search_args(q["q"], q["n"]))
                    ms = (time.perf_counter() - t) * 1e3
                    text = response_text(res)
                    rows.append({
                        "arm": arm.name, "query": q["q"], "rep": rep, "ms": ms,
                        "is_error": bool(res.isError), "response_tokens": tokens(text),
                    })
                    # Spacing >= arXiv's 3 s guidance also keeps the incumbent's
                    # built-in 3 s limiter from adding sleep to its measured time.
                    time.sleep(args.pause)
        errlog.close()
        shutil.rmtree(state_dir, ignore_errors=True)
    return rows


INCUMBENT_CONVERT = r"""
import json, sys, time
kind, path, iters, out = sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4]
samples = []
if kind == "html":
    from arxiv_mcp_server.tools.download import _html_to_text
    html = open(path, encoding="utf-8", errors="replace").read()
    for _ in range(iters):
        t = time.perf_counter(); text = _html_to_text(html); samples.append((time.perf_counter() - t) * 1e3)
else:
    import pymupdf4llm, fitz
    fitz.TOOLS.mupdf_display_errors(False)
    for _ in range(iters):
        t = time.perf_counter(); text = pymupdf4llm.to_markdown(path, show_progress=False); samples.append((time.perf_counter() - t) * 1e3)
open(out, "w", encoding="utf-8").write(text)
print(json.dumps({"samples_ms": samples}))
"""


def scenario_convert(args) -> list[dict]:
    rows = []
    for paper in CORPUS["papers"]:
        kind = "pdf" if paper.get("pdf_only") else "html"
        fixture = FIXTURES / f"{paper['id']}.{kind}"
        abstract = paper_abstract(paper["id"])
        for arm, cmd in (
            ("arxiv-search", [str(OUR_CONVERT), str(fixture), kind, str(args.convert_iters)]),
            ("arxiv-mcp-server", [str(incumbent_python()), "-c", INCUMBENT_CONVERT, kind, str(fixture), str(args.convert_iters)]),
        ):
            log(f"[convert] {paper['id']} ({kind}) {arm}")
            with tempfile.NamedTemporaryFile(suffix=".md", delete=False) as tf:
                out_path = Path(tf.name)
            p = subprocess.run([*cmd, str(out_path)], capture_output=True, text=True)
            if p.returncode != 0:
                rows.append({"arm": arm, "paper": paper["id"], "kind": kind, "error": p.stderr[-300:]})
                continue
            stats = json.loads(p.stdout.strip().splitlines()[-1])
            text = out_path.read_text(encoding="utf-8", errors="replace")
            raw_path = Path(f"{out_path}.raw")
            # arxiv-search prunes references by default; the incumbent does not.
            # Report our unpruned size too so the saving can be attributed.
            unpruned = raw_path.read_text(encoding="utf-8", errors="replace") if raw_path.exists() else None
            out_path.unlink(missing_ok=True)
            raw_path.unlink(missing_ok=True)
            rows.append({
                "arm": arm, "paper": paper["id"], "kind": kind,
                "input_bytes": fixture.stat().st_size,
                "samples_ms": stats["samples_ms"],
                "output_chars": len(text),
                "output_tokens": tokens(text),
                "unpruned_tokens": tokens(unpruned) if unpruned is not None else None,
                "abstract_recall": abstract_recall(text, abstract),
            })
    return rows


# --------------------------------------------------------------------------- setup


def paper_abstract(paper_id: str) -> str:
    return json.loads((FIXTURES / f"{paper_id}.meta.json").read_text())["abstract"]


def cmd_setup(args) -> None:
    if not args.no_build:
        log("[setup] building arxiv-search (release)")
        subprocess.run(["cargo", "build", "--release", "-p", "arxiv-search-rs-mcp", "--bin", "arxiv-search-mcp"], cwd=REPO, check=True)
        subprocess.run(["cargo", "build", "--release", "-p", "arxiv-search-rs-mcp-core", "--example", "convert_bench"], cwd=REPO, check=True)

    inc = CORPUS["incumbent"]
    log(f"[setup] installing {inc['requirement']} (frozen: incumbent.lock.txt) into {INCUMBENT_VENV}")
    if not incumbent_python().exists():
        subprocess.run(["uv", "venv", "--python", inc["python"], str(INCUMBENT_VENV)], check=True)
    # Install from the frozen lock (every transitive dep pinned) so reruns
    # measure the same incumbent code, not whatever resolves today.
    subprocess.run(["uv", "pip", "sync", "--python", str(incumbent_python()), str(HERE / "incumbent.lock.txt")], check=True)

    import httpx
    import xml.etree.ElementTree as ET

    FIXTURES.mkdir(parents=True, exist_ok=True)
    ns = {"a": "http://www.w3.org/2005/Atom"}
    with httpx.Client(follow_redirects=True, timeout=60, headers={"User-Agent": UA}) as c:
        for paper in CORPUS["papers"]:
            pid = paper["id"]
            meta = FIXTURES / f"{pid}.meta.json"
            if not meta.exists():
                r = c.get("https://export.arxiv.org/api/query", params={"id_list": pid})
                r.raise_for_status()
                entry = ET.fromstring(r.text).find("a:entry", ns)
                meta.write_text(json.dumps({
                    "title": " ".join(entry.findtext("a:title", "", ns).split()),
                    "abstract": " ".join(entry.findtext("a:summary", "", ns).split()),
                }))
                time.sleep(3)
            kind = "pdf" if paper.get("pdf_only") else "html"
            fixture = FIXTURES / f"{pid}.{kind}"
            if not fixture.exists():
                log(f"[setup] fixture {pid}.{kind}")
                r = c.get(f"https://arxiv.org/{kind}/{pid}")
                r.raise_for_status()
                fixture.write_bytes(r.content)
                time.sleep(3)
    tokens("warm up tokenizer download")
    log("[setup] done")


# --------------------------------------------------------------------------- env + report


def sh(cmd: list[str], cwd: Path | None = None) -> str:
    try:
        return subprocess.run(cmd, capture_output=True, text=True, cwd=cwd, check=True).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return "unknown"


def environment() -> dict:
    cpu = platform.processor() or "unknown"
    if Path("/proc/cpuinfo").exists():
        m = re.search(r"model name\s*:\s*(.+)", Path("/proc/cpuinfo").read_text())
        cpu = m.group(1).strip() if m else cpu
    elif sys.platform == "darwin":
        cpu = sh(["sysctl", "-n", "machdep.cpu.brand_string"])
    mem = None
    if Path("/proc/meminfo").exists():
        m = re.search(r"MemTotal:\s*(\d+)", Path("/proc/meminfo").read_text())
        mem = round(int(m.group(1)) / 1024 / 1024, 1) if m else None
    return {
        "date_utc": dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds"),
        "os": platform.platform(),
        "cpu": cpu,
        "cpus": os.cpu_count(),
        "mem_gib": mem,
        "python": platform.python_version(),
        "rustc": sh(["rustc", "--version"]),
        "git_sha": sh(["git", "rev-parse", "--short", "HEAD"], REPO),
        "git_dirty": sh(["git", "status", "--porcelain"], REPO) != "",
        "incumbent": CORPUS["incumbent"]["requirement"],
        "incumbent_resolved": sh([str(incumbent_python()), "-c", "import importlib.metadata as m;print(m.version('arxiv-mcp-server'))"]),
        "tokenizer": f"tiktoken {CORPUS['tokenizer']}",
        "network_note": os.environ.get("BENCH_NETWORK_NOTE", "unspecified"),
    }


def fmt(s: dict | None, unit: str = "") -> str:
    if not s:
        return "—"
    if s["n"] == 1:
        return f"{s['median']:,.0f}{unit}"
    return f"{s['median']:,.0f}{unit} ({s['min']:,.0f}–{s['max']:,.0f})"


def by(rows, *keys):
    out: dict[tuple, list] = {}
    for r in rows:
        out.setdefault(tuple(r.get(k) for k in keys), []).append(r)
    return out


def report(data: dict) -> str:
    env, cfg = data["environment"], data["config"]
    L = [
        f"# arXiv benchmark results — {env['date_utc']}",
        "",
        "Generated by `benchmarks/arxiv/bench.py`. Methodology and claims policy: `benchmarks/README.md`.",
        "",
        "## Setup",
        "",
        "| | |",
        "|---|---|",
        *(f"| {k} | {v} |" for k, v in env.items()),
        f"| config | {json.dumps(cfg)} |",
        "",
    ]
    arm_order = ["arxiv-search", "arxiv-mcp-server", "raw-fetch", "jina-reader"]

    if rows := data["results"].get("retrieve"):
        ok = [r for r in rows if "error" not in r]
        errs = [r for r in rows if "error" in r]
        L += [
            "## retrieve — paper ID → full text returned to the agent (end-to-end, over MCP stdio)",
            "",
            "Median (min–max) across cold reps. **Cold** = fresh process + empty cache, so it includes arXiv network time.",
            "**Warm** = same process, repeat call served from the tool's local cache. **Response tokens** = everything",
            "the tool returns into context; **content tokens** = the paper text inside that response.",
            "",
            "| paper | tool | source | cold ms | warm ms | response tokens | content tokens | abstract recall | peak RSS MiB |",
            "|---|---|---|---:|---:|---:|---:|---:|---:|",
        ]
        groups = by(ok, "paper", "arm")
        for paper in [p["id"] for p in CORPUS["papers"]]:
            for arm in arm_order:
                g = groups.get((paper, arm))
                if not g:
                    continue
                warm = [x for r in g for x in r.get("warm_ms", [])]
                L.append("| " + " | ".join([
                    paper, arm, g[0].get("source") or "—",
                    fmt(summarize([r["cold_ms"] for r in g])),
                    fmt(summarize(warm)),
                    fmt(summarize([r.get("response_tokens") for r in g])),
                    fmt(summarize([r.get("content_tokens") for r in g])),
                    str(g[0].get("abstract_recall", "—")),
                    fmt(summarize([r.get("peak_rss_mib") for r in g])),
                ]) + " |")
        # Sum only over papers every arm completed, so a failure never makes an arm's total look smaller.
        arms_present = [a for a in arm_order if any(r["arm"] == a for r in ok)]
        common = set.intersection(*({r["paper"] for r in ok if r["arm"] == a} for a in arms_present))
        L += ["", f"### retrieve — totals over the {len(common)} papers every tool completed (per-paper median, summed)", "",
              "| tool | papers ok | Σ cold ms | Σ warm ms | Σ response tokens | Σ content tokens | startup ms | max peak RSS MiB |",
              "|---|---:|---:|---:|---:|---:|---:|---:|"]
        for arm in arm_order:
            g = [r for r in ok if r["arm"] == arm and r["paper"] in common]
            if not g:
                continue
            per_paper = by(g, "paper")
            def total(key, agg=statistics.median):
                vals = [agg([r[key] for r in rs if r.get(key) is not None] or [0]) for rs in per_paper.values()]
                return f"{sum(vals):,.0f}"
            warm_total = sum(statistics.median([x for r in rs for x in r.get("warm_ms", [])] or [0]) for rs in per_paper.values())
            rss = [r["peak_rss_mib"] for r in g if r.get("peak_rss_mib")]
            n_ok = len({r["paper"] for r in ok if r["arm"] == arm})
            L.append(f"| {arm} | {n_ok}/{len(CORPUS['papers'])} | {total('cold_ms')} | "
                     f"{f'{warm_total:,.0f}' if warm_total else '—'} | {total('response_tokens')} | "
                     f"{total('content_tokens') if any(r.get('content_tokens') for r in g) else '—'} | "
                     f"{fmt(summarize([r.get('startup_ms') for r in g]))} | {max(rss) if rss else '—'} |")
        if errs:
            L += ["", "### retrieve — failures", ""]
            L += [f"- `{r['arm']}` `{r['paper']}` rep {r.get('rep')}: {r['error'][:200]!s}" for r in errs]
        L.append("")

    if rows := data["results"].get("search"):
        L += ["## search — same queries, both MCP servers (warm process)", "",
              "| query | tool | ms | response tokens | errors |", "|---|---|---:|---:|---:|"]
        for (q, arm), g in by(rows, "query", "arm").items():
            L.append(f"| `{q}` | {arm} | {fmt(summarize([r['ms'] for r in g]))} | "
                     f"{fmt(summarize([r['response_tokens'] for r in g]))} | {sum(r['is_error'] for r in g)} |")
        L.append("")

    if rows := data["results"].get("convert"):
        L += ["## convert — offline HTML/PDF → text on identical local bytes (no network)", "",
              "Median (min–max) wall time per conversion inside each tool's own runtime (process start excluded).", "",
              "| paper | input | tool | ms | output tokens | without reference pruning | abstract recall |", "|---|---|---|---:|---:|---:|---:|"]
        for r in rows:
            if "error" in r:
                L.append(f"| {r['paper']} | {r['kind']} | {r['arm']} | error | — | — | — |")
                continue
            L.append(f"| {r['paper']} | {r['kind']} {r['input_bytes'] / 1024:,.0f} KiB | {r['arm']} | "
                     f"{fmt(summarize(r['samples_ms']))} | {r['output_tokens']:,} | "
                     f"{format(r['unpruned_tokens'], ',') if r.get('unpruned_tokens') is not None else 'n/a (never prunes)'} | {r['abstract_recall']} |")
        L.append("")
    return "\n".join(L)


async def cmd_run(args) -> None:
    missing = [p for p in (OUR_BIN, OUR_CONVERT, incumbent_python()) if not p.exists()]
    if missing:
        sys.exit(f"missing {', '.join(map(str, missing))}; run `bench.py setup` first")
    scenarios = args.scenarios.split(",")
    if args.papers:
        wanted = set(args.papers.split(","))
        CORPUS["papers"] = [p for p in CORPUS["papers"] if p["id"] in wanted]
    results: dict[str, list] = {}
    if "convert" in scenarios:
        results["convert"] = scenario_convert(args)
    if "search" in scenarios:
        results["search"] = await scenario_search(args)
    if "retrieve" in scenarios:
        results["retrieve"] = await scenario_retrieve(args)
    data = {
        "environment": environment(),
        "config": {k: v for k, v in vars(args).items() if k not in {"func"}},
        "corpus": CORPUS,
        "results": results,
    }
    RESULTS.mkdir(parents=True, exist_ok=True)
    stamp = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    out = RESULTS / f"{stamp}-{args.label}.json"
    out.write_text(json.dumps(data, indent=1))
    out.with_suffix(".md").write_text(report(data))
    log(f"wrote {out} and {out.with_suffix('.md')}")


def main() -> None:
    if len(sys.argv) > 2 and sys.argv[1] == "_rsswrap":
        sys.exit(rsswrap(sys.argv[2], sys.argv[3:]))

    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(required=True)
    s = sub.add_parser("setup", help="build, install pinned incumbent, fetch fixtures")
    s.add_argument("--no-build", action="store_true")
    s.set_defaults(func=cmd_setup)

    r = sub.add_parser("run", help="run scenarios and write results/")
    r.add_argument("--scenarios", default="convert,search,retrieve")
    r.add_argument("--cold-reps", type=int, default=3)
    r.add_argument("--warm-reps", type=int, default=5)
    r.add_argument("--search-reps", type=int, default=3)
    r.add_argument("--convert-iters", type=int, default=10)
    r.add_argument("--pause", type=float, default=3.5, help="seconds between network calls (arXiv asks for >= 3)")
    r.add_argument("--papers", help="comma-separated subset of corpus IDs (for smoke tests)")
    r.add_argument("--jina", action="store_true", help="also run the Jina Reader baseline (external service)")
    r.add_argument("--label", default=platform.system().lower())
    r.set_defaults(func=lambda a: asyncio.run(cmd_run(a)))

    p = sub.add_parser("report", help="re-render markdown from a results JSON")
    p.add_argument("json", type=Path)
    p.set_defaults(func=lambda a: print(report(json.loads(a.json.read_text()))))

    args = ap.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
