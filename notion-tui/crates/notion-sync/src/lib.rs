mod puller;
mod pusher;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use notion_api::{ApiError, NotionClient};
use tokio::sync::watch;

pub type SharedStore = Arc<Mutex<notion_store::Store>>;

/// Error surfaced by a single sync cycle (pull, and later push).
#[derive(Debug)]
pub enum SyncError {
    Api(ApiError),
    Store(String),
}

impl std::fmt::Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyncError::Api(e) => write!(f, "api: {e}"),
            SyncError::Store(msg) => write!(f, "store: {msg}"),
        }
    }
}

impl std::error::Error for SyncError {}

impl From<ApiError> for SyncError {
    fn from(e: ApiError) -> Self {
        SyncError::Api(e)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SyncStatus {
    Starting,
    Syncing { done: u32, total: u32 },
    Idle { updated: u32 },
    Offline,
    Failed(String),
}

pub struct SyncHandle {
    pub status: watch::Receiver<SyncStatus>,
    pub data_version: watch::Receiver<u64>,
    pub pending: watch::Receiver<u32>,
    pub notify: std::sync::Arc<tokio::sync::Notify>,
}

impl SyncHandle {
    /// Wakes the sync loop immediately instead of waiting out its poll interval.
    pub fn request_sync_now(&self) {
        self.notify.notify_one();
    }
}

pub use puller::{pull_once, reconcile_deletions};
pub use pusher::push_once;

/// Locks the shared store, recovering from mutex poisoning rather than propagating a panic.
/// SQLite is transactional, so the data behind the lock stays consistent even if a previous
/// holder panicked mid-logical-operation; treating a poisoned lock as merely "poisoned" (not
/// corrupted) keeps one panicking cycle from taking down every future cycle.
pub(crate) fn lock_store(store: &SharedStore) -> std::sync::MutexGuard<'_, notion_store::Store> {
    store.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Runs a single push-then-pull cycle, updating `status`/`data_version`/`pending` as it goes.
/// Extracted from `spawn_sync`'s loop so the whole cycle can be wrapped in `catch_unwind`:
/// `watch::Sender` is not `Clone`, so the senders are threaded through by reference rather than
/// captured by a separately-spawned task per cycle.
/// How often (in sync cycles) a full-workspace reconciliation crawl runs to catch
/// remote deletions that a checkpointed/hwm-filtered `pull_once` would never see
/// (a deleted page simply stops appearing in search results, rather than showing
/// up with a newer `last_edited_time`). `cycle_count % N == 0` is true on the very
/// first cycle too, so reconciliation also runs once immediately on startup.
const RECONCILE_EVERY_N_CYCLES: u32 = 20;

async fn one_cycle(
    client: &Arc<NotionClient>,
    store: &SharedStore,
    status_tx: &watch::Sender<SyncStatus>,
    data_tx: &watch::Sender<u64>,
    pending_tx: &watch::Sender<u32>,
    cycle_count: u32,
) {
    status_tx.send_replace(SyncStatus::Syncing { done: 0, total: 0 });

    match push_once(client, store).await {
        Ok(_) => {}
        Err(ApiError::Network(_)) => {
            status_tx.send_replace(SyncStatus::Offline);
        }
        Err(e) => {
            status_tx.send_replace(SyncStatus::Failed(e.to_string()));
        }
    };
    pending_tx.send_replace(lock_store(store).pending_count().unwrap_or(0));

    match pull_once(client, store, status_tx).await {
        Ok(updated) => {
            if updated > 0 {
                data_tx.send_modify(|v| *v += 1);
            }
            status_tx.send_replace(SyncStatus::Idle { updated });
            if cycle_count.is_multiple_of(RECONCILE_EVERY_N_CYCLES) {
                if let Ok((removed_pages, removed_ds)) = reconcile_deletions(client, store).await {
                    if removed_pages + removed_ds > 0 {
                        data_tx.send_modify(|v| *v += 1);
                    }
                }
            }
        }
        Err(SyncError::Api(ApiError::Network(_))) => {
            status_tx.send_replace(SyncStatus::Offline);
        }
        Err(e) => {
            status_tx.send_replace(SyncStatus::Failed(e.to_string()));
        }
    }
    pending_tx.send_replace(lock_store(store).pending_count().unwrap_or(0));
}

/// Spawns the combined sync loop: each cycle drains `pending_ops` (push), then polls for
/// remote changes (pull). Network errors flip status to `Offline` without dropping queued ops;
/// API rejections/conflicts are recorded per-op by `push_once` and reported via `status`. A
/// panic inside a cycle (a genuine bug, not mutex poisoning — that's handled by `lock_store`) is
/// caught so the loop keeps running instead of silently dying; the status is set to `Failed` so
/// the TUI can surface it and the user knows to restart.
pub fn spawn_sync(client: NotionClient, store: SharedStore, interval: Duration) -> SyncHandle {
    spawn_sync_inner(client, store, interval, None)
}

/// Test-only entry point: runs `hook(cycle_index)` at the start of every cycle, inside the same
/// `catch_unwind` boundary as the cycle itself, so tests can inject a genuine panic into a
/// specific cycle and observe the containment path below without any global/static flag that
/// could bleed into other tests running concurrently in this binary.
#[doc(hidden)]
pub fn spawn_sync_with_test_hook(
    client: NotionClient,
    store: SharedStore,
    interval: Duration,
    hook: impl Fn(u64) + Send + Sync + 'static,
) -> SyncHandle {
    spawn_sync_inner(client, store, interval, Some(Box::new(hook)))
}

fn spawn_sync_inner(
    client: NotionClient,
    store: SharedStore,
    interval: Duration,
    hook: Option<Box<dyn Fn(u64) + Send + Sync>>,
) -> SyncHandle {
    let (status_tx, status_rx) = watch::channel(SyncStatus::Starting);
    let (data_tx, data_rx) = watch::channel(0u64);
    let (pending_tx, pending_rx) = watch::channel(0u32);
    let notify = std::sync::Arc::new(tokio::sync::Notify::new());
    let notify_loop = notify.clone();
    tokio::spawn(async move {
        let client = Arc::new(client);
        let mut cycle_index: u64 = 0;
        let mut cycle_count: u32 = 0;
        loop {
            let cycle = async {
                if let Some(h) = &hook {
                    h(cycle_index);
                }
                one_cycle(&client, &store, &status_tx, &data_tx, &pending_tx, cycle_count).await
            };
            if let Err(payload) = futures::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(cycle)).await
            {
                let _ = payload;
                status_tx.send_replace(SyncStatus::Failed(
                    "sync engine crashed — restart notion-tui".into(),
                ));
            }
            cycle_index += 1;
            cycle_count = cycle_count.wrapping_add(1);
            tokio::select! {
                _ = tokio::time::sleep(interval) => {}
                _ = notify_loop.notified() => {}
            }
        }
    });
    SyncHandle {
        status: status_rx,
        data_version: data_rx,
        pending: pending_rx,
        notify,
    }
}
