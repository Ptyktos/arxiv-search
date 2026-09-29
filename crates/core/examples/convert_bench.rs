//! Offline conversion benchmark: times the pure content-prep pipeline
//! (HTML/PDF -> markdown -> prune -> chunk) on a local file, with no network.
//!
//! Used by `benchmarks/arxiv/bench.py convert` to compare conversion cost against
//! incumbents on byte-identical inputs. See `benchmarks/README.md`.
//!
//! Usage: `convert_bench <input> <html|pdf> <iterations> <text_out>`
//!
//! Prints one JSON object to stdout and writes the pruned markdown (the text an
//! agent would read) to `<text_out>`, plus the unpruned markdown to
//! `<text_out>.raw`, so the harness can tokenize both and attribute savings.

use std::io::Write as _;
use std::time::Instant;

use arxiv_search_rs_mcp_core::content::{prepare_paper, PreparationOptions};
use arxiv_search_rs_mcp_core::{html, pdf, Paper};

const fn placeholder_paper() -> Paper {
    Paper {
        id: String::new(),
        title: String::new(),
        authors: Vec::new(),
        abstract_text: String::new(),
        categories: Vec::new(),
        published: String::new(),
        url: String::new(),
        doi: None,
        journal_ref: None,
    }
}

fn convert(bytes: &[u8], kind: &str) -> Result<String, String> {
    match kind {
        "html" => html::to_markdown(&String::from_utf8_lossy(bytes)).map_err(|e| e.to_string()),
        "pdf" => pdf::extract_text(bytes).map_err(|e| e.to_string()),
        other => Err(format!("unknown kind {other:?}; expected html or pdf")),
    }
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let [_, input, kind, iters, text_out] = args.as_slice() else {
        return Err("usage: convert_bench <input> <html|pdf> <iterations> <text_out>".into());
    };
    let iters: usize = iters.parse().map_err(|e| format!("iterations: {e}"))?;
    let bytes = std::fs::read(input).map_err(|e| format!("read {input}: {e}"))?;

    let mut samples_ms = Vec::with_capacity(iters);
    let mut last = None;
    for _ in 0..iters.max(1) {
        let start = Instant::now();
        let markdown = convert(&bytes, kind)?;
        let prepared = prepare_paper(
            placeholder_paper(),
            kind.as_str(),
            markdown,
            PreparationOptions::default(),
        );
        samples_ms.push(start.elapsed().as_secs_f64() * 1_000.0);
        last = Some(prepared);
    }
    let prepared = last.ok_or("no iterations ran")?;
    std::fs::write(text_out, &prepared.pruned_markdown)
        .map_err(|e| format!("write {text_out}: {e}"))?;
    std::fs::write(format!("{text_out}.raw"), &prepared.raw_markdown)
        .map_err(|e| format!("write {text_out}.raw: {e}"))?;

    let out = serde_json::json!({
        "samples_ms": samples_ms,
        "input_bytes": bytes.len(),
        "raw_chars": prepared.raw_markdown.chars().count(),
        "pruned_chars": prepared.pruned_markdown.chars().count(),
        "chunks": prepared.chunks.len(),
    });
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{out}").map_err(|e| e.to_string())
}
