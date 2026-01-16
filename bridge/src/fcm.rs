//! FCM listener management
//!
//! Manages FCM connections for registered apps and forwards messages to UP endpoints.

use crate::db::Database;
use anyhow::Result;
use fcm_listener::{FcmCredentials, Message, MessageStream, Registration};
use futures_util::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

pub struct FcmManager {
    /// Active listeners by app_id
    listeners: HashMap<String, ListenerHandle>,
    /// HTTP client for FCM registration
    http_client: reqwest::Client,
}

struct ListenerHandle {
    /// Channel to stop the listener
    stop_tx: mpsc::Sender<()>,
    /// FCM token for this registration
    fcm_token: String,
}

impl FcmManager {
    pub fn new() -> Self {
        Self {
            listeners: HashMap::new(),
            http_client: reqwest::Client::builder()
                .http1_only()
                .build()
                .expect("failed to build HTTP client"),
        }
    }

    pub fn active_count(&self) -> usize {
        self.listeners.len()
    }

    pub async fn start_listener(
        &mut self,
        app_id: String,
        firebase_app_id: String,
        firebase_project_id: String,
        firebase_api_key: String,
        cert_sha1: Option<String>,
        app_version: Option<i32>,
        app_version_name: Option<String>,
        target_sdk: Option<i32>,
        endpoint: String,
        db: Arc<Database>,
    ) -> Result<String> {
        // Stop existing listener if any
        if let Some(handle) = self.listeners.remove(&app_id) {
            let _ = handle.stop_tx.send(()).await;
        }

        // Extract sender_id from firebase_app_id
        // Format: "1:<sender_id>:android:<hash>"
        let sender_id = extract_sender_id(&firebase_app_id)?;

        // Build FCM credentials
        let credentials = FcmCredentials {
            sender_id: sender_id.clone(),
            api_key: firebase_api_key,
            app_id: firebase_app_id,
            project_id: firebase_project_id,
            package_name: app_id.clone(),
            cert_sha1,
            app_version,
            app_version_name,
            target_sdk,
        };

        // Try to load existing session first
        let registration = if let Ok(Some(session_json)) = db.get_fcm_session(&app_id).await {
            match serde_json::from_str::<Registration>(&session_json) {
                Ok(existing) => {
                    info!(
                        "Reusing existing FCM session for {} (token: {}...)",
                        app_id,
                        &existing.fcm_token()[..20.min(existing.fcm_token().len())]
                    );
                    existing
                }
                Err(e) => {
                    warn!("Failed to deserialize saved session for {}: {}, re-registering", app_id, e);
                    Registration::register(&self.http_client, &credentials).await?
                }
            }
        } else {
            info!(
                "Registering with FCM for app: {} (sender_id: {}, cert: {})",
                app_id,
                sender_id,
                credentials.cert_sha1.as_deref().unwrap_or("none")
            );
            Registration::register(&self.http_client, &credentials).await?
        };

        let fcm_token = registration.fcm_token().to_string();
        info!(
            "Got FCM token for {}: {}...",
            app_id,
            &fcm_token[..20.min(fcm_token.len())]
        );

        // Save registration for reconnection
        if let Ok(reg_json) = serde_json::to_string(&registration) {
            let _ = db.save_fcm_session(&app_id, &reg_json).await;
        }

        // Create stop channel
        let (stop_tx, stop_rx) = mpsc::channel(1);

        // Clone values for the listener task
        let app_id_for_log = app_id.clone();
        let fcm_token_clone = fcm_token.clone();
        let http_client = self.http_client.clone();

        // Spawn listener task
        tokio::spawn(async move {
            run_listener(app_id_for_log, registration, endpoint, http_client, stop_rx).await;
        });

        self.listeners.insert(
            app_id,
            ListenerHandle {
                stop_tx,
                fcm_token: fcm_token_clone,
            },
        );

        Ok(fcm_token)
    }

    pub fn stop_listener(&mut self, app_id: &str) {
        if let Some(handle) = self.listeners.remove(app_id) {
            let _ = handle.stop_tx.try_send(());
            info!("Stopped FCM listener for {}", app_id);
        }
    }

    #[allow(dead_code)]
    pub fn get_fcm_token(&self, app_id: &str) -> Option<&str> {
        self.listeners.get(app_id).map(|h| h.fcm_token.as_str())
    }
}

