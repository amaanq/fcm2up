//! Database storage for app registrations

use anyhow::{Context, Result};
use rusqlite::params;
use tokio_rusqlite::Connection;

#[derive(Debug, Clone)]
pub struct Registration {
    pub app_id: String,
    pub endpoint: String,
    pub fcm_token: Option<String>,
    pub firebase_app_id: String,
    pub firebase_project_id: String,
    pub firebase_api_key: String,
}

pub struct Database {
    conn: Connection,
}

impl Database {
    pub async fn new(path: &str) -> Result<Self> {
        let conn = Connection::open(path)
            .await
            .context("Failed to open database")?;

        // Initialize schema
        conn.call(|conn| {
            conn.execute(
                "CREATE TABLE IF NOT EXISTS registrations (
                    app_id TEXT PRIMARY KEY,
                    endpoint TEXT NOT NULL,
                    fcm_token TEXT,
                    firebase_app_id TEXT NOT NULL,
                    firebase_project_id TEXT NOT NULL,
                    firebase_api_key TEXT NOT NULL,
                    created_at TEXT DEFAULT CURRENT_TIMESTAMP,
                    updated_at TEXT DEFAULT CURRENT_TIMESTAMP
                )",
                [],
            )?;

            // Store FCM session data for reconnection
            conn.execute(
                "CREATE TABLE IF NOT EXISTS fcm_sessions (
                    app_id TEXT PRIMARY KEY,
                    registration_data TEXT NOT NULL,
                    created_at TEXT DEFAULT CURRENT_TIMESTAMP
                )",
                [],
            )?;

            Ok(())
        })
        .await
        .context("Failed to initialize database schema")?;

        Ok(Self { conn })
    }

    pub async fn save_registration(&self, reg: &Registration) -> Result<()> {
        let reg = reg.clone();
        self.conn
            .call(move |conn| {
                conn.execute(
                    "INSERT OR REPLACE INTO registrations
                     (app_id, endpoint, fcm_token, firebase_app_id, firebase_project_id, firebase_api_key, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, CURRENT_TIMESTAMP)",
                    params![
                        reg.app_id,
                        reg.endpoint,
                        reg.fcm_token,
                        reg.firebase_app_id,
                        reg.firebase_project_id,
                        reg.firebase_api_key
                    ],
                )?;
                Ok(())
            })
            .await
            .context("Failed to save registration")?;
        Ok(())
    }

    pub async fn get_registration(&self, app_id: &str) -> Result<Option<Registration>> {
        let app_id = app_id.to_string();
        let result = self
            .conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT app_id, endpoint, fcm_token, firebase_app_id, firebase_project_id, firebase_api_key
                     FROM registrations WHERE app_id = ?1",
                )?;

                let result = stmt.query_row([&app_id], |row| {
                    Ok(Registration {
                        app_id: row.get(0)?,
                        endpoint: row.get(1)?,
                        fcm_token: row.get(2)?,
                        firebase_app_id: row.get(3)?,
                        firebase_project_id: row.get(4)?,
                        firebase_api_key: row.get(5)?,
                    })
                });

                match result {
                    Ok(reg) => Ok(Some(reg)),
                    Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                    Err(e) => Err(tokio_rusqlite::Error::Rusqlite(e)),
                }
            })
            .await
            .context("Failed to get registration")?;
        Ok(result)
    }

    pub async fn get_firebase_credentials(
        &self,
        app_id: &str,
    ) -> Result<Option<(String, String, String)>> {
        let app_id = app_id.to_string();
        let result = self
            .conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT firebase_app_id, firebase_project_id, firebase_api_key
                     FROM registrations WHERE app_id = ?1",
                )?;

                let result = stmt.query_row([&app_id], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                });

                match result {
                    Ok(creds) => Ok(Some(creds)),
                    Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                    Err(e) => Err(tokio_rusqlite::Error::Rusqlite(e)),
                }
            })
            .await
            .context("Failed to get Firebase credentials")?;
        Ok(result)
    }

    pub async fn update_endpoint(&self, app_id: &str, endpoint: &str) -> Result<()> {
        let app_id = app_id.to_string();
        let endpoint = endpoint.to_string();
        self.conn
            .call(move |conn| {
                conn.execute(
                    "UPDATE registrations SET endpoint = ?1, updated_at = CURRENT_TIMESTAMP WHERE app_id = ?2",
                    params![endpoint, app_id],
                )?;
                Ok(())
            })
            .await
            .context("Failed to update endpoint")?;
        Ok(())
    }

    pub async fn delete_registration(&self, app_id: &str) -> Result<()> {
        let app_id = app_id.to_string();
        self.conn
            .call(move |conn| {
                conn.execute("DELETE FROM registrations WHERE app_id = ?1", [&app_id])?;
                conn.execute("DELETE FROM fcm_sessions WHERE app_id = ?1", [&app_id])?;
                Ok(())
            })
            .await
            .context("Failed to delete registration")?;
        Ok(())
    }

    pub async fn list_registrations(&self) -> Result<Vec<Registration>> {
        let result = self
            .conn
            .call(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT app_id, endpoint, fcm_token, firebase_app_id, firebase_project_id, firebase_api_key
                     FROM registrations",
                )?;

                let rows = stmt.query_map([], |row| {
                    Ok(Registration {
                        app_id: row.get(0)?,
                        endpoint: row.get(1)?,
                        fcm_token: row.get(2)?,
                        firebase_app_id: row.get(3)?,
                        firebase_project_id: row.get(4)?,
                        firebase_api_key: row.get(5)?,
                    })
                })?;

                let mut registrations = Vec::new();
                for row in rows {
                    registrations.push(row?);
                }

                Ok(registrations)
            })
            .await
            .context("Failed to list registrations")?;
        Ok(result)
    }

    pub async fn count_registrations(&self) -> Result<usize> {
        let count = self
            .conn
            .call(|conn| {
                let count: i64 =
                    conn.query_row("SELECT COUNT(*) FROM registrations", [], |row| row.get(0))?;
                Ok(count as usize)
            })
            .await
            .context("Failed to count registrations")?;
        Ok(count)
    }

    pub async fn save_fcm_session(&self, app_id: &str, data: &str) -> Result<()> {
        let app_id = app_id.to_string();
        let data = data.to_string();
        self.conn
            .call(move |conn| {
                conn.execute(
                    "INSERT OR REPLACE INTO fcm_sessions (app_id, registration_data) VALUES (?1, ?2)",
                    params![app_id, data],
                )?;
                Ok(())
            })
            .await
            .context("Failed to save FCM session")?;
        Ok(())
    }

    pub async fn get_fcm_session(&self, app_id: &str) -> Result<Option<String>> {
        let app_id = app_id.to_string();
        let result = self
            .conn
            .call(move |conn| {
                let result: Result<String, _> = conn.query_row(
                    "SELECT registration_data FROM fcm_sessions WHERE app_id = ?1",
                    [&app_id],
                    |row| row.get(0),
                );

                match result {
                    Ok(data) => Ok(Some(data)),
                    Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                    Err(e) => Err(tokio_rusqlite::Error::Rusqlite(e)),
                }
            })
            .await
            .context("Failed to get FCM session")?;
        Ok(result)
    }
}
