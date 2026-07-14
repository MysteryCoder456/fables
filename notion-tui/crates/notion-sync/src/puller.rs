use notion_api::{NotionClient, ParentRef, SearchItem};
use notion_store::{BlockRec, DataSourceRec, PageRec, RowRec};
use tokio::sync::watch;

use crate::{lock_store, SharedStore, SyncError, SyncStatus};

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

pub async fn pull_once(
    client: &NotionClient,
    store: &SharedStore,
    status_tx: &watch::Sender<SyncStatus>,
) -> Result<u32, SyncError> {
    let hwm = lock_store(store)
        .meta_get("hwm")
        .map_err(|e| SyncError::Store(e.to_string()))?
        .unwrap_or_default();
    let mut cursor = lock_store(store)
        .meta_get("pull_cursor")
        .map_err(|e| SyncError::Store(e.to_string()))?
        .filter(|c| !c.is_empty());
    let mut max_seen = lock_store(store)
        .meta_get("pull_max_seen")
        .map_err(|e| SyncError::Store(e.to_string()))?
        .unwrap_or_else(|| hwm.clone());
    let mut done_count: u32 = lock_store(store)
        .meta_get("pull_done")
        .map_err(|e| SyncError::Store(e.to_string()))?
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let mut updated: u32 = 0;
    let mut finished = false;

    while !finished {
        let page = client.search_page(cursor.as_deref()).await?;
        let has_more = page.next_cursor.is_some();
        for item in &page.items {
            let edited = match item {
                SearchItem::Page(p) => p.last_edited_time.clone(),
                SearchItem::DataSource(d) => d.last_edited_time.clone(),
                SearchItem::Other => continue,
            };
            if !hwm.is_empty() && edited.as_str() <= hwm.as_str() {
                finished = true;
                break;
            }
            if edited > max_seen {
                max_seen = edited.clone();
            }
            match item {
                SearchItem::Page(p) => {
                    let (parent_type, parent_id) = parent_cols(&p.parent);
                    lock_store(store)
                        .upsert_page(&PageRec {
                            id: p.id.clone(),
                            parent_type,
                            parent_id,
                            title: p.title.clone(),
                            icon: p.icon.clone(),
                            archived: p.archived,
                            last_edited_time: p.last_edited_time.clone(),
                        })
                        .map_err(|e| SyncError::Store(e.to_string()))?;
                    let dirty = lock_store(store).is_page_dirty(&p.id).unwrap_or(false);
                    if dirty {
                        continue;
                    }
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
                    lock_store(store)
                        .replace_page_blocks(&p.id, &recs)
                        .map_err(|e| SyncError::Store(e.to_string()))?;
                    let comments = client.list_comments(&p.id).await?;
                    let comment_recs: Vec<notion_store::CommentRec> = comments
                        .iter()
                        .map(|c| notion_store::CommentRec {
                            id: c.id.clone(),
                            parent_id: p.id.clone(),
                            parent_kind: "page".into(),
                            thread_id: Some(c.discussion_id.clone()),
                            author: c.author.clone(),
                            body: c.body.clone(),
                            created_time: c.created_time.clone(),
                        })
                        .collect();
                    lock_store(store)
                        .replace_comments(&p.id, &comment_recs)
                        .map_err(|e| SyncError::Store(e.to_string()))?;
                    updated += 1;
                }
                SearchItem::DataSource(d) => {
                    let ds = client.get_data_source(&d.id).await?;
                    lock_store(store)
                        .upsert_data_source(&DataSourceRec {
                            id: ds.meta.id.clone(),
                            database_id: ds.meta.database_id.clone(),
                            title: ds.meta.title.clone(),
                            schema_json: ds.schema.to_string(),
                            last_edited_time: ds.meta.last_edited_time.clone(),
                        })
                        .map_err(|e| SyncError::Store(e.to_string()))?;
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
                    lock_store(store)
                        .replace_rows(&d.id, &recs)
                        .map_err(|e| SyncError::Store(e.to_string()))?;
                    updated += 1;
                }
                SearchItem::Other => {}
            }
            done_count += 1;
            let total_estimate = (done_count + if has_more { 100 } else { 0 }).max(done_count);
            status_tx.send_replace(SyncStatus::Syncing {
                done: done_count,
                total: total_estimate,
            });
        }
        cursor = page.next_cursor.clone();
        lock_store(store)
            .meta_set("pull_cursor", cursor.as_deref().unwrap_or(""))
            .map_err(|e| SyncError::Store(e.to_string()))?;
        lock_store(store)
            .meta_set("pull_done", &done_count.to_string())
            .map_err(|e| SyncError::Store(e.to_string()))?;
        lock_store(store)
            .meta_set("pull_max_seen", &max_seen)
            .map_err(|e| SyncError::Store(e.to_string()))?;
        if cursor.is_none() {
            finished = true;
        }
    }

    if max_seen > hwm {
        lock_store(store)
            .meta_set("hwm", &max_seen)
            .map_err(|e| SyncError::Store(e.to_string()))?;
    }
    lock_store(store)
        .meta_delete("pull_cursor")
        .map_err(|e| SyncError::Store(e.to_string()))?;
    lock_store(store)
        .meta_delete("pull_max_seen")
        .map_err(|e| SyncError::Store(e.to_string()))?;
    lock_store(store)
        .meta_delete("pull_done")
        .map_err(|e| SyncError::Store(e.to_string()))?;
    Ok(updated)
}

/// A full, unfiltered workspace crawl (no hwm cutoff) used purely to build
/// the "seen" set for `Store::prune_missing`. Runs far less often than
/// `pull_once` (see `spawn_sync`'s cycle counter) since it always pages
/// through the entire workspace.
pub async fn reconcile_deletions(
    client: &NotionClient,
    store: &SharedStore,
) -> Result<(u32, u32), SyncError> {
    let mut seen_pages = Vec::new();
    let mut seen_ds = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let page = client.search_page(cursor.as_deref()).await?;
        for item in &page.items {
            match item {
                SearchItem::Page(p) => seen_pages.push(p.id.clone()),
                SearchItem::DataSource(d) => seen_ds.push(d.id.clone()),
                SearchItem::Other => {}
            }
        }
        cursor = page.next_cursor.clone();
        if cursor.is_none() {
            break;
        }
    }
    lock_store(store)
        .prune_missing(&seen_pages, &seen_ds)
        .map_err(|e| SyncError::Store(e.to_string()))
}
