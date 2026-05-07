/// Example: Search for papers and download only relevant ones from S3
///
/// Usage:
///   cargo run --example search_and_download --features s3 -- --query "ddos prevention" --top 50
///   cargo run --example search_and_download --features s3 -- --preset networking --concurrent 20
///   cargo run --example search_and_download --features s3 -- --list-presets

use arxiv_search_rs_mcp_core::search::presets;
use arxiv_search_rs_mcp_core::search::{QueryBuilder, SearchFilter, PaperMetadata};

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        print_usage();
        return;
    }

    match args[1].as_str() {
        "--list-presets" => list_presets(),
        "--help" => print_usage(),
        _ => {
            eprintln!("Comprehensive search example coming with OAI-PMH/Kaggle metadata integration.");
            eprintln!();
            eprintln!("For now, use:");
            eprintln!("  cargo run --example search_and_download -- --list-presets");
        }
    }
}

fn list_presets() {
    println!("Available search presets for your infrastructure stack:\n");

    let presets_list = vec![
        (
            "networking",
            "Papers on network architecture, protocols, routing, bandwidth optimization",
            presets::networking(),
        ),
        (
            "ddos_prevention",
            "DDoS detection, attack prevention, anomaly detection, rate limiting",
            presets::ddos_prevention(),
        ),
        (
            "siem_soar",
            "SIEM, SOAR, incident response, threat detection, log analysis",
            presets::siem_soar(),
        ),
        (
            "virtual_hosting",
            "Virtualization, hypervisors, containers, Kubernetes, resource allocation",
            presets::virtual_hosting(),
        ),
        (
            "storage_optimization",
            "Distributed storage, replication, caching, deduplication, tiering",
            presets::storage_optimization(),
        ),
        (
            "infrastructure_optimization",
            "Performance tuning, scalability, resource efficiency, profiling, monitoring",
            presets::infrastructure_optimization(),
        ),
        (
            "your_stack",
            "Composite: everything relevant to your infrastructure (recommended)",
            presets::your_stack(),
        ),
    ];

    for (name, description, query) in presets_list {
        println!("📌 {}", name);
        println!("   Description: {}", description);
        println!("   Keywords: {}", query.keywords.join(", "));
        println!("   Categories: {}", query.categories.join(", "));
        println!("   Min relevance: {}", query.min_relevance);
        println!();
    }

    println!("Usage in Rust:");
    println!();
    println!("  use arxiv_search_rs_mcp_core::search::{{QueryBuilder, SearchFilter, presets}};");
    println!();
    println!("  // Use a preset");
    println!("  let query = presets::ddos_prevention();");
    println!("  let filter = SearchFilter::new(query);");
    println!();
    println!("  // Custom query");
    println!("  let query = QueryBuilder::new()");
    println!("    .keywords(&[\"virtual hosting\", \"kubernetes\"])");
    println!("    .categories(&[\"cs.DC\", \"cs.SY\"])");
    println!("    .min_relevance(0.6)");
    println!("    .build();");
    println!();
    println!("  // Search papers");
    println!("  let results = filter.search(&papers);");
    println!("  let ranked = filter.rank(results);");
}

fn print_usage() {
    println!("Search and Download: Find arXiv papers relevant to your infrastructure");
    println!();
    println!("USAGE:");
    println!("  cargo run --example search_and_download --features s3 -- [OPTIONS]");
    println!();
    println!("OPTIONS:");
    println!("  --list-presets     List all available search presets");
    println!("  --help              Show this help message");
    println!();
    println!("WORKFLOW:");
    println!("  1. List presets:          --list-presets");
    println!("  2. (Coming) Search via OAI-PMH or Kaggle metadata");
    println!("  3. (Coming) Download only relevant papers from S3");
    println!("  4. Extract text, chunk, embed, search");
    println!();
    println!("INTEGRATION:");
    println!("  This example will integrate with:");
    println!("  - OAI-PMH API for metadata (list papers without downloading)");
    println!("  - Kaggle arXiv dataset (pre-indexed metadata)");
    println!("  - S3 downloader for parallel high-throughput download");
    println!();
    println!("EXPECTED WORKFLOW:");
    println!();
    println!("  // 1. Get metadata (OAI-PMH or Kaggle)");
    println!("  let metadata = fetch_metadata_from_kaggle().await?;");
    println!();
    println!("  // 2. Search using preset");
    println!("  let query = presets::ddos_prevention();");
    println!("  let filter = SearchFilter::new(query);");
    println!("  let matching = filter.search(&metadata);");
    println!("  let ranked = filter.rank(matching);");
    println!();
    println!("  // 3. Extract S3 keys and download");
    println!("  let keys: Vec<_> = ranked.iter()");
    println!("    .map(|(paper, _score)| paper.s3_key.as_ref().unwrap().as_str())");
    println!("    .collect();");
    println!();
    println!("  let results = downloader.download_papers_parallel(keys, &output_dir).await?;");
    println!();
    println!("Cost example:");
    println!("  • Search 2.3M papers: FREE (metadata only)");
    println!("  • Download 5K matching papers (~12 GB): $1.44");
    println!("  • Total: $1.44 instead of $635 for full corpus");
}
