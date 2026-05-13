use std::io::{Read, Seek, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use tokio::sync::Mutex;

use arxiv_search_rs_mcp_core::RateLimiter;

/// A tokio-based rate limiter implementation for in-process synchronization.
pub struct TokioRateLimiter {
    last_request: Arc<Mutex<Option<std::time::Instant>>>,
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
        let now = std::time::Instant::now();
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

/// A file-based rate limiter for cross-process synchronization.
pub struct FileRateLimiter {
    lock_file_path: PathBuf,
    delay: Duration,
}

impl FileRateLimiter {
    #[must_use]
    pub fn new(cache_dir: PathBuf, delay: Duration, filename: &str) -> Self {
        let lock_file_path = cache_dir.join(filename);
        Self {
            lock_file_path,
            delay,
        }
    }

    fn get_now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }
}

#[async_trait]
impl RateLimiter for FileRateLimiter {
    async fn wait(&self) {
        let lock_file_path = self.lock_file_path.clone();
        let delay_ms = self.delay.as_millis() as u64;

        // Step 1: Acquire lock and claim the next available slot
        let sleep_ms = tokio::task::spawn_blocking(move || -> u64 {
            tracing::debug!("Attempting to open rate limit lock file at {:?}", lock_file_path);
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .open(&lock_file_path)
                .unwrap_or_else(|e| panic!("Failed to open rate limit lock file: {:?}", e));

            let mut lock = fd_lock::RwLock::new(file);
            tracing::debug!("Attempting to acquire write lock on rate limit file...");
            
            // Wait up to 30 seconds to acquire the lock to detect indefinite deadlocks
            let lock_start = std::time::Instant::now();
            let mut guard = loop {
                match lock.try_write() {
                    Ok(g) => break g,
                    Err(e) => {
                        if lock_start.elapsed() > Duration::from_secs(30) {
                            tracing::error!("Indefinite hang detected: Could not acquire rate limit lock after 30 seconds. Another process may be deadlocked.");
                            panic!("Rate limit lock acquisition timed out");
                        }
                        tracing::trace!("Lock held by another process, waiting... ({:?})", e);
                        std::thread::sleep(Duration::from_millis(50));
                    }
                }
            };

            tracing::debug!("Acquired write lock on rate limit file in {:?}", lock_start.elapsed());

            let now = Self::get_now_ms();
            let mut content = String::new();
            if let Err(e) = guard.read_to_string(&mut content) {
                tracing::warn!("Failed to read rate limit file content: {:?}", e);
            }
            
            let mut last_request = content.trim().parse::<u64>().unwrap_or(0);
            tracing::debug!("Rate limit file contained last_request: {}", last_request);
            
            // Safety check: if last_request is too far in the future (e.g. clock skew), reset it.
            // We increase this to 10 minutes (600,000ms) to allow for long request queues 
            // without triggering a reset that causes a burst of 429s.
            if last_request > now + 600_000 {
                tracing::warn!("Rate limit file has extreme future timestamp ({}), likely clock skew, resetting to now", last_request);
                last_request = 0;
            }

            // Calculate the earliest time the next request can start.
            let next_allowed_start = if last_request == 0 {
                now
            } else {
                (last_request + delay_ms).max(now)
            };
            let sleep_ms = next_allowed_start - now;

            if let Err(e) = guard.set_len(0) {
                tracing::error!("Failed to truncate rate limit file: {:?}", e);
            }
            if let Err(e) = guard.seek(std::io::SeekFrom::Start(0)) {
                tracing::error!("Failed to seek rate limit file: {:?}", e);
            }
            if let Err(e) = guard.write_all(next_allowed_start.to_string().as_bytes()) {
                tracing::error!("Failed to write to rate limit file: {:?}", e);
            }
            
            if sleep_ms > 5000 {
                tracing::info!("arXiv rate limit queue deep: {}ms wait ahead", sleep_ms);
            }
            
            tracing::debug!("Releasing rate limit lock. Next allowed start: {}, sleeping for {}ms", next_allowed_start, sleep_ms);
            sleep_ms
        }).await.expect("spawn_blocking failed");

        // Step 2: Sleep if necessary outside of the lock
        if sleep_ms > 0 {
            tracing::info!("arXiv rate limit: serializing request, sleeping for {}ms", sleep_ms);
            tokio::time::sleep(Duration::from_millis(sleep_ms)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use std::time::Instant;

    #[tokio::test]
    async fn test_file_rate_limiter_serialization() {
        let temp = tempdir().unwrap();
        let cache_dir = temp.path().to_path_buf();
        let delay = Duration::from_millis(100);
        let _limiter = FileRateLimiter::new(cache_dir, delay, "test_rate_limit.lock");

        let start = Instant::now();
        
        // Spawn 3 concurrent requests
        let mut handles = vec![];
        for _ in 0..3 {
            let l = FileRateLimiter::new(temp.path().to_path_buf(), delay, "test_rate_limit.lock");
            handles.push(tokio::spawn(async move {
                l.wait().await;
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        let elapsed = start.elapsed();
        // 3 requests with 100ms delay should take at least 200ms (0ms, 100ms, 200ms)
        assert!(elapsed >= Duration::from_millis(200), "Should have taken at least 200ms, took {:?}", elapsed);
    }
}
