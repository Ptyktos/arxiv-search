use serde::{Deserialize, Serialize};

use crate::paper::Paper;

const DEFAULT_CHUNK_CHARS: usize = 4_000;
const DEFAULT_CHUNK_OVERLAP: usize = 200;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PreparationOptions {
    pub prune_references: bool,
    pub chunk_chars: usize,
    pub chunk_overlap: usize,
}

impl Default for PreparationOptions {
    fn default() -> Self {
        Self {
            prune_references: true,
            chunk_chars: DEFAULT_CHUNK_CHARS,
            chunk_overlap: DEFAULT_CHUNK_OVERLAP,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PaperChunk {
    pub index: usize,
    pub start_char: usize,
    pub end_char: usize,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PreparedPaper {
    pub paper: Paper,
    pub source: String,
    pub raw_markdown: String,
    pub pruned_markdown: String,
    pub chunks: Vec<PaperChunk>,
}

#[must_use]
pub fn prepare_paper(
    paper: Paper,
    source: impl Into<String>,
    markdown: impl AsRef<str>,
    options: PreparationOptions,
) -> PreparedPaper {
    let raw_markdown = normalize_markdown(markdown.as_ref());
    let pruned_markdown = prune_markdown(&raw_markdown, options.prune_references);
    let chunks = chunk_text(
        &pruned_markdown,
        options.chunk_chars.max(1_000),
        options
            .chunk_overlap
            .min(options.chunk_chars.saturating_sub(1)),
    );

    PreparedPaper {
        paper,
        source: source.into(),
        raw_markdown,
        pruned_markdown,
        chunks,
    }
}

#[must_use]
pub fn normalize_markdown(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut previous_blank = false;

    for line in text.replace("\r\n", "\n").lines() {
        let trimmed_end = line.trim_end();
        let blank = trimmed_end.trim().is_empty();
        if blank {
            if previous_blank {
                continue;
            }
            previous_blank = true;
            out.push('\n');
            continue;
        }
        previous_blank = false;
        out.push_str(trimmed_end);
        out.push('\n');
    }

    out.trim().to_string()
}

#[must_use]
pub fn prune_markdown(text: &str, prune_references: bool) -> String {
    let mut lines = Vec::new();
    let mut skipping_references = false;

    for line in text.lines() {
        let trimmed = line.trim();

        if prune_references && is_reference_heading(trimmed) {
            skipping_references = true;
            continue;
        }

        if skipping_references {
            continue;
        }

        if is_noise_line(trimmed) {
            continue;
        }

        lines.push(line.trim_end().to_string());
    }

    collapse_blank_lines(lines.join("\n").trim())
}

#[must_use]
pub fn chunk_text(text: &str, chunk_chars: usize, chunk_overlap: usize) -> Vec<PaperChunk> {
    if text.trim().is_empty() {
        return Vec::new();
    }

    let paragraphs = split_paragraphs(text);
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut current_start = 0usize;
    let mut cursor = 0usize;

    for paragraph in paragraphs {
        let paragraph_len = paragraph.chars().count();
        if paragraph_len > chunk_chars {
            if !current.trim().is_empty() {
                push_chunk(&mut chunks, &current, current_start);
                current.clear();
            }
            for part in split_long_paragraph(&paragraph, chunk_chars) {
                let start = cursor;
                let end = start + part.chars().count();
                chunks.push(PaperChunk {
                    index: chunks.len(),
                    start_char: start,
                    end_char: end,
                    text: part.clone(),
                });
                cursor = end.saturating_sub(chunk_overlap.min(end));
            }
            continue;
        }

        let candidate = if current.is_empty() {
            paragraph.clone()
        } else {
            format!("{current}\n\n{paragraph}")
        };

        if candidate.chars().count() > chunk_chars && !current.is_empty() {
            push_chunk(&mut chunks, &current, current_start);
            cursor = current_start + current.chars().count();
            current = paragraph;
            current_start = cursor;
        } else {
            if current.is_empty() {
                current_start = cursor;
            }
            current = candidate;
        }
    }

    if !current.trim().is_empty() {
        push_chunk(&mut chunks, &current, current_start);
    }

    chunks
        .into_iter()
        .enumerate()
        .map(|(index, mut chunk)| {
            chunk.index = index;
            chunk
        })
        .collect()
}

fn push_chunk(chunks: &mut Vec<PaperChunk>, text: &str, start_char: usize) {
    let end_char = start_char + text.chars().count();
    chunks.push(PaperChunk {
        index: chunks.len(),
        start_char,
        end_char,
        text: text.to_string(),
    });
}

fn split_paragraphs(text: &str) -> Vec<String> {
    let mut paragraphs = Vec::new();
    let mut current = Vec::new();

    for line in text.lines() {
        if line.trim().is_empty() {
            if !current.is_empty() {
                paragraphs.push(current.join("\n"));
                current.clear();
            }
            continue;
        }
        current.push(line.to_string());
    }

    if !current.is_empty() {
        paragraphs.push(current.join("\n"));
    }

    paragraphs
}

fn split_long_paragraph(paragraph: &str, chunk_chars: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();

    for line in paragraph.lines() {
        let line_len = line.chars().count();
        if line_len > chunk_chars {
            if !current.is_empty() {
                out.push(current.trim().to_string());
                current.clear();
            }
            for part in split_by_char_count(line, chunk_chars) {
                out.push(part);
            }
            continue;
        }

        let candidate = if current.is_empty() {
            line.to_string()
        } else {
            format!("{current}\n{line}")
        };

        if candidate.chars().count() > chunk_chars && !current.is_empty() {
            out.push(current.trim().to_string());
            current = line.to_string();
        } else {
            current = candidate;
        }
    }

    if !current.trim().is_empty() {
        out.push(current.trim().to_string());
    }

    out
}

fn split_by_char_count(text: &str, chunk_chars: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();

    for ch in text.chars() {
        current.push(ch);
        if current.chars().count() >= chunk_chars {
            out.push(current.clone());
            current.clear();
        }
    }

    if !current.is_empty() {
        out.push(current);
    }

    out
}

fn collapse_blank_lines(text: &str) -> String {
    let mut out = String::new();
    let mut previous_blank = false;

    for line in text.lines() {
        let blank = line.trim().is_empty();
        if blank {
            if previous_blank {
                continue;
            }
            previous_blank = true;
            out.push('\n');
            continue;
        }

        previous_blank = false;
        out.push_str(line.trim_end());
        out.push('\n');
    }

    out.trim().to_string()
}

fn is_reference_heading(line: &str) -> bool {
    matches!(
        line.to_ascii_lowercase().as_str(),
        "references" | "# references" | "## references" | "bibliography" | "## bibliography"
    )
}

fn is_noise_line(line: &str) -> bool {
    if line.is_empty() {
        return false;
    }

    let lower = line.to_ascii_lowercase();
    lower.starts_with("arxiv:")
        || lower.starts_with("copyright")
        || lower.starts_with("preprint")
        || lower.starts_with("available at")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_duplicate_blank_lines() {
        let input = "a\r\n\r\n\r\nb";
        assert_eq!(normalize_markdown(input), "a\n\nb");
    }

    #[test]
    fn prunes_references_section() {
        let input = "Intro\n\nReferences\n[1] one\n[2] two";
        let output = prune_markdown(input, true);
        assert_eq!(output, "Intro");
    }

    #[test]
    fn chunks_long_text() {
        let input = "para one\n\npara two\n\npara three";
        let chunks = chunk_text(input, 12, 0);
        assert!(chunks.len() >= 2);
        assert_eq!(chunks[0].index, 0);
    }

    #[test]
    fn prepares_paper_content() {
        let paper = Paper {
            id: "1234.5678".into(),
            title: "Paper".into(),
            authors: vec!["A".into()],
            abstract_text: "Abstract".into(),
            categories: vec![],
            published: "2024".into(),
            url: "https://arxiv.org/abs/1234.5678".into(),
        };
        let prepared = prepare_paper(
            paper,
            "html",
            "Intro\n\nReferences\n[1]",
            PreparationOptions::default(),
        );
        assert_eq!(prepared.source, "html");
        assert_eq!(prepared.pruned_markdown, "Intro");
        assert!(!prepared.chunks.is_empty());
    }
}
