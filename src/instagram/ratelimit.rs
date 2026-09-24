use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug)]
struct State {
    last_retry_after_ms: u64,
    consecutive_429: u32,
    user_throttle: Duration,

    parallelism: u32,
    last_request_at: Option<Instant>,
    cooldown_until: Option<Instant>,
    cancel: Arc<AtomicBool>,
}

#[derive(Debug, Clone)]
pub struct RateLimiter {
    state: Arc<Mutex<State>>,
}

impl RateLimiter {
    pub fn new(user_throttle: Duration, cancel: Arc<AtomicBool>) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                last_retry_after_ms: 0,
                consecutive_429: 0,
                user_throttle,
                parallelism: 1,
                last_request_at: None,
                cooldown_until: None,
                cancel,
            })),
        }
    }

    pub fn set_user_throttle(&self, throttle: Duration) {
        let mut s = self.state.lock().expect("rate limiter poisoned");
        s.user_throttle = throttle;
    }

    pub fn set_parallelism(&self, n: u32) {
        let mut s = self.state.lock().expect("rate limiter poisoned");
        s.parallelism = n.max(1);
    }

    fn spacing(s: &State) -> Duration {
        s.user_throttle / s.parallelism.max(1)
    }

    #[allow(dead_code)]
    pub fn reset(&self) {
        let mut s = self.state.lock().expect("rate limiter poisoned");
        s.last_retry_after_ms = 0;
        s.consecutive_429 = 0;
        s.last_request_at = None;
        s.cooldown_until = None;
    }

    pub async fn wait(&self) -> Duration {
        let started_waiting = Instant::now();
        loop {
            let (delay, cancel) = {
                let s = self.state.lock().expect("rate limiter poisoned");
                let now = Instant::now();
                let spacing_ready_at = s
                    .last_request_at
                    .map_or(now, |last| last + Self::spacing(&s));
                let ready_at = s
                    .cooldown_until
                    .map_or(spacing_ready_at, |until| spacing_ready_at.max(until));
                (ready_at.saturating_duration_since(now), s.cancel.clone())
            };
            if delay > Duration::ZERO {
                poll_sleep_with_cancel(delay, cancel.clone()).await;
            }
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return started_waiting.elapsed();
            }

            let mut s = self.state.lock().expect("rate limiter poisoned");
            let now = Instant::now();
            let spacing = Self::spacing(&s);
            let spacing_ready = s.last_request_at.is_none_or(|last| now >= last + spacing);
            let cooldown_ready = s.cooldown_until.is_none_or(|until| now >= until);
            if spacing_ready && cooldown_ready {
                s.last_request_at = Some(now);
                s.cooldown_until = None;
                return started_waiting.elapsed();
            }
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.state
            .lock()
            .expect("rate limiter poisoned")
            .cancel
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn observe_success(&self) {
        let mut s = self.state.lock().expect("rate limiter poisoned");
        s.consecutive_429 = 0;
    }

    pub fn observe_rate_limit(&self, retry_after_ms: u64) -> u64 {
        let mut s = self.state.lock().expect("rate limiter poisoned");
        s.consecutive_429 = s.consecutive_429.saturating_add(1);
        let exp = s.consecutive_429.saturating_sub(1).min(6);
        let multiplier = 1u64 << exp;
        let actual_wait_ms = retry_after_ms.saturating_mul(multiplier);
        s.last_retry_after_ms = actual_wait_ms;
        let until = Instant::now() + Duration::from_millis(actual_wait_ms);
        s.cooldown_until = match s.cooldown_until {
            Some(prev) if prev > until => Some(prev),
            _ => Some(until),
        };
        actual_wait_ms
    }
}

pub(crate) async fn poll_sleep_with_cancel(delay: Duration, cancel: Arc<AtomicBool>) {
    let deadline = Instant::now() + delay;
    loop {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let now = Instant::now();
        if now >= deadline {
            return;
        }
        let slice = (deadline - now).min(Duration::from_millis(60));
        tokio::time::sleep(slice).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn throttle_enforced() {
        let cancel = Arc::new(AtomicBool::new(false));
        let lim = RateLimiter::new(Duration::from_millis(40), cancel);
        let _ = lim.wait().await;
        let t0 = Instant::now();
        let slept = lim.wait().await;
        assert!(slept >= Duration::from_millis(30));
        assert!(t0.elapsed() >= Duration::from_millis(30));
    }

    #[tokio::test]
    async fn parallelism_divides_the_spacing() {
        let cancel = Arc::new(AtomicBool::new(false));
        let lim = RateLimiter::new(Duration::from_millis(80), cancel);
        lim.set_parallelism(4);
        let _ = lim.wait().await;
        let slept = lim.wait().await;

        assert!(slept <= Duration::from_millis(40), "slept {slept:?}");

        lim.set_parallelism(0);
        let _ = lim.wait().await;
        let slept = lim.wait().await;
        assert!(slept >= Duration::from_millis(60), "slept {slept:?}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_waiters_are_spaced_instead_of_released_as_a_burst() {
        let cancel = Arc::new(AtomicBool::new(false));
        let limiter = RateLimiter::new(Duration::from_millis(80), cancel);
        limiter.set_parallelism(4);

        let start = Instant::now();
        let tasks = (0..4).map(|_| {
            let limiter = limiter.clone();
            tokio::spawn(async move {
                limiter.wait().await;
                Instant::now()
            })
        });
        let mut times = futures_util::future::join_all(tasks)
            .await
            .into_iter()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        times.sort_unstable();

        for pair in times.windows(2) {
            assert!(
                pair[1].duration_since(pair[0]) >= Duration::from_millis(12),
                "concurrent requests were too close: {:?}",
                pair[1].duration_since(pair[0])
            );
        }
        assert!(times[0].duration_since(start) < Duration::from_millis(20));
    }

    #[tokio::test]
    async fn cancellation_interrupts_a_throttle_wait() {
        let cancel = Arc::new(AtomicBool::new(false));
        let limiter = RateLimiter::new(Duration::from_secs(1), cancel.clone());
        limiter.wait().await;

        let waiter = {
            let limiter = limiter.clone();
            tokio::spawn(async move { limiter.wait().await })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        let waited = waiter.await.unwrap();
        assert!(waited < Duration::from_millis(100));
    }

    #[test]
    fn backoff_scales() {
        let cancel = Arc::new(AtomicBool::new(false));
        let lim = RateLimiter::new(Duration::ZERO, cancel);
        assert_eq!(lim.observe_rate_limit(1000), 1000);
        assert_eq!(lim.observe_rate_limit(1000), 2000);
        assert_eq!(lim.observe_rate_limit(1000), 4000);
        lim.observe_success();
        assert_eq!(lim.observe_rate_limit(1000), 1000);
    }
}
