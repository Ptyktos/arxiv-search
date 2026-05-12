use arxiv_search_rs_mcp_core::paper::{Paper, Author};
use arxiv_search_rs_mcp_core::content::{prepare_paper, PreparationOptions};
use arxiv_search_rs_mcp_native::db::Database;
use std::path::Path;

#[tokio::test]
async fn test_full_rag_pipeline() -> Result<(), Box<dyn std::error::Error>> {
    let test_db = Path::new("test_full_rag.db");
    if test_db.exists() {
        std::fs::remove_file(test_db)?;
    }
    
    let db = Database::init(test_db)?;
    
    // 1. Ingest
    let paper = Paper {
        id: "2401.0001".into(),
        title: "Test Paper".into(),
        authors: vec![Author { name: "A".into(), affiliations: vec![] }],
        abstract_text: "Abstract".into(),
        categories: vec![],
        published: "2024".into(),
        url: "".into(),
        doi: None,
        journal_ref: None,
    };
    
    let prepared = prepare_paper(
        paper.clone(),
        "test",
        "This is the methods section of the paper.",
        PreparationOptions {
            segmentation_k: Some(1.2),
            ..Default::default()
        }
    );
    
    db.store_paper(&paper.id, &paper.title, &paper.abstract_text)?;
    for chunk in &prepared.chunks {
        let id = format!("{}-{}", paper.id, chunk.index);
        db.store_chunk(&id, &paper.id, &chunk.text, Some(&[0.1; 384]), chunk.cluster_id.as_deref())?;
    }
    
    // 2. Route
    let routed = db.route_documents("Test", 1)?;
    assert_eq!(routed.len(), 1);
    assert_eq!(routed[0], "2401.0001");
    
    // 3. Scoped Retrieval
    let chunks = db.retrieve_chunks_scoped("methods", &routed, 1)?;
    assert_eq!(chunks.len(), 1);
    assert!(chunks[0].1.contains("methods"));
    
    std::fs::remove_file(test_db)?;
    Ok(())
}
