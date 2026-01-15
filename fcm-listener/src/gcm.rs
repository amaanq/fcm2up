pub mod contract {
    include!(concat!(env!("OUT_DIR"), "/checkin_proto.rs"));
}

use crate::Error;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use prost::bytes::BufMut;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use std::io::{Read, Write};
use tokio_rustls::rustls::pki_types::ServerName;

fn require_some<T>(value: Option<T>, reason: &'static str) -> Result<T, Error> {
    match value {
        Some(value) => Ok(value),
        None => Err(Error::DependencyFailure("Android device check-in", reason)),
    }
}

const CHECKIN_URL: &str = "https://android.clients.google.com/checkin";
// microG uses android.clients.google.com
const REGISTER_URL: &str = "https://android.clients.google.com/c2dm/register3";

// Normal JSON serialization will lose precision and change the number, so we must
// force the i64/u64 to serialize to string.
#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GcmSession {
    #[serde_as(as = "serde_with::DisplayFromStr")]
    pub android_id: i64,

    #[serde_as(as = "serde_with::DisplayFromStr")]
    pub security_token: u64,
}

/// Token received from GCM registration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GcmToken {
    pub token: String,
}

/// Firebase Installations credentials
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FirebaseInstallation {
    /// Firebase Installation ID (FID)
    pub fid: String,
    /// Auth token (JWT) for FCM registration
    pub auth_token: String,
    /// Refresh token for obtaining new auth tokens
    pub refresh_token: String,
}

/// Firebase app configuration needed for registration
#[derive(Clone, Debug)]
pub struct FirebaseConfig {
    /// Firebase project ID (e.g., "github-mobile-cc45e")
    pub project_id: String,
    /// Firebase API key (from google-services.json)
    pub api_key: String,
    /// Firebase App ID (e.g., "1:890224420307:android:835ea94c9a536bb0")
    pub app_id: String,
}

