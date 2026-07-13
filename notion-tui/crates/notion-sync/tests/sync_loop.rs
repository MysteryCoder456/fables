use std::sync::{Arc, Mutex};
use std::time::Duration;

use notion_api::NotionClient;
use notion_store::{BlockRec, PageRec, Store};
use notion_sync::{spawn_sync, spawn_sync_with_test_hook, SharedStore, SyncStatus};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Mock server with an empty search result, in-memory store, ready to hand to `spawn_sync`.
async fn empty_workspace_fixture() -> (MockServer, NotionClient, SharedStore) {
    let store: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [], "has_more": false, "next_cursor": null
        })))
        .mount(&server)
        .await;

    let mut client = NotionClient::with_base_url("t", server.uri());
    client.set_timing(Duration::from_millis(1), Duration::from_millis(1));
    (server, client, store)
}

#[tokio::test]
async fn spawn_sync_pushes_then_pulls_and_reports_pending() {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&PageRec {
        id: "p1".into(),
        parent_type: "workspace".into(),
        parent_id: None,
        title: "P".into(),
        icon: None,
        archived: false,
        last_edited_time: "2026-07-05T10:00:00.000Z".into(),
    })
    .unwrap();
    s.replace_page_blocks(
        "p1",
        &[BlockRec {
            id: "b1".into(),
            page_id: "p1".into(),
            parent_block_id: None,
            ordinal: 0,
            block_type: "to_do".into(),
            payload: r#"{"checked": false}"#.into(),
            plain_text: "Buy milk".into(),
            has_children: false,
        }],
    )
    .unwrap();
    s.edit_toggle_todo("b1").unwrap();
    let store = Arc::new(Mutex::new(s));

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/pages/p1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "page", "id": "p1", "last_edited_time": "2026-07-05T10:00:00.000Z"
        })))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/v1/blocks/b1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "block", "id": "b1", "last_edited_time": "2026-07-05T11:00:00.000Z"
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [], "has_more": false, "next_cursor": null
        })))
        .mount(&server)
        .await;

    let mut client = NotionClient::with_base_url("t", server.uri());
    client.set_timing(Duration::from_millis(1), Duration::from_millis(1));
    let mut handle = spawn_sync(client, store.clone(), Duration::from_millis(50));

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if matches!(*handle.status.borrow(), SyncStatus::Idle { .. }) {
                break;
            }
            handle.status.changed().await.unwrap();
        }
    })
    .await
    .expect("sync loop never reached Idle");

    assert!(store.lock().unwrap().ops().unwrap().is_empty());
    assert!(!store.lock().unwrap().is_page_dirty("p1").unwrap());
    assert_eq!(*handle.pending.borrow(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn poisoned_store_mutex_does_not_kill_sync() {
    let (server, client, store) = empty_workspace_fixture().await;

    // Poison the mutex from a scratch thread.
    let poisoner = store.clone();
    let _ = std::thread::spawn(move || {
        let _guard = poisoner.lock().unwrap();
        panic!("poison");
    })
    .join();
    assert!(store.lock().is_err(), "mutex must actually be poisoned");

    let mut handle = spawn_sync(client, store, Duration::from_millis(50));
    // The loop must still reach Idle despite the poisoned mutex.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            handle.status.changed().await.unwrap();
            if matches!(*handle.status.borrow(), SyncStatus::Idle { .. }) {
                break;
            }
        }
    })
    .await
    .expect("sync must recover from a poisoned mutex");
    let _ = server;
}

#[tokio::test]
async fn cycle_panic_is_contained_and_sync_recovers() {
    let (server, client, store) = empty_workspace_fixture().await;

    // Test-only hook: forces a genuine panic on the very first cycle so we exercise the
    // `catch_unwind` containment path in `spawn_sync`'s loop (not the poisoned-mutex path, which
    // is covered separately above). Every later cycle runs untouched.
    let mut handle = spawn_sync_with_test_hook(client, store, Duration::from_millis(20), |cycle| {
        if cycle == 0 {
            panic!("injected test panic: simulated cycle bug");
        }
    });

    // (a) the crash must be observed as SyncStatus::Failed with the exact crash message.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            handle.status.changed().await.unwrap();
            if matches!(*handle.status.borrow(), SyncStatus::Failed(_)) {
                break;
            }
        }
    })
    .await
    .expect("panicking cycle must surface as SyncStatus::Failed");
    assert_eq!(
        *handle.status.borrow(),
        SyncStatus::Failed("sync engine crashed — restart notion-tui".into())
    );

    // (b) the loop must survive the panic and recover on a later cycle.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            handle.status.changed().await.unwrap();
            if matches!(*handle.status.borrow(), SyncStatus::Idle { .. }) {
                break;
            }
        }
    })
    .await
    .expect("sync loop must recover to Idle after the panic");

    let _ = server;
}

#[tokio::test]
async fn panic_while_holding_store_lock_poisons_mutex_and_sync_still_recovers() {
    let (server, client, store) = empty_workspace_fixture().await;

    // Test-only hook: on the first cycle, lock the store (like a real cycle body would while
    // doing store work) and panic while still holding the guard. This poisons the mutex from
    // *inside* a cycle, exercising the path where `lock_store`'s poison-recovery (see
    // `notion_sync::lock_store`) and the `catch_unwind` crash containment must both fire together —
    // the next cycle needs to recover the lock AND clear the Failed status. Every later cycle
    // runs untouched.
    let hook_store = store.clone();
    let mut handle =
        spawn_sync_with_test_hook(client, store.clone(), Duration::from_millis(20), move |cycle| {
            if cycle == 0 {
                let _guard = hook_store.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                panic!("injected test panic: panic while holding the store lock");
            }
        });

    // (a) the crash must be observed as SyncStatus::Failed with the exact crash message.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            handle.status.changed().await.unwrap();
            if matches!(*handle.status.borrow(), SyncStatus::Failed(_)) {
                break;
            }
        }
    })
    .await
    .expect("panic while holding the store lock must surface as SyncStatus::Failed");
    assert_eq!(
        *handle.status.borrow(),
        SyncStatus::Failed("sync engine crashed — restart notion-tui".into())
    );

    // (b) the loop must survive the panic, recover the poisoned lock, and reach Idle again.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            handle.status.changed().await.unwrap();
            if matches!(*handle.status.borrow(), SyncStatus::Idle { .. }) {
                break;
            }
        }
    })
    .await
    .expect("sync loop must recover to Idle after a panic while the store lock was held");

    // (c) after recovery, the store itself must still be usable: locking it and running a
    // trivial query must not error, even though the mutex was poisoned mid-cycle.
    let pending = store
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .pending_count()
        .expect("store must still be queryable after recovering from the poisoned lock");
    assert_eq!(pending, 0);

    let _ = server;
}
