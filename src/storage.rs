use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbMessage {
    pub id: i64,
    pub session_id: String,
    pub role: String,
    pub content: Option<String>,
    pub tool_call_id: Option<String>,
    pub tool_calls: Option<String>,
    pub tool_name: Option<String>,
    pub reasoning: Option<String>,
    pub timestamp: f64,
    pub active: i32,
}

#[derive(Clone)]
pub struct SessionDb {
    conn: Arc<Mutex<Connection>>,
}

impl SessionDb {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path)?;
        let _ = conn.query_row("PRAGMA journal_mode = WAL;", [], |_| Ok(()));
        let _ = conn.query_row("PRAGMA foreign_keys = ON;", [], |_| Ok(()));
        let _ = conn.query_row("PRAGMA busy_timeout = 5000;", [], |_| Ok(()));
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        db.init_schema()?;
        Ok(db)
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        db.init_schema()?;
        Ok(db)
    }

    fn init_schema(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS system_prompts (
                hash TEXT PRIMARY KEY,
                prompt TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                source TEXT NOT NULL,
                user_id TEXT,
                chat_id TEXT,
                chat_type TEXT,
                model TEXT,
                started_at REAL NOT NULL,
                ended_at REAL,
                message_count INTEGER DEFAULT 0,
                tool_call_count INTEGER DEFAULT 0,
                input_tokens INTEGER DEFAULT 0,
                output_tokens INTEGER DEFAULT 0,
                cwd TEXT,
                title TEXT,
                archived INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                role TEXT NOT NULL,
                content TEXT,
                tool_call_id TEXT,
                tool_calls TEXT,
                tool_name TEXT,
                reasoning TEXT,
                timestamp REAL NOT NULL,
                active INTEGER NOT NULL DEFAULT 1
            );

            CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id, active);
            CREATE INDEX IF NOT EXISTS idx_sessions_started ON sessions(started_at);"
        )?;
        Ok(())
    }

    pub fn ensure_session(&self, session_id: &str, source: &str, model: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        conn.execute(
            "INSERT INTO sessions (id, source, model, started_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET model = excluded.model",
            params![session_id, source, model, now],
        )?;
        Ok(())
    }

    pub fn append_message(
        &self,
        session_id: &str,
        role: &str,
        content: Option<&str>,
        tool_call_id: Option<&str>,
        tool_calls: Option<&str>,
        tool_name: Option<&str>,
        reasoning: Option<&str>,
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        conn.execute(
            "INSERT INTO messages (session_id, role, content, tool_call_id, tool_calls, tool_name, reasoning, timestamp, active)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1)",
            params![session_id, role, content, tool_call_id, tool_calls, tool_name, reasoning, now],
        )?;
        let id = conn.last_insert_rowid();

        // Increment session message_count
        let is_tool = if tool_calls.is_some() { 1 } else { 0 };
        conn.execute(
            "UPDATE sessions 
             SET message_count = message_count + 1,
                 tool_call_count = tool_call_count + ?2
             WHERE id = ?1",
            params![session_id, is_tool],
        )?;

        Ok(id)
    }

    pub fn get_active_messages(&self, session_id: &str) -> Result<Vec<DbMessage>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, session_id, role, content, tool_call_id, tool_calls, tool_name, reasoning, timestamp, active
             FROM messages
             WHERE session_id = ?1 AND active = 1
             ORDER BY id ASC"
        )?;

        let rows = stmt.query_map(params![session_id], |row| {
            Ok(DbMessage {
                id: row.get(0)?,
                session_id: row.get(1)?,
                role: row.get(2)?,
                content: row.get(3)?,
                tool_call_id: row.get(4)?,
                tool_calls: row.get(5)?,
                tool_name: row.get(6)?,
                reasoning: row.get(7)?,
                timestamp: row.get(8)?,
                active: row.get(9)?,
            })
        })?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    pub fn clear_messages(&self, session_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE messages SET active = 0 WHERE session_id = ?1",
            params![session_id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_db_session_and_messages() {
        let db = SessionDb::open_in_memory().unwrap();
        db.ensure_session("sess_1", "cli", "Gemini").unwrap();
        let msg_id = db.append_message("sess_1", "user", Some("Hello"), None, None, None, None).unwrap();
        assert_eq!(msg_id, 1);
        let active = db.get_active_messages("sess_1").unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].content.as_deref(), Some("Hello"));
    }
}
