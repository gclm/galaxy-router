use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub struct ChannelLoadSnapshot {
    pub channel_id: String,
    pub current_concurrency: u32,
    pub max_concurrency: u32,
    pub load_rate: u32,
}

#[derive(Debug, Default)]
struct ChannelCapacityState {
    active: AtomicU32,
}

#[derive(Debug, Clone, Default)]
pub struct ChannelCapacityManager {
    states: Arc<Mutex<HashMap<String, Arc<ChannelCapacityState>>>>,
}

impl ChannelCapacityManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn try_acquire(
        &self,
        channel_id: &str,
        max_concurrency: u32,
    ) -> Option<ChannelCapacityPermit> {
        if max_concurrency == 0 {
            return Some(ChannelCapacityPermit::unlimited());
        }

        let state = self.state_for(channel_id);
        loop {
            let current = state.active.load(Ordering::Acquire);
            if current >= max_concurrency {
                return None;
            }
            if state
                .active
                .compare_exchange(current, current + 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return Some(ChannelCapacityPermit::limited(state));
            }
        }
    }

    #[allow(dead_code)]
    pub fn load_snapshot(&self, channel_id: &str, max_concurrency: u32) -> ChannelLoadSnapshot {
        let current = if max_concurrency == 0 {
            0
        } else {
            self.states
                .lock()
                .expect("capacity state mutex poisoned")
                .get(channel_id)
                .map(|s| s.active.load(Ordering::Acquire))
                .unwrap_or(0)
        };
        let load_rate = if max_concurrency == 0 {
            0
        } else {
            current.saturating_mul(100) / max_concurrency
        };

        ChannelLoadSnapshot {
            channel_id: channel_id.to_string(),
            current_concurrency: current,
            max_concurrency,
            load_rate,
        }
    }

    fn state_for(&self, channel_id: &str) -> Arc<ChannelCapacityState> {
        let mut states = self.states.lock().expect("capacity state mutex poisoned");
        states
            .entry(channel_id.to_string())
            .or_insert_with(|| Arc::new(ChannelCapacityState::default()))
            .clone()
    }
}

#[derive(Debug)]
pub struct ChannelCapacityPermit {
    state: Option<Arc<ChannelCapacityState>>,
}

impl ChannelCapacityPermit {
    fn unlimited() -> Self {
        Self { state: None }
    }

    fn limited(state: Arc<ChannelCapacityState>) -> Self {
        Self { state: Some(state) }
    }
}

impl Drop for ChannelCapacityPermit {
    fn drop(&mut self) {
        if let Some(state) = &self.state {
            state.active.fetch_sub(1, Ordering::AcqRel);
        }
    }
}
