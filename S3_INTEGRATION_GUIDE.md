# arXiv S3 Bulk Download Integration

Complete guide for using AWS S3 Requester Pays bucket to download arXiv papers at maximum throughput.

## Overview

arXiv hosts their entire corpus (~2.3M papers, ~5TB) on AWS S3 specifically for bulk access. This is the **official** and fastest way to access the complete dataset.

- **Speed**: Unlimited concurrency (AWS network speeds)
- **Cost**: You pay AWS data transfer fees (typically $0.12/GB)
- **Rate Limits**: None (S3 has no rate limiting for requester-pays buckets)

## Setup

### 1. AWS Credentials

Create AWS credentials with S3 read access:

```bash
# Option A: Use AWS CLI
aws configure

# Option B: Set environment variables
export AWS_ACCESS_KEY_ID="your-access-key"
export AWS_SECRET_ACCESS_KEY="your-secret-key"
export AWS_REGION="us-east-1"
```

### 2. Enable Feature Flag

Add to your `Cargo.toml`:

```toml
[dependencies]
arxiv-search-rs-mcp-core = { path = "./crates/core", features = ["s3"] }
```

Or in a binary:

```bash
cargo build --features s3
```

### 3. IAM Policy (Minimal)

Attach this policy to your AWS user for minimal permissions:

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Action": [
        "s3:GetObject",
        "s3:ListBucket"
      ],
      "Resource": [
        "arn:aws:s3:::arxiv",
        "arn:aws:s3:::arxiv/*"
      ]
    }
  ]
}
```

## Usage

### Basic Example: List Papers

```rust
use arxiv_search_rs_mcp_core::ingestion::S3Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = S3Config::default(); // Uses arxiv bucket
    let downloader = S3Downloader::new(config).await?;

    // List first 100 papers
    let papers = downloader.list_papers(100).await?;
    println!("Found {} papers", papers.len());

    for paper in papers {
        println!("{}", paper);
    }

    Ok(())
}
```

### Download Single Paper

```rust
let key = "pdf/2401/2401.00001v1.pdf"; // arXiv paper key
let output_path = PathBuf::from("./papers/2401.00001v1.pdf");

let bytes = downloader.download_paper(key, &output_path).await?;
println!("Downloaded {} bytes", bytes);
```

### Parallel Downloads (High Throughput)

```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = S3Config::default();
    config.max_concurrent_downloads = 50; // Increase for higher throughput
    
    let downloader = S3Downloader::new(config).await?;

    // Get list of papers to download
    let papers = downloader.list_papers(10000).await?;
    let keys: Vec<&str> = papers.iter().map(|p| p.as_str()).collect();

    // Create output directory
    let output_dir = PathBuf::from("./papers");
    tokio::fs::create_dir_all(&output_dir).await?;

    // Download in parallel
    let results = downloader
        .download_papers_parallel(keys, &output_dir)
        .await?;

    // Check results
    let succeeded = results.iter().filter(|(_, r)| r.is_ok()).count();
    let failed = results.iter().filter(|(_, r)| r.is_err()).count();

    println!("Downloaded: {}, Failed: {}", succeeded, failed);

    Ok(())
}
```

### Cost Estimation

```rust
let config = S3Config::default();
let downloader = S3Downloader::new(config).await?;

// Estimate cost for downloading all 2.3M papers (~5TB, avg 2.2MB per paper)
let estimate = downloader.estimate_cost(2_300_000, 2);
println!("Estimated cost: ${:.2}", estimate.total_estimated_cost_usd);
println!("S3 transfer: ${:.2}", estimate.s3_transfer_cost_usd);
println!("EC2 time (if running on instance): ${:.2}", estimate.s3_transfer_cost_usd);
```

## S3 Bucket Structure

arXiv S3 bucket paths follow this pattern:

```
s3://arxiv/
  pdf/
    2401/  (year-month)
      2401.00001v1.pdf
      2401.00001v2.pdf
      2401.00002v1.pdf
    2312/
      ...
```

**Common prefixes:**
- `pdf/` — All PDF files (searchable by date)
- `src/` — Source files (LaTeX, code, etc.)

Use the `prefix` option in `S3Config` to filter by path:

```rust
let mut config = S3Config::default();
config.prefix = Some("pdf/2401".to_string()); // Only Jan 2024 papers
```

## Cost Matrix

### Complete Corpus Download (All 2.3M papers, ~5TB)

| Scenario | Throughput | Duration | S3 Cost | EC2 Cost* | Total |
|----------|-----------|----------|---------|----------|-------|
| **Standard** (10 concurrent) | ~10 MB/s | 58 days | $600 | ~$70 | **$670** |
| **High** (50 concurrent) | ~50 MB/s | 12 days | $600 | ~$14 | **$614** |
| **Maximum** (200 concurrent) | ~200 MB/s | 3 days | $600 | ~$4 | **$604** |
| **On-Premises** | ~100 MB/s | 6 days | $600 | $0** | **$600** |

*EC2 instance cost assumes t3.xlarge ($0.1664/hr). Your actual costs vary by instance type.  
**If running on existing infrastructure.

### Subset Downloads

| Dataset | Size | Papers | S3 Cost | Notes |
|---------|------|--------|---------|-------|
| Last 30 days | ~70 GB | ~32K | $8 | Great for recent papers + SME training |
| Last 6 months | ~420 GB | ~191K | $50 | Good balance: covers trends + breadth |
| Last year | ~840 GB | ~382K | $101 | Solid foundation for most domains |
| 2-year window | ~1.7 TB | ~764K | $204 | Industry standard for research |
| Full corpus | ~5 TB | ~2.3M | $600 | Complete knowledge base |

### Cost Breakdown Example (1TB subset)

```
S3 Data Transfer:     $120  (1TB × $0.12/GB)
EC2 t3.xlarge:        $40   (~10 hours @ $0.1664/hr)
─────────────────────
Total:                $160
```

## Alternative: Google Cloud Kaggle Dataset

If you want to **avoid S3 costs entirely**:

1. Use the **Kaggle arXiv Dataset** (free)
2. Access via `gs://arxiv-dataset` in Google Cloud
3. Zero transfer cost if in same region

