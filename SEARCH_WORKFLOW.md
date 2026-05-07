# Search & Discovery Workflow: Find Papers Before Downloading

Use fuzzy search to find relevant papers, then download only what you need. This saves 99% on S3 costs.

## The Problem with "Download All"

- **Full corpus**: 2.3M papers, 5.3 TB, $635 in S3 costs
- **Your actual need**: Maybe 1-5% are relevant to your domain

## The Solution: Search First, Download Later

1. **Search metadata** (free) → Find 1K-10K relevant papers
2. **Download only matches** (cheap) → $2-20 instead of $635

---

## Search Presets (Recommended)

Pre-built queries for your infrastructure domains:

```rust
use arxiv_search_rs_mcp_core::search::presets;

// One-liner queries
let ddos_papers = presets::ddos_prevention();
let siem_papers = presets::siem_soar();
let vhost_papers = presets::virtual_hosting();
let storage_papers = presets::storage_optimization();

// Combined: everything relevant to your stack
let everything = presets::your_stack();
```

**Presets include:**
- Keywords (title/abstract search)
- Categories (arXiv subject classifications)
- Minimum relevance threshold (0.5-0.7)

---

## Custom Queries

Build queries from scratch:

```rust
use arxiv_search_rs_mcp_core::search::QueryBuilder;

let query = QueryBuilder::new()
    .keywords(&["ddos", "attack detection", "network security"])
    .categories(&["cs.NI", "cs.CR"])  // Networking & Security
    .exclude("machine learning")       // Skip ML papers (optional)
    .min_relevance(0.7)               // High confidence only
    .build();
```

### Available Categories

| Code | Topic |
|------|-------|
| `cs.NI` | Networking & Internet Architecture |
| `cs.CR` | Cryptography & Security |
| `cs.DC` | Distributed, Parallel, Cluster Computing |
| `cs.SY` | Systems & Control |
| `cs.OS` | Operating Systems |
| `cs.SE` | Software Engineering |

---

## Workflow: Search → Download → Embed

### Step 1: Get Metadata (Choose One)

#### Option A: OAI-PMH (Real-time, Free)

arXiv's official API for metadata harvesting:

```rust
// Coming: OAI-PMH integration module
// For now: use the arXiv API directly
// Example: fetch all papers from past 30 days
let metadata = fetch_from_oai_pmh(
    "2024-12-01",  // from date
    &["cs.NI", "cs.CR"],  // categories
).await?;
```

**Pros:**
- Real-time (latest papers)
- Free
- Official, no rate limits

**Cons:**
- Slower (sequential API calls)
- 4 requests/second limit

#### Option B: Kaggle Dataset (Bulk, Free)

Pre-indexed arXiv metadata:

```rust
// Download kaggle dataset ~4GB (one-time)
// then load locally
let metadata = load_kaggle_arxiv_dataset("./arxiv-metadata.json")?;
```

**Pros:**
- Fast (local JSON, instant search)
- Pre-processed
- Everything at once

**Cons:**
- 3 months behind
- Requires initial 4GB download

**Setup:**
```bash
# Get Kaggle credentials from https://www.kaggle.com/settings/account
kaggle datasets download -d Cornell-University/arxiv

# Extract metadata
unzip arxiv
head -1000 arxiv_metadata.json > sample.json  # ~1000 papers
```

### Step 2: Search Using Filter

```rust
use arxiv_search_rs_mcp_core::search::{SearchFilter, presets};

// Load metadata (from either source)
let papers = load_metadata().await?;

// Create search query
let query = presets::ddos_prevention();
let filter = SearchFilter::new(query);

// Search: returns papers with relevance scores
let matches = filter.search(&papers);
println!("Found {} matching papers", matches.len());

// Rank by relevance (highest first)
let ranked = filter.rank(matches);

// Display results
for (i, (paper, score)) in ranked.iter().take(20).enumerate() {
    println!(
        "{}. [{}] {} (score: {:.2})",
        i + 1, paper.arxiv_id, paper.title, score
    );
}
```

### Step 3: Download from S3

```rust
use arxiv_search_rs_mcp_core::ingestion::{S3Downloader, S3Config};

let downloader = S3Downloader::new(S3Config::default()).await?;

// Extract S3 keys from search results
let keys: Vec<&str> = ranked.iter()
    .map(|(paper, _)| {
        let key = PaperMetadata::s3_key_from_arxiv_id(&paper.arxiv_id);
        // Store for this batch
        key
    })
    .collect();

// Download in parallel
let results = downloader
    .download_papers_parallel(keys, &PathBuf::from("./papers"))
    .await?;

println!("Downloaded {} papers", results.iter().filter(|(_, r)| r.is_ok()).count());
```

### Step 4: Process → Embed → Index

```rust
// Use existing modules
use arxiv_search_rs_mcp_core::pdf::extract_text;
use arxiv_search_rs_mcp_core::{PaperChunk, PreparationOptions};

for (paper, _score) in ranked.iter() {
    // Extract text from PDF
    let text = extract_text(&pdf_bytes)?;
    
    // Chunk for embedding
    let options = PreparationOptions::default();
    let chunks = prepare_for_embedding(&text, &options)?;
    
    // Store in vector DB (Qdrant, Milvus, etc.)
    for chunk in chunks {
        vector_db.insert(&chunk).await?;
    }
}
```

---

## Cost Comparison

### Scenario: Build SIEM/DDoS/Networking Knowledge Base

**Search Results**: ~3,500 relevant papers (from 2.3M total)

#### Old Way (Download Everything)
```
S3: 5.3 TB × $0.12/GB = $635
EC2: 200 concurrent, 3 days = ~$15
Total: $650
```

