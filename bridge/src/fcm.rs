//! FCM listener management
//!
//! Manages FCM connections for registered apps and forwards messages to UP endpoints.

use crate::db::Database;
use anyhow::Result;
use fcm_push_listener::{Message, MessageStream, Registration};
use futures_util::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
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
            http_client: reqwest::Client::new(),
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
        endpoint: String,
        db: Arc<Database>,
    ) -> Result<String> {
        // Stop existing listener if any
        if let Some(handle) = self.listeners.remove(&app_id) {
            let _ = handle.stop_tx.send(()).await;
        }

        info!(
            "Registering with FCM for app: {} (firebase_app_id: {})",
            app_id, firebase_app_id
        );

        // Register with FCM
        let registration = fcm_push_listener::register(
            &self.http_client,
            &firebase_app_id,
            &firebase_project_id,
            &firebase_api_key,
            None, // No VAPID key
        )
        .await?;

        let fcm_token = registration.fcm_token.clone();
        info!("Got FCM token for {}: {}...", app_id, &fcm_token[..20.min(fcm_token.len())]);

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

        // Checkin to get a CheckedSession
        let checked_session = match registration.gcm.checkin(&http_client).await {
            Ok(session) => session,
            Err(e) => {
                error!("FCM checkin failed for {}: {}", app_id, e);
                tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
                continue;
            }
        };

        // Create connection to mtalk.google.com
        let connection = match checked_session.new_connection(persistent_ids.clone()).await {
            Ok(conn) => conn,
            Err(e) => {
                error!("FCM connection failed for {}: {}", app_id, e);
                tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
                continue;
            }
        };

        info!("FCM connection established for {}", app_id);

        // Wrap connection in MessageStream
        let mut stream = MessageStream::wrap(connection, &registration.keys);

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
                            info!(
                                "Received FCM message for {}: {} bytes, persistent_id: {:?}",
                                app_id,
                                data.body.len(),
                                data.persistent_id
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
                            if let Err(e) = forward_to_up(&endpoint, &data.body, &http_client).await {
                                error!("Failed to forward to UP for {}: {}", app_id, e);
                            } else {
                                info!("Forwarded message to UP endpoint for {}", app_id);
                            }
                        }

                        Some(Ok(Message::HeartbeatPing)) => {
                            // Heartbeat ping - the library handles ack automatically
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
