use async_trait::async_trait;
use opencrust_agents::{AgentRuntime, LlmProvider, LlmRequest, LlmResponse};
use opencrust_common::Result;
use std::time::{Duration, Instant};

struct MockProvider {
    id: String,
    delay: Duration,
}

#[async_trait]
impl LlmProvider for MockProvider {
    fn provider_id(&self) -> &str {
        &self.id
    }

    async fn complete(&self, _request: &LlmRequest) -> Result<LlmResponse> {
        unimplemented!()
    }

    async fn health_check(&self) -> Result<bool> {
        tokio::time::sleep(self.delay).await;
        Ok(true)
    }
}

#[tokio::test]
async fn test_health_check_performance() {
    let mut runtime = AgentRuntime::new();
    let delay = Duration::from_millis(100);
    let count = 5;

    for i in 0..count {
        let provider = MockProvider {
            id: format!("mock-{}", i),
            delay,
        };
        runtime.register_provider(Box::new(provider));
    }

    let start = Instant::now();
    let results = runtime.health_check_all().await.unwrap();
    let elapsed = start.elapsed();

    println!("Elapsed: {:?}", elapsed);
    assert_eq!(results.len(), count);

    // For verification: check if execution is concurrent
    // The sequential execution time would be at least (count * delay).
    // We expect concurrent execution to be significantly faster.
    // To be robust against slow CI runners, we assert that it took less than
    // half the time required for sequential execution. This proves concurrency
    // without enforcing tight timing bounds that might flake.
    let sequential_duration = delay * count as u32;
    let max_allowed_duration = sequential_duration / 2;

    assert!(
        elapsed < max_allowed_duration,
        "Execution took {:?}, which is too slow for concurrent processing. Expected less than {:?} (half of sequential time {:?})",
        elapsed,
        max_allowed_duration,
        sequential_duration
    );

    // Also ensure it took at least the delay (sanity check)
    assert!(elapsed >= delay, "Execution was faster than the delay itself!");
}
