use crate::error::Result;
use rusqlite::{params, Connection};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct SyncEventRow {
    #[allow(dead_code)]
    pub id: i64,
    pub synced_at: String,
    pub zone: String,
    pub record_name: String,
    #[allow(dead_code)]
    pub record_type: String,
    pub ip_address: String,
    pub proxied: bool,
    pub status: String,
    #[allow(dead_code)]
    pub details: Option<String>,
}

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }

        let conn = Connection::open(path)?;

        // WAL mode for high performance and crash resilience
        conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
            PRAGMA busy_timeout = 5000;

            CREATE TABLE IF NOT EXISTS ip_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                detected_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                ip_address TEXT NOT NULL,
                ip_version TEXT NOT NULL,
                provider TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS dns_sync_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                synced_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                zone TEXT NOT NULL,
                record_name TEXT NOT NULL,
                record_type TEXT NOT NULL,
                ip_address TEXT NOT NULL,
                proxied BOOLEAN NOT NULL,
                status TEXT NOT NULL,
                details TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_ip_history_detected_at ON ip_history(detected_at DESC);
            CREATE INDEX IF NOT EXISTS idx_dns_sync_events_synced_at ON dns_sync_events(synced_at DESC);
            "#,
        )?;

        Ok(Self { conn })
    }

    pub fn get_last_cached_ip(&self) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT ip_address FROM ip_history ORDER BY id DESC LIMIT 1")?;
        let mut rows = stmt.query([])?;

        if let Some(row) = rows.next()? {
            let ip: String = row.get(0)?;
            Ok(Some(ip))
        } else {
            Ok(None)
        }
    }

    pub fn save_detected_ip(&self, ip: &str, version: &str, provider: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO ip_history (ip_address, ip_version, provider) VALUES (?1, ?2, ?3)",
            params![ip, version, provider],
        )?;
        Ok(())
    }

    pub fn save_sync_event(
        &self,
        zone: &str,
        record_name: &str,
        record_type: &str,
        ip: &str,
        proxied: bool,
        status: &str,
        details: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT INTO dns_sync_events (
                zone, record_name, record_type, ip_address, proxied, status, details
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
            params![zone, record_name, record_type, ip, proxied, status, details],
        )?;
        Ok(())
    }

    pub fn get_recent_events(&self, limit: usize) -> Result<Vec<SyncEventRow>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, synced_at, zone, record_name, record_type, ip_address, proxied, status, details
            FROM dns_sync_events
            ORDER BY id DESC
            LIMIT ?1
            "#,
        )?;

        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(SyncEventRow {
                id: row.get(0)?,
                synced_at: row.get(1)?,
                zone: row.get(2)?,
                record_name: row.get(3)?,
                record_type: row.get(4)?,
                ip_address: row.get(5)?,
                proxied: row.get(6)?,
                status: row.get(7)?,
                details: row.get(8)?,
            })
        })?;

        let mut events = Vec::new();
        for event in rows {
            events.push(event?);
        }
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_db_init_and_operations() {
        let db = Database::open(":memory:").expect("Open in-memory db");

        assert_eq!(db.get_last_cached_ip().unwrap(), None);

        db.save_detected_ip("198.51.100.42", "IPv4", "test_provider")
            .unwrap();
        assert_eq!(
            db.get_last_cached_ip().unwrap().as_deref(),
            Some("198.51.100.42")
        );

        db.save_sync_event(
            "siamer.xyz",
            "siamer.xyz",
            "A",
            "198.51.100.42",
            false,
            "UPDATED",
            None,
        )
        .unwrap();

        let events = db.get_recent_events(5).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].record_name, "siamer.xyz");
        assert_eq!(events[0].status, "UPDATED");
        assert_eq!(events[0].ip_address, "198.51.100.42");
    }
}
