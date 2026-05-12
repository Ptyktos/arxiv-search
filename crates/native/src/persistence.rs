use anyhow::{Context, Result};
use directories::ProjectDirs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};
use tokio::fs;

/// Default cache TTL: 7 days (604,800 seconds)
pub const DEFAULT_CACHE_TTL: u64 = 604_800;

/// A filesystem-based caching layer for arXiv HTML and PDF payloads.
///
/// This avoids redundant network fetches and bypasses the arXiv API rate limit
/// by serving previously downloaded papers directly from the user's OS cache directory.
#[derive(Debug, Clone)]
pub struct ArxivCache {
    cache_dir: PathBuf,
    ttl_seconds: u64,
}

impl ArxivCache {
    /// Initializes a new cache instance.
    ///
    /// Determines the standard cache directory for the OS (e.g. `~/.cache/arxiv-search-mcp` on Linux)
    /// and ensures that it exists.
    ///
    /// # Errors
    /// Returns an error if the directory cannot be created.
    pub async fn new(ttl_seconds: u64) -> Result<Self> {
        // Use standard OS cache directory to avoid littering the workspace
        let cache_dir = ProjectDirs::from("org", "arxiv-search", "mcp").map_or_else(
            || std::env::temp_dir().join("arxiv-search-mcp"), // Fallback to temp dir if standard paths are unavailable
            |proj_dirs| proj_dirs.cache_dir().to_path_buf(),
        );

        if !cache_dir.exists() {
            fs::create_dir_all(&cache_dir).await.with_context(|| {
                format!(
                    "Failed to create cache directory at {}",
                    cache_dir.display()
                )
            })?;
        }

        let cache = Self {
            cache_dir,
            ttl_seconds,
        };

        // Run cleanup on initialization to ensure stale files are removed
        if let Err(e) = cache.cleanup_expired().await {
            tracing::error!("Failed to cleanup expired cache files: {:?}", e);
        }

        Ok(cache)
    }

    /// Scans the cache directory and deletes files that have exceeded the TTL.
    ///
    /// # Errors
    /// Returns an error if the directory cannot be read or files cannot be deleted.
    pub async fn cleanup_expired(&self) -> Result<()> {
        let mut entries = fs::read_dir(&self.cache_dir).await.with_context(|| {
            format!(
                "Failed to read cache directory for cleanup: {}",
                self.cache_dir.display()
            )
        })?;

        let now = SystemTime::now();
        let ttl = Duration::from_secs(self.ttl_seconds);

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();

            // Security: Ensure we only delete files within the cache directory
            // and skip directories/symlinks to avoid accidental deletions.
            if !path.starts_with(&self.cache_dir) || !path.is_file() {
                continue;
            }

            // Additional security: ensure no path traversal in the filename itself
            if path.to_string_lossy().contains("..") {
                continue;
            }

            if let Ok(metadata) = entry.metadata().await {
                if let Ok(modified) = metadata.modified() {
                    if let Ok(elapsed) = now.duration_since(modified) {
                        if elapsed > ttl {
                            tracing::info!("Deleting expired cache file: {:?}", path);
                            let _ = fs::remove_file(&path).await;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Attempts to retrieve a cached HTML payload for the given arXiv paper ID.
    ///
    /// # Errors
    /// Returns an error if reading the file from disk fails.
    pub async fn get_html(&self, paper_id: &str) -> Result<Option<String>> {
        let path = self.cache_dir.join(format!("{paper_id}.html"));
        if path.exists() {
            let content = fs::read_to_string(path).await?;
            return Ok(Some(content));
        }
        Ok(None)
    }

    /// Writes an HTML payload to the cache for the given arXiv paper ID.
    ///
    /// # Errors
    /// Returns an error if writing to disk fails.
    pub async fn set_html(&self, paper_id: &str, content: &str) -> Result<()> {
        let path = self.cache_dir.join(format!("{paper_id}.html"));
        fs::write(path, content).await?;
        Ok(())
    }

    /// Attempts to retrieve a cached PDF payload for the given arXiv paper ID.
    ///
    /// # Errors
    /// Returns an error if reading the file from disk fails.
    pub async fn get_pdf(&self, paper_id: &str) -> Result<Option<Vec<u8>>> {
        let path = self.cache_dir.join(format!("{paper_id}.pdf"));
        if path.exists() {
            let content = fs::read(path).await?;
            return Ok(Some(content));
        }
        Ok(None)
    }

    /// Writes a PDF payload to the cache for the given arXiv paper ID.
    ///
    /// # Errors
    /// Returns an error if writing to disk fails.
    pub async fn set_pdf(&self, paper_id: &str, content: &[u8]) -> Result<()> {
        let path = self.cache_dir.join(format!("{paper_id}.pdf"));
        fs::write(path, content).await?;
        Ok(())
    }

    /// Returns the path to the cache directory.
    #[must_use]
    pub const fn get_cache_dir(&self) -> &std::path::PathBuf {
        &self.cache_dir
    }
}

#[cfg(test)]
#[expect(clippy::panic_in_result_fn)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_cache_hit() -> Result<()> {
        let temp = tempdir()?;
        let cache_dir = temp.path().join(".arxiv_cache");
        fs::create_dir_all(&cache_dir).await?;

        let cache = ArxivCache {
            cache_dir: cache_dir.clone(),
            ttl_seconds: DEFAULT_CACHE_TTL,
        };

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

        let cache = ArxivCache {
            cache_dir: cache_dir.clone(),
            ttl_seconds: DEFAULT_CACHE_TTL,
        };

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

    #[tokio::test]
    async fn test_cache_ttl_cleanup() -> Result<()> {
        let temp = tempdir()?;
        let cache_dir = temp.path().join(".arxiv_cache");
        fs::create_dir_all(&cache_dir).await?;

        // Use a very short TTL for testing
        let ttl_seconds = 1;
        let cache = ArxivCache {
            cache_dir: cache_dir.clone(),
            ttl_seconds,
        };

        let old_paper = "old.paper";
        let new_paper = "new.paper";
        let content = "test content";

        // 1. Create an "old" file
        cache.set_html(old_paper, content).await?;
        let old_path = cache_dir.join(format!("{old_paper}.html"));

        // Artificially set the modified time to be in the past
        let past = SystemTime::now() - Duration::from_secs(10);
        filetime::set_file_mtime(&old_path, filetime::FileTime::from_system_time(past))?;

        // 2. Create a "new" file
        cache.set_html(new_paper, content).await?;

        // 3. Run cleanup
        cache.cleanup_expired().await?;

        // 4. Verify results
        assert!(
            !old_path.exists(),
            "Old file should have been deleted by TTL cleanup"
        );
        assert!(
            cache_dir.join(format!("{new_paper}.html")).exists(),
            "New file should still exist"
        );

        Ok(())
    }
}
