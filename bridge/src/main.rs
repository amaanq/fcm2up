//! fcm2up-bridge - FCM to UnifiedPush relay server
//!
//! This server:
//! 1. Accepts app registrations with Firebase credentials and UP endpoints
//! 2. Maintains FCM connections for each registered app
//! 3. Forwards FCM messages to UP endpoints as raw bytes

mod db;
mod fcm;

use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info};

#[derive(Clone)]
struct AppState {
    db: Arc<db::Database>,
    fcm_manager: Arc<RwLock<fcm::FcmManager>>,
}

#[derive(Debug, Deserialize)]
struct RegisterRequest {
    /// UnifiedPush endpoint URL
    endpoint: String,
    /// FCM token from the app (not used for server-side FCM, but stored for reference)
    #[serde(default)]
    fcm_token: Option<String>,
    /// App package name
    app_id: String,
    /// Firebase credentials (required for initial registration)
    #[serde(default)]
    firebase_app_id: Option<String>,
    #[serde(default)]
    firebase_project_id: Option<String>,
    #[serde(default)]
    firebase_api_key: Option<String>,
}

#[derive(Debug, Serialize)]
struct RegisterResponse {
    success: bool,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    fcm_token: Option<String>,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: String,
    registered_apps: usize,
    active_connections: usize,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("fcm2up_bridge=info".parse()?),
        )
        .init();

    // Get config from environment
    let port: u16 = std::env::var("PORT")
        .unwrap_or_else(|_| "8080".to_string())
        .parse()?;
    let db_path = std::env::var("DB_PATH").unwrap_or_else(|_| "fcm2up.db".to_string());

    // Initialize database
    let db = Arc::new(db::Database::new(&db_path).await?);

    // Initialize FCM manager
    let fcm_manager = Arc::new(RwLock::new(fcm::FcmManager::new()));

    let state = AppState { db, fcm_manager };

    // Restore existing registrations
    restore_registrations(state.clone()).await?;

    // Build router
    let app = Router::new()
        .route("/health", get(health))
        .route("/register", post(register))
        .route("/unregister", post(unregister))
        .with_state(state);

    let addr = format!("[::]:{}", port);
    info!("FCM2UP Bridge listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    let apps = state.db.count_registrations().await.unwrap_or(0);
    let connections = state.fcm_manager.read().await.active_count();

    Json(HealthResponse {
        status: "ok".to_string(),
        registered_apps: apps,
        active_connections: connections,
    })
}

async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<RegisterResponse>, (StatusCode, String)> {
    info!("Registration request for app: {}", req.app_id);

    // Check if we have Firebase credentials
    let (firebase_app_id, firebase_project_id, firebase_api_key) =
        match (&req.firebase_app_id, &req.firebase_project_id, &req.firebase_api_key) {
            (Some(app_id), Some(project_id), Some(api_key)) => {
                (app_id.clone(), project_id.clone(), api_key.clone())
            }
            _ => {
                // Try to get from existing registration
                match state.db.get_firebase_credentials(&req.app_id).await {
                    Ok(Some(creds)) => creds,
                    Ok(None) => {
                        return Err((
                            StatusCode::BAD_REQUEST,
                            "Firebase credentials required for first registration".to_string(),
                        ));
                    }
                    Err(e) => {
                        error!("Database error: {}", e);
                        return Err((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "Database error".to_string(),
                        ));
                    }
                }
            }
        };

    // Store registration
    let registration = db::Registration {
        app_id: req.app_id.clone(),
        endpoint: req.endpoint.clone(),
        fcm_token: req.fcm_token.clone(),
        firebase_app_id: firebase_app_id.clone(),
        firebase_project_id: firebase_project_id.clone(),
        firebase_api_key: firebase_api_key.clone(),
    };

    if let Err(e) = state.db.save_registration(&registration).await {
        error!("Failed to save registration: {}", e);
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to save registration".to_string(),
        ));
    }

    // Start FCM listener for this app
    let manager = state.fcm_manager.clone();
    let db = state.db.clone();

    let fcm_token = match manager
        .write()
        .await
        .start_listener(
            req.app_id.clone(),
            firebase_app_id,
            firebase_project_id,
            firebase_api_key,
            req.endpoint.clone(),
            db,
        )
        .await
    {
        Ok(token) => {
            info!("FCM listener started for {}", req.app_id);
            Some(token)
        }
        Err(e) => {
            error!("Failed to start FCM listener for {}: {}", req.app_id, e);
            // Still return success since registration was saved
            None
        }
    };

    Ok(Json(RegisterResponse {
        success: true,
        message: "Registration successful".to_string(),
        fcm_token,
    }))
}

async fn unregister(
    State(state): State<AppState>,
    Json(req): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let app_id = req["app_id"]
        .as_str()
        .ok_or((StatusCode::BAD_REQUEST, "app_id required".to_string()))?;

    info!("Unregister request for app: {}", app_id);

    // Stop FCM listener
    state.fcm_manager.write().await.stop_listener(app_id);

    // Remove from database
    if let Err(e) = state.db.delete_registration(app_id).await {
        error!("Failed to delete registration: {}", e);
    }

    Ok(Json(serde_json::json!({
        "success": true,
        "message": "Unregistered"
    })))
}

async fn restore_registrations(state: AppState) -> anyhow::Result<()> {
    let registrations = state.db.list_registrations().await?;

    info!("Restoring {} registrations", registrations.len());

    for reg in registrations {
        let db = state.db.clone();
        let app_id = reg.app_id.clone();

        let result = state
            .fcm_manager
            .write()
            .await
            .start_listener(
                reg.app_id,
                reg.firebase_app_id,
                reg.firebase_project_id,
                reg.firebase_api_key,
                reg.endpoint,
                db,
            )
            .await;

        if let Err(e) = result {
            error!("Failed to restore FCM listener for {}: {}", app_id, e);
        }
    }

    Ok(())
}
