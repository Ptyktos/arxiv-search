use arxiv_search_rs_mcp_core::content::{prepare_paper, PreparationOptions};
use arxiv_search_rs_mcp_core::paper::{Author, Paper};
use arxiv_search_rs_mcp_native::db::Database;
use arxiv_search_rs_mcp_native::fetch::FetchClient;
use arxiv_search_rs_mcp_native::tool::{ArxivServer, RetrieveInput};
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
        authors: vec![Author {
            name: "A".into(),
            affiliations: vec![],
        }],
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
        },
    );

    db.store_paper(&paper.id, &paper.title, &paper.abstract_text)?;
    for chunk in &prepared.chunks {
        let id = format!("{}-{}", paper.id, chunk.index);
        db.store_chunk(
            &id,
            &paper.id,
            &chunk.text,
            Some(&[0.1; 384]),
            chunk.cluster_id.as_deref(),
        )?;
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

#[tokio::test]
#[ignore = "requires network"]
async fn test_run_retrieve_stores_real_metadata() -> Result<(), Box<dyn std::error::Error>> {
    let client = FetchClient::new(None).await?;
    let server = ArxivServer::new(client);

    let input = RetrieveInput {
        paper_id: "2206.06912".into(),
        prune_references: true,
        chunk_chars: 4000,
        chunk_overlap: 200,
        segmentation_k: None,
    };

    let result = server.run_retrieve(input).await?;
    let obj = result.as_object().ok_or("result not an object")?;
    let paper = obj.get("paper").ok_or("missing paper field")?;
    let title = paper
        .get("title")
        .and_then(|t| t.as_str())
        .ok_or("missing title")?;
    assert!(
        !title.starts_with("2206.06912"),
        "title should be real, not the paper ID"
    );
    assert!(
        title.contains("Octonion") || title.contains("Standard Model"),
        "title '{}' should contain Octonion or Standard Model",
        title
    );

    Ok(())
}

#[tokio::test]
#[ignore = "requires network"]
async fn test_segmentation_k_produces_hierarchical_chunks() -> Result<(), Box<dyn std::error::Error>>
{
    let client = FetchClient::new(None).await?;
    let server = ArxivServer::new(client);

    let input = RetrieveInput {
        paper_id: "2206.06912".into(),
        prune_references: true,
        chunk_chars: 4000,
        chunk_overlap: 200,
        segmentation_k: Some(1.2),
    };

    let result = server.run_retrieve(input).await?;
    let obj = result.as_object().ok_or("result not an object")?;

    let hier = obj
        .get("hierarchical_chunks")
        .ok_or("missing hierarchical_chunks")?;
    assert!(!hier.is_null(), "hierarchical_chunks should not be null");
    let hier_arr = hier.as_array().ok_or("hierarchical_chunks not an array")?;
    assert!(
        !hier_arr.is_empty(),
        "hierarchical_chunks should be non-empty"
    );

    for chunk in hier_arr {
        let emb = chunk
            .get("cluster_embedding")
            .and_then(|e| e.as_array())
            .ok_or("missing or invalid cluster_embedding")?;
        assert!(!emb.is_empty(), "cluster_embedding should be non-empty");
    }

    Ok(())
}
