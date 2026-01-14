pub mod contract {
    include!(concat!(env!("OUT_DIR"), "/checkin_proto.rs"));
}

use crate::Error;
use prost::bytes::BufMut;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use tokio_rustls::rustls::pki_types::ServerName;

fn require_some<T>(value: Option<T>, reason: &'static str) -> Result<T, Error> {
    match value {
        Some(value) => Ok(value),
        None => Err(Error::DependencyFailure("Android device check-in", reason)),
    }
}

const CHECKIN_URL: &str = "https://android.clients.google.com/checkin";
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

impl GcmSession {
    async fn request(
        http: &reqwest::Client,
        android_id: Option<i64>,
        security_token: Option<u64>,
    ) -> Result<Self, Error> {
        use prost::Message;

        // Use Android device type (3) with Chrome build info
        // This mimics what an Android device would send
        let request = contract::AndroidCheckinRequest {
            version: Some(3),
            id: android_id,
            security_token,
            user_serial_number: Some(0),
            checkin: contract::AndroidCheckinProto {
                r#type: Some(3), // DEVICE_CHROME_BROWSER
                chrome_build: Some(contract::ChromeBuildProto {
                    platform: Some(2), // PLATFORM_ANDROID
                    channel: Some(1),  // CHANNEL_STABLE
                    chrome_version: Some(String::from("63.0.3234.0")),
                }),
                ..Default::default()
            },
            ..Default::default()
        };

        const API_NAME: &str = "GCM checkin";

        let response = http
            .post(CHECKIN_URL)
            .body(request.encode_to_vec())
            .header(reqwest::header::CONTENT_TYPE, "application/x-protobuf")
            .send()
            .await
            .map_err(|e| Error::Request(API_NAME, e))?;

        let response_bytes = response
            .bytes()
            .await
            .map_err(|e| Error::Response(API_NAME, e))?;
        let response = contract::AndroidCheckinResponse::decode(response_bytes)
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

    /// Register with GCM to get a token for receiving messages
    ///
    /// # Arguments
    /// * `http` - HTTP client
    /// * `sender_id` - Firebase sender ID (project number), e.g., "890224420307"
    /// * `package_name` - Android package name, e.g., "com.github.android"
    pub async fn register(
        &self,
        http: &reqwest::Client,
        sender_id: &str,
        package_name: &str,
    ) -> Result<GcmToken, Error> {
        let android_id = self.android_id.to_string();
        let auth_header = format!("AidLogin {}:{}", &android_id, &self.security_token);

        // Build registration parameters - these are for Android FCM, not web push
        let mut params = std::collections::HashMap::with_capacity(5);
        params.insert("app", package_name);
        params.insert("X-subtype", sender_id);
        params.insert("device", &android_id);
        params.insert("sender", sender_id);

        const API_NAME: &str = "GCM registration";
        let result = http
            .post(REGISTER_URL)
            .form(&params)
            .header(reqwest::header::AUTHORIZATION, auth_header)
            .send()
            .await
            .map_err(|e| Error::Request(API_NAME, e))?;

        let response_text = result
            .text()
            .await
            .map_err(|e| Error::Response(API_NAME, e))?;

        tracing::debug!("GCM register response: {}", response_text);

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
