# arXiv Ingestion: Cost Matrix & Scenarios

Real-world cost breakdown for different download strategies.

## TL;DR: Quick Cost Reference

| Goal | Method | Cost | Speed | Notes |
|------|--------|------|-------|-------|
| **Single domain (30 days)** | S3 + 10 concurrent | $8–15 | 2–3 days | Great for starting |
| **Industry standard (1 year)** | S3 + 50 concurrent | $120–150 | 2–3 days | Solid SME base |
| **Full corpus** | S3 + 200 concurrent | $600–700 | 3–5 days | Complete knowledge |
| **Zero-cost option** | Google Cloud Kaggle | $0* | 2–5 days | *If on Google Cloud VM |
| **Pre-indexed (no download)** | Kaggle metadata only | $0–10 | Instant | Filter before downloading |

---

## Detailed Cost Breakdown

### Setup Costs (One-time)

| Item | Cost | Notes |
|------|------|-------|
| AWS Account creation | Free | Free tier includes 5GB/month for 12 months |
| S3 bucket access (read-only) | Free | Requester Pays — you only pay for what you download |
| EC2 instance (optional) | $0.05–0.50/hr | Can run on existing infrastructure |
| **Total one-time** | **$0–50** | Mostly optional infrastructure |

### S3 Data Transfer Costs

**arXiv Corpus Stats:**
- **Total papers**: 2,300,000
- **Average size**: 2.0–2.5 MB per paper
- **Total corpus**: ~5 TB
- **AWS rate**: $0.12 per GB (US regions, outbound)

| Scenario | Papers | Size | S3 Cost | Notes |
|----------|--------|------|---------|-------|
| **Last 7 days** | 1,500 | 3 GB | $0.36 | Cutting edge |
| **Last 30 days** | 7,000 | 16 GB | $1.92 | Weekly catch-up |
| **Last 90 days** | 21,000 | 48 GB | $5.76 | Quarterly SME |
| **Last 6 months** | 42,000 | 98 GB | $11.76 | Solid foundation |
| **Last 1 year** | 84,000 | 195 GB | $23.40 | Industry standard |
| **Last 2 years** | 168,000 | 390 GB | $46.80 | Broad coverage |
| **Last 3 years** | 252,000 | 585 GB | $70.20 | Deep history |
| **Last 5 years** | 420,000 | 975 GB | $117.00 | Comprehensive |
| **Full corpus** | 2,300,000 | 5,290 GB (5.3 TB) | **$634.80** | Complete archive |

### Infrastructure Costs (EC2 per Download Session)

Choose an instance based on your network needs:

| Instance Type | vCPU | Memory | Hourly Cost | Throughput | Use Case |
|---------------|------|--------|-------------|-----------|----------|
| **t3.micro** | 1 | 1 GB | $0.0104 | ~5 MB/s | Testing |
| **t3.small** | 2 | 2 GB | $0.0208 | ~15 MB/s | Small batches (≤100 GB) |
| **t3.medium** | 2 | 4 GB | $0.0416 | ~20 MB/s | Medium batches (≤500 GB) |
| **t3.large** | 2 | 8 GB | $0.0832 | ~30 MB/s | Large batches (≤2 TB) |
| **t3.xlarge** | 4 | 16 GB | $0.1664 | ~50 MB/s | Full corpus |
| **c5.2xlarge** | 8 | 16 GB | $0.34 | ~100 MB/s | Maximum throughput |
| **c5.4xlarge** | 16 | 32 GB | $0.68 | ~200 MB/s | Ultra-fast (enterprise) |

**Note:** Throughput is network-limited (AWS can do 1+ Gbps), but bounded by your instance's network interface (typically 10 Gbps for larger instances). Actual throughput depends on S3 latency and your download parallelism.

### Calculation Example: Download 1 Year of arXiv

**Scenario**: Download last 12 months (~84K papers, ~195 GB)

```
S3 Transfer Cost:
  195 GB × $0.12/GB = $23.40

Compute (t3.xlarge, ~4 hours):
  4 hours × $0.1664/hr = $0.67

Network (included in instance cost)

Storage (EBS, temporary):
  195 GB × $0.10/month = ~$0.50

──────────────────────────
Total: $24.57
```

