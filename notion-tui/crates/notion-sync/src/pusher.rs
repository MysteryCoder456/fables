use std::collections::HashSet;

use notion_api::{ApiError, NotionClient};
use notion_store::OpRec;
use serde_json::{json, Value};

use crate::SharedStore;

enum PushOutcome {
    Success,
    Conflicted,
    Failed(String),
}

fn extract_page_id(op: &OpRec) -> Option<String> {
    match op.op_type.as_str() {
        "update_block" | "append_block" | "delete_block" => {
            let v: Value = serde_json::from_str(&op.payload).ok()?;
            v["page_id"].as_str().map(str::to_string)
        }
        _ => None,
    }
}

fn remaining_ops_reference_page(store: &SharedStore, page_id: &str) -> bool {
    store
        .lock()
        .unwrap()
        .ops()
        .unwrap_or_default()
        .iter()
        .any(|o| extract_page_id(o).as_deref() == Some(page_id))
}

fn remaining_ops_reference_row(store: &SharedStore, row_id: &str) -> bool {
    store
        .lock()
        .unwrap()
        .ops()
        .unwrap_or_default()
        .iter()
        .any(|o| {
            matches!(o.op_type.as_str(), "update_row" | "create_row" | "delete_row" | "restore_row")
                && o.target_id == row_id
        })
}

async fn push_update_block(
    client: &NotionClient,
    store: &SharedStore,
    op: &OpRec,
) -> Result<PushOutcome, ApiError> {
    let payload: Value = serde_json::from_str(&op.payload).unwrap_or_default();
    let page_id = payload["page_id"].as_str().unwrap_or_default().to_string();
    if let Some(base) = &op.base_edited_time {
        if !base.is_empty() {
            let current = client.get_page_edited_time(&page_id).await?;
            if current > *base {
                return Ok(PushOutcome::Conflicted);
            }
        }
    }
    let block_type = payload["block_type"].as_str().unwrap_or_default();
    let block_payload: Value = payload["block_payload"]
        .as_str()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    client.update_block(&op.target_id, block_type, &block_payload).await?;

    store.lock().unwrap().delete_op(op.seq).ok();
    if !remaining_ops_reference_page(store, &page_id) {
        store.lock().unwrap().clear_page_dirty(&page_id).ok();
    }
    Ok(PushOutcome::Success)
}

async fn push_delete_block(
    client: &NotionClient,
    store: &SharedStore,
    op: &OpRec,
) -> Result<PushOutcome, ApiError> {
    let payload: Value = serde_json::from_str(&op.payload).unwrap_or_default();
    let page_id = payload["page_id"].as_str().unwrap_or_default().to_string();
    if let Some(base) = &op.base_edited_time {
        if !base.is_empty() {
            let current = client.get_page_edited_time(&page_id).await?;
            if current > *base {
                return Ok(PushOutcome::Conflicted);
            }
        }
    }
    client.delete_block(&op.target_id).await?;

    store.lock().unwrap().delete_op(op.seq).ok();
    if !remaining_ops_reference_page(store, &page_id) {
        store.lock().unwrap().clear_page_dirty(&page_id).ok();
    }
    Ok(PushOutcome::Success)
}

async fn push_append_block(
    client: &NotionClient,
    store: &SharedStore,
    op: &OpRec,
) -> Result<PushOutcome, ApiError> {
    let payload: Value = serde_json::from_str(&op.payload).unwrap_or_default();
    let page_id = payload["page_id"].as_str().unwrap_or_default().to_string();
    let container = payload["parent_id"].as_str().map(str::to_string).unwrap_or_else(|| page_id.clone());
    let after = payload["after"].as_str().map(str::to_string);
    let block_type = payload["block_type"].as_str().unwrap_or_default();
    let text = payload["text"].as_str().unwrap_or_default();
    let block_body = json!({ block_type: {"rich_text": [{"type": "text", "text": {"content": text}}]} });

    let resp = client.append_children(&container, after.as_deref(), block_body).await?;
    let real_id = resp["results"][0]["id"].as_str().unwrap_or_default().to_string();

    {
        let mut guard = store.lock().unwrap();
        guard.rewrite_block_id(&op.target_id, &real_id).ok();
        guard.delete_op(op.seq).ok();
    }
    if !remaining_ops_reference_page(store, &page_id) {
        store.lock().unwrap().clear_page_dirty(&page_id).ok();
    }
    Ok(PushOutcome::Success)
}

async fn push_update_row(
    client: &NotionClient,
    store: &SharedStore,
    op: &OpRec,
) -> Result<PushOutcome, ApiError> {
    if let Some(base) = &op.base_edited_time {
        if !base.is_empty() {
            let current = client.get_page_edited_time(&op.target_id).await?;
            if current > *base {
                return Ok(PushOutcome::Conflicted);
            }
        }
    }
    let payload: Value = serde_json::from_str(&op.payload).unwrap_or_default();
    let properties = payload["properties"].clone();
    client.update_page(&op.target_id, json!({"properties": properties})).await?;

    store.lock().unwrap().delete_op(op.seq).ok();
    if !remaining_ops_reference_row(store, &op.target_id) {
        store.lock().unwrap().clear_row_dirty(&op.target_id).ok();
    }
    Ok(PushOutcome::Success)
}

