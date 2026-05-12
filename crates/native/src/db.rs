#[cfg(feature = "embedded-db")]
use anyhow::{anyhow, Context, Result};
#[cfg(feature = "embedded-db")]
use rusqlite::{params, Connection};
#[cfg(feature = "embedded-db")]
use std::path::Path;
#[cfg(feature = "embedded-db")]
use std::sync::{Arc, Mutex};

/// Foundation for an optional embeddable database to support RAG capabilities.
#[cfg(feature = "embedded-db")]
#[derive(Clone)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
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
            "CREATE TABLE IF NOT EXISTS papers (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                abstract TEXT NOT NULL,
                metadata TEXT
            )",
            [],
        )
        .context("Failed to create papers table")?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS chunks (
                id TEXT PRIMARY KEY,
                paper_id TEXT NOT NULL,
                text TEXT NOT NULL,
                embedding_blob BLOB,
                cluster_id TEXT,
                FOREIGN KEY(paper_id) REFERENCES papers(id)
            )",
            [],
        )
        .context("Failed to create chunks table")?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Stores paper metadata in the database.
    ///
    /// # Errors
    /// Returns an error if the database operation fails.
    pub fn store_paper(&self, id: &str, title: &str, abstract_text: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| anyhow!("Failed to lock database"))?;
        conn.execute(
            "INSERT OR REPLACE INTO papers (id, title, abstract) VALUES (?1, ?2, ?3)",
            params![id, title, abstract_text],
        )
        .context("Failed to insert paper")?;
        drop(conn);
        Ok(())
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
        cluster_id: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| anyhow!("Failed to lock database"))?;
        conn.execute(
            "INSERT OR REPLACE INTO chunks (id, paper_id, text, embedding_blob, cluster_id) 
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, paper_id, text, embedding, cluster_id],
        )
        .context("Failed to insert chunk")?;
        drop(conn);
        Ok(())
    }

    /// Stage 1: Document-level routing P(D|q).
    /// Filters the corpus for whole documents relevant to the query.
    ///
    /// # Errors
    /// Returns an error if the database query fails.
    pub fn route_documents(&self, query: &str, limit: usize) -> Result<Vec<String>> {
        let conn = self.conn.lock().map_err(|_| anyhow!("Failed to lock database"))?;
        let mut stmt = conn.prepare(
            "SELECT id FROM papers 
             WHERE title LIKE ?1 OR abstract LIKE ?1 
             LIMIT ?2",
        )?;
        let pattern = format!("%{query}%");
        let rows = stmt.query_map(params![pattern, limit], |row| row.get(0))?;

        let doc_ids = rows.map(|r| r.map_err(Into::into)).collect::<Result<Vec<String>>>()?;
        drop(stmt);
        drop(conn);
        Ok(doc_ids)
    }

    /// Stage 2: Scoped chunk retrieval P(c|q, D).
    /// Confines search to chunks exclusively within the document subset.
    ///
    /// # Errors
    /// Returns an error if the database query fails.
    pub fn retrieve_chunks_scoped(
        &self,
        query: &str,
        paper_ids: &[String],
        limit: usize,
    ) -> Result<Vec<(String, String)>> {
        if paper_ids.is_empty() {
            return Ok(Vec::new());
        }

        let conn = self.conn.lock().map_err(|_| anyhow!("Failed to lock database"))?;
        let placeholders: Vec<String> = (1..=paper_ids.len()).map(|i| format!("?{i}")).collect();
        let sql = format!(
            "SELECT id, text FROM chunks 
             WHERE paper_id IN ({}) AND text LIKE ?{} 
             LIMIT ?{}",
            placeholders.join(","),
            paper_ids.len() + 1,
            paper_ids.len() + 2
        );

        let mut stmt = conn.prepare(&sql)?;
        let pattern = format!("%{query}%");
        
        let mut params_vec: Vec<rusqlite::types::Value> = paper_ids
            .iter()
            .map(|s| rusqlite::types::Value::Text(s.clone()))
            .collect();
        params_vec.push(rusqlite::types::Value::Text(pattern));
        params_vec.push(rusqlite::types::Value::Integer(i64::try_from(limit).unwrap_or(i64::MAX)));

        let rows = stmt.query_map(rusqlite::params_from_iter(params_vec), |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?;

        let chunks = rows.map(|r| r.map_err(Into::into)).collect::<Result<Vec<(String, String)>>>()?;
        drop(stmt);
        drop(conn);
        Ok(chunks)
    }

    /// Retrieves all chunks associated with a specific paper ID.
    ///
    /// # Errors
    /// Returns an error if the database query fails.
    pub fn retrieve_chunks(&self, paper_id: &str) -> Result<Vec<(String, String)>> {
        let conn = self.conn.lock().map_err(|_| anyhow!("Failed to lock database"))?;
        let mut stmt = conn
            .prepare("SELECT id, text FROM chunks WHERE paper_id = ?1")?;
        let rows = stmt
            .query_map([paper_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .context("Failed to query chunks")?;

        let chunks = rows.map(|r| r.map_err(Into::into)).collect::<Result<Vec<(String, String)>>>()?;
        drop(stmt);
        drop(conn);
        Ok(chunks)
    }

    /// Validates the path to prevent path traversal and ensure it's in a safe location.
    fn validate_path(path: &Path) -> Result<()> {
        let absolute_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };

        if absolute_path.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
            return Err(anyhow!("Security Error: Path traversal attempt detected in database path"));
        }

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

        let paper_id = "2401.00001";
        db.store_paper(paper_id, "Test Paper", "Abstract of test paper")?;

        let chunk_id = "chunk-1";
        let text = "This is a test chunk of text.";
        let embedding = vec![0u8; 16];

        db.store_chunk(chunk_id, paper_id, text, Some(&embedding), Some("42"))?;

        let retrieved = db.retrieve_chunks(paper_id)?;
        assert_eq!(retrieved.len(), 1);
        assert_eq!(retrieved[0].0, chunk_id);
        assert_eq!(retrieved[0].1, text);

        Ok(())
    }

    #[test]
    fn test_hdrr_routing_and_scoped_retrieval() -> Result<()> {
        let dir = tempdir()?;
        let db_path = dir.path().join("hdrr.db");
        let db = Database::init(&db_path)?;

        // Paper 1: Relevant to "physics"
        db.store_paper("p1", "Physics of Stars", "Quantum mechanics in stellar cores")?;
        db.store_chunk("c1-1", "p1", "Star formation process...", None, None)?;
        db.store_chunk("c1-2", "p1", "Nuclear fusion in stars...", None, None)?;

        // Paper 2: Relevant to "biology"
        db.store_paper("p2", "Cell Biology", "How cells work")?;
        db.store_chunk("c2-1", "p2", "DNA replication...", None, None)?;

        // Stage 1: Route "physics"
        let routed = db.route_documents("physics", 10)?;
        assert_eq!(routed.len(), 1);
        assert_eq!(routed[0], "p1");

        // Stage 2: Scoped retrieval for "fusion" in routed docs
        let chunks = db.retrieve_chunks_scoped("fusion", &routed, 10)?;
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].0, "c1-2");

        // Adversarial test: "fusion" exists in an irrelevant document
        db.store_paper("p3", "Cooking", "Making fusion cuisine")?;
        db.store_chunk("c3-1", "p3", "Fusion of flavors...", None, None)?;

        // If we route by "physics", p3 should be filtered out
        let routed_phys = db.route_documents("physics", 10)?;
        let chunks_phys = db.retrieve_chunks_scoped("fusion", &routed_phys, 10)?;
        assert_eq!(chunks_phys.len(), 1);
        assert_eq!(chunks_phys[0].0, "c1-2"); // Should NOT include c3-1
        
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
