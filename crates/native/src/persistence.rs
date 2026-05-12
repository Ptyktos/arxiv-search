use std::path::PathBuf;
use anyhow::{Result, Context};
use tokio::fs;

#[derive(Debug, Clone)]
pub struct ArxivCache {
    cache_dir: PathBuf,
}

impl ArxivCache {
    pub async fn new() -> Result<Self> {
        // Try current directory first (workspace)
        let mut cache_dir = std::env::current_dir()?;
        cache_dir.push(".arxiv_cache");
        
        if !cache_dir.exists() {
            // Try to create it. If it fails, fallback to temp dir.
            if let Err(e) = fs::create_dir_all(&cache_dir).await {
                tracing::warn!("Failed to create cache dir in current workspace: {e}. Falling back to temp dir.");
                cache_dir = std::env::temp_dir();
                cache_dir.push(".arxiv_cache");
                fs::create_dir_all(&cache_dir).await.context("Failed to create temp cache dir")?;
            }
        }
        
        Ok(Self { cache_dir })
    }

    pub async fn get_html(&self, paper_id: &str) -> Result<Option<String>> {
        let path = self.cache_dir.join(format!("{paper_id}.html"));
        if path.exists() {
            let content = fs::read_to_string(path).await?;
            return Ok(Some(content));
        }
        Ok(None)
    }

    pub async fn set_html(&self, paper_id: &str, content: &str) -> Result<()> {
        let path = self.cache_dir.join(format!("{paper_id}.html"));
        fs::write(path, content).await?;
        Ok(())
    }

    pub async fn get_pdf(&self, paper_id: &str) -> Result<Option<Vec<u8>>> {
        let path = self.cache_dir.join(format!("{paper_id}.pdf"));
        if path.exists() {
            let content = fs::read(path).await?;
            return Ok(Some(content));
        }
        Ok(None)
    }

    pub async fn set_pdf(&self, paper_id: &str, content: &[u8]) -> Result<()> {
        let path = self.cache_dir.join(format!("{paper_id}.pdf"));
        fs::write(path, content).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_cache_hit() -> Result<()> {
        let temp = tempdir()?;
        let cache_dir = temp.path().join(".arxiv_cache");
        fs::create_dir_all(&cache_dir).await?;
        
        let cache = ArxivCache { cache_dir: cache_dir.clone() };
        
        let paper_id = "1234.5678";
        let html_content = "<html><body>Test</body></html>";
        
        // Initial state: cache miss
        assert!(cache.get_html(paper_id).await?.is_none());
        
        // Set cache
        cache.set_html(paper_id, html_content).await?;
        
        // Cache hit
        let retrieved = cache.get_html(paper_id).await?;
        assert_eq!(retrieved, Some(html_content.to_string()));
        
        Ok(())
    }

    #[tokio::test]
    async fn test_pdf_cache_hit() -> Result<()> {
        let temp = tempdir()?;
        let cache_dir = temp.path().join(".arxiv_cache");
        fs::create_dir_all(&cache_dir).await?;
        
        let cache = ArxivCache { cache_dir: cache_dir.clone() };
        
        let paper_id = "1234.5678";
        let pdf_content = vec![0xDE, 0xAD, 0xBE, 0xEF];
        
        // Initial state: cache miss
        assert!(cache.get_pdf(paper_id).await?.is_none());
        
        // Set cache
        cache.set_pdf(paper_id, &pdf_content).await?;
        
        // Cache hit
        let retrieved = cache.get_pdf(paper_id).await?;
        assert_eq!(retrieved, Some(pdf_content));
        
        Ok(())
    }
}
