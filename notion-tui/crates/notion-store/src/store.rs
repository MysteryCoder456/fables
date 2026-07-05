use rusqlite::Connection;
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
                                           archived=?6, last_edited_time=?7",
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
        tx.execute("DELETE FROM rows WHERE data_source_id = ?1", [data_source_id])?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO rows (id, data_source_id, properties, last_edited_time, archived)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
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
pub struct OpRec {
    pub seq: i64,
    pub op_type: String,
    pub target_id: String,
    pub payload: String,
    pub base_edited_time: Option<String>,
    pub state: String,
    pub error: Option<String>,
}