#### Smart Way (Search First)
```
Kaggle metadata: $0 (or $4 for one-time 4GB download)
S3: 3,500 papers × 2.3 MB avg = 8 GB × $0.12 = $0.96
EC2: 50 concurrent, 3 hours = ~$0.50
Total: $1.46
```

**Savings: 99.8% ($650 → $1.46)**

---

## Search Quality by Min Relevance Threshold

Adjust `min_relevance` to balance precision vs recall:

| Threshold | Papers | Precision | Use Case |
|-----------|--------|-----------|----------|
| 0.3 | 10K+ | Low | Exploratory, broad research |
| 0.5 | 5K–10K | Medium | Good balance (recommended) |
| 0.7 | 1K–3K | High | Focused domain experts |
| 0.9 | 100–500 | Very High | Exact matches only |

---

## Advanced: Combined Queries

Chain multiple searches:

```rust
// Find papers on BOTH DDoS AND storage optimization
let ddos = presets::ddos_prevention();
let storage = presets::storage_optimization();

let ddos_papers = filter_ddos.search(&papers);
let storage_papers = filter_storage.search(&papers);

// Intersection: papers relevant to both
let intersection: Vec<_> = ddos_papers.iter()
    .filter(|(p, _)| storage_papers.iter().any(|(sp, _)| sp.arxiv_id == p.arxiv_id))
    .collect();

println!("Found {} papers on BOTH DDoS AND storage", intersection.len());
```

---

## Integration Points

### With Native CLI

```bash
# (Coming) Search from command line
cargo run --example search_and_download --features s3 -- \
  --preset ddos_prevention \
  --top 100 \
  --download \
  --concurrent 20

# (Coming) Custom query
cargo run --example search_and_download --features s3 -- \
  --keywords "virtual hosting,kubernetes" \
  --categories "cs.DC,cs.SY" \
  --min-relevance 0.6
```

### With MCP Server

```rust
// Your MCP tool interface
#[mcp::tool]
async fn search_papers(query: String) -> Result<Vec<PaperMatch>> {
    let q = QueryBuilder::new().keywords(&[&query]).build();
    let filter = SearchFilter::new(q);
    let results = filter.search(&load_metadata().await?);
    Ok(results.into_iter().map(|(p, s)| PaperMatch { paper: p, score: s }).collect())
}

#[mcp::tool]
async fn get_paper_text(arxiv_id: String) -> Result<String> {
    // Download from S3, extract text, return
    let bytes = downloader.download_paper(&arxiv_id).await?;
    let text = extract_text(&bytes)?;
    Ok(text)
}
```

---

## Debugging & Tuning

### See Why Papers Match/Don't Match

```rust
let query = presets::ddos_prevention();
let paper = load_one_paper().await?;

let score = paper.relevance_score(&query);
println!("Paper: {}", paper.title);
println!("Score: {:.2}", score);
println!("Keywords matched: {:?}", 
    query.keywords.iter()
    .filter(|kw| paper.title.to_lowercase().contains(kw))
    .collect::<Vec<_>>()
);
```

### Adjust Threshold Dynamically

```rust
for min_relevance in [0.3, 0.5, 0.7, 0.9] {
    let mut q = presets::your_stack();
    q.min_relevance = min_relevance;
    
    let results = filter.search(&papers);
    println!("Threshold {}: {} papers", min_relevance, results.len());
}
```

---

## Next: Metadata Integration

Currently, search logic is ready but needs data source integration:

- [ ] **OAI-PMH API client** — Real-time paper discovery
- [ ] **Kaggle dataset loader** — Bulk metadata import
- [ ] **Caching layer** — Store metadata locally for fast re-searches
- [ ] **CLI search tool** — User-friendly command-line interface
- [ ] **MCP search tool** — Query interface for agents

---

## Example: Full Workflow

```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use arxiv_search_rs_mcp_core::search::{SearchFilter, presets};
    use arxiv_search_rs_mcp_core::ingestion::{S3Downloader, S3Config};
    use std::path::PathBuf;

    // 1. Load metadata
    println!("📚 Loading arXiv metadata...");
    let papers = load_papers_from_kaggle().await?;
    println!("Loaded {} papers", papers.len());

    // 2. Search using preset
    println!("\n🔍 Searching for DDoS papers...");
    let query = presets::ddos_prevention();
    let filter = SearchFilter::new(query);
    let matches = filter.search(&papers);
    let ranked = filter.rank(matches);
    println!("Found {} matching papers", ranked.len());

    // 3. Show top results
    println!("\n📄 Top 10 results:");
    for (i, (paper, score)) in ranked.iter().take(10).enumerate() {
        println!("  {}. [{}] {} ({:.2})", i + 1, paper.arxiv_id, paper.title, score);
    }

    // 4. Download top papers
    println!("\n📥 Downloading top 50 papers...");
    let keys: Vec<_> = ranked.iter()
        .take(50)
        .map(|(p, _)| p.arxiv_id.as_str())
        .collect();

    let mut config = S3Config::default();
    config.max_concurrent_downloads = 20;
    let downloader = S3Downloader::new(config).await?;
    
    let results = downloader
        .download_papers_parallel(keys, &PathBuf::from("./ddos-papers"))
        .await?;

    let succeeded = results.iter().filter(|(_, r)| r.is_ok()).count();
    println!("✅ Downloaded {} papers", succeeded);

    // 5. Process (extract text, chunk, embed, etc.)
    println!("\n⚙️  Processing papers...");
    process_papers(&PathBuf::from("./ddos-papers")).await?;

    println!("\n✨ Done! Papers ready for embedding & search.");
    Ok(())
}
```

---

**Key Insight**: Search metadata is free. Only pay for what's relevant.

Start with a preset, refine the query, download a sample, then scale up.
