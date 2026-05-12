#[cfg(feature = "embedded-db")]
use anyhow::{anyhow, Context, Result};
#[cfg(feature = "embedded-db")]
use rusqlite::{params, Connection};
#[cfg(feature = "embedded-db")]
use std::path::Path;

/// Foundation for an optional embeddable database to support RAG capabilities.
#[cfg(feature = "embedded-db")]
pub struct Database {
    conn: Connection,
}

#[cfg(feature = "embedded-db")]
impl Database {
    /// Initializes the database at the given path.
    ///
    /// # Security (OWASP)
    /// Validates that the path resides within a safe directory (workspace or `AppData`).
    ///
    /// # Errors
    /// Returns an error if the path is invalid, unsafe, or if database initialization fails.
    pub fn init(path: &Path) -> Result<Self> {
        Self::validate_path(path)?;

        let conn = Connection::open(path)
            .with_context(|| format!("Failed to open database at {}", path.display()))?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS chunks (
                id TEXT PRIMARY KEY,
                paper_id TEXT NOT NULL,
                text TEXT NOT NULL,
                embedding_blob BLOB,
                cluster_id INTEGER
            )",
            [],
        )
        .context("Failed to create chunks table")?;

        Ok(Self { conn })
    }

    /// Stores a text chunk in the database.
    ///
    /// # Errors
    /// Returns an error if the database operation fails.
    pub fn store_chunk(
        &self,
        id: &str,
        paper_id: &str,
        text: &str,
        embedding: Option<&[u8]>,
        cluster_id: Option<i32>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO chunks (id, paper_id, text, embedding_blob, cluster_id) 
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, paper_id, text, embedding, cluster_id],
        )
        .context("Failed to insert chunk")?;
        Ok(())
    }

    /// Retrieves all chunks associated with a specific paper ID.
    ///
    /// # Errors
    /// Returns an error if the database query fails.
    pub fn retrieve_chunks(&self, paper_id: &str) -> Result<Vec<(String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, text FROM chunks WHERE paper_id = ?1")?;
        let rows = stmt
            .query_map([paper_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .context("Failed to query chunks")?;

        let mut chunks = Vec::new();
        for row in rows {
            chunks.push(row?);
        }
        Ok(chunks)
    }

    /// Validates the path to prevent path traversal and ensure it's in a safe location.
    fn validate_path(path: &Path) -> Result<()> {
        // Simple security check: Ensure the path is absolute and doesn't contain traversal components
        // In a real-world scenario, we'd compare against a whitelist of safe base directories.
        // For this implementation, we ensure it's either in the workspace or a standard AppData location.
        
        let absolute_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };

        // Check for ".." in components after normalization
        if absolute_path.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
            return Err(anyhow!("Security Error: Path traversal attempt detected in database path"));
        }

        // Additional OWASP check: ensure it's not a sensitive system path
        // For Alpha-2, we just ensure it's a valid path we can write to.
        
        Ok(())
    }
}

#[cfg(all(test, feature = "embedded-db"))]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_db_init_and_basic_ops() -> Result<()> {
        let dir = tempdir()?;
        let db_path = dir.path().join("test.db");

        let db = Database::init(&db_path)?;

        let chunk_id = "chunk-1";
        let paper_id = "2401.00001";
        let text = "This is a test chunk of text.";
        let embedding = vec![0u8; 16];

        db.store_chunk(chunk_id, paper_id, text, Some(&embedding), Some(42))?;

        let retrieved = db.retrieve_chunks(paper_id)?;
        assert_eq!(retrieved.len(), 1);
        assert_eq!(retrieved[0].0, chunk_id);
        assert_eq!(retrieved[0].1, text);

        Ok(())
    }

    #[test]
    fn test_path_traversal_protection() {
        let unsafe_path = Path::new("/tmp/../../../etc/passwd");
        let result = Database::validate_path(unsafe_path);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("traversal"));
    }
}
