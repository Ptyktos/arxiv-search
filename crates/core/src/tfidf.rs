use std::collections::HashMap;

/// A TF-IDF vectorizer. Build from a corpus, then vectorize any text
/// into a dense `Vec<f32>` for cosine-similarity comparison.
pub struct TfidfVectorizer {
    vocabulary: Vec<String>,
    term_index: HashMap<String, usize>,
    idf: Vec<f32>,
}

impl TfidfVectorizer {
    /// Build from a corpus of documents. Each document is tokenized into
    /// lowercase alphanumeric terms.
    #[must_use]
    #[expect(clippy::cast_precision_loss)]
    pub fn new(corpus: &[&str]) -> Self {
        let mut df: HashMap<String, usize> = HashMap::new();
        let n_docs = corpus.len();

        let tokenized: Vec<Vec<String>> = corpus.iter().map(|d| tokenize(d)).collect();

        for tokens in &tokenized {
            let unique: std::collections::HashSet<&String> = tokens.iter().collect();
            for term in unique {
                *df.entry(term.clone()).or_insert(0) += 1;
            }
        }

        let vocabulary: Vec<String> = df.keys().cloned().collect();
        let term_index: HashMap<String, usize> = vocabulary
            .iter()
            .enumerate()
            .map(|(i, t)| (t.clone(), i))
            .collect();

        let idf: Vec<f32> = vocabulary
            .iter()
            .map(|term| {
                let df_t = *df.get(term).unwrap_or(&0) as f32;
                ((1.0 + n_docs as f32) / (1.0 + df_t)).ln() + 1.0
            })
            .collect();

        Self {
            vocabulary,
            term_index,
            idf,
        }
    }

    /// Vectorize a text into a dense TF-IDF vector.
    #[must_use]
    #[expect(clippy::cast_precision_loss)]
    pub fn vectorize(&self, text: &str) -> Vec<f32> {
        let tokens = tokenize(text);
        let total = tokens.len() as f32;
        let mut tf: HashMap<usize, f32> = HashMap::new();

        for term in &tokens {
            if let Some(&idx) = self.term_index.get(term) {
                *tf.entry(idx).or_insert(0.0) += 1.0;
            }
        }

        let mut vec = vec![0.0f32; self.vocabulary.len()];
        if total > 0.0 {
            for (idx, count) in &tf {
                vec[*idx] = (count / total) * self.idf[*idx];
            }
        }
        vec
    }

    /// Cosine similarity between two vectors.
    #[must_use]
    pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
        let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
        let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
        if na == 0.0 || nb == 0.0 {
            0.0
        } else {
            dot / (na * nb)
        }
    }
}

fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vectorizer_basic() {
        let corpus = ["the cat sat", "the dog ran", "physics of stars"];
        let v = TfidfVectorizer::new(&corpus);
        let q = v.vectorize("physics stars");
        let d = v.vectorize("physics of stars");
        assert!(TfidfVectorizer::cosine_similarity(&q, &d) > 0.5);
    }

    #[test]
    fn vectorizer_distinguishes_topics() {
        let corpus = [
            "quantum mechanics stellar cores",
            "cell biology dna replication",
            "cooking fusion cuisine",
        ];
        let v = TfidfVectorizer::new(&corpus);
        let q = v.vectorize("stellar quantum mechanics");
        let sim_physics =
            TfidfVectorizer::cosine_similarity(&q, &v.vectorize("quantum mechanics stellar cores"));
        let sim_cooking =
            TfidfVectorizer::cosine_similarity(&q, &v.vectorize("cooking fusion cuisine"));
        assert!(
            sim_physics > sim_cooking,
            "physics should rank higher than cooking"
        );
    }

    #[test]
    fn empty_text_returns_zero_vector() {
        let v = TfidfVectorizer::new(&["hello world"]);
        let vec = v.vectorize("");
        assert!(vec.iter().all(|&x| x == 0.0));
    }
}