```bash
# List available files
gsutil ls gs://arxiv-dataset/arxiv/

# Download subset (free within Google Cloud)
gsutil -m cp gs://arxiv-dataset/arxiv/pdf/2401/* ./papers/
```

**Cost**: $0 if running on Google Cloud VM in same region (~$0.30/hr for n1-standard-2).

## Optimization Tips

### 1. Filter by Category First

Download only papers in your domain of interest:

```rust
// Only CS papers
config.prefix = Some("pdf/23".to_string()); // 2023 CS papers

// Or use metadata filtering first (via OAI-PMH or Kaggle metadata)
```

### 2. Batch with Prefix Filtering

Instead of downloading all 2.3M:

```
pdf/2024/  → Latest papers
pdf/2023/  → Previous year
pdf/2022/  → Two years back
```

Download only what you need.

### 3. Increase Concurrency for Speed

```rust
config.max_concurrent_downloads = 100;  // Faster, but watch bandwidth limits
```

AWS S3 supports unlimited concurrency, but your ISP/network will be the bottleneck above ~500 Mbps.

### 4. Resume from Failures

Track downloaded papers and skip already-fetched files:

```rust
// Pseudo-code
let downloaded = load_downloaded_list("./downloaded.json")?;
let to_download = papers.iter()
    .filter(|p| !downloaded.contains(p))
    .collect();
```

### 5. Combine with Local Caching

Once downloaded:
1. Extract text + metadata
2. Chunk (using existing `content.rs` module)
3. Embed with your LLM
4. Store in vector DB (Qdrant, Milvus, etc.)

This transforms raw PDFs → searchable SME knowledge base.

## Architecture: Multi-Tier Ingestion

```
S3 Requester Pays (All papers)
        ↓
  Download Layer (Parallel, resumable)
        ↓
  PDF Extraction Layer (Your pdf.rs module)
        ↓
  Content Chunking (Your content.rs module)
        ↓
  Embedding + Vectorization (e.g., Ollama, OpenAI)
        ↓
  Vector DB (Qdrant, Milvus, Pinecone)
        ↓
  Agent Query Layer (Your MCP server)
```

Each layer can run independently, allowing for:
- **Incremental ingestion** (download + process in batches)
- **Cost distribution** (download when cheaper, embed when convenient)
- **Fault tolerance** (resume at any stage)

## Monitoring & Cost Control

### AWS Cost Explorer

Monitor S3 transfer costs in real-time:

```bash
# View S3 costs (requires CloudWatch)
aws ce get-cost-and-usage \
  --time-period Start=2024-01-01,End=2024-01-31 \
  --granularity MONTHLY \
  --metrics BlendedCost \
  --filter file://filter.json
```

### Dry Run (Estimate Before Download)

List papers without downloading:

```rust
let papers = downloader.list_papers(100_000).await?;
let estimate = downloader.estimate_cost(papers.len() as u64, 2);
println!("Estimated cost: ${:.2}", estimate.total_estimated_cost_usd);
```

### Budget Alert

Set AWS billing alert:

```bash
aws budgets create-budget \
  --account-id YOUR_ACCOUNT_ID \
  --budget BudgetName=arxiv-s3,BudgetLimit={Amount=200,Unit=USD}
```

## Troubleshooting

### "Access Denied" Error

```
Error: Access Denied (Code: AccessDenied)
```

**Solution**: Ensure your AWS credentials have `s3:GetObject` and `s3:ListBucket` permissions.

### Slow Download Speed

**Check:**
1. Concurrency: Increase `max_concurrent_downloads` to 50-100
2. Network: Run `speedtest-cli` to check ISP limits
3. AWS region: Ensure you're in a region close to you

### High Costs

**Options:**
1. Use Kaggle dataset instead (Google Cloud, free transfer)
2. Download only specific categories/date ranges
3. Download in batches and delete after processing

## Next Steps

1. **Implement parallel ingestion** — Use S3 downloader in a background job
2. **Add metadata filtering** — Use OAI-PMH or Kaggle metadata to pre-filter papers
3. **Integrate with vector DB** — Process downloaded PDFs → chunks → embeddings → store
4. **Build agent query layer** — Query vector DB via MCP tools
5. **Monitor costs** — Set up AWS billing alerts

---

**Questions?** Check AWS S3 docs or arXiv's official guidance: https://info.arxiv.org/help/bulk_data.html
