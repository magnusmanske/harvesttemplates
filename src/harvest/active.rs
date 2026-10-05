//! Which runs have a task working on them right now.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

#[derive(Debug, Default)]
pub struct ActiveRuns {
    runs: Mutex<HashMap<u64, Active>>,
}

#[derive(Debug)]
struct Active {
    user_id: u64,
    stop: Arc<AtomicBool>,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ClaimError {
    #[error("this run is already being worked on")]
    Busy,
    #[error("you already have {0} active runs; wait for one to finish")]
    TooMany(usize),
}

/// Held by the task working on a run; releases the run when dropped, even on panic.
#[derive(Debug)]
pub struct Claim {
    runs: Arc<ActiveRuns>,
    run_id: u64,
    stop: Arc<AtomicBool>,
}

impl Claim {
    pub fn stop_requested(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        self.runs.lock().remove(&self.run_id);
    }
}

impl ActiveRuns {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u64, Active>> {
        self.runs.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn claim(self: &Arc<Self>, run_id: u64, user_id: u64, max_per_user: usize) -> Result<Claim, ClaimError> {
        let mut runs = self.lock();
        if runs.contains_key(&run_id) {
            return Err(ClaimError::Busy);
        }
        if runs.values().filter(|a| a.user_id == user_id).count() >= max_per_user {
            return Err(ClaimError::TooMany(max_per_user));
        }
        let stop = Arc::new(AtomicBool::new(false));
        runs.insert(
            run_id,
            Active {
                user_id,
                stop: stop.clone(),
            },
        );
        Ok(Claim {
            runs: self.clone(),
            run_id,
            stop,
        })
    }

    /// Ask the run's task to stop after the current row. `false` if nothing is running.
    pub fn stop(&self, run_id: u64) -> bool {
        self.lock()
            .get(&run_id)
            .map(|a| a.stop.store(true, Ordering::Relaxed))
            .is_some()
    }

    pub fn is_active(&self, run_id: u64) -> bool {
        self.lock().contains_key(&run_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claims() {
        let runs = Arc::new(ActiveRuns::default());
        let a = runs.claim(1, 7, 2).unwrap();
        assert_eq!(runs.claim(1, 8, 2).unwrap_err(), ClaimError::Busy);
        let _b = runs.claim(2, 7, 2).unwrap();
        assert_eq!(runs.claim(3, 7, 2).unwrap_err(), ClaimError::TooMany(2));
        assert!(runs.stop(1) && a.stop_requested());
        drop(a);
        assert!(!runs.is_active(1));
        assert!(runs.claim(3, 7, 2).is_ok(), "a slot is free again");
    }
}
