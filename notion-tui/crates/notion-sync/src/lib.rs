mod puller;
mod pusher;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use notion_api::{ApiError, NotionClient};
use tokio::sync::watch;

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
    pub status: watch::Receiver<SyncStatus>,
    pub data_version: watch::Receiver<u64>,
    pub pending: watch::Receiver<u32>,
}

pub use puller::pull_once;
pub use pusher::push_once;

/// Spawns the combined sync loop: each cycle drains `pending_ops` (push), then polls for
/// remote changes (pull). Network errors flip status to `Offline` without dropping queued ops;
/// API rejections/conflicts are recorded per-op by `push_once` and reported via `status`.
pub fn spawn_sync(client: NotionClient, store: SharedStore, interval: Duration) -> SyncHandle {
    let (status_tx, status_rx) = watch::channel(SyncStatus::Starting);
    let (data_tx, data_rx) = watch::channel(0u64);
    let (pending_tx, pending_rx) = watch::channel(0u32);
    tokio::spawn(async move {
        let client = Arc::new(client);
        loop {
            status_tx.send_replace(SyncStatus::Syncing { done: 0 });

            match push_once(&client, &store).await {
                Ok(_) => {}
                Err(ApiError::Network(_)) => {
                    status_tx.send_replace(SyncStatus::Offline);
                }
                Err(e) => {
                    status_tx.send_replace(SyncStatus::Failed(e.to_string()));
                }
            };
            pending_tx.send_replace(store.lock().unwrap().pending_count().unwrap_or(0));

            match pull_once(&client, &store).await {
                Ok(updated) => {
                    if updated > 0 {
                        data_tx.send_modify(|v| *v += 1);
                    }
                    status_tx.send_replace(SyncStatus::Idle { updated });
                }
                Err(ApiError::Network(_)) => {
                    status_tx.send_replace(SyncStatus::Offline);
                }
                Err(e) => {
                    status_tx.send_replace(SyncStatus::Failed(e.to_string()));
                }
            }
            pending_tx.send_replace(store.lock().unwrap().pending_count().unwrap_or(0));

            tokio::time::sleep(interval).await;
        }
    });
    SyncHandle { status: status_rx, data_version: data_rx, pending: pending_rx }
}
