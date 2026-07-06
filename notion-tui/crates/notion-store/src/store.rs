use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::Path;

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> anyhow::Result<Store> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        crate::schema::migrate(&conn)?;
        Ok(Store { conn })
    }

    pub fn open_in_memory() -> anyhow::Result<Store> {
        let conn = Connection::open_in_memory()?;
        crate::schema::migrate(&conn)?;
        Ok(Store { conn })
    }

    /// Raw connection access (tests and internal use).
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn upsert_page(&self, p: &PageRec) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO pages (id, parent_type, parent_id, title, icon, archived, last_edited_time)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET parent_type=?2, parent_id=?3, title=?4, icon=?5,
                                           archived=?6, last_edited_time=?7
             WHERE dirty = 0",
            rusqlite::params![
                p.id,
                p.parent_type,
                p.parent_id,
                p.title,
                p.icon,
                p.archived as i64,
                p.last_edited_time
            ],
        )?;
        Ok(())
    }

    pub fn replace_page_blocks(&mut self, page_id: &str, blocks: &[BlockRec]) -> anyhow::Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM blocks WHERE page_id = ?1", [page_id])?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO blocks (id, page_id, parent_block_id, ordinal, block_type,
                                     payload, plain_text, has_children)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?;
            for b in blocks {
                stmt.execute(rusqlite::params![
                    b.id,
                    b.page_id,
                    b.parent_block_id,
                    b.ordinal,
                    b.block_type,
                    b.payload,
                    b.plain_text,
                    b.has_children as i64
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn upsert_data_source(&self, ds: &DataSourceRec) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO data_sources (id, database_id, title, schema_json, last_edited_time)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET database_id=?2, title=?3, schema_json=?4,
                                           last_edited_time=?5",
            rusqlite::params![ds.id, ds.database_id, ds.title, ds.schema_json, ds.last_edited_time],
        )?;
        Ok(())
    }

    pub fn replace_rows(&mut self, data_source_id: &str, rows: &[RowRec]) -> anyhow::Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM rows WHERE data_source_id = ?1 AND dirty = 0",
            [data_source_id],
        )?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO rows (id, data_source_id, properties, last_edited_time, archived)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(id) DO UPDATE SET data_source_id=?2, properties=?3,
                                               last_edited_time=?4, archived=?5
                 WHERE dirty = 0",
            )?;
            for r in rows {
                stmt.execute(rusqlite::params![
                    r.id,
                    r.data_source_id,
                    r.properties,
                    r.last_edited_time,
                    r.archived as i64
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn get_page(&self, id: &str) -> anyhow::Result<Option<PageRec>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, parent_type, parent_id, title, icon, archived, last_edited_time
             FROM pages WHERE id = ?1",
        )?;
        let mut rows = stmt.query([id])?;
        Ok(match rows.next()? {
            Some(r) => Some(PageRec {
                id: r.get(0)?,
                parent_type: r.get(1)?,
                parent_id: r.get(2)?,
                title: r.get(3)?,
                icon: r.get(4)?,
                archived: r.get::<_, i64>(5)? != 0,
                last_edited_time: r.get(6)?,
            }),
            None => None,
        })
    }

    pub fn page_blocks(&self, page_id: &str) -> anyhow::Result<Vec<BlockRec>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, page_id, parent_block_id, ordinal, block_type, payload, plain_text, has_children
             FROM blocks WHERE page_id = ?1
             ORDER BY parent_block_id IS NOT NULL, parent_block_id, ordinal",
        )?;
        let out = stmt
            .query_map([page_id], |r| {
                Ok(BlockRec {
                    id: r.get(0)?,
                    page_id: r.get(1)?,
                    parent_block_id: r.get(2)?,
                    ordinal: r.get(3)?,
                    block_type: r.get(4)?,
                    payload: r.get(5)?,
                    plain_text: r.get(6)?,
                    has_children: r.get::<_, i64>(7)? != 0,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(out)
    }

    pub fn sidebar_nodes(&self) -> anyhow::Result<Vec<TreeNode>> {
        let mut out = Vec::new();
        let mut stmt = self.conn.prepare(
            "SELECT id, title, parent_id FROM pages
             WHERE archived = 0 AND parent_type IN ('workspace', 'page_id')
             ORDER BY title COLLATE NOCASE",
        )?;
        let pages = stmt.query_map([], |r| {
            Ok(TreeNode {
                id: r.get(0)?,
                title: r.get(1)?,
                parent_id: r.get(2)?,
                kind: NodeKind::Page,
            })
        })?;
        for n in pages {
            out.push(n?);
        }
        let mut stmt = self
            .conn
            .prepare("SELECT id, title FROM data_sources ORDER BY title COLLATE NOCASE")?;
        let sources = stmt.query_map([], |r| {
            Ok(TreeNode {
                id: r.get(0)?,
                title: r.get(1)?,
                parent_id: None,
                kind: NodeKind::DataSource,
            })
        })?;
        for n in sources {
            out.push(n?);
        }
        Ok(out)
    }

    pub fn get_data_source(&self, id: &str) -> anyhow::Result<Option<DataSourceRec>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, database_id, title, schema_json, last_edited_time
             FROM data_sources WHERE id = ?1",
        )?;
        let mut rows = stmt.query([id])?;
        Ok(match rows.next()? {
            Some(r) => Some(DataSourceRec {
                id: r.get(0)?,
                database_id: r.get(1)?,
                title: r.get(2)?,
                schema_json: r.get(3)?,
                last_edited_time: r.get(4)?,
            }),
            None => None,
        })
    }

    pub fn rows(&self, data_source_id: &str) -> anyhow::Result<Vec<RowRec>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, data_source_id, properties, last_edited_time, archived
             FROM rows WHERE data_source_id = ?1 AND archived = 0",
        )?;
        let out = stmt
            .query_map([data_source_id], |r| {
                Ok(RowRec {
                    id: r.get(0)?,
                    data_source_id: r.get(1)?,
                    properties: r.get(2)?,
                    last_edited_time: r.get(3)?,
                    archived: r.get::<_, i64>(4)? != 0,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(out)
    }

    pub fn search(&self, query: &str) -> anyhow::Result<Vec<SearchHit>> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        // Quote the query so user input can't hit FTS syntax errors; suffix * for prefix match.
        let fts_query = format!("\"{}\"*", query.replace('"', " "));
        let mut stmt = self.conn.prepare(
            "SELECT f.page_id, COALESCE(p.title, ''), snippet(fts, 0, '', '', '…', 12)
             FROM fts f JOIN pages p ON p.id = f.page_id
             WHERE fts MATCH ?1 ORDER BY rank LIMIT 50",
        )?;
        let out = stmt
            .query_map([&fts_query], |r| {
                Ok(SearchHit {
                    page_id: r.get(0)?,
                    title: r.get(1)?,
                    snippet: r.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(out)
    }

    pub fn meta_get(&self, key: &str) -> anyhow::Result<Option<String>> {
        let mut stmt = self.conn.prepare("SELECT value FROM sync_meta WHERE key = ?1")?;
        let mut rows = stmt.query([key])?;
        Ok(match rows.next()? {
            Some(r) => Some(r.get(0)?),
            None => None,
        })
    }

    pub fn meta_set(&self, key: &str, value: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO sync_meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = ?2",
            rusqlite::params![key, value],
        )?;
        Ok(())
    }

    pub fn enqueue_op(
        &self,
        op_type: &str,
        target_id: &str,
        payload: &str,
        base: Option<&str>,
    ) -> anyhow::Result<i64> {
        self.conn.execute(
            "INSERT INTO pending_ops (op_type, target_id, payload, base_edited_time)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![op_type, target_id, payload, base],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn ops(&self) -> anyhow::Result<Vec<OpRec>> {
        let mut stmt = self.conn.prepare(
            "SELECT seq, op_type, target_id, payload, base_edited_time, state, error
             FROM pending_ops ORDER BY seq",
        )?;
        let out = stmt
            .query_map([], |r| {
                Ok(OpRec {
                    seq: r.get(0)?,
                    op_type: r.get(1)?,
                    target_id: r.get(2)?,
                    payload: r.get(3)?,
                    base_edited_time: r.get(4)?,
                    state: r.get(5)?,
                    error: r.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(out)
    }

    pub fn set_op_state(&self, seq: i64, state: &str, error: Option<&str>) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE pending_ops SET state = ?2, error = ?3 WHERE seq = ?1",
            rusqlite::params![seq, state, error],
        )?;
        Ok(())
    }

    pub fn delete_op(&self, seq: i64) -> anyhow::Result<()> {
        self.conn.execute("DELETE FROM pending_ops WHERE seq = ?1", [seq])?;
        Ok(())
    }

    pub fn pending_count(&self) -> anyhow::Result<u32> {
        Ok(self.conn.query_row("SELECT count(*) FROM pending_ops", [], |r| r.get(0))?)
    }

    pub fn has_ops_for(&self, target_id: &str) -> anyhow::Result<bool> {
        let n: i64 = self.conn.query_row(
            "SELECT count(*) FROM pending_ops WHERE target_id = ?1",
            [target_id],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    pub fn is_page_dirty(&self, page_id: &str) -> anyhow::Result<bool> {
        let dirty: i64 =
            self.conn
                .query_row("SELECT dirty FROM pages WHERE id = ?1", [page_id], |r| r.get(0))?;
        Ok(dirty != 0)
    }

    pub fn clear_page_dirty(&self, page_id: &str) -> anyhow::Result<()> {
        self.conn.execute("UPDATE pages SET dirty = 0 WHERE id = ?1", [page_id])?;
        Ok(())
    }

    pub fn edit_toggle_todo(&mut self, block_id: &str) -> anyhow::Result<EditReceipt> {
        let (page_id, payload_str, block_type): (String, String, String) = self.conn.query_row(
            "SELECT page_id, payload, block_type FROM blocks WHERE id = ?1",
            [block_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let mut payload: Value = serde_json::from_str(&payload_str).unwrap_or_else(|_| json!({}));
        let checked = payload["checked"].as_bool().unwrap_or(false);
        payload["checked"] = json!(!checked);
        let new_payload = payload.to_string();

        let tx = self.conn.transaction()?;
        tx.execute(
            "UPDATE blocks SET payload = ?2 WHERE id = ?1",
            rusqlite::params![block_id, new_payload],
        )?;
        tx.execute("UPDATE pages SET dirty = 1 WHERE id = ?1", [&page_id])?;
        tx.commit()?;

        let base = self.get_page(&page_id)?.map(|p| p.last_edited_time);
        let op_payload =
            json!({"page_id": page_id, "block_type": block_type, "block_payload": new_payload}).to_string();
        let op_seq = self.enqueue_op("update_block", block_id, &op_payload, base.as_deref())?;

        Ok(EditReceipt {
            op_seq,
            inverse: Inverse::ToggleTodo { block_id: block_id.to_string() },
        })
    }

    pub fn edit_update_block_text(&mut self, block_id: &str, new_text: &str) -> anyhow::Result<EditReceipt> {
        let (page_id, old_payload, old_plain_text, block_type): (String, String, String, String) =
            self.conn.query_row(
                "SELECT page_id, payload, plain_text, block_type FROM blocks WHERE id = ?1",
                [block_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?;
        let mut payload: Value = serde_json::from_str(&old_payload).unwrap_or_else(|_| json!({}));
        if let Value::Object(ref mut map) = payload {
            map.insert(
                "rich_text".into(),
                json!([{"plain_text": new_text, "type": "text", "text": {"content": new_text}}]),
            );
        }
        let new_payload = payload.to_string();

        let tx = self.conn.transaction()?;
        tx.execute(
            "UPDATE blocks SET payload = ?2, plain_text = ?3 WHERE id = ?1",
            rusqlite::params![block_id, new_payload, new_text],
        )?;
        tx.execute("UPDATE pages SET dirty = 1 WHERE id = ?1", [&page_id])?;
        tx.commit()?;

        let base = self.get_page(&page_id)?.map(|p| p.last_edited_time);
        let op_payload =
            json!({"page_id": page_id, "block_type": block_type, "block_payload": new_payload}).to_string();
        let op_seq = self.enqueue_op("update_block", block_id, &op_payload, base.as_deref())?;

        Ok(EditReceipt {
            op_seq,
            inverse: Inverse::UpdateBlockText {
                block_id: block_id.to_string(),
                old_payload,
                old_plain_text,
            },
        })
    }

    pub fn edit_insert_block_after(
        &mut self,
        page_id: &str,
        after_block_id: Option<&str>,
        block_type: &str,
        text: &str,
    ) -> anyhow::Result<(String, EditReceipt)> {
        let (parent_block_id, insert_ordinal): (Option<String>, i64) = match after_block_id {
            Some(id) => {
                let (parent, ord): (Option<String>, i64) = self.conn.query_row(
                    "SELECT parent_block_id, ordinal FROM blocks WHERE id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                (parent, ord + 1)
            }
            None => (None, 0),
        };

        let block_id = format!(
            "tmp-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let payload = json!({"rich_text": [{"plain_text": text, "type": "text", "text": {"content": text}}]})
            .to_string();

        let tx = self.conn.transaction()?;
        tx.execute(
            "UPDATE blocks SET ordinal = ordinal + 1
             WHERE page_id = ?1 AND ordinal >= ?2
               AND COALESCE(parent_block_id, '') = COALESCE(?3, '')",
            rusqlite::params![page_id, insert_ordinal, parent_block_id],
        )?;
        tx.execute(
            "INSERT INTO blocks (id, page_id, parent_block_id, ordinal, block_type, payload, plain_text, has_children)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0)",
            rusqlite::params![block_id, page_id, parent_block_id, insert_ordinal, block_type, payload, text],
        )?;
        tx.execute("UPDATE pages SET dirty = 1 WHERE id = ?1", [page_id])?;
        tx.commit()?;

        let op_payload = json!({
            "page_id": page_id,
            "parent_id": parent_block_id,
            "after": after_block_id,
            "block_type": block_type,
            "text": text,
        })
        .to_string();
        let op_seq = self.enqueue_op("append_block", &block_id, &op_payload, None)?;

        let receipt = EditReceipt {
            op_seq,
            inverse: Inverse::DeleteInsertedBlock { block_id: block_id.clone() },
        };
        Ok((block_id, receipt))
    }

    pub fn edit_delete_block(&mut self, block_id: &str) -> anyhow::Result<EditReceipt> {
        let (page_id, parent_block_id, ordinal, block_type, payload, plain_text, has_children): (
            String,
            Option<String>,
            i64,
            String,
            String,
            String,
            bool,
        ) = self.conn.query_row(
            "SELECT page_id, parent_block_id, ordinal, block_type, payload, plain_text, has_children
             FROM blocks WHERE id = ?1",
            [block_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get::<_, i64>(6)? != 0,
                ))
            },
        )?;

        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM blocks WHERE id = ?1", [block_id])?;
        tx.execute("UPDATE pages SET dirty = 1 WHERE id = ?1", [&page_id])?;
        tx.commit()?;

        let base = self.get_page(&page_id)?.map(|p| p.last_edited_time);
        let op_payload = json!({"page_id": page_id}).to_string();
        let op_seq = self.enqueue_op("delete_block", block_id, &op_payload, base.as_deref())?;

        Ok(EditReceipt {
            op_seq,
            inverse: Inverse::RecreateBlock {
                page_id,
                block_id: block_id.to_string(),
                parent_block_id,
                ordinal,
                block_type,
                payload,
                plain_text,
                has_children,
            },
        })
    }

    pub fn edit_reorder_block(
        &mut self,
        block_id: &str,
        new_parent: Option<&str>,
        new_after: Option<&str>,
        new_ordinal: i64,
    ) -> anyhow::Result<()> {
        let (page_id, old_parent, old_ordinal): (String, Option<String>, i64) = self.conn.query_row(
            "SELECT page_id, parent_block_id, ordinal FROM blocks WHERE id = ?1",
            [block_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        if old_parent.as_deref() == new_parent && old_ordinal == new_ordinal {
            return Ok(());
        }
        self.conn.execute(
            "UPDATE blocks SET parent_block_id = ?2, ordinal = ?3 WHERE id = ?1",
            rusqlite::params![block_id, new_parent, new_ordinal],
        )?;

        // If this block hasn't been pushed yet (its creation is still queued), just
        // repoint that pending creation at the new spot instead of enqueuing a
        // separate reorder op — there's nothing remote to reorder yet.
        let pending_append = self.ops()?.into_iter().find(|o| o.op_type == "append_block" && o.target_id == block_id);
        if let Some(op) = pending_append {
            let mut payload: Value = serde_json::from_str(&op.payload).unwrap_or_default();
            payload["parent_id"] = json!(new_parent);
            payload["after"] = json!(new_after);
            self.conn.execute(
                "UPDATE pending_ops SET payload = ?2 WHERE seq = ?1",
                rusqlite::params![op.seq, payload.to_string()],
            )?;
            return Ok(());
        }

        self.conn.execute("UPDATE pages SET dirty = 1 WHERE id = ?1", [&page_id])?;
        self.enqueue_op("reorder_block", block_id, &json!({"page_id": page_id}).to_string(), None)?;
        Ok(())
    }

    pub fn undo(&mut self, receipt: EditReceipt) -> anyhow::Result<()> {
        let still_pending = self.ops()?.iter().any(|o| o.seq == receipt.op_seq);
        if still_pending {
            self.delete_op(receipt.op_seq)?;
            self.apply_inverse_locally(&receipt.inverse)
        } else {
            self.apply_inverse_as_new_edit(&receipt.inverse)
        }
    }

    fn apply_inverse_locally(&mut self, inv: &Inverse) -> anyhow::Result<()> {
        match inv {
            Inverse::ToggleTodo { block_id } => {
                let payload_str: String =
                    self.conn
                        .query_row("SELECT payload FROM blocks WHERE id = ?1", [block_id], |r| r.get(0))?;
                let mut payload: Value = serde_json::from_str(&payload_str).unwrap_or_else(|_| json!({}));
                let checked = payload["checked"].as_bool().unwrap_or(false);
                payload["checked"] = json!(!checked);
                self.conn.execute(
                    "UPDATE blocks SET payload = ?2 WHERE id = ?1",
                    rusqlite::params![block_id, payload.to_string()],
                )?;
            }
            Inverse::UpdateBlockText { block_id, old_payload, old_plain_text } => {
                self.conn.execute(
                    "UPDATE blocks SET payload = ?2, plain_text = ?3 WHERE id = ?1",
                    rusqlite::params![block_id, old_payload, old_plain_text],
                )?;
            }
            Inverse::DeleteInsertedBlock { block_id } => {
                self.conn.execute("DELETE FROM blocks WHERE id = ?1", [block_id])?;
            }
            Inverse::RecreateBlock {
                page_id,
                block_id,
                parent_block_id,
                ordinal,
                block_type,
                payload,
                plain_text,
                has_children,
            } => {
                self.conn.execute(
                    "INSERT INTO blocks (id, page_id, parent_block_id, ordinal, block_type,
                                         payload, plain_text, has_children)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    rusqlite::params![
                        block_id,
                        page_id,
                        parent_block_id,
                        ordinal,
                        block_type,
                        payload,
                        plain_text,
                        *has_children as i64
                    ],
                )?;
            }
            Inverse::DeleteInsertedRow { row_id } => {
                self.conn.execute("DELETE FROM rows WHERE id = ?1", [row_id])?;
            }
            Inverse::UpdateRowProperties { row_id, old_properties, .. } => {
                self.conn.execute(
                    "UPDATE rows SET properties = ?2 WHERE id = ?1",
                    rusqlite::params![row_id, old_properties],
                )?;
            }
            Inverse::RestoreRow { row_id } => {
                self.conn.execute("UPDATE rows SET archived = 0 WHERE id = ?1", [row_id])?;
            }
        }
        Ok(())
    }

    pub fn is_row_dirty(&self, row_id: &str) -> anyhow::Result<bool> {
        let dirty: i64 =
            self.conn
                .query_row("SELECT dirty FROM rows WHERE id = ?1", [row_id], |r| r.get(0))?;
        Ok(dirty != 0)
    }

    pub fn clear_row_dirty(&self, row_id: &str) -> anyhow::Result<()> {
        self.conn.execute("UPDATE rows SET dirty = 0 WHERE id = ?1", [row_id])?;
        Ok(())
    }

    pub fn edit_create_row(
        &mut self,
        data_source_id: &str,
        properties: Value,
    ) -> anyhow::Result<(String, EditReceipt)> {
        let row_id = format!(
            "tmp-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let props_str = properties.to_string();

        self.conn.execute(
            "INSERT INTO rows (id, data_source_id, properties, last_edited_time, archived, dirty)
             VALUES (?1, ?2, ?3, '', 0, 1)",
            rusqlite::params![row_id, data_source_id, props_str],
        )?;

        let op_payload = json!({"data_source_id": data_source_id, "properties": properties}).to_string();
        let op_seq = self.enqueue_op("create_row", &row_id, &op_payload, None)?;

        let receipt = EditReceipt {
            op_seq,
            inverse: Inverse::DeleteInsertedRow { row_id: row_id.clone() },
        };
        Ok((row_id, receipt))
    }

    pub fn edit_update_row(&mut self, row_id: &str, patch: Value) -> anyhow::Result<EditReceipt> {
        let (data_source_id, old_properties, base): (String, String, String) = self.conn.query_row(
            "SELECT data_source_id, properties, last_edited_time FROM rows WHERE id = ?1",
            [row_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let mut merged: Value = serde_json::from_str(&old_properties).unwrap_or_else(|_| json!({}));
        if let (Value::Object(ref mut map), Value::Object(patch_map)) = (&mut merged, &patch) {
            for (k, v) in patch_map {
                map.insert(k.clone(), v.clone());
            }
        }
        let new_properties = merged.to_string();

        self.conn.execute(
            "UPDATE rows SET properties = ?2, dirty = 1 WHERE id = ?1",
            rusqlite::params![row_id, new_properties],
        )?;

        let op_payload = json!({"properties": patch}).to_string();
        let op_seq = self.enqueue_op("update_row", row_id, &op_payload, Some(&base))?;

        Ok(EditReceipt {
            op_seq,
            inverse: Inverse::UpdateRowProperties {
                row_id: row_id.to_string(),
                data_source_id,
                old_properties,
            },
        })
    }

    pub fn edit_delete_row(&mut self, row_id: &str) -> anyhow::Result<EditReceipt> {
        let base: String = self
            .conn
            .query_row("SELECT last_edited_time FROM rows WHERE id = ?1", [row_id], |r| r.get(0))?;

        self.conn
            .execute("UPDATE rows SET archived = 1, dirty = 1 WHERE id = ?1", [row_id])?;

        let op_seq = self.enqueue_op("delete_row", row_id, "{}", Some(&base))?;

        Ok(EditReceipt {
            op_seq,
            inverse: Inverse::RestoreRow { row_id: row_id.to_string() },
        })
    }

    pub fn rewrite_row_id(&mut self, old_id: &str, new_id: &str) -> anyhow::Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("UPDATE rows SET id = ?2 WHERE id = ?1", rusqlite::params![old_id, new_id])?;
        tx.execute(
            "UPDATE pending_ops SET target_id = ?2 WHERE target_id = ?1",
            rusqlite::params![old_id, new_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn rewrite_block_id(&mut self, old_id: &str, new_id: &str) -> anyhow::Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("UPDATE blocks SET id = ?2 WHERE id = ?1", rusqlite::params![old_id, new_id])?;
        tx.execute(
            "UPDATE blocks SET parent_block_id = ?2 WHERE parent_block_id = ?1",
            rusqlite::params![old_id, new_id],
        )?;
        tx.execute(
            "UPDATE pending_ops SET target_id = ?2 WHERE target_id = ?1",
            rusqlite::params![old_id, new_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn apply_inverse_as_new_edit(&mut self, inv: &Inverse) -> anyhow::Result<()> {
        match inv {
            Inverse::ToggleTodo { block_id } => {
                self.edit_toggle_todo(block_id)?;
            }
            Inverse::UpdateBlockText { block_id, old_plain_text, .. } => {
                self.edit_update_block_text(block_id, old_plain_text)?;
            }
            Inverse::DeleteInsertedBlock { block_id } => {
                self.edit_delete_block(block_id)?;
            }
            Inverse::RecreateBlock { page_id, block_type, plain_text, .. } => {
                self.edit_insert_block_after(page_id, None, block_type, plain_text)?;
            }
            Inverse::DeleteInsertedRow { row_id } => {
                self.edit_delete_row(row_id)?;
            }
            Inverse::UpdateRowProperties { row_id, data_source_id: _, old_properties } => {
                let patch: Value = serde_json::from_str(old_properties).unwrap_or_else(|_| json!({}));
                self.edit_update_row(row_id, patch)?;
            }
            Inverse::RestoreRow { row_id } => {
                self.conn.execute("UPDATE rows SET archived = 0, dirty = 1 WHERE id = ?1", [row_id])?;
                let base: String = self.conn.query_row(
                    "SELECT last_edited_time FROM rows WHERE id = ?1",
                    [row_id],
                    |r| r.get(0),
                )?;
                self.enqueue_op("restore_row", row_id, "{}", Some(&base))?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct PageRec {
    pub id: String,
    pub parent_type: String,
    pub parent_id: Option<String>,
    pub title: String,
    pub icon: Option<String>,
    pub archived: bool,
    pub last_edited_time: String,
}

#[derive(Debug, Clone)]
pub struct BlockRec {
    pub id: String,
    pub page_id: String,
    pub parent_block_id: Option<String>,
    pub ordinal: i64,
    pub block_type: String,
    pub payload: String,
    pub plain_text: String,
    pub has_children: bool,
}

#[derive(Debug, Clone)]
pub struct DataSourceRec {
    pub id: String,
    pub database_id: String,
    pub title: String,
    pub schema_json: String,
    pub last_edited_time: String,
}

#[derive(Debug, Clone)]
pub struct RowRec {
    pub id: String,
    pub data_source_id: String,
    pub properties: String,
    pub last_edited_time: String,
    pub archived: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    Page,
    DataSource,
}

#[derive(Debug, Clone)]
pub struct TreeNode {
    pub id: String,
    pub title: String,
    pub parent_id: Option<String>,
    pub kind: NodeKind,
}

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub page_id: String,
    pub title: String,
    pub snippet: String,
}

#[derive(Debug, Clone)]
pub struct EditReceipt {
    pub op_seq: i64,
    pub inverse: Inverse,
}

#[derive(Debug, Clone)]
pub enum Inverse {
    ToggleTodo { block_id: String },
    UpdateBlockText { block_id: String, old_payload: String, old_plain_text: String },
    DeleteInsertedBlock { block_id: String },
    RecreateBlock {
        page_id: String,
        block_id: String,
        parent_block_id: Option<String>,
        ordinal: i64,
        block_type: String,
        payload: String,
        plain_text: String,
        has_children: bool,
    },
    DeleteInsertedRow { row_id: String },
    UpdateRowProperties { row_id: String, data_source_id: String, old_properties: String },
    RestoreRow { row_id: String },
}

#[derive(Debug, Clone)]
pub struct OpRec {
    pub seq: i64,
    pub op_type: String,
    pub target_id: String,
    pub payload: String,
    pub base_edited_time: Option<String>,
    pub state: String,
    pub error: Option<String>,
}
