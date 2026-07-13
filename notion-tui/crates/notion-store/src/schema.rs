use rusqlite::Connection;

const SCHEMA_V1: &str = r#"
CREATE TABLE pages (
    id TEXT PRIMARY KEY,
    parent_type TEXT NOT NULL,
    parent_id TEXT,
    title TEXT NOT NULL DEFAULT '',
    icon TEXT,
    archived INTEGER NOT NULL DEFAULT 0,
    last_edited_time TEXT NOT NULL,
    local_edited_at TEXT,
    dirty INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE blocks (
    id TEXT PRIMARY KEY,
    page_id TEXT NOT NULL,
    parent_block_id TEXT,
    ordinal INTEGER NOT NULL,
    block_type TEXT NOT NULL,
    payload TEXT NOT NULL,
    plain_text TEXT NOT NULL DEFAULT '',
    has_children INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_blocks_page ON blocks(page_id, parent_block_id, ordinal);
CREATE TABLE data_sources (
    id TEXT PRIMARY KEY,
    database_id TEXT NOT NULL,
    title TEXT NOT NULL DEFAULT '',
    schema_json TEXT NOT NULL,
    last_edited_time TEXT NOT NULL DEFAULT ''
);
CREATE TABLE rows (
    id TEXT PRIMARY KEY,
    data_source_id TEXT NOT NULL,
    properties TEXT NOT NULL,
    last_edited_time TEXT NOT NULL,
    archived INTEGER NOT NULL DEFAULT 0,
    dirty INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_rows_ds ON rows(data_source_id);
CREATE TABLE comments (
    id TEXT PRIMARY KEY,
    parent_id TEXT NOT NULL,
    parent_kind TEXT NOT NULL,
    thread_id TEXT,
    author TEXT NOT NULL DEFAULT '',
    body TEXT NOT NULL DEFAULT '',
    created_time TEXT NOT NULL DEFAULT ''
);
CREATE TABLE pending_ops (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    op_type TEXT NOT NULL,
    target_id TEXT NOT NULL,
    payload TEXT NOT NULL,
    base_edited_time TEXT,
    state TEXT NOT NULL DEFAULT 'pending',
    error TEXT
);
CREATE TABLE sync_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);

CREATE VIRTUAL TABLE fts USING fts5(content, page_id UNINDEXED, block_id UNINDEXED);

CREATE TRIGGER blocks_fts_ai AFTER INSERT ON blocks BEGIN
    INSERT INTO fts(content, page_id, block_id) VALUES (new.plain_text, new.page_id, new.id);
END;
CREATE TRIGGER blocks_fts_ad AFTER DELETE ON blocks BEGIN
    DELETE FROM fts WHERE block_id = old.id;
END;
CREATE TRIGGER blocks_fts_au AFTER UPDATE ON blocks BEGIN
    DELETE FROM fts WHERE block_id = old.id;
    INSERT INTO fts(content, page_id, block_id) VALUES (new.plain_text, new.page_id, new.id);
END;
CREATE TRIGGER pages_fts_ai AFTER INSERT ON pages BEGIN
    INSERT INTO fts(content, page_id, block_id) VALUES (new.title, new.id, NULL);
END;
CREATE TRIGGER pages_fts_ad AFTER DELETE ON pages BEGIN
    DELETE FROM fts WHERE page_id = old.id AND block_id IS NULL;
END;
CREATE TRIGGER pages_fts_au AFTER UPDATE OF title ON pages BEGIN
    DELETE FROM fts WHERE page_id = old.id AND block_id IS NULL;
    INSERT INTO fts(content, page_id, block_id) VALUES (new.title, new.id, NULL);
END;
"#;

pub const LATEST_VERSION: i64 = 1;

pub fn migrate(conn: &Connection) -> anyhow::Result<()> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version > LATEST_VERSION {
        anyhow::bail!(
            "this database (schema v{version}) was created by a newer notion-tui; \
             this build supports up to v{LATEST_VERSION} — upgrade notion-tui, \
             or point --db-path at a fresh file"
        );
    }
    if version < 1 {
        conn.execute_batch(SCHEMA_V1)?;
        conn.pragma_update(None, "user_version", 1)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::Store;
    use rusqlite::Connection;

    #[test]
    fn migrates_fresh_db_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("n.db");
        {
            let s = Store::open(&path).unwrap();
            let v: i64 = s
                .conn()
                .pragma_query_value(None, "user_version", |r| r.get(0))
                .unwrap();
            assert_eq!(v, 1);
        }
        // Re-open: must not fail or re-run migrations.
        let s = Store::open(&path).unwrap();
        let n: i64 = s
            .conn()
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name = 'pages'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn fts_triggers_index_blocks_and_pages() {
        let s = Store::open_in_memory().unwrap();
        s.conn()
            .execute(
                "INSERT INTO pages (id, parent_type, parent_id, title, last_edited_time)
             VALUES ('p1', 'workspace', NULL, 'Meeting notes', 't1')",
                [],
            )
            .unwrap();
        s.conn()
            .execute(
                "INSERT INTO blocks (id, page_id, parent_block_id, ordinal, block_type, payload, plain_text)
             VALUES ('b1', 'p1', NULL, 0, 'paragraph', '{}', 'quarterly roadmap discussion')",
                [],
            )
            .unwrap();

        let hit: String = s
            .conn()
            .query_row("SELECT page_id FROM fts WHERE fts MATCH 'roadmap'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(hit, "p1");
        let title_hit: String = s
            .conn()
            .query_row("SELECT page_id FROM fts WHERE fts MATCH 'meeting'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(title_hit, "p1");

        // Update re-indexes; delete removes.
        s.conn()
            .execute(
                "UPDATE blocks SET plain_text = 'budget review' WHERE id = 'b1'",
                [],
            )
            .unwrap();
        let n: i64 = s
            .conn()
            .query_row("SELECT count(*) FROM fts WHERE fts MATCH 'roadmap'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(n, 0);
        s.conn()
            .execute("DELETE FROM blocks WHERE id = 'b1'", [])
            .unwrap();
        let n: i64 = s
            .conn()
            .query_row("SELECT count(*) FROM fts WHERE fts MATCH 'budget'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn refuses_db_from_newer_app_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("n.db");
        {
            let s = Store::open(&path).unwrap();
            s.conn().pragma_update(None, "user_version", 99).unwrap();
        }
        let err = Store::open(&path).unwrap_err().to_string();
        assert!(err.contains("newer"), "unhelpful error: {err}");
        assert!(err.contains("99"));
    }

    #[test]
    fn backs_up_db_before_upgrading_from_older_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("n.db");
        {
            let s = Store::open(&path).unwrap();
            // Simulate a database created by an older build.
            s.conn().pragma_update(None, "user_version", 0).unwrap();
        }
        // Note: user_version 0 on an already-initialized DB would re-run
        // SCHEMA_V1 and fail on existing tables; the backup must happen
        // BEFORE migrate is attempted, so assert on the backup file even
        // if open() then errors.
        let _ = Store::open(&path);
        assert!(
            dir.path().join("n.db.bak-v0").exists(),
            "backup file must exist before migration runs"
        );
    }

    #[test]
    fn backup_includes_rows_committed_only_to_the_wal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("n.db");

        // Create a v0-stamped DB (mimicking an older build's schema) and insert a row that is
        // committed but, because we deliberately don't checkpoint and keep this connection open,
        // stays resident only in the `-wal` file rather than the main `.db` file.
        let keep_alive = {
            let s = Store::open(&path).unwrap();
            s.conn().pragma_update(None, "user_version", 0).unwrap();
            s.conn()
                .execute(
                    "INSERT INTO pages (id, parent_type, parent_id, title, last_edited_time)
                     VALUES ('p-wal', 'workspace', NULL, 'WAL row', 't1')",
                    [],
                )
                .unwrap();
            s
        };

        // Reopening triggers the v0 -> v1 backup+migrate path. `keep_alive` is still open here,
        // so SQLite has not auto-checkpointed the WAL back into the main file on connection close.
        let _ = Store::open(&path);

        let backup_path = dir.path().join("n.db.bak-v0");
        assert!(
            backup_path.exists(),
            "backup file must exist before migration runs"
        );

        let backup_conn = Connection::open(&backup_path).unwrap();
        let count: i64 = backup_conn
            .query_row("SELECT count(*) FROM pages WHERE id = 'p-wal'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            count, 1,
            "backup must include data committed to the WAL, not just the stale main file"
        );

        drop(keep_alive);
    }
}
