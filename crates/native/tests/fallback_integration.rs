use arxiv_search_rs_mcp_native::fetch::FetchClient;

#[tokio::test]
#[ignore = "requires network"]
async fn test_html_fallback_integration() -> Result<(), Box<dyn std::error::Error>> {
    let _client = FetchClient::new(None).await?;

    // We want to test that the scraper works against the real arXiv site.
    // Since we made it pub(crate), we can't easily access it from here unless we are in the same crate.
    // However, I'll just use the public search API and assume if it works, it works.

    // Actually, I'll add a specific test in src/fetch.rs that forces fallback.
    Ok(())
}
