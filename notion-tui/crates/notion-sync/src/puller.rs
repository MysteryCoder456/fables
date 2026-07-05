use std::sync::Arc;
use std::time::Duration;

use notion_api::{ApiError, NotionClient, ParentRef, SearchItem};
use notion_store::{BlockRec, DataSourceRec, PageRec, RowRec};
use tokio::sync::watch;

use crate::{SharedStore, SyncHandle, SyncStatus};

fn parent_cols(p: &ParentRef) -> (String, Option<String>) {
    match p {
        ParentRef::Workspace => ("workspace".into(), None),
        ParentRef::Page(id) => ("page_id".into(), Some(id.clone())),
        ParentRef::DataSource(id) => ("data_source_id".into(), Some(id.clone())),
        ParentRef::Database(id) => ("database_id".into(), Some(id.clone())),
        ParentRef::Block(id) => ("page_id".into(), Some(id.clone())),
        ParentRef::Unknown => ("unknown".into(), None),
    }
}

pub async fn pull_once(client: &NotionClient, store: &SharedStore) -> Result<u32, ApiError> {
    let hwm = store.lock().unwrap().meta_get("hwm").ok().flatten().unwrap_or_default();
    let mut max_seen = hwm.clone();
    let mut updated: u32 = 0;
    let mut cursor: Option<String> = None;
    let mut done = false;

    while !done {
        let page = client.search_page(cursor.as_deref()).await?;
        for item in &page.items {
            let edited = match item {
                SearchItem::Page(p) => p.last_edited_time.clone(),
                SearchItem::DataSource(d) => d.last_edited_time.clone(),
                SearchItem::Other => continue,
            };
            if !hwm.is_empty() && edited.as_str() <= hwm.as_str() {
                done = true;
                break;
            }
            if edited > max_seen {
                max_seen = edited.clone();
            }
            match item {
                SearchItem::Page(p) => {
                    let (parent_type, parent_id) = parent_cols(&p.parent);
                    store
                        .lock()
                        .unwrap()
                        .upsert_page(&PageRec {
                            id: p.id.clone(),
                            parent_type,
                            parent_id,
                            title: p.title.clone(),
                            icon: p.icon.clone(),
                            archived: p.archived,
                            last_edited_time: p.last_edited_time.clone(),
                        })
                        .ok();
                    let flat = client.fetch_block_tree(&p.id).await?;
                    let recs: Vec<BlockRec> = flat
                        .iter()
                        .map(|f| BlockRec {
                            id: f.block.id.clone(),
                            page_id: p.id.clone(),
                            parent_block_id: f.parent_block_id.clone(),
                            ordinal: f.ordinal,
                            block_type: f.block.block_type.clone(),
                            payload: f.block.payload.to_string(),
                            plain_text: f.block.plain_text.clone(),
                            has_children: f.block.has_children,
                        })
                        .collect();
                    store.lock().unwrap().replace_page_blocks(&p.id, &recs).ok();
                    updated += 1;
                }
                SearchItem::DataSource(d) => {
                    let ds = client.get_data_source(&d.id).await?;
                    store
                        .lock()
                        .unwrap()
                        .upsert_data_source(&DataSourceRec {
                            id: ds.meta.id.clone(),
                            database_id: ds.meta.database_id.clone(),
                            title: ds.meta.title.clone(),
                            schema_json: ds.schema.to_string(),
                            last_edited_time: ds.meta.last_edited_time.clone(),
                        })
                        .ok();
                    let rows = client.query_data_source_all(&d.id).await?;
                    let recs: Vec<RowRec> = rows
                        .iter()
                        .map(|r| RowRec {
                            id: r.id.clone(),
                            data_source_id: d.id.clone(),
                            properties: r.properties.to_string(),
                            last_edited_time: r.last_edited_time.clone(),
                            archived: r.archived,
                        })
                        .collect();
                    store.lock().unwrap().replace_rows(&d.id, &recs).ok();
                    updated += 1;
                }
                SearchItem::Other => {}
            }
        }
        cursor = page.next_cursor.clone();
        if cursor.is_none() {
            done = true;
        }
    }

    if max_seen > hwm {
        store.lock().unwrap().meta_set("hwm", &max_seen).ok();
    }
    Ok(updated)
}

pub fn spawn_puller(client: NotionClient, store: SharedStore, interval: Duration) -> SyncHandle {
    let (status_tx, status_rx) = watch::channel(SyncStatus::Starting);
    let (data_tx, data_rx) = watch::channel(0u64);
    tokio::spawn(async move {
        let client = Arc::new(client);
        loop {
            status_tx.send_replace(SyncStatus::Syncing { done: 0 });
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
            tokio::time::sleep(interval).await;
        }
    });
    SyncHandle {
        status: status_rx,
        data_version: data_rx,
    }
}
