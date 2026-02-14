use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredSnapshot {
    pub id: i64,
    pub epoch: u32,
    pub snapshot_kind: String,
    pub payload: Value,
    pub fetched_at: DateTime<Utc>,
}

pub struct SnapshotStore {
    conn: Connection,
}

impl SnapshotStore {
    pub fn open(path: &str) -> Result<Self> {
        let resolved = resolve_path(path);
        if resolved != PathBuf::from(":memory:") {
            ensure_parent_dir_exists(&resolved)?;
        }
        let conn = Connection::open(&resolved)
            .with_context(|| format!("failed to open sqlite database at {}", resolved.display()))?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    pub fn save_payload(
        &self,
        epoch: u32,
        snapshot_kind: &str,
        payload: &impl Serialize,
    ) -> Result<StoredSnapshot> {
        let fetched_at = Utc::now();
        let payload_json =
            serde_json::to_string(payload).context("failed to serialize snapshot payload")?;
        self.conn.execute(
            "INSERT INTO snapshots (epoch, snapshot_kind, payload_json, fetched_at) VALUES (?1, ?2, ?3, ?4)",
            params![epoch, snapshot_kind, payload_json, fetched_at.to_rfc3339()],
        )?;
        let id = self.conn.last_insert_rowid();
        self.load_by_id(id)?
            .ok_or_else(|| anyhow!("saved snapshot was not retrievable"))
    }

    pub fn load(&self, snapshot_kind: &str, epoch: Option<u32>) -> Result<Option<StoredSnapshot>> {
        if let Some(epoch) = epoch {
            let mut stmt = self.conn.prepare(
                "SELECT id, epoch, snapshot_kind, payload_json, fetched_at
                 FROM snapshots
                 WHERE snapshot_kind = ?1 AND epoch = ?2
                 ORDER BY id DESC
                 LIMIT 1",
            )?;
            let mut rows = stmt.query(params![snapshot_kind, epoch])?;
            if let Some(row) = rows.next()? {
                return map_row(row).map(Some);
            }
            return Ok(None);
        }

        let mut stmt = self.conn.prepare(
            "SELECT id, epoch, snapshot_kind, payload_json, fetched_at
             FROM snapshots
             WHERE snapshot_kind = ?1
             ORDER BY id DESC
             LIMIT 1",
        )?;
        let mut rows = stmt.query(params![snapshot_kind])?;
        if let Some(row) = rows.next()? {
            return map_row(row).map(Some);
        }
        Ok(None)
    }

    pub fn list(&self, snapshot_kind: Option<&str>, limit: usize) -> Result<Vec<StoredSnapshot>> {
        let limit = limit.max(1) as i64;
        let mut output = Vec::new();

        if let Some(kind) = snapshot_kind {
            let mut stmt = self.conn.prepare(
                "SELECT id, epoch, snapshot_kind, payload_json, fetched_at
                 FROM snapshots
                 WHERE snapshot_kind = ?1
                 ORDER BY id DESC
                 LIMIT ?2",
            )?;
            let mut rows = stmt.query(params![kind, limit])?;
            while let Some(row) = rows.next()? {
                output.push(map_row(row)?);
            }
            return Ok(output);
        }

        let mut stmt = self.conn.prepare(
            "SELECT id, epoch, snapshot_kind, payload_json, fetched_at
             FROM snapshots
             ORDER BY id DESC
             LIMIT ?1",
        )?;
        let mut rows = stmt.query(params![limit])?;
        while let Some(row) = rows.next()? {
            output.push(map_row(row)?);
        }
        Ok(output)
    }

    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS snapshots (
                id INTEGER PRIMARY KEY,
                epoch INTEGER NOT NULL,
                snapshot_kind TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                fetched_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_snapshots_kind_epoch
                ON snapshots(snapshot_kind, epoch);
            "#,
        )?;
        Ok(())
    }

    fn load_by_id(&self, id: i64) -> Result<Option<StoredSnapshot>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, epoch, snapshot_kind, payload_json, fetched_at
             FROM snapshots
             WHERE id = ?1
             LIMIT 1",
        )?;
        let mut rows = stmt.query(params![id])?;
        if let Some(row) = rows.next()? {
            return map_row(row).map(Some);
        }
        Ok(None)
    }
}

fn map_row(row: &rusqlite::Row<'_>) -> Result<StoredSnapshot> {
    let id: i64 = row.get(0)?;
    let epoch: u32 = row.get(1)?;
    let snapshot_kind: String = row.get(2)?;
    let payload_json: String = row.get(3)?;
    let fetched_at_raw: String = row.get(4)?;

    let payload = serde_json::from_str::<Value>(&payload_json)
        .with_context(|| format!("failed to parse payload JSON for snapshot id {id}"))?;
    let fetched_at = DateTime::parse_from_rfc3339(&fetched_at_raw)
        .with_context(|| format!("failed to parse fetched_at timestamp for snapshot id {id}"))?
        .with_timezone(&Utc);

    Ok(StoredSnapshot {
        id,
        epoch,
        snapshot_kind,
        payload,
        fetched_at,
    })
}

pub fn resolve_path(path: &str) -> PathBuf {
    if path == ":memory:" {
        return PathBuf::from(path);
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

fn ensure_parent_dir_exists(path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("snapshot path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent).with_context(|| {
        format!(
            "failed to create snapshot parent directory {}",
            parent.display()
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn snapshot_roundtrip_memory_db() {
        let store = SnapshotStore::open(":memory:").unwrap();
        let saved = store
            .save_payload(42, "state", &json!({"hello": "world"}))
            .unwrap();
        assert_eq!(saved.epoch, 42);

        let loaded = store.load("state", Some(42)).unwrap().unwrap();
        assert_eq!(loaded.id, saved.id);
        assert_eq!(loaded.payload["hello"], "world");
    }

    #[test]
    fn list_returns_latest_first() {
        let store = SnapshotStore::open(":memory:").unwrap();
        store.save_payload(1, "state", &json!({"n": 1})).unwrap();
        store.save_payload(2, "state", &json!({"n": 2})).unwrap();
        let listed = store.list(Some("state"), 10).unwrap();
        assert_eq!(listed.len(), 2);
        assert!(listed[0].epoch >= listed[1].epoch);
    }

    #[test]
    fn resolve_tilde_path() {
        let resolved = resolve_path("~/foo/bar.db");
        assert!(resolved.to_string_lossy().contains("foo/bar.db"));
    }
}