async fn push_create_row(
    client: &NotionClient,
    store: &SharedStore,
    op: &OpRec,
) -> Result<PushOutcome, ApiError> {
    let payload: Value = serde_json::from_str(&op.payload).unwrap_or_default();
    let data_source_id = payload["data_source_id"].as_str().unwrap_or_default().to_string();
    let properties = payload["properties"].clone();
    let parent = json!({"data_source_id": data_source_id});
    let resp = client.create_page(parent, properties).await?;
    let real_id = resp["id"].as_str().unwrap_or_default().to_string();

    {
        let mut guard = store.lock().unwrap();
        guard.rewrite_row_id(&op.target_id, &real_id).ok();
        guard.delete_op(op.seq).ok();
    }
    if !remaining_ops_reference_row(store, &real_id) {
        store.lock().unwrap().clear_row_dirty(&real_id).ok();
    }
    Ok(PushOutcome::Success)
}

async fn push_delete_row(
    client: &NotionClient,
    store: &SharedStore,
    op: &OpRec,
) -> Result<PushOutcome, ApiError> {
    if let Some(base) = &op.base_edited_time {
        if !base.is_empty() {
            let current = client.get_page_edited_time(&op.target_id).await?;
            if current > *base {
                return Ok(PushOutcome::Conflicted);
            }
        }
    }
    client.update_page(&op.target_id, json!({"archived": true})).await?;

    store.lock().unwrap().delete_op(op.seq).ok();
    if !remaining_ops_reference_row(store, &op.target_id) {
        store.lock().unwrap().clear_row_dirty(&op.target_id).ok();
    }
    Ok(PushOutcome::Success)
}

async fn push_restore_row(
    client: &NotionClient,
    store: &SharedStore,
    op: &OpRec,
) -> Result<PushOutcome, ApiError> {
    if let Some(base) = &op.base_edited_time {
        if !base.is_empty() {
            let current = client.get_page_edited_time(&op.target_id).await?;
            if current > *base {
                return Ok(PushOutcome::Conflicted);
            }
        }
    }
    client.update_page(&op.target_id, json!({"archived": false})).await?;

    store.lock().unwrap().delete_op(op.seq).ok();
    if !remaining_ops_reference_row(store, &op.target_id) {
        store.lock().unwrap().clear_row_dirty(&op.target_id).ok();
    }
    Ok(PushOutcome::Success)
}

async fn push_one(client: &NotionClient, store: &SharedStore, op: &OpRec) -> Result<PushOutcome, ApiError> {
    match op.op_type.as_str() {
        "update_block" => push_update_block(client, store, op).await,
        "delete_block" => push_delete_block(client, store, op).await,
        "append_block" => push_append_block(client, store, op).await,
        "update_row" => push_update_row(client, store, op).await,
        "create_row" => push_create_row(client, store, op).await,
        "delete_row" => push_delete_row(client, store, op).await,
        "restore_row" => push_restore_row(client, store, op).await,
        other => Ok(PushOutcome::Failed(format!("unknown op_type {other}"))),
    }
}

/// Drains `pending_ops` in sequence order, FIFO per target. A failed or conflicted op blocks
/// later ops against the same target (so edits to one page/row serialize correctly) but never
/// blocks unrelated targets. Returns the number of ops successfully pushed this pass; a network
/// error aborts the pass early (remaining ops stay `pending` for the next cycle).
pub async fn push_once(client: &NotionClient, store: &SharedStore) -> Result<u32, ApiError> {
    let ops = store.lock().unwrap().ops().unwrap_or_default();
    let mut blocked: HashSet<String> = HashSet::new();
    let mut pushed = 0u32;

    for op in ops {
        if op.state == "failed" || op.state == "conflicted" {
            blocked.insert(op.target_id.clone());
            continue;
        }
        if blocked.contains(&op.target_id) {
            continue;
        }

        match push_one(client, store, &op).await {
            Ok(PushOutcome::Success) => pushed += 1,
            Ok(PushOutcome::Conflicted) => {
                store.lock().unwrap().set_op_state(op.seq, "conflicted", None).ok();
                blocked.insert(op.target_id.clone());
            }
            Ok(PushOutcome::Failed(msg)) => {
                store.lock().unwrap().set_op_state(op.seq, "failed", Some(&msg)).ok();
                blocked.insert(op.target_id.clone());
            }
            Err(ApiError::Network(e)) => return Err(ApiError::Network(e)),
            Err(e) => {
                store.lock().unwrap().set_op_state(op.seq, "failed", Some(&e.to_string())).ok();
                blocked.insert(op.target_id.clone());
            }
        }
    }
    Ok(pushed)
}
