mod puller;

use std::sync::{Arc, Mutex};

pub type SharedStore = Arc<Mutex<notion_store::Store>>;

#[derive(Debug, Clone, PartialEq)]
pub enum SyncStatus {
    Starting,
    Syncing { done: u32 },
    Idle { updated: u32 },
    Offline,
    Failed(String),
}

pub struct SyncHandle {
    pub status: tokio::sync::watch::Receiver<SyncStatus>,
    pub data_version: tokio::sync::watch::Receiver<u64>,
}

pub use puller::{pull_once, spawn_puller};
