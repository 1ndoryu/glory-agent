/* Rate-limit y presupuestos. Patrón de NAKOMI chat_timing (budgets por
 * visitor/IP + semáforo de concurrencia), reducido al mínimo viable. */

use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

const WINDOW: Duration = Duration::from_secs(60);
const MAX_PER_WINDOW: u32 = 20;
pub const MAX_CONCURRENT_AI: usize = 8;

#[derive(Debug, Clone)]
struct Budget {
    count: u32,
    window_start: Instant,
}

#[derive(Clone)]
pub struct TimingService {
    budgets: Arc<DashMap<String, Budget>>,
    semaphore: Arc<Semaphore>,
}

impl TimingService {
    #[must_use]
    pub fn new() -> Self {
        Self {
            budgets: Arc::new(DashMap::new()),
            semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_AI)),
        }
    }

    /// `true` si la petición cabe en el presupuesto; `false` si debe rechazarse (429).
    pub fn check_budget(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut entry = self.budgets.entry(key.to_string()).or_insert(Budget {
            count: 0,
            window_start: now,
        });
        if now.duration_since(entry.window_start) > WINDOW {
            entry.count = 0;
            entry.window_start = now;
        }
        if entry.count >= MAX_PER_WINDOW {
            return false;
        }
        entry.count += 1;
        true
    }

    #[must_use]
    pub fn ai_permits_available(&self) -> usize {
        self.semaphore.available_permits()
    }
}

impl Default for TimingService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_over_budget() {
        let svc = TimingService::new();
        for _ in 0..MAX_PER_WINDOW {
            assert!(svc.check_budget("ip-1"));
        }
        assert!(!svc.check_budget("ip-1"));
        assert!(svc.check_budget("ip-2"));
    }
}
