use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::Mutex;

use arxiv_search_rs_mcp_core::RateLimiter;

/// A tokio-based rate limiter implementation.
pub struct TokioRateLimiter {
    last_request: Arc<Mutex<Option<Instant>>>,
    delay: Duration,
}

impl TokioRateLimiter {
    #[must_use]
    pub fn new(delay: Duration) -> Self {
        Self {
            last_request: Arc::new(Mutex::new(None)),
            delay,
        }
    }
}

#[async_trait]
impl RateLimiter for TokioRateLimiter {
    async fn wait(&self) {
        let sleep_duration = {
            let mut last = self.last_request.lock().await;
            let elapsed = last.map(|t| t.elapsed());
            *last = Some(Instant::now());
            drop(last);
            elapsed.and_then(|e| self.delay.checked_sub(e))
        };
        if let Some(d) = sleep_duration {
            tokio::time::sleep(d).await;
        }
    }
}
