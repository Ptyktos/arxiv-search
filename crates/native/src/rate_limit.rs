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
        let now = Instant::now();
        let sleep_duration = {
            let mut last = self.last_request.lock().await;
            let next_allowed = match *last {
                Some(t) => (t + self.delay).max(now),
                None => now,
            };
            *last = Some(next_allowed);
            if next_allowed > now {
                Some(next_allowed - now)
            } else {
                None
            }
        };

        if let Some(d) = sleep_duration {
            tokio::time::sleep(d).await;
        }
    }
}