**If running from your own server** (no EC2):
```
S3 Transfer:  $23.40
Compute:      $0 (existing infra)
──────────────────────────
Total: $23.40
```

---

## Cost Scenarios by Goal

### Scenario 1: SME Training (30-day Recent Papers)

**Goal:** Build knowledge base for a specific domain using the latest papers.

```
Papers to download:    ~7,000 (30-day window)
Total size:            ~16 GB
Concurrency:           10 connections

S3 cost:               $1.92
EC2 (t3.small, 1hr):   $0.02
Storage (temp):        $0.02
────────────────────────────
TOTAL:                 $1.96
```

**Throughput:** ~5 MB/s = ~1 hour download time.

**ROI:** Excellent — $2 investment for a month of latest papers.

---

### Scenario 2: Production Knowledge Base (1-Year Window)

**Goal:** Build a production SME system with solid historical context.

```
Papers to download:    ~84,000 (1-year window)
Total size:            ~195 GB
Concurrency:           50 connections

S3 cost:               $23.40
EC2 (t3.xlarge, 4hr):  $0.67
Storage (temp):        $0.50
────────────────────────────
TOTAL:                 $24.57
```

**Throughput:** ~50 MB/s = ~1 hour download time.

**ROI:** Solid — $25 for a production-grade knowledge base covering 12 months.

---

### Scenario 3: Comprehensive Archive (Full Corpus)

**Goal:** Download all of arXiv for maximum flexibility and offline access.

```
Papers to download:    ~2,300,000 (full corpus)
Total size:            ~5.3 TB
Concurrency:           200 connections (maximum)

S3 cost:               $634.80
EC2 (c5.4xlarge, 8hr): $5.44
Storage (temp):        $53.00 (5.3 TB × $0.10/month, allocated)
────────────────────────────
TOTAL:                 $693.24
```

**Throughput:** ~200 MB/s = ~6.5 hours download time.

**ROI:** One-time cost for complete historical access across all domains.

---

## Alternative: Google Cloud + Kaggle (Zero S3 Cost)

**If you use Google Cloud infrastructure:**

```
Kaggle Dataset:        Free (maintained by Cornell University)
Google Cloud Storage:  gs://arxiv-dataset
Transfer cost:         $0 (same region)
GCP Compute (n1-std-2, 10hr): $0.30/hr = $3.00

────────────────────────────
TOTAL:                 $3.00 (vs $634.80 with S3)
```

**Catch:** Data is ~3 months behind arXiv (updated quarterly). Good for stable/historical data; not ideal for latest papers.

---

## Cost Optimization Strategies

### 1. **Category Filtering** (Reduce Download Size)

Instead of all 2.3M papers, filter by arXiv category:

| Category | Estimated Papers | Size | S3 Cost |
|----------|------------------|------|---------|
| cs.LG (Machine Learning) | 180K | 415 GB | $50 |
| cs.AI (AI) | 120K | 270 GB | $32 |
| cs.NLP (NLP) | 85K | 195 GB | $23 |
| math.ST (Statistics) | 65K | 150 GB | $18 |
| Multiple categories | 400K | 900 GB | $108 |

**Savings:** Download only what you need (50K–400K papers vs 2.3M).

### 2. **Time-Window Filtering**

| Window | Papers | Size | S3 Cost | Strategy |
|--------|--------|------|---------|----------|
| Last 30 days | 7K | 16 GB | $2 | Weekly updates |
| Last 90 days | 21K | 48 GB | $6 | Monthly catch-up |
| Last 1 year | 84K | 195 GB | $23 | Quarterly refresh |
| Last 5 years | 420K | 975 GB | $117 | Annual dump |
| All time | 2.3M | 5.3 TB | $635 | One-time setup |

### 3. **Batch Downloads with Checkpointing**

Download in weekly/monthly batches, stop/resume as needed:

```
Week 1: $1–2 (cost of 7K papers)
Week 2: $1–2
...
52 weeks: ~$100 total

Benefits:
- Monitor costs in real-time
- Pause if budget exceeded
- Process papers incrementally
- Easier to parallelize with workers
```

### 4. **Use Pre-Indexed Metadata First**

