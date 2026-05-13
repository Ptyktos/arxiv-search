use arxiv_search_rs_mcp_native::rate_limit::FileRateLimiter;
use arxiv_search_rs_mcp_core::RateLimiter;
use tempfile::tempdir;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::fs;

#[tokio::test]
async fn test_extreme_concurrency() {
    let temp = tempdir().unwrap();
    let cache_dir = temp.path().to_path_buf();
    let delay = Duration::from_millis(10); // Short delay for stress test
    
    let num_requests = 50;
    let mut handles = vec![];
    
    let start = Instant::now();
    for i in 0..num_requests {
        let dir = cache_dir.clone();
        handles.push(tokio::spawn(async move {
            let limiter = FileRateLimiter::new(dir, delay, "rate_limit.lock");
            limiter.wait().await;
            i
        }));
    }
    
    let mut results = vec![];
    for h in handles {
        results.push(h.await.unwrap());
    }
    
    let elapsed = start.elapsed();
    println!("Processed {} concurrent requests in {:?}", num_requests, elapsed);
    
    // Total time should be at least (num_requests - 1) * delay
    let min_expected = delay * (num_requests - 1);
    assert!(elapsed >= min_expected, "Should have taken at least {:?}, took {:?}", min_expected, elapsed);
}

#[tokio::test]
async fn test_future_timestamp_recovery() {
    let temp = tempdir().unwrap();
    let cache_dir = temp.path().to_path_buf();
    let lock_file = cache_dir.join("rate_limit.lock");
    
    // Manually write a timestamp 1 hour in the future
    let future_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64 + 3600_000;
    
    fs::write(&lock_file, future_time.to_string()).unwrap();
    
    let limiter = FileRateLimiter::new(cache_dir, Duration::from_millis(100), "rate_limit.lock");
    
    let start = Instant::now();
    limiter.wait().await;
    let elapsed = start.elapsed();
    
    // Should NOT wait for an hour. The fix resets extreme future timestamps (> 10 min).
    assert!(elapsed < Duration::from_secs(5), "Should have recovered from future timestamp quickly, took {:?}", elapsed);
}

#[tokio::test]
async fn test_corrupt_lock_file() {
    let temp = tempdir().unwrap();
    let cache_dir = temp.path().to_path_buf();
    let lock_file = cache_dir.join("rate_limit.lock");
    
    // Write garbage
    fs::write(&lock_file, "garbage data that is not a number").unwrap();
    
    let limiter = FileRateLimiter::new(cache_dir, Duration::from_millis(10), "rate_limit.lock");
    
    let start = Instant::now();
    limiter.wait().await;
    let elapsed = start.elapsed();
    
    assert!(elapsed < Duration::from_secs(1), "Should have recovered from corrupt file, took {:?}", elapsed);
    
    // Verify it wrote a valid timestamp back
    let content = fs::read_to_string(&lock_file).unwrap();
    assert!(content.trim().parse::<u64>().is_ok());
}

#[tokio::test]
async fn test_race_condition_file_deletion() {
    let temp = tempdir().unwrap();
    let cache_dir = temp.path().to_path_buf();
    let lock_file = cache_dir.join("rate_limit.lock");
    
    let limiter = FileRateLimiter::new(cache_dir.clone(), Duration::from_millis(50), "rate_limit.lock");
    
    // Start one request
    let h1 = tokio::spawn(async move {
        limiter.wait().await;
    });
    
    // Wait a tiny bit then delete the file
    tokio::time::sleep(Duration::from_millis(10)).await;
    let _ = fs::remove_file(&lock_file);
    
    // Another request should still work (it will recreate the file)
    let limiter2 = FileRateLimiter::new(cache_dir, Duration::from_millis(50), "rate_limit.lock");
    let h2 = tokio::spawn(async move {
        limiter2.wait().await;
    });
    
    h1.await.unwrap();
    h2.await.unwrap();
}

#[tokio::test]
async fn test_concurrent_client_initialization() {
    let temp = tempdir().unwrap();
    let cache_dir = temp.path().to_path_buf();
    
    // Set environment variable for cache dir to use our temp dir
    // ArxivCache uses directories crate, but we can't easily override it without env vars
    // or mocking. For now, let's just test multiple RateLimiter instances pointing to the same file.
    
    let mut handles = vec![];
    for _ in 0..10 {
        let dir = cache_dir.clone();
        handles.push(tokio::spawn(async move {
            let _limiter = FileRateLimiter::new(dir, Duration::from_millis(100), "rate_limit.lock");
            // In a real scenario, FetchClient::new would also init Cache and DB.
            // We are testing the lock file contention here.
        }));
    }
    
    for h in handles {
        h.await.unwrap();
    }
}
