//! Shared denial backoff for automatic writers already serialized by Authority.
use os_api::ProcessSettingsError;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

type Instance = (u32, u64);
const MAX_DENIED_INSTANCES: usize = 4096;
const PENDING_LIFETIME: Duration = Duration::from_secs(300);

struct Denial {
    count: u8,
    observed: bool,
    at: Instant,
}

#[derive(Default)]
pub(crate) struct CorrectionState {
    denied: Mutex<HashMap<Instance, Denial>>,
}

#[derive(Debug)]
pub(crate) struct CorrectionFailure {
    pub message: String,
    pub access_limited: bool,
    pub paused: bool,
}
impl CorrectionFailure {
    pub fn other(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            access_limited: false,
            paused: false,
        }
    }
}
impl std::fmt::Display for CorrectionFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(f)
    }
}

impl CorrectionState {
    /// Called inside the existing Authority write gate. No native work holds this mutex.
    pub fn attempt<T>(
        &self,
        instance: Instance,
        write: bool,
        observed: bool,
        native: impl FnOnce() -> Result<T, ProcessSettingsError>,
    ) -> Result<T, CorrectionFailure> {
        {
            let mut denied = self
                .denied
                .lock()
                .map_err(|_| CorrectionFailure::other("Correction state is unavailable"))?;
            denied.retain(|_, entry| entry.observed || entry.at.elapsed() < PENDING_LIFETIME);
            if denied.get(&instance).is_some_and(|entry| entry.count >= 3) {
                return Err(CorrectionFailure { message: "Automatic correction is paused after 3 access denials; use Retry to try again".into(), access_limited: true, paused: true });
            }
            if write && denied.len() >= MAX_DENIED_INSTANCES && !denied.contains_key(&instance) {
                return Err(CorrectionFailure::other(
                    "Automatic correction capacity reached; retry after pending processes retire",
                ));
            }
        }
        let result = native();
        let count = if write {
            let mut denied = self
                .denied
                .lock()
                .map_err(|_| CorrectionFailure::other("Correction state is unavailable"))?;
            if result.as_ref().is_err_and(|error| error.access_denied) {
                let entry = denied.entry(instance).or_insert(Denial {
                    count: 0,
                    observed,
                    at: Instant::now(),
                });
                entry.count = entry.count.saturating_add(1).min(3);
                entry.observed |= observed;
                entry.count
            } else {
                denied.remove(&instance);
                0
            }
        } else {
            0
        };
        result.map_err(|error| {
            let mut message = error.message;
            if count == 3 {
                message.push_str(
                    ". Automatic correction paused after 3 access denials; use Retry to try again.",
                );
            }
            CorrectionFailure {
                message,
                access_limited: error.access_denied,
                paused: false,
            }
        })
    }

    pub fn count(&self, instance: Instance) -> u8 {
        self.denied
            .try_lock()
            .ok()
            .and_then(|denied| denied.get(&instance).map(|entry| entry.count))
            .unwrap_or(0)
    }
    pub fn is_empty(&self) -> bool {
        self.denied.try_lock().is_ok_and(|denied| denied.is_empty())
    }
    pub fn reset(&self, instances: impl IntoIterator<Item = Instance>) -> Result<(), String> {
        let mut denied = self
            .denied
            .try_lock()
            .map_err(|_| "Correction state is temporarily busy")?;
        for instance in instances {
            denied.remove(&instance);
        }
        Ok(())
    }
    pub fn observe(&self, instances: impl IntoIterator<Item = Instance>) {
        let Ok(mut denied) = self.denied.lock() else {
            return;
        };
        if denied.is_empty() {
            return;
        }
        for instance in instances {
            if let Some(entry) = denied.get_mut(&instance) {
                entry.observed = true;
            }
        }
    }
    pub fn retain_members(&self, current: &HashSet<Instance>, previous: &HashSet<Instance>) {
        let Ok(mut denied) = self.denied.lock() else {
            return;
        };
        denied.retain(|instance, entry| {
            if current.contains(instance) {
                entry.observed = true;
                true
            } else {
                !entry.observed
                    && !previous.contains(instance)
                    && entry.at.elapsed() < PENDING_LIFETIME
            }
        });
        if denied.is_empty() {
            *denied = HashMap::new();
        }
    }
}
