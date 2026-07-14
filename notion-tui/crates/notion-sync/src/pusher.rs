use std::collections::HashSet;

use notion_api::{rich_text_plain, ApiError, NotionClient};
use notion_store::OpRec;
use serde_json::{json, Value};

use crate::{lock_store, SharedStore};

/// Create-type ops are not blind-retried by the client (Task 2's fail-fast policy), so an
/// ambiguous failure (network error / 5xx / retries exhausted) leaves them `inflight` for
/// verify-before-resend on the next pass, instead of being blindly resent or marked `failed`.
fn is_create_op(op_type: &str) -> bool {
    matches!(op_type, "append_block" | "create_row" | "create_comment")
}

fn is_ambiguous(e: &ApiError) -> bool {
    matches!(e, ApiError::Network(_) | ApiError::RetriesExhausted(_))
        || matches!(e, ApiError::Api { status, .. } if *status >= 500)
}

/// The title property of a Notion `properties` object is whichever entry has `"type": "title"`.
fn extract_title_text(properties: &Value) -> Option<String> {
    properties
        .as_object()?
        .values()
        .find(|p| p["type"] == "title")
        .map(|p| rich_text_plain(&p["title"]))
}

enum PushOutcome {
    Success,
    Conflicted,
    Failed(String),
}

/// Outcome of a `verify_*` check for a previously `inflight` create op.
enum VerifyResult {
    /// A remote match was found and the local record was successfully rewritten
    /// to the real id; the op is resolved.
    Adopted,
    /// No remote match: the create genuinely never landed. Safe to resend.
    NoMatch,
    /// A remote match was found, but rewriting the local id failed (e.g. a
    /// primary-key collision because the matched remote record is already known
    /// locally under a different id). Resending would create a duplicate on
    /// Notion, so the op must NOT be resent — leave it `inflight` and block the
    /// target for the next pass instead.
    RewriteFailed,
}

fn extract_page_id(op: &OpRec) -> Option<String> {
    match op.op_type.as_str() {
        "update_block" | "append_block" | "delete_block" | "reorder_block" => {
            let v: Value = serde_json::from_str(&op.payload).ok()?;
            v["page_id"].as_str().map(str::to_string)
        }
        "rename_page" => Some(op.target_id.clone()),
        "move_page" => Some(op.target_id.clone()),
        _ => None,
    }
}

fn remaining_ops_reference_page(store: &SharedStore, page_id: &str) -> bool {
    lock_store(store)
        .ops()
        .unwrap_or_default()
        .iter()
        .any(|o| extract_page_id(o).as_deref() == Some(page_id))
}

