use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct S3Config {
    pub bucket: String,
    pub region: String,
    pub requester_pays: bool,
    pub prefix: Option<String>,
    pub max_concurrent_downloads: usize,
    pub chunk_size_mb: usize,
}

impl Default for S3Config {
    fn default() -> Self {
        Self {
            bucket: "arxiv".to_string(),
            region: "us-east-1".to_string(),
            requester_pays: true,
            prefix: Some("pdf".to_string()),
            max_concurrent_downloads: 10,
            chunk_size_mb: 10,
        }
    }
}

#[derive(Debug)]
pub struct S3Downloader {
    config: S3Config,
    #[cfg(feature = "s3")]
    client: aws_sdk_s3::Client,
}

#[cfg(feature = "s3")]
impl S3Downloader {
    pub async fn new(config: S3Config) -> Result<Self, Box<dyn std::error::Error>> {
        let sdk_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
        let client = aws_sdk_s3::Client::new(&sdk_config);
        Ok(Self { config, client })
    }

    pub async fn list_papers(&self, max_keys: i32) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let prefix = self.config.prefix.clone().unwrap_or_default();

        let response = self
            .client
            .list_objects_v2()
            .bucket(&self.config.bucket)
            .prefix(&prefix)
            .max_keys(max_keys)
            .send()
            .await?;

        let papers = response
            .contents()
            .iter()
            .map(|obj| obj.key().unwrap_or("").to_string())
            .collect();

        Ok(papers)
    }

    pub async fn download_paper(
        &self,
        key: &str,
        output_path: &PathBuf,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        let mut response = self
            .client
            .get_object()
            .bucket(&self.config.bucket)
            .key(key)
            .send()
            .await?;

        let mut bytes_downloaded = 0u64;
        let mut file = tokio::fs::File::create(output_path).await?;

        while let Some(bytes) = response.body.try_next().await? {
            tokio::io::AsyncWriteExt::write_all(&mut file, &bytes).await?;
            bytes_downloaded += bytes.len() as u64;
        }

        Ok(bytes_downloaded)
    }

    pub async fn download_papers_parallel(
        &self,
        keys: Vec<&str>,
        output_dir: &PathBuf,
    ) -> Result<Vec<(String, Result<u64, String>)>, Box<dyn std::error::Error>> {
        use std::sync::Arc;
        use tokio::sync::Semaphore;

        let semaphore = Arc::new(Semaphore::new(self.config.max_concurrent_downloads));
        let mut tasks = vec![];

        for key in keys {
            let sem = Arc::clone(&semaphore);
            let client = self.client.clone();
            let bucket = self.config.bucket.clone();
            let output_dir = output_dir.clone();
            let key = key.to_string();

            let task = tokio::spawn(async move {
                let _permit = sem.acquire().await.ok();

                let filename = key.split('/').last().unwrap_or(&key).to_string();
                let output_path = output_dir.join(&filename);

                let result = async {
                    let mut response = client.get_object().bucket(&bucket).key(&key).send().await?;
                    let mut file = tokio::fs::File::create(&output_path).await?;
                    let mut bytes_downloaded = 0u64;

                    while let Some(bytes) = response.body.try_next().await? {
                        tokio::io::AsyncWriteExt::write_all(&mut file, &bytes).await?;
                        bytes_downloaded += bytes.len() as u64;
                    }

                    Ok::<_, Box<dyn std::error::Error>>(bytes_downloaded)
                }
                .await;

                (filename, result.map_err(|e| e.to_string()))
            });

            tasks.push(task);
        }

        let mut results = vec![];
        for task in tasks {
            results.push(task.await.map_err(|e| e.to_string())?);
        }

        Ok(results)
    }

    pub fn estimate_cost(&self, total_papers: u64, avg_size_mb: u64) -> CostEstimate {
        let total_gb = (total_papers * avg_size_mb) / 1024;

        CostEstimate {
            total_papers,
            total_size_gb: total_gb,
            s3_transfer_cost_usd: (total_gb as f64) * 0.12,
            ec2_hourly_rate: 0.05,
            estimated_hours: (total_papers / 1000) as f64,
            total_estimated_cost_usd: ((total_gb as f64) * 0.12) + ((total_papers / 1000) as f64 * 0.05),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CostEstimate {
    pub total_papers: u64,
    pub total_size_gb: u64,
    pub s3_transfer_cost_usd: f64,
    pub ec2_hourly_rate: f64,
    pub estimated_hours: f64,
    pub total_estimated_cost_usd: f64,
}
