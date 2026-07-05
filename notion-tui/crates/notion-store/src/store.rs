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
}