1. Download metadata only (~4 GB, free on Kaggle)
2. Filter by category/date/citations
3. Download only papers you need

**Cost saved:** If you filter down 80% of papers, you save $127 on S3.

---

## Cost Comparison: S3 vs Alternatives

| Method | Total Size | Download Cost | Speed | Freshness | Access |
|--------|-----------|---|----|----|--------|
| **S3 Requester Pays** | 5.3 TB | $635 | ~6 hrs | Real-time | Unlimited |
| **Google Cloud Kaggle** | 5.3 TB | $0* | ~24 hrs | 3mo lag | GCP only |
| **arXiv OAI-PMH API** | Metadata only | $0 | Instant | Real-time | Public |
| **Local Sync (rsync)** | 5.3 TB | $0** | 7+ days | 1mo lag | Peer CDN |

*Zero transfer if on GCP; free tier compute.  
**Requires peer/collab maintaining sync; can be unreliable.

---

## Monthly Cost Projections

If you download in steady-state increments:

### Option A: Weekly Incremental Downloads

```
Papers per week:  ~7,000
Size per week:    ~16 GB
Cost per week:    $1.92

Monthly:          4 weeks × $1.92 = $7.68
Yearly:           52 weeks × $1.92 = $99.84
```

### Option B: Monthly Full Sync

```
Papers per month:  ~30,000
Size per month:    ~70 GB
Cost per month:    $8.40

Yearly:            12 months × $8.40 = $100.80
```

### Option C: Quarterly Deep Update

```
Papers per quarter: ~90,000
Size per quarter:   ~205 GB
Cost per quarter:   $24.60

Yearly:             4 quarters × $24.60 = $98.40
```

---

## ROI Analysis: Worth It?

### Break-Even Analysis

If your use case saves time or improves outcomes:

```
Cost per paper downloaded:  $0.27 (at full corpus price)
Cost per paper (1-year):    $0.28

If LLM inference saves 10 hours of human research:
  Value:      10 hours × $50/hr = $500
  Cost:       $25 (1-year corpus)
  ROI:        20:1 (pay $25, gain $500 in value)
```

### When S3 is Worth It

✅ **Yes, worth it if:**
- You run inference at scale (many queries per day)
- You need real-time data (latest papers)
- You're building commercial product (B2B)
- You want offline access (compliance, privacy)

❌ **Maybe not worth it if:**
- You only need metadata (use free OAI-PMH API)
- You query <1000 papers/month (use web search)
- Budget is extremely constrained (use Kaggle, accept lag)

---

## AWS Cost Controls

### 1. Set Billing Alerts

```bash
aws budgets create-budget \
  --account-id YOUR_ACCOUNT \
  --budget BudgetName=arxiv,BudgetLimit={Amount=100,Unit=USD} \
  --notifications-with-subscribers "file://notifications.json"
```

### 2. Use IAM Budget User

Create a separate AWS user with `s3:GetObject` permission only—can't accidentally launch expensive compute.

### 3. Dry-Run First

```rust
// List papers (no download)
let papers = downloader.list_papers(10000).await?;

// Estimate cost
let estimate = downloader.estimate_cost(papers.len(), 2);
println!("Estimated: ${:.2}", estimate.total_estimated_cost_usd);
```

### 4. Enable S3 Request Metrics

Monitor API calls in CloudWatch to catch unexpected traffic.

---

## Final Recommendation

| Budget | Strategy |
|--------|----------|
| **< $10/month** | Kaggle metadata + selective downloads |
| **$10–50/month** | Weekly S3 sync of latest papers (category-filtered) |
| **$50–200/month** | Monthly corpus refresh + category-specific ingestion |
| **$200+/month** | Quarterly full-corpus dump + continuous weekly updates |
| **$0 (GCP users)** | Google Cloud Kaggle (free transfer, zero cost) |

---

**Calculate your exact cost:**

```bash
# Download in examples/s3_downloader.rs
cargo run --example s3_downloader --features s3 -- \
  --estimate <PAPER_COUNT> <AVG_SIZE_MB>

# Example: 84K papers, 2.3MB avg
cargo run --example s3_downloader --features s3 -- \
  --estimate 84000 2
```
