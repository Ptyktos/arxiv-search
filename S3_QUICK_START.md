# S3 Integration Quick Start

One-minute setup to download arXiv papers from AWS S3.

## 1. Enable Feature

Add to your project's `Cargo.toml`:

```toml
[dependencies]
arxiv-search-rs-mcp-core = { path = "./crates/core", features = ["s3"] }
```

Or add the dependency:

```bash
cargo add arxiv-search-rs-mcp-core --features s3
```

## 2. Set AWS Credentials

```bash
# Option A: AWS CLI (recommended)
aws configure

# Option B: Environment variables
export AWS_ACCESS_KEY_ID="your-key"
export AWS_SECRET_ACCESS_KEY="your-secret"
export AWS_REGION="us-east-1"
```

**Get credentials:** Sign up at https://aws.amazon.com (free tier available).

## 3. List Papers

```rust
use arxiv_search_rs_mcp_core::ingestion::S3Downloader;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let downloader = S3Downloader::new(S3Config::default()).await?;
    let papers = downloader.list_papers(100).await?;
    
    for paper in papers {
        println!("{}", paper);
    }
    Ok(())
}
```

## 4. Download Papers

```rust
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let downloader = S3Downloader::new(S3Config::default()).await?;
    
    let papers = downloader.list_papers(100).await?;
    let keys: Vec<&str> = papers.iter().map(|p| p.as_str()).collect();
    
    let output_dir = PathBuf::from("./papers");
    tokio::fs::create_dir_all(&output_dir).await?;
    
    let results = downloader.download_papers_parallel(keys, &output_dir).await?;
    
    let succeeded = results.iter().filter(|(_, r)| r.is_ok()).count();
    println!("Downloaded {} papers", succeeded);
    Ok(())
}
```

## 5. Try the Example

```bash
# List papers
cargo run --example s3_downloader --features s3 -- --list 50

# Download 100 papers from January 2024
cargo run --example s3_downloader --features s3 -- \
  --download pdf/2401 \
  --output ./papers \
  --concurrent 20

# Estimate cost (84K papers, avg 2.3 MB each)
cargo run --example s3_downloader --features s3 -- \
  --estimate 84000 2
```

## Configuration

Customize S3 behavior:

```rust
let mut config = S3Config::default();
config.max_concurrent_downloads = 50;    // Increase throughput
config.prefix = Some("pdf/2024".to_string()); // Download 2024 papers only
config.chunk_size_mb = 20;                // 20MB chunks (for resumable downloads)

let downloader = S3Downloader::new(config).await?;
```

## Cost

| Dataset | Size | Cost |
|---------|------|------|
| Last 30 days | 16 GB | $2 |
| Last 6 months | 98 GB | $12 |
| Last 1 year | 195 GB | $23 |
| Full corpus | 5.3 TB | $635 |

**See COST_MATRIX.md for detailed breakdown.**

## Files

- **S3_INTEGRATION_GUIDE.md** — Complete setup & architecture guide
- **COST_MATRIX.md** — Detailed cost analysis & scenarios
- **crates/core/src/ingestion/s3.rs** — Implementation
- **crates/native/examples/s3_downloader.rs** — Executable example

## Next Steps

1. Download papers to local disk (done ✓)
2. Extract text with `crates/core/src/pdf.rs`
3. Chunk with `crates/core/src/content.rs`
4. Embed with your LLM
5. Store in vector database (Qdrant, Milvus)
6. Query via MCP tools

## Troubleshooting

**"Access Denied"**
→ Check AWS credentials: `aws sts get-caller-identity`

**Slow downloads**
→ Increase `max_concurrent_downloads` to 50–100

**High costs**
→ Download only specific categories/dates using `prefix` option

**Want zero cost?**
→ Use [Google Cloud + Kaggle](S3_INTEGRATION_GUIDE.md#alternative-google-cloud-kaggle-dataset) instead

---

**Full guide:** See S3_INTEGRATION_GUIDE.md