fn remaining_ops_reference_row(store: &SharedStore, row_id: &str) -> bool {
    lock_store(store).ops().unwrap_or_default().iter().any(|o| {
        matches!(
            o.op_type.as_str(),
            "update_row" | "create_row" | "delete_row" | "restore_row"
        ) && o.target_id == row_id
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
    client
        .update_block(&op.target_id, block_type, &block_payload)
        .await?;

    lock_store(store).delete_op(op.seq).ok();
    if !remaining_ops_reference_page(store, &page_id) {
        lock_store(store).clear_page_dirty(&page_id).ok();
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

    lock_store(store).delete_op(op.seq).ok();
    if !remaining_ops_reference_page(store, &page_id) {
        lock_store(store).clear_page_dirty(&page_id).ok();
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
    let container = payload["parent_id"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| page_id.clone());
    let after = payload["after"].as_str().map(str::to_string);
    let block_type = payload["block_type"].as_str().unwrap_or_default();
    let text = payload["text"].as_str().unwrap_or_default();
    let block_body = json!({ block_type: {"rich_text": [{"type": "text", "text": {"content": text}}]} });

    let resp = client
        .append_children(&container, after.as_deref(), block_body)
        .await?;
    let real_id = resp["results"][0]["id"].as_str().unwrap_or_default().to_string();

    {
        let mut guard = lock_store(store);
        guard.rewrite_block_id(&op.target_id, &real_id).ok();
        guard.delete_op(op.seq).ok();
    }
    if !remaining_ops_reference_page(store, &page_id) {
        lock_store(store).clear_page_dirty(&page_id).ok();
    }
    Ok(PushOutcome::Success)
}

/// Verifies whether a previously `inflight` append actually landed on Notion (response lost to
/// a network blip / 5xx) before resending it. Lists the container's children remotely; if a
/// *direct child* block with matching `plain_text` exists and isn't already known locally,
/// adopts its id instead of appending a duplicate.
async fn verify_append_block(
    client: &NotionClient,
    store: &SharedStore,
    op: &OpRec,
) -> Result<VerifyResult, ApiError> {
    let payload: Value = serde_json::from_str(&op.payload).unwrap_or_default();
    let page_id = payload["page_id"].as_str().unwrap_or_default().to_string();
    let container = payload["parent_id"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| page_id.clone());
    let text = payload["text"].as_str().unwrap_or_default();

    let remote = client.fetch_block_tree(&container).await?;
    let known_ids: HashSet<String> = lock_store(store)
        .page_blocks(&page_id)
        .unwrap_or_default()
        .into_iter()
        .map(|b| b.id)
        .collect();

    // `fetch_block_tree` returns the whole recursive subtree; only direct children of the
    // append's container are eligible candidates — a coincidentally-identical-text descendant
    // several levels down must not be adopted.
    let real_id = remote
        .iter()
        .filter(|fb| fb.parent_block_id.is_none())
        .find(|fb| fb.block.plain_text == text && !known_ids.contains(&fb.block.id))
        .map(|fb| fb.block.id.clone());

    let Some(real_id) = real_id else {
        return Ok(VerifyResult::NoMatch);
    };

    let rewritten = {
        let mut guard = lock_store(store);
        guard.rewrite_block_id(&op.target_id, &real_id).is_ok()
    };
    if !rewritten {
        return Ok(VerifyResult::RewriteFailed);
    }
    lock_store(store).delete_op(op.seq).ok();
    if !remaining_ops_reference_page(store, &page_id) {
        lock_store(store).clear_page_dirty(&page_id).ok();
    }
    Ok(VerifyResult::Adopted)
}

/// Verifies whether a previously `inflight` row creation actually landed, matching on the
/// title property's plain text against the local op payload's title. Excludes rows already
/// known locally (under a different, real id) so a title collision with an already-synced row
/// is never mistaken for this op's own creation.
async fn verify_create_row(
    client: &NotionClient,
    store: &SharedStore,
    op: &OpRec,
) -> Result<VerifyResult, ApiError> {
    let payload: Value = serde_json::from_str(&op.payload).unwrap_or_default();
    let data_source_id = payload["data_source_id"].as_str().unwrap_or_default().to_string();
    let local_title = extract_title_text(&payload["properties"]);

    let rows = client.query_data_source_all(&data_source_id).await?;
    let known_ids: HashSet<String> = lock_store(store)
        .rows(&data_source_id)
        .unwrap_or_default()
        .into_iter()
        .map(|r| r.id)
        .collect();
    let real_id = local_title.as_ref().and_then(|title| {
        rows.iter()
            .find(|r| !known_ids.contains(&r.id) && extract_title_text(&r.properties).as_ref() == Some(title))
            .map(|r| r.id.clone())
    });

    let Some(real_id) = real_id else {
        return Ok(VerifyResult::NoMatch);
    };

    let rewritten = {
        let mut guard = lock_store(store);
        guard.rewrite_row_id(&op.target_id, &real_id).is_ok()
    };
    if !rewritten {
        return Ok(VerifyResult::RewriteFailed);
    }
    lock_store(store).delete_op(op.seq).ok();
    if !remaining_ops_reference_row(store, &real_id) {
        lock_store(store).clear_row_dirty(&real_id).ok();
    }
    Ok(VerifyResult::Adopted)
}

/// Verifies whether a previously `inflight` comment creation actually landed, matching on body.
/// Excludes comments already known locally (under a different, real id) so a body collision
/// with an already-synced comment is never mistaken for this op's own creation.
async fn verify_create_comment(
    client: &NotionClient,
    store: &SharedStore,
    op: &OpRec,
) -> Result<VerifyResult, ApiError> {
    let payload: Value = serde_json::from_str(&op.payload).unwrap_or_default();
    let parent_id = payload["parent_id"].as_str().unwrap_or_default();
    let body = payload["body"].as_str().unwrap_or_default();

    let comments = client.list_comments(parent_id).await?;
    let known_ids: HashSet<String> = lock_store(store)
        .comments_for(parent_id)
        .unwrap_or_default()
        .into_iter()
        .map(|c| c.id)
        .collect();
    let real_id = comments
        .iter()
        .find(|c| c.body == body && !known_ids.contains(&c.id))
        .map(|c| c.id.clone());

    let Some(real_id) = real_id else {
        return Ok(VerifyResult::NoMatch);
    };

    let rewritten = {
        let mut guard = lock_store(store);
        guard.rewrite_comment_id(&op.target_id, &real_id).is_ok()
    };
    if !rewritten {
        return Ok(VerifyResult::RewriteFailed);
    }
    lock_store(store).delete_op(op.seq).ok();
    Ok(VerifyResult::Adopted)
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
    client
        .update_page(&op.target_id, json!({"properties": properties}))
        .await?;

    lock_store(store).delete_op(op.seq).ok();
    if !remaining_ops_reference_row(store, &op.target_id) {
        lock_store(store).clear_row_dirty(&op.target_id).ok();
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
        let mut guard = lock_store(store);
        guard.rewrite_row_id(&op.target_id, &real_id).ok();
        guard.delete_op(op.seq).ok();
    }
    if !remaining_ops_reference_row(store, &real_id) {
        lock_store(store).clear_row_dirty(&real_id).ok();
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
    client
        .update_page(&op.target_id, json!({"archived": true}))
        .await?;

    lock_store(store).delete_op(op.seq).ok();
    if !remaining_ops_reference_row(store, &op.target_id) {
        lock_store(store).clear_row_dirty(&op.target_id).ok();
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
    client
        .update_page(&op.target_id, json!({"archived": false}))
        .await?;

    lock_store(store).delete_op(op.seq).ok();
    if !remaining_ops_reference_row(store, &op.target_id) {
        lock_store(store).clear_row_dirty(&op.target_id).ok();
    }
    Ok(PushOutcome::Success)
}

async fn push_reorder_block(store: &SharedStore, op: &OpRec) -> Result<PushOutcome, ApiError> {
    // Notion's Blocks API has no reorder/move endpoint — there is nothing to call.
    // This op exists purely to drive the same dirty-clearing machinery every
    // other op type uses (see plan design notes).
    let payload: Value = serde_json::from_str(&op.payload).unwrap_or_default();
    let page_id = payload["page_id"].as_str().unwrap_or_default().to_string();

    lock_store(store).delete_op(op.seq).ok();
    if !remaining_ops_reference_page(store, &page_id) {
        lock_store(store).clear_page_dirty(&page_id).ok();
    }
    Ok(PushOutcome::Success)
}

async fn push_create_comment(
    client: &NotionClient,
    store: &SharedStore,
    op: &OpRec,
) -> Result<PushOutcome, ApiError> {
    let payload: Value = serde_json::from_str(&op.payload).unwrap_or_default();
    let body = payload["body"].as_str().unwrap_or_default();
    let resp = if let Some(discussion_id) = payload["thread_id"].as_str() {
        client.create_comment_reply(discussion_id, body).await?
    } else {
        let parent_id = payload["parent_id"].as_str().unwrap_or_default();
        let parent = match payload["parent_kind"].as_str() {
            Some("block") => json!({"block_id": parent_id}),
            _ => json!({"page_id": parent_id}),
        };
        client.create_comment(parent, body).await?
    };
    let real_id = resp["id"].as_str().unwrap_or_default().to_string();

    let mut guard = lock_store(store);
    guard.rewrite_comment_id(&op.target_id, &real_id).ok();
    guard.delete_op(op.seq).ok();
    Ok(PushOutcome::Success)
}

async fn push_rename_page(
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
    let title = payload["title"].as_str().unwrap_or_default();
    let body = json!({
        "properties": {"title": {"title": [{"type": "text", "text": {"content": title}}]}}
    });
    client.update_page(&op.target_id, body).await?;

    lock_store(store).delete_op(op.seq).ok();
    if !remaining_ops_reference_page(store, &op.target_id) {
        lock_store(store).clear_page_dirty(&op.target_id).ok();
    }
    Ok(PushOutcome::Success)
}

async fn push_move_page(
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
    let new_parent_id = payload["new_parent_id"].as_str().unwrap_or_default();
    client
        .update_page(&op.target_id, json!({"parent": {"page_id": new_parent_id}}))
        .await?;
    lock_store(store).delete_op(op.seq).ok();
    if !remaining_ops_reference_page(store, &op.target_id) {
        lock_store(store).clear_page_dirty(&op.target_id).ok();
    }
    Ok(PushOutcome::Success)
}

async fn push_one(client: &NotionClient, store: &SharedStore, op: &OpRec) -> Result<PushOutcome, ApiError> {
    match op.op_type.as_str() {
        "update_block" => push_update_block(client, store, op).await,
        "delete_block" => push_delete_block(client, store, op).await,
        "append_block" => push_append_block(client, store, op).await,
        "reorder_block" => push_reorder_block(store, op).await,
        "update_row" => push_update_row(client, store, op).await,
        "create_row" => push_create_row(client, store, op).await,
        "delete_row" => push_delete_row(client, store, op).await,
        "restore_row" => push_restore_row(client, store, op).await,
        "create_comment" => push_create_comment(client, store, op).await,
        "rename_page" => push_rename_page(client, store, op).await,
        "move_page" => push_move_page(client, store, op).await,
        other => Ok(PushOutcome::Failed(format!("unknown op_type {other}"))),
    }
}

/// Drains `pending_ops` in sequence order, FIFO per target. A failed or conflicted op blocks
/// later ops against the same target (so edits to one page/row serialize correctly) but never
/// blocks unrelated targets. Returns the number of ops successfully pushed this pass; a network
/// error aborts the pass early (remaining ops stay `pending` for the next cycle).
pub async fn push_once(client: &NotionClient, store: &SharedStore) -> Result<u32, ApiError> {
    let ops = lock_store(store).ops().unwrap_or_default();
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

        // A prior pass may have died ambiguously right after this create reached Notion.
        // Verify against the remote before resending, so we don't create a duplicate.
        if is_create_op(&op.op_type) && op.state == "inflight" {
            let verified = match op.op_type.as_str() {
                "append_block" => verify_append_block(client, store, &op).await,
                "create_row" => verify_create_row(client, store, &op).await,
                "create_comment" => verify_create_comment(client, store, &op).await,
                _ => unreachable!("is_create_op only matches the three arms above"),
            };
            match verified {
                Ok(VerifyResult::Adopted) => {
                    pushed += 1;
                    continue;
                }
                Ok(VerifyResult::NoMatch) => {
                    // No remote match: fall through and resend as normal.
                }
                Ok(VerifyResult::RewriteFailed) => {
                    // A remote match existed but adopting it locally failed (e.g. a PK
                    // collision with an id already known under a different local record).
                    // Resending would create a duplicate on Notion — leave the op `inflight`
                    // and block the target instead of silently dropping it.
                    blocked.insert(op.target_id.clone());
                    continue;
                }
                Err(e) if is_ambiguous(&e) => {
                    // Ambiguous verification-call failure (network error / 5xx / retries
                    // exhausted): stay `inflight` for the next pass to retry verification.
                    blocked.insert(op.target_id.clone());
                    continue;
                }
                Err(e) => {
                    // Definite rejection of the verification call itself (e.g. 404 — the
                    // container/data source/parent was deleted). There's no path to success:
                    // mark the op `failed` rather than leaving it inflight forever.
                    lock_store(store)
                        .set_op_state(op.seq, "failed", Some(&e.to_string()))
                        .ok();
                    blocked.insert(op.target_id.clone());
                    continue;
                }
            }
        }

        if is_create_op(&op.op_type) {
            lock_store(store).set_op_state(op.seq, "inflight", None).ok();
        }

        match push_one(client, store, &op).await {
            Ok(PushOutcome::Success) => pushed += 1,
            Ok(PushOutcome::Conflicted) => {
                lock_store(store).set_op_state(op.seq, "conflicted", None).ok();
                blocked.insert(op.target_id.clone());
            }
            Ok(PushOutcome::Failed(msg)) => {
                lock_store(store).set_op_state(op.seq, "failed", Some(&msg)).ok();
                blocked.insert(op.target_id.clone());
            }
            Err(ApiError::Network(e)) => return Err(ApiError::Network(e)),
            Err(e) if is_create_op(&op.op_type) && is_ambiguous(&e) => {
                // Ambiguous 5xx/retries-exhausted on a create: stays `inflight` for the next
                // pass to verify-before-resend. Block the target so nothing races ahead of it.
                blocked.insert(op.target_id.clone());
            }
            Err(e) => {
                lock_store(store)
                    .set_op_state(op.seq, "failed", Some(&e.to_string()))
                    .ok();
                blocked.insert(op.target_id.clone());
            }
        }
    }
    Ok(pushed)
}
