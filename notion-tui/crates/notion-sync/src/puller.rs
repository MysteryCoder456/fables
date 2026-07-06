use notion_api::{ApiError, NotionClient, ParentRef, SearchItem};
use notion_store::{BlockRec, DataSourceRec, PageRec, RowRec};

use crate::SharedStore;

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
                    let dirty = store.lock().unwrap().is_page_dirty(&p.id).unwrap_or(false);
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
                    store.lock().unwrap().replace_page_blocks(&p.id, &recs).ok();
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
                    store.lock().unwrap().replace_comments(&p.id, &comment_recs).ok();
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