impl GcmSession {
    async fn request(
        http: &reqwest::Client,
        android_id: Option<i64>,
        security_token: Option<u64>,
    ) -> Result<Self, Error> {
        use prost::Message;

        // Current timestamp for event
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        // Build event list - microG sends "event_log_start" on first checkin, "system_update" on re-checkin
        // GMS has this structure (axdz class) but may not always populate it
        let event = if android_id.is_none() {
            // First checkin - send "event_log_start"
            vec![contract::AndroidCheckinEvent {
                tag: Some("event_log_start".into()),
                value: None,
                time_msec: Some(now_ms),
            }]
        } else {
            // Re-checkin - send "system_update"
            vec![contract::AndroidCheckinEvent {
                tag: Some("system_update".into()),
                value: Some("1536,0,-1,NULL".into()),
                time_msec: Some(now_ms),
            }]
        };

        // Use Android device type with proper Android build info
        // This mimics what a real Android device (Pixel 5) would send
        let request = contract::AndroidCheckinRequest {
            version: Some(3),
            id: android_id,
            security_token,
            user_serial_number: Some(0),
            fragment: Some(if android_id.is_some() { 1 } else { 0 }),
            locale: Some("en_US".into()),
            time_zone: Some("America/Los_Angeles".into()),
            logging_id: Some(rand::random::<i64>().abs()),
            // microG uses this specific initial digest value
            digest: Some("1-929a0dca0eee55513280171a8585da7dcd3700f8".into()),
            ota_cert: vec!["71Q6Rn2DDZl1zPDVaaeEHItd".into()],
            account_cookie: vec!["".into()],
            serial_number: Some("RF8M33YQXMR".into()),
            mac_addr: vec!["aabbccddeeff".into()],
            mac_addr_type: vec!["wifi".into()],
            checkin: contract::AndroidCheckinProto {
                r#type: Some(1), // DEVICE_ANDROID_OS
                build: Some(contract::AndroidBuildProto {
                    fingerprint: Some(
                        "google/redfin/redfin:14/AP2A.240805.005/12025142:user/release-keys".into(),
                    ),
                    hardware: Some("redfin".into()),
                    brand: Some("google".into()),
                    radio: Some("g7250-00217-231219-B-11446880".into()),
                    bootloader: Some("slider-1.2-10323765".into()),
                    client_id: Some("android-google".into()),
                    time: Some(1722859200), // Aug 2024
                    device: Some("redfin".into()),
                    sdk_version: Some(34),
                    model: Some("Pixel 5".into()),
                    manufacturer: Some("Google".into()),
                    product: Some("redfin".into()),
                    ota_installed: Some(false),
                    ..Default::default()
                }),
                last_checkin_msec: Some(0),
                event, // Add the event list (microG CheckinClient.java:108-112)
                roaming: Some("WIFI::".into()),
                user_number: Some(0),
                ..Default::default()
            },
            ..Default::default()
        };

        const API_NAME: &str = "GCM checkin";

        // User-Agent matching microG's CheckinClient.java
        let user_agent = "Android-Checkin/2.0 (redfin AP2A.240805.005); gzip";

        // Gzip compress the request body (both GMS and microG do this)
        let proto_bytes = request.encode_to_vec();
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(&proto_bytes)
            .map_err(|_| Error::DependencyFailure(API_NAME, "failed to gzip compress request"))?;
        let compressed_body = encoder
            .finish()
            .map_err(|_| Error::DependencyFailure(API_NAME, "failed to finish gzip compression"))?;

        tracing::debug!(
            "GCM checkin: compressed {} bytes -> {} bytes",
            proto_bytes.len(),
            compressed_body.len()
        );

        let response = http
            .post(CHECKIN_URL)
            .body(compressed_body)
            // Content-Type must be "application/x-protobuffer" (with 'buffer' suffix)
            // Both GMS (awzn.java:92) and microG use this exact value
            .header(reqwest::header::CONTENT_TYPE, "application/x-protobuffer")
            // GMS and microG both send gzip-compressed bodies
            .header(reqwest::header::CONTENT_ENCODING, "gzip")
            .header(reqwest::header::ACCEPT_ENCODING, "gzip")
            .header(reqwest::header::USER_AGENT, user_agent)
            .send()
            .await
            .map_err(|e| Error::Request(API_NAME, e))?;

        // Check if response is gzip-encoded and decompress if needed
        let is_gzip = response
            .headers()
            .get(reqwest::header::CONTENT_ENCODING)
            .map(|v| v.to_str().unwrap_or("").contains("gzip"))
            .unwrap_or(false);

        let response_bytes = response
            .bytes()
            .await
            .map_err(|e| Error::Response(API_NAME, e))?;

        let decoded_bytes = if is_gzip {
            let mut decoder = GzDecoder::new(&response_bytes[..]);
            let mut decompressed = Vec::new();
            decoder
                .read_to_end(&mut decompressed)
                .map_err(|_| Error::DependencyFailure(API_NAME, "failed to decompress gzip response"))?;
            tracing::debug!(
                "GCM checkin: decompressed {} bytes -> {} bytes",
                response_bytes.len(),
                decompressed.len()
            );
            decompressed
        } else {
            response_bytes.to_vec()
        };

        let response = contract::AndroidCheckinResponse::decode(&decoded_bytes[..])
            .map_err(|e| Error::ProtobufDecode("android checkin response", e))?;

        let android_id = require_some(response.android_id, "response is missing android id")?;

        const BAD_ID: Result<i64, Error> = Err(Error::DependencyFailure(
            API_NAME,
            "responded with non-numeric android id",
        ));
        let android_id = i64::try_from(android_id).or(BAD_ID)?;
        let security_token = require_some(
            response.security_token,
            "response is missing security token",
        )?;

        Ok(Self {
            android_id,
            security_token,
        })
    }

    /// Perform initial GCM checkin to get android_id and security_token
    pub async fn checkin(http: &reqwest::Client) -> Result<Self, Error> {
        Self::request(http, None, None).await
    }

    /// Refresh the session (re-checkin with existing credentials)
    pub async fn refresh(&self, http: &reqwest::Client) -> Result<Self, Error> {
        Self::request(http, Some(self.android_id), Some(self.security_token)).await
    }

