use serde::{Deserialize, Serialize};

use crate::paper::Paper;

const DEFAULT_CHUNK_CHARS: usize = 4_000;
const DEFAULT_CHUNK_OVERLAP: usize = 200;

#[derive(Debug, Copy, Clone, Serialize, Deserialize, PartialEq)]
pub struct PreparationOptions {
    pub prune_references: bool,
    pub chunk_chars: usize,
    pub chunk_overlap: usize,
    /// If Some, use hierarchical segmentation with the given k parameter.
    pub segmentation_k: Option<f32>,
}

impl Default for PreparationOptions {
    fn default() -> Self {
        Self {
            prune_references: true,
            chunk_chars: DEFAULT_CHUNK_CHARS,
            chunk_overlap: DEFAULT_CHUNK_OVERLAP,
            segmentation_k: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PaperChunk {
    pub index: usize,
    pub start_char: usize,
    pub end_char: usize,
    pub text: String,
    pub cluster_id: Option<String>,
    pub parent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HierarchicalPaperChunk {
    pub index: usize,
    pub start_char: usize,
    pub end_char: usize,
    pub text: String,
    pub segments: Vec<PaperChunk>,
    /// Mean-pooled embedding of the segments in this cluster.
    pub cluster_embedding: Vec<f32>,
    pub cluster_id: Option<String>,
    pub parent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TopicChunk {
    pub id: String,
    pub text: String,
    pub citations: Vec<String>,
    pub source_chunks: Vec<CrossDocumentPaperChunk>,
    pub cluster_embedding: Vec<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CrossDocumentPaperChunk {
    pub paper_id: String,
    pub chunk: PaperChunk,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PreparedPaper {
    pub paper: Paper,
    pub source: String,
    pub raw_markdown: String,
    pub pruned_markdown: String,
    pub chunks: Vec<PaperChunk>,
    pub hierarchical_chunks: Option<Vec<HierarchicalPaperChunk>>,
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

    let hierarchical_chunks = options
        .segmentation_k
        .map(|k| {
            let segment_texts = segments_from_text(&pruned_markdown);
            if segment_texts.len() < 2 {
                return Vec::new();
            }
            let corpus: Vec<&str> = segment_texts.iter().map(String::as_str).collect();
            let vectorizer = crate::tfidf::TfidfVectorizer::new(&corpus);
            let segments: Vec<crate::segmentation::Segment> = segment_texts
                .into_iter()
                .map(|text| crate::segmentation::Segment {
                    embedding: vectorizer.vectorize(&text),
                    text,
                })
                .collect();
            hierarchical_chunk_text(&segments, k)
        })
        .filter(|v| !v.is_empty());

    PreparedPaper {
        paper,
        source: source.into(),
        raw_markdown,
        pruned_markdown,
        chunks,
        hierarchical_chunks,
    }
}

/// Split text into sentence-like segments for hierarchical clustering.
/// Splits on sentence boundaries (`.`, `?`, `!`) and newlines, keeping
/// only segments with at least 20 characters of content.
#[must_use]
pub fn segments_from_text(text: &str) -> Vec<String> {
    text.split(['.', '?', '!', '\n'])
        .map(str::trim)
        .filter(|s| s.len() >= 20)
        .map(str::to_string)
        .collect()
}

/// Performs hierarchical chunking given segments and their embeddings.
#[must_use]
pub fn hierarchical_chunk_text(
    segments: &[crate::segmentation::Segment],
    k: f32,
) -> Vec<HierarchicalPaperChunk> {
    use crate::segmentation::{ClusteringOptions, HierarchicalSegmenter};

    let segmenter = HierarchicalSegmenter::new(ClusteringOptions { k });
    let clusters = segmenter.cluster(segments);

    clusters
        .into_iter()
        .enumerate()
        .map(|(cluster_idx, segment_indices)| {
            let mut cluster_text = String::new();
            let mut cluster_segments = Vec::new();
            let mut embeddings = Vec::new();

            for (i, &seg_idx) in segment_indices.iter().enumerate() {
                let segment = &segments[seg_idx];
                if i > 0 {
                    cluster_text.push_str("\n\n");
                }
                let seg_start = cluster_text.chars().count();
                cluster_text.push_str(&segment.text);
                let seg_end = cluster_text.chars().count();

                cluster_segments.push(PaperChunk {
                    index: i,
                    start_char: seg_start,
                    end_char: seg_end,
                    text: segment.text.clone(),
                    cluster_id: Some(format!("cluster_{cluster_idx}")),
                    parent_id: None,
                });
                embeddings.push(segment.embedding.clone());
            }

            // Mean pooling for cluster embedding
            let cluster_embedding = if embeddings.is_empty() {
                Vec::new()
            } else {
                let dim = embeddings[0].len();
                let mut mean = vec![0.0f32; dim];
                for e in &embeddings {
                    for (m, &v) in mean.iter_mut().zip(e) {
                        *m += v;
                    }
                }
                #[expect(clippy::cast_precision_loss)]
                let count = embeddings.len() as f32;
                for m in &mut mean {
                    *m /= count;
                }
                mean
            };

            HierarchicalPaperChunk {
                index: cluster_idx,
                start_char: 0,
                end_char: cluster_text.chars().count(),
                text: cluster_text,
                segments: cluster_segments,
                cluster_embedding,
                cluster_id: Some(format!("hierarchical_{cluster_idx}")),
                parent_id: None,
            }
        })
        .collect()
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
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut chunks = Vec::new();
    let mut start = 0usize;

    while start < n {
        let end = (start + chunk_chars).min(n);

        let break_point = find_break_point(&chars, start, end);
        let actual_end = break_point.unwrap_or(end);

        let chunk_text: String = chars[start..actual_end].iter().collect();
        if !chunk_text.trim().is_empty() {
            chunks.push(PaperChunk {
                index: chunks.len(),
                start_char: start,
                end_char: actual_end,
                text: chunk_text,
                cluster_id: None,
                parent_id: None,
            });
        }

        if actual_end == start {
            start += 1;
        } else {
            let step = actual_end - start;
            start = actual_end.saturating_sub(chunk_overlap.min(step.saturating_sub(1)));
        }
    }

    chunks
}

/// Find the best break point within a chunk window: prefer double-newline
/// (paragraph break), then sentence end (`. `), then word boundary (` `).
/// Returns the absolute char offset to break at, or `None` if the chunk is
/// short enough to take whole.
fn find_break_point(chars: &[char], start: usize, end: usize) -> Option<usize> {
    if end - start < 100 {
        return None;
    }
    let search_start = start + (end - start) * 6 / 10;
    for i in (search_start..end.saturating_sub(1)).rev() {
        if chars[i] == '\n' && chars.get(i + 1) == Some(&'\n') {
            return Some(i);
        }
    }
    for i in (search_start..end.saturating_sub(1)).rev() {
        if chars[i] == '.' && chars.get(i + 1) == Some(&' ') {
            return Some(i + 1);
        }
    }
    (search_start..end).rev().find(|&i| chars[i] == ' ')
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
    let stripped = line
        .trim()
        .trim_start_matches('#')
        .trim()
        .to_ascii_lowercase();
    stripped == "references"
        || stripped == "bibliography"
        || stripped == "acknowledgments"
        || stripped == "acknowledgements"
        || stripped.starts_with("references and")
        || stripped.starts_with("references &")
        || stripped.starts_with("references notes")
        || stripped.starts_with("bibliography and")
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
    fn prunes_deep_reference_headings() {
        let input = "Intro\n\n### References\n[1] one\n[2] two";
        assert_eq!(prune_markdown(input, true), "Intro");
    }

    #[test]
    fn prunes_acknowledgments_section() {
        let input = "Intro\n\n## Acknowledgments\nThanks to everyone.\n\nReferences\n[1] one";
        assert_eq!(prune_markdown(input, true), "Intro");
    }

    #[test]
    fn prunes_references_and_notes() {
        let input = "Body text\n\n# References and Notes\n[1] foo";
        assert_eq!(prune_markdown(input, true), "Body text");
    }

    #[test]
    fn preserves_references_when_disabled() {
        let input = "Intro\n\nReferences\n[1] one";
        assert_eq!(prune_markdown(input, false), "Intro\n\nReferences\n[1] one");
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
            authors: vec![crate::paper::Author {
                name: "A".into(),
                affiliations: vec![],
            }],
            abstract_text: "Abstract".into(),
            categories: vec![],
            published: "2024".into(),
            url: "https://arxiv.org/abs/1234.5678".into(),
            doi: None,
            journal_ref: None,
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
