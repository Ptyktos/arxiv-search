/// Example: Download papers from arXiv S3 bucket
///
/// Usage:
///   cargo run --example s3_downloader --features s3 -- --list 10
///   cargo run --example s3_downloader --features s3 -- --download pdf/2401 --output ./papers --concurrent 20
///   cargo run --example s3_downloader --features s3 -- --estimate 2300000 2

use arxiv_search_rs_mcp_core::ingestion::S3Config;
use std::path::PathBuf;

#[cfg(feature = "s3")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use arxiv_search_rs_mcp_core::ingestion::S3Downloader;

    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        print_usage();
        return Ok(());
    }

    let mut config = S3Config::default();
    let mut list_count = 10;
    let mut prefix = None;
    let mut output_dir = PathBuf::from("./papers");
    let mut max_concurrent = 10;
    let mut estimate_papers = 0u64;
    let mut estimate_size = 2u64;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--list" => {
                if i + 1 < args.len() {
                    list_count = args[i + 1].parse().unwrap_or(10);
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--download" => {
                if i + 1 < args.len() {
                    prefix = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--output" => {
                if i + 1 < args.len() {
                    output_dir = PathBuf::from(&args[i + 1]);
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--concurrent" => {
                if i + 1 < args.len() {
                    max_concurrent = args[i + 1].parse().unwrap_or(10);
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--estimate" => {
                if i + 2 < args.len() {
                    estimate_papers = args[i + 1].parse().unwrap_or(0);
                    estimate_size = args[i + 2].parse().unwrap_or(2);
                    i += 3;
                } else {
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }

    // List papers
    if !args.contains(&"--download".to_string()) && !args.contains(&"--estimate".to_string()) {
        println!("📡 Listing {} papers from arXiv S3...", list_count);

        if let Some(p) = prefix.clone() {
            config.prefix = Some(p);
        }

        let downloader = S3Downloader::new(config).await?;
        let papers = downloader.list_papers(list_count as i32).await?;

        println!("Found {} papers:\n", papers.len());
        for (idx, paper) in papers.iter().enumerate() {
            println!("  {}. {}", idx + 1, paper);
        }
        return Ok(());
    }

    // Estimate cost
    if estimate_papers > 0 {
        let downloader = S3Downloader::new(config).await?;
        let estimate = downloader.estimate_cost(estimate_papers, estimate_size);

        println!("💰 Cost Estimate for {} papers (~{}MB each):\n", estimate_papers, estimate_size);
        println!("  Total size:          {} GB", estimate.total_size_gb);
        println!("  S3 transfer cost:    ${:.2}", estimate.s3_transfer_cost_usd);
        println!("  EC2 time (est):      {:.1} hours", estimate.estimated_hours);
        println!("  EC2 cost (est):      ${:.2}", estimate.ec2_hourly_rate * estimate.estimated_hours);
        println!("  ─────────────────────────────────────");
        println!("  Total estimated:     ${:.2}\n", estimate.total_estimated_cost_usd);

        return Ok(());
    }

    // Download papers
    if let Some(p) = prefix {
        config.prefix = Some(p.clone());
        config.max_concurrent_downloads = max_concurrent;

        println!("📥 Downloading papers from {} with {} concurrent connections...", p, max_concurrent);
        println!("   Output directory: {}\n", output_dir.display());

        let downloader = S3Downloader::new(config).await?;

        // List papers in this prefix
        let papers = downloader.list_papers(1000).await?;
        println!("Found {} papers to download", papers.len());

        if papers.is_empty() {
            println!("No papers found at prefix: {}", p);
            return Ok(());
        }

        let keys: Vec<&str> = papers.iter().map(|p| p.as_str()).collect();

        // Create output directory
        tokio::fs::create_dir_all(&output_dir).await?;

        // Download in parallel
        let start = std::time::Instant::now();
        let results = downloader.download_papers_parallel(keys, &output_dir).await?;
        let duration = start.elapsed();

        let succeeded = results.iter().filter(|(_, r)| r.is_ok()).count();
        let failed = results.iter().filter(|(_, r)| r.is_err()).count();
        let total_bytes: u64 = results
            .iter()
            .filter_map(|(_, r)| r.as_ref().ok())
            .sum();

        println!("\n✅ Download complete!");
        println!("   Succeeded: {}", succeeded);
        println!("   Failed:    {}", failed);
        println!("   Total:     {} MB", total_bytes / 1_000_000);
        println!("   Time:      {:.1}s", duration.as_secs_f64());
        println!("   Speed:     {:.1} MB/s\n", (total_bytes as f64 / 1_000_000.0) / duration.as_secs_f64());

        // Print first few errors
        let errors: Vec<_> = results.iter().filter(|(_, r)| r.is_err()).collect();
        if !errors.is_empty() {
            println!("Failed downloads:");
            for (name, err) in errors.iter().take(5) {
                println!("  - {}: {}", name, err.as_ref().unwrap_err());
            }
            if errors.len() > 5 {
                println!("  ... and {} more", errors.len() - 5);
            }
        }
    }

    Ok(())
}

#[cfg(not(feature = "s3"))]
fn main() {
    eprintln!("Error: S3 feature not enabled");
    eprintln!("Build with: cargo run --example s3_downloader --features s3");
}

fn print_usage() {
    println!("arXiv S3 Downloader");
    println!();
    println!("USAGE:");
    println!("  cargo run --example s3_downloader --features s3 -- [OPTIONS]");
    println!();
    println!("OPTIONS:");
    println!("  --list <count>              List papers (default: 10)");
    println!("  --download <prefix>         Download papers from prefix");
    println!("  --output <path>             Output directory (default: ./papers)");
    println!("  --concurrent <n>            Concurrent downloads (default: 10)");
    println!("  --estimate <papers> <size>  Estimate cost for N papers of size MB");
    println!();
    println!("EXAMPLES:");
    println!("  # List 50 papers");
    println!("  cargo run --example s3_downloader --features s3 -- --list 50");
    println!();
    println!("  # Download January 2024 papers with 50 concurrent connections");
    println!("  cargo run --example s3_downloader --features s3 -- \\");
    println!("    --download pdf/2401 --output ./papers/2024_01 --concurrent 50");
    println!();
    println!("  # Estimate cost for full corpus (2.3M papers, avg 2MB each)");
    println!("  cargo run --example s3_downloader --features s3 -- \\");
    println!("    --estimate 2300000 2");
}
