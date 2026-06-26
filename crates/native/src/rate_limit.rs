use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
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
            let next_allowed = last.map_or(now, |t| (t + self.delay).max(now));
            *last = Some(next_allowed);
            drop(last);
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
    pub fn new(cache_dir: &Path, delay: Duration, filename: &str) -> Self {
        let lock_file_path = cache_dir.join(filename);
        Self {
            lock_file_path,
            delay,
        }
    }

    #[must_use]
    fn get_now_ms() -> u64 {
        #[expect(clippy::cast_possible_truncation)]
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        ms
    }
}

#[async_trait]
impl RateLimiter for FileRateLimiter {
    #[expect(clippy::expect_used)]
    async fn wait(&self) {
        let lock_file_path = self.lock_file_path.clone();
        #[expect(clippy::cast_possible_truncation)]
        let delay_ms = self.delay.as_millis() as u64;

        // Use a spawn_blocking to handle synchronous file locking
        let (sleep_ms, wait_for_lock) = tokio::task::spawn_blocking(move || -> (u64, Duration) {
            let start = std::time::Instant::now();

            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&lock_file_path)
                .expect("Failed to open rate limit lock file");

            let mut lock = fd_lock::RwLock::new(file);

            // Blocking lock acquisition is more robust than a spin loop
            let mut guard = lock.write().expect("Failed to acquire rate limit lock");
            let wait_for_lock = start.elapsed();

            let now = Self::get_now_ms();
            let mut content = String::new();
            let _ = guard.read_to_string(&mut content);

            let mut last_request = content.trim().parse::<u64>().unwrap_or(0);

            // Reset if too far in the future (clock skew or corruption)
            if last_request > now + 600_000 {
                last_request = 0;
            }

            let next_allowed_start = if last_request == 0 {
                now
            } else {
                (last_request + delay_ms).max(now)
            };

            let sleep_ms = next_allowed_start - now;

            // Atomically update the file
            let _ = guard.set_len(0);
            let _ = guard.seek(std::io::SeekFrom::Start(0));
            let _ = guard.write_all(next_allowed_start.to_string().as_bytes());
            let _ = guard.flush();

            (sleep_ms, wait_for_lock)
        })
        .await
        .expect("spawn_blocking failed");

        if wait_for_lock > Duration::from_millis(100) {
            tracing::warn!(?wait_for_lock, "High contention on rate limit lock");
        }

        if sleep_ms > 0 {
            tracing::info!(sleep_ms, "ArXiv rate limit: serializing request");
            tokio::time::sleep(Duration::from_millis(sleep_ms)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use tempfile::tempdir;

    #[tokio::test]
    #[expect(clippy::unwrap_used, clippy::uninlined_format_args)]
    async fn test_file_rate_limiter_serialization() {
        let temp = tempdir().unwrap();
        let cache_dir = temp.path().to_path_buf();
        let delay = Duration::from_millis(100);
        let _limiter = FileRateLimiter::new(&cache_dir, delay, "test_rate_limit.lock");

        let start = Instant::now();

        // Spawn 3 concurrent requests
        let mut handles = vec![];
        for _ in 0..3 {
            let l = FileRateLimiter::new(temp.path(), delay, "test_rate_limit.lock");
            handles.push(tokio::spawn(async move {
                l.wait().await;
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        let elapsed = start.elapsed();
        // 3 requests with 100ms delay should take at least 200ms (0ms, 100ms, 200ms)
        assert!(
            elapsed >= Duration::from_millis(200),
            "Should have taken at least 200ms, took {:?}",
            elapsed
        );
    }
}
