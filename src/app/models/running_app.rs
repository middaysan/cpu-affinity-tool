#![allow(dead_code)]
use std::collections::HashMap;

pub struct RunningApp {
    pub pids: Vec<u32>,
    pub group_index: usize,
    pub prog_index: usize,
    pub created_at: std::time::SystemTime,
}

#[derive(Default)]
pub struct RunningApps {
    pub apps: HashMap<String, RunningApp>,
}

impl RunningApps {
    pub fn add_app(&mut self, app_key: &str, pid: u32, group_index: usize, prog_index: usize) {
        self.apps.insert(app_key.to_string(), RunningApp {
            pids: vec![pid],
            group_index: group_index,
            prog_index: prog_index,
            created_at: std::time::SystemTime::now(),
        });
    }

    pub fn remove_app(&mut self, app_key: &str) {
        self.apps.remove(app_key);
    }
}