    /// Register with Firebase Installations to get FID and auth token
    ///
    /// This is required for FCM registration with modern Firebase SDK (>= 20.1.1)
    pub async fn register_firebase_installation(
        http: &reqwest::Client,
        firebase_config: &FirebaseConfig,
        package_name: &str,
        cert_sha1: &str,
    ) -> Result<FirebaseInstallation, Error> {
        use base64::Engine;
        use rand::Rng;

        const API_NAME: &str = "Firebase Installations";

        // Generate a random FID (Firebase Installation ID)
        // FID is a 22-character base64url string starting with 'c' or similar
        // Use OsRng instead of thread_rng() because thread_rng() is not Send
        let fid = {
            let mut rng = rand::rngs::OsRng;
            let fid_bytes: [u8; 17] = rng.gen();
            let mut fid = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(fid_bytes);
            fid.truncate(22);
            // FID should start with a valid char (c, d, e, f)
            let first_byte = fid_bytes[0] & 0x0F;
            let first_char = match first_byte % 4 {
                0 => 'c',
                1 => 'd',
                2 => 'e',
                _ => 'f',
            };
            fid.replace_range(0..1, &first_char.to_string());
            fid
        };

        let url = format!(
            "https://firebaseinstallations.googleapis.com/v1/projects/{}/installations",
            firebase_config.project_id
        );

        let payload = serde_json::json!({
            "fid": fid,
            "appId": firebase_config.app_id,
            "authVersion": "FIS_v2",
            "sdkVersion": "a:17.0.0",
        });

        tracing::info!("Firebase Installations URL: {}", url);
        tracing::debug!("Firebase Installations payload: {:?}", payload);

        let response = http
            .post(&url)
            .header("Content-Type", "application/json")
            .header("x-goog-api-key", &firebase_config.api_key)
            .header("x-android-package", package_name)
            .header("x-android-cert", cert_sha1.to_uppercase())
            .json(&payload)
            .send()
            .await
            .map_err(|e| Error::Request(API_NAME, e))?;

        let status = response.status();
        let response_text = response
            .text()
            .await
            .map_err(|e| Error::Response(API_NAME, e))?;

        if !status.is_success() {
            tracing::error!("Firebase Installations failed: {} - {}", status, response_text);
            return Err(Error::DependencyRejection(
                API_NAME,
                format!("HTTP {}: {}", status, &response_text[..200.min(response_text.len())]),
            ));
        }

        let response_json: serde_json::Value = serde_json::from_str(&response_text)
            .map_err(|_| Error::DependencyFailure(API_NAME, "invalid JSON response"))?;

        let fid = response_json["fid"]
            .as_str()
            .ok_or(Error::DependencyFailure(API_NAME, "missing fid in response"))?
            .to_string();

        let auth_token = response_json["authToken"]["token"]
            .as_str()
            .ok_or(Error::DependencyFailure(API_NAME, "missing authToken in response"))?
            .to_string();

        let refresh_token = response_json["refreshToken"]
            .as_str()
            .ok_or(Error::DependencyFailure(API_NAME, "missing refreshToken in response"))?
            .to_string();

        tracing::info!("Firebase Installations succeeded, FID: {}", fid);

        Ok(FirebaseInstallation {
            fid,
            auth_token,
            refresh_token,
        })
    }

    /// Register with GCM to get a token for receiving messages
    ///
    /// # Arguments
    /// * `http` - HTTP client
    /// * `sender_id` - Firebase sender ID (project number), e.g., "890224420307"
    /// * `package_name` - Android package name, e.g., "com.github.android"
    /// * `cert_sha1` - SHA1 of signing certificate (lowercase hex, no colons), or None
    /// * `app_version` - App version code (versionCode from APK)
    /// * `app_version_name` - App version name (versionName from APK), sent as X-app_ver_name
    /// * `target_sdk` - Target SDK version from APK
    /// * `firebase_config` - Firebase configuration for Installations API
    /// * `firebase_installation` - Pre-registered Firebase Installation
    pub async fn register(
        &self,
        http: &reqwest::Client,
        sender_id: &str,
        package_name: &str,
        cert_sha1: Option<&str>,
        app_version: Option<i32>,
        app_version_name: Option<&str>,
        target_sdk: Option<i32>,
        firebase_config: Option<&FirebaseConfig>,
        firebase_installation: Option<&FirebaseInstallation>,
    ) -> Result<GcmToken, Error> {
        let android_id = self.android_id.to_string();
        let auth_header = format!("AidLogin {}:{}", &android_id, &self.security_token);
        let user_agent = "Android-GCM/1.5 (redfin AP2A.240805.005)";

        let app_ver_str = app_version.unwrap_or(1).to_string();
        let target_ver_str = target_sdk.unwrap_or(34).to_string();

        // Build registration parameters
        let mut params = std::collections::HashMap::with_capacity(20);
        params.insert("app", package_name.to_string());
        params.insert("device", android_id.clone());
        params.insert("sender", sender_id.to_string());
        params.insert("app_ver", app_ver_str.clone());
        params.insert("target_ver", target_ver_str.clone());

        // Cert is required
        if let Some(cert) = cert_sha1 {
            params.insert("cert", cert.to_lowercase());
        }

        // App version name
        if let Some(ver_name) = app_version_name {
            params.insert("X-app_ver_name", ver_name.to_string());
        }

        // Firebase Installations parameters (required for modern Firebase SDK >= 20.1.1)
        if let Some(fis) = firebase_installation {
            params.insert("X-appid", fis.fid.clone());
            params.insert("X-Goog-Firebase-Installations-Auth", fis.auth_token.clone());
            params.insert("X-cliv", "fiid-21.0.0".to_string());
            params.insert("X-scope", "*".to_string());
            params.insert("X-subtype", sender_id.to_string());

            if let Some(config) = firebase_config {
                params.insert("X-gmp_app_id", config.app_id.clone());
            }

            params.insert("X-Firebase-Client", "fire-installations/17.0.0".to_string());
        }

        const API_NAME: &str = "GCM registration";

        tracing::info!("GCM register URL: {}", REGISTER_URL);
        tracing::info!("GCM register params: {:?}", params);
        tracing::info!("GCM auth header: {}", auth_header);

        let result = http
            .post(REGISTER_URL)
            .form(&params)
            .header(reqwest::header::AUTHORIZATION, &auth_header)
            .header(reqwest::header::USER_AGENT, user_agent)
            .header("app", package_name)
            .send()
            .await
            .map_err(|e| Error::Request(API_NAME, e))?;

        let response_text = result
            .text()
            .await
            .map_err(|e| Error::Response(API_NAME, e))?;

        tracing::info!("GCM register response: {}", response_text);

        const ERR_EOF: Error = Error::DependencyFailure(API_NAME, "malformed response");

        // Response format is "token=<token>" or "Error=<reason>"
        let mut tokens = response_text.split('=');
        match tokens.next() {
            Some("Error") => {
                return Err(Error::DependencyRejection(
                    API_NAME,
                    tokens.next().unwrap_or("no reason given").into(),
                ))
            }
            Some("token") => {
                // Success case
            }
            None => return Err(ERR_EOF),
            Some(other) => {
                tracing::warn!("Unexpected GCM response key: {}", other);
            }
        }

        match tokens.next() {
            Some(v) => Ok(GcmToken {
                token: String::from(v),
            }),
            None => Err(ERR_EOF),
        }
    }

