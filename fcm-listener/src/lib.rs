//! FCM Listener - Listen for FCM messages as an Android device
//!
//! This crate allows a server to register with Firebase Cloud Messaging
//! and receive push messages as if it were an Android device.
//!
//! ## Usage
//!
//! ```rust,no_run
//! use fcm_listener::{FcmCredentials, Registration};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let http = reqwest::Client::new();
//!     let creds = FcmCredentials {
//!         sender_id: "123456789".into(),
//!         api_key: "AIza...".into(),
//!         app_id: "1:123456789:android:abc123".into(),
//!         project_id: "my-project".into(),
//!         package_name: "com.example.app".into(),
//!     };
//!
//!     let registration = Registration::register(&http, &creds).await?;
//!     println!("FCM Token: {}", registration.fcm_token());
//!
//!     let mut stream = registration.connect(vec![]).await?;
//!     // Use tokio_stream::StreamExt to receive messages
//!
//!     Ok(())
//! }
//! ```

mod mcs {
    include!(concat!(env!("OUT_DIR"), "/mcs_proto.rs"));
}

mod error;
mod gcm;
mod push;

pub use error::Error;
pub use gcm::{Connection, GcmSession, GcmToken};
pub use push::{new_heartbeat_ack, DataMessage, Message, MessageStream, MessageTag};

use serde::{Deserialize, Serialize};

/// Firebase/FCM credentials extracted from an Android app
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FcmCredentials {
    /// Firebase sender ID (project number), e.g., "890224420307"
    pub sender_id: String,
    /// Firebase API key
    pub api_key: String,
    /// Firebase app ID, e.g., "1:890224420307:android:835ea94c9a536bb0"
    pub app_id: String,
    /// Firebase project ID, e.g., "github-mobile-cc45e"
    pub project_id: String,
    /// Android package name, e.g., "com.github.android"
    pub package_name: String,
}

/// A registered FCM client that can receive messages
#[derive(Serialize, Deserialize)]
pub struct Registration {
    /// GCM session with android_id and security_token
    pub gcm_session: GcmSession,
    /// GCM/FCM token for receiving messages
    pub gcm_token: GcmToken,
    /// The credentials used for registration
    pub credentials: FcmCredentials,
}

impl Registration {
    /// Register with FCM and get a token
    pub async fn register(http: &reqwest::Client, creds: &FcmCredentials) -> Result<Self, Error> {
        // Step 1: GCM checkin to get android_id and security_token
        tracing::debug!("Performing GCM checkin...");
        let gcm_session = GcmSession::checkin(http).await?;
        tracing::info!(
            "GCM checkin complete: android_id={}",
            gcm_session.android_id
        );

        // Step 2: Register with GCM to get a token
        tracing::debug!("Registering with GCM...");
        let gcm_token = gcm_session
            .register(http, &creds.sender_id, &creds.package_name)
            .await?;
        tracing::info!(
            "GCM registration complete: token={}...",
            &gcm_token.token[..20.min(gcm_token.token.len())]
        );

        Ok(Self {
            gcm_session,
            gcm_token,
            credentials: creds.clone(),
        })
    }

    /// Get the FCM token that can be used to receive messages
    pub fn fcm_token(&self) -> &str {
        &self.gcm_token.token
    }

    /// Refresh the GCM session (checkin again)
    pub async fn refresh_session(&mut self, http: &reqwest::Client) -> Result<(), Error> {
        self.gcm_session = self.gcm_session.refresh(http).await?;
        Ok(())
    }

    /// Connect to mtalk.google.com and return a message stream
    pub async fn connect(
        &self,
        persistent_ids: Vec<String>,
    ) -> Result<MessageStream<tokio_rustls::client::TlsStream<tokio::net::TcpStream>>, Error> {
        let connection = self.gcm_session.connect(persistent_ids).await?;
        Ok(MessageStream::new(connection.0))
    }
}