/// Extract sender_id from Firebase app ID
/// Format: "1:<sender_id>:android:<hash>" or "1:<sender_id>:web:<hash>"
fn extract_sender_id(firebase_app_id: &str) -> Result<String> {
    let parts: Vec<&str> = firebase_app_id.split(':').collect();
    if parts.len() >= 2 {
        Ok(parts[1].to_string())
    } else {
        anyhow::bail!("Invalid firebase_app_id format: {}", firebase_app_id)
    }
}

async fn run_listener(
    app_id: String,
    registration: Registration,
    endpoint: String,
    http_client: reqwest::Client,
    mut stop_rx: mpsc::Receiver<()>,
) {
    info!("Starting FCM listener for {}", app_id);

    // Track persistent IDs to avoid duplicate messages
    let mut persistent_ids: Vec<String> = Vec::new();

    loop {
        // Check if we should stop
        if stop_rx.try_recv().is_ok() {
            info!("FCM listener stopped for {}", app_id);
            break;
        }

        // Connect to mtalk.google.com
        let connection = match registration
            .gcm_session
            .connect(persistent_ids.clone())
            .await
        {
            Ok(conn) => conn,
            Err(e) => {
                error!("FCM connection failed for {}: {}", app_id, e);
                tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
                continue;
            }
        };

        info!("FCM connection established for {}", app_id);

        // Wrap connection in MessageStream (no encryption keys needed for Android FCM)
        let mut stream = MessageStream::new(connection.0);

        // Listen for messages
        loop {
            tokio::select! {
                _ = stop_rx.recv() => {
                    info!("FCM listener stopped for {}", app_id);
                    return;
                }

                msg = stream.next() => {
                    match msg {
                        Some(Ok(Message::Data(data))) => {
                            let payload_len = data.raw_data.as_ref().map(|d| d.len()).unwrap_or(0);
                            info!(
                                "Received FCM message for {}: {} bytes, persistent_id: {:?}, from: {:?}",
                                app_id,
                                payload_len,
                                data.persistent_id,
                                data.from
                            );

                            // Track persistent ID
                            if let Some(pid) = &data.persistent_id {
                                if !persistent_ids.contains(pid) {
                                    persistent_ids.push(pid.clone());
                                    // Keep only last 100 IDs
                                    if persistent_ids.len() > 100 {
                                        persistent_ids.remove(0);
                                    }
                                }
                            }

                            // Forward to UnifiedPush endpoint
                            // For Android FCM, the payload might be in raw_data or app_data
                            let body = if let Some(raw) = &data.raw_data {
                                raw.clone()
                            } else {
                                // Serialize app_data as JSON if no raw_data
                                let app_data: HashMap<&str, &str> = data
                                    .app_data
                                    .iter()
                                    .map(|(k, v)| (k.as_str(), v.as_str()))
                                    .collect();
                                serde_json::to_vec(&app_data).unwrap_or_default()
                            };

                            if !body.is_empty() {
                                if let Err(e) = forward_to_up(&endpoint, &body, &http_client).await {
                                    error!("Failed to forward to UP for {}: {}", app_id, e);
                                } else {
                                    info!("Forwarded message to UP endpoint for {}", app_id);
                                }
                            } else {
                                warn!("Empty payload in FCM message for {}", app_id);
                            }
                        }

                        Some(Ok(Message::HeartbeatPing)) => {
                            // Send heartbeat ack
                            let ack = fcm_listener::new_heartbeat_ack();
                            if let Err(e) = stream.write_all(&ack).await {
                                error!("Failed to send heartbeat ack for {}: {}", app_id, e);
                                break; // Reconnect
                            }
                        }

                        Some(Ok(Message::Other(tag, _))) => {
                            warn!("Unknown FCM message type {} for {}", tag, app_id);
                        }

                        Some(Err(e)) => {
                            error!("FCM receive error for {}: {}", app_id, e);
                            break; // Reconnect
                        }

                        None => {
                            warn!("FCM stream ended for {}", app_id);
                            break; // Reconnect
                        }
                    }
                }
            }
        }

        // Wait before reconnecting
        warn!("FCM connection lost for {}, reconnecting in 5s...", app_id);
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    }
}

async fn forward_to_up(endpoint: &str, body: &[u8], http_client: &reqwest::Client) -> Result<()> {
    let response = http_client
        .post(endpoint)
        .header("Content-Type", "application/octet-stream")
        .body(body.to_vec())
        .send()
        .await?;

    if !response.status().is_success() {
        anyhow::bail!("UP endpoint returned {}", response.status());
    }

    Ok(())
}

impl Default for FcmManager {
    fn default() -> Self {
        Self::new()
    }
}