    /// Connect to mtalk.google.com MCS server
    pub async fn connect(&self, received_persistent_id: Vec<String>) -> Result<Connection, Error> {
        use prost::Message;

        // Install the default crypto provider if not already installed
        let _ = rustls::crypto::ring::default_provider().install_default();

        const ERR_RESOLVE: Error =
            Error::DependencyFailure("name resolution", "unable to resolve google talk host name");

        let domain = ServerName::try_from("mtalk.google.com").or(Err(ERR_RESOLVE))?;

        let login_request = self.new_mcs_login_request(received_persistent_id);

        let mut login_bytes = bytes::BytesMut::with_capacity(2 + login_request.encoded_len() + 4);
        login_bytes.put_u8(Self::MCS_VERSION);
        login_bytes.put_u8(Self::LOGIN_REQUEST_TAG);
        login_request
            .encode_length_delimited(&mut login_bytes)
            .expect("login request encoding failure");

        Self::try_connect(domain, &login_bytes)
            .await
            .map_err(Error::Socket)
    }

    const MCS_VERSION: u8 = 41;
    const LOGIN_REQUEST_TAG: u8 = 2;

    fn new_mcs_login_request(
        &self,
        received_persistent_id: Vec<String>,
    ) -> crate::mcs::LoginRequest {
        let android_id = self.android_id.to_string();
        crate::mcs::LoginRequest {
            adaptive_heartbeat: Some(false),
            auth_service: Some(2),
            auth_token: self.security_token.to_string(),
            id: "chrome-63.0.3234.0".into(),
            domain: "mcs.android.com".into(),
            device_id: Some(format!("android-{:x}", self.android_id)),
            network_type: Some(1),
            resource: android_id.clone(),
            user: android_id,
            use_rmq2: Some(true),
            setting: vec![crate::mcs::Setting {
                name: "new_vc".into(),
                value: "1".into(),
            }],
            received_persistent_id,
            ..Default::default()
        }
    }

    async fn try_connect(
        domain: ServerName<'static>,
        login_bytes: &[u8],
    ) -> Result<Connection, tokio::io::Error> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let stream = tokio::net::TcpStream::connect("mtalk.google.com:5228").await?;
        let tls = new_tls_initiator();
        let mut stream = tls.connect(domain, stream).await?;

        stream.write_all(login_bytes).await?;

        // Read the version byte from server
        stream.read_i8().await?;

        Ok(Connection(stream))
    }
}

fn new_tls_initiator() -> tokio_rustls::TlsConnector {
    let root_store = tokio_rustls::rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };

    let config = tokio_rustls::rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    tokio_rustls::TlsConnector::from(std::sync::Arc::new(config))
}

pub struct Connection(pub tokio_rustls::client::TlsStream<tokio::net::TcpStream>);

impl std::ops::Deref for Connection {
    type Target = tokio_rustls::client::TlsStream<tokio::net::TcpStream>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for Connection {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
