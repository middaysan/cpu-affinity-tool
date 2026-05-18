use crate::app::models::{AppStateStorage, StateStorageMode};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

#[derive(Clone)]
pub struct StorageAdapter {
    shared: Arc<RwLock<AppStateStorage>>,
}

impl StorageAdapter {
    pub fn load() -> Self {
        Self {
            shared: Arc::new(RwLock::new(AppStateStorage::load_state())),
        }
    }

    pub fn shared(&self) -> Arc<RwLock<AppStateStorage>> {
        self.shared.clone()
    }

    pub fn active_data_dir() -> PathBuf {
        AppStateStorage::active_data_dir()
    }

    pub fn active_storage_mode() -> StateStorageMode {
        AppStateStorage::active_storage_mode()
    }
}
