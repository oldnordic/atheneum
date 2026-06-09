use anyhow::Result;

pub trait TextEmbedder: Send + Sync {
    fn embed(&self, text: &str) -> Result<Vec<f32>>;
    fn dimension(&self) -> usize;
}

pub struct HashEmbedder {
    dim: usize,
}

impl HashEmbedder {
    pub fn new(dim: usize) -> Self {
        Self { dim }
    }
}

impl TextEmbedder for HashEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        Ok(hash_embed(text, self.dim))
    }

    fn dimension(&self) -> usize {
        self.dim
    }
}

pub(crate) fn hash_embed(text: &str, dim: usize) -> Vec<f32> {
    use std::hash::{Hash, Hasher};
    let mut vector = vec![0.0_f32; dim];
    for token in text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
    {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        token.to_ascii_lowercase().hash(&mut hasher);
        let bucket = (hasher.finish() as usize) % dim;
        vector[bucket] += 1.0;
    }
    let norm: f32 = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for v in &mut vector {
            *v /= norm;
        }
    }
    vector
}

#[cfg(feature = "neural-embed")]
pub struct OllamaEmbedder {
    model: String,
    url: String,
    dim: usize,
}

#[cfg(feature = "neural-embed")]
impl OllamaEmbedder {
    pub fn new(model: &str, url: &str, dim: usize) -> Self {
        Self {
            model: model.to_string(),
            url: url.to_string(),
            dim,
        }
    }

    pub fn nomic_embed_text() -> Self {
        Self::new("nomic-embed-text", "http://localhost:11434", 768)
    }
}

#[cfg(feature = "neural-embed")]
impl TextEmbedder for OllamaEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let response: serde_json::Value = ureq::post(&format!("{}/api/embed", self.url))
            .send_json(ureq::json!({
                "model": self.model,
                "input": text,
            }))?
            .into_json()?;

        let embeddings = response
            .get("embeddings")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow::anyhow!("missing embeddings in ollama response"))?;

        let first = embeddings
            .first()
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow::anyhow!("empty embeddings array"))?;

        let vector: Vec<f32> = first
            .iter()
            .filter_map(|v| v.as_f64().map(|f| f as f32))
            .collect();

        if vector.len() != self.dim {
            anyhow::bail!(
                "embedding dimension mismatch: expected {}, got {}",
                self.dim,
                vector.len()
            );
        }

        Ok(vector)
    }

    fn dimension(&self) -> usize {
        self.dim
    }
}
