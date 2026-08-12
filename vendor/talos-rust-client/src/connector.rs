//! Connection builder for Talos gRPC API with mTLS support

use crate::error::{Error, Result};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use std::sync::Arc;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint, Identity};
use hyper_util::rt::TokioIo;
use tracing::{debug, info, instrument};

/// Connection builder for Talos gRPC API
#[derive(Debug, Clone)]
pub struct TalosConnector {
    endpoint: String,
    ca_cert: Option<Vec<u8>>,
    client_cert: Option<Vec<u8>>,
    client_key: Option<Vec<u8>>,
    server_name: Option<String>,
    insecure: bool,
}

impl TalosConnector {
    /// Create a new connection builder
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            ca_cert: None,
            client_cert: None,
            client_key: None,
            server_name: None,
            insecure: false,
        }
    }

    /// Set the CA certificate from a PEM-encoded byte slice
    pub fn ca_pem(mut self, pem: Vec<u8>) -> Self {
        self.ca_cert = Some(pem);
        self
    }

    /// Set the client certificate from a PEM-encoded byte slice
    pub fn cert_pem(mut self, pem: Vec<u8>) -> Self {
        self.client_cert = Some(pem);
        self
    }

    /// Set the client private key from a PEM-encoded byte slice
    pub fn key_pem(mut self, pem: Vec<u8>) -> Self {
        self.client_key = Some(pem);
        self
    }

    /// Set the CA certificate from a file path
    pub fn ca_pem_file(mut self, path: impl AsRef<std::path::Path>) -> std::result::Result<Self, Error> {
        let pem = std::fs::read(path)?;
        self.ca_cert = Some(pem);
        Ok(self)
    }

    /// Set the client certificate from a file path
    pub fn cert_pem_file(mut self, path: impl AsRef<std::path::Path>) -> std::result::Result<Self, Error> {
        let pem = std::fs::read(path)?;
        self.client_cert = Some(pem);
        Ok(self)
    }

    /// Set the client private key from a file path
    pub fn key_pem_file(mut self, path: impl AsRef<std::path::Path>) -> std::result::Result<Self, Error> {
        let pem = std::fs::read(path)?;
        self.client_key = Some(pem);
        Ok(self)
    }

    /// Set the server name for SNI (Server Name Indication)
    pub fn server_name(mut self, name: impl Into<String>) -> Self {
        self.server_name = Some(name.into());
        self
    }

    /// Disable TLS verification (InsecureSkipVerify equivalent).
    /// Mimics talosctl's WithMaintenanceMode: no CA, no client cert, no domain validation.
    pub fn insecure(mut self) -> Self {
        self.insecure = true;
        self
    }

    /// Fetch the server's self-signed certificate via raw TLS handshake.
    /// Returns the DER-encoded certificate bytes.
    async fn fetch_server_cert(host: &str, port: u16) -> std::result::Result<Vec<u8>, Error> {
        let tls_config = Self::build_insecure_rustls_config();
        let addr = format!("{}:{}", host, port);
        let stream = tokio::net::TcpStream::connect(&addr).await
            .map_err(|e| Error::Other(format!("TCP connect failed: {}", e)))?;

        let server_name: ServerName<'static> =
            ServerName::try_from(host.to_string())
                .map_err(|_| Error::Other("invalid server name".to_string()))?;

        // Use raw rustls ClientConnection to get access to peer cert
        let mut conn = rustls::ClientConnection::new(tls_config, server_name)
            .map_err(|e| Error::Other(format!("rustls connection init failed: {}", e)))?;

        let mut io = TokioIo::new(stream);
        conn.complete_io(&mut io).await
            .map_err(|e| Error::Other(format!("TLS handshake failed: {}", e)))?;

        conn.peer_cert()
            .cloned()
            .ok_or_else(|| Error::Other("server did not present a certificate".to_string()))?
            .into()
    }

    /// Build a rustls ClientConfig with AcceptAnyCert verifier and no client auth.
    fn build_insecure_rustls_config() -> rustls::ClientConfig {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("safe defaults")
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAnyCert))
            .with_no_client_auth()
    }

    /// Connect to the Talos API
    #[instrument(skip(self))]
    pub async fn connect(self) -> Result<Channel> {
        debug!("Connecting to Talos API at {} (insecure={})", self.endpoint, self.insecure);

        let channel = if self.insecure {
            info!(endpoint = %self.endpoint, "using insecure maintenance TLS (InsecureSkipVerify)");

            let parsed = url::Url::parse(&self.endpoint)
                .map_err(|e| Error::Other(format!("invalid endpoint: {}", e)))?;
            let host = parsed.host_str()
                .ok_or_else(|| Error::Other("no host in endpoint".to_string()))?;
            let port = parsed.port().unwrap_or(443);

            // Fetch the server's self-signed cert at runtime (it changes on each boot)
            debug!(%host, port, "fetching server certificate");
            let server_cert_der = Self::fetch_server_cert(host, port).await?;

            // Encode the DER cert as PEM for tonic's Certificate
            let server_cert_pem = der_to_pem(&server_cert_der);

            let ca = Certificate::from_pem(server_cert_pem.into_bytes());

            let tls_config = ClientTlsConfig::new()
                .ca_certificate(ca)
                .domain_name(host)
                .assume_http2(true);

            let endpoint = Endpoint::from_shared(self.endpoint.clone())
                .map_err(|e| Error::Other(format!("invalid endpoint: {}", e)))?;

            let channel = endpoint
                .tls_config(tls_config)
                .map_err(|e| Error::TlsConfig(format!("Failed to set TLS config: {}", e)))?
                .connect()
                .await
                .map_err(|e| Error::TlsConfig(format!("connection failed: {}", e)))?;

            info!(%host, "connected with InsecureSkipVerify");
            channel
        } else {
            debug!("Using mTLS connector");
            let ca_cert = self.ca_cert.ok_or_else(|| Error::MissingConfig("CA certificate".into()))?;
            let client_cert = self.client_cert.ok_or_else(|| Error::MissingConfig("Client certificate".into()))?;
            let client_key = self.client_key.ok_or_else(|| Error::MissingConfig("Client key".into()))?;

            let ca = Certificate::from_pem(ca_cert);
            let identity = Identity::from_pem(client_cert, client_key);

            let mut tls_config = ClientTlsConfig::new()
                .ca_certificate(ca)
                .identity(identity);

            if let Some(domain) = self.server_name {
                tls_config = tls_config.domain_name(domain);
            } else {
                if let Ok(url) = url::Url::parse(&self.endpoint) {
                    if let Some(host) = url.host_str() {
                        tls_config = tls_config.domain_name(host);
                    }
                }
            }

            Endpoint::from_shared(self.endpoint.clone())
                .map_err(|e| Error::Other(format!("invalid endpoint: {}", e)))?
                .tls_config(tls_config)
                .map_err(|e| Error::TlsConfig(format!("Failed to set TLS config: {}", e)))?
                .connect()
                .await?
        };

        debug!("Successfully connected to Talos API");
        Ok(channel)
    }
}

/// Encode DER bytes as a PEM certificate string.
fn der_to_pem(der: &[u8]) -> String {
    let b64 = base64_encode(der);
    let mut pem = String::from("-----BEGIN CERTIFICATE-----\n");
    for chunk in b64.as_bytes().chunks(64) {
        pem.push_str(std::str::from_utf8(chunk).unwrap_or(""));
        pem.push('\n');
    }
    pem.push_str("-----END CERTIFICATE-----\n");
    pem
}

/// Encode bytes as base64 string (no newlines).
fn base64_encode(data: &[u8]) -> String {
    let mut result = Vec::with_capacity((data.len() + 2) / 3 * 4);
    let table = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let i = data.as_ptr();
    let mut j = 0;
    let len = data.len();
    while j + 2 < len {
        let b0 = unsafe { *i.add(j) };
        let b1 = unsafe { *i.add(j + 1) };
        let b2 = unsafe { *i.add(j + 2) };
        result.push(table[((b0 >> 2) & 0x3F) as usize] as char);
        result.push(table[((b0 << 4 | b1 >> 4) & 0x3F) as usize] as char);
        result.push(table[((b1 << 2 | b2 >> 6) & 0x3F) as usize] as char);
        result.push(table[(b2 & 0x3F) as usize] as char);
        j += 3;
    }
    if j < len {
        let b0 = unsafe { *i.add(j) };
        result.push(table[((b0 >> 2) & 0x3F) as usize] as char);
        if j + 1 < len {
            let b1 = unsafe { *i.add(j + 1) };
            result.push(table[((b0 << 4 | b1 >> 4) & 0x3F) as usize] as char);
            result.push(table[((b1 << 2) & 0x3F) as usize] as char);
            result.push('=');
        } else {
            result.push(table[((b0 << 4) & 0x3F) as usize] as char);
            result.push('=');
            result.push('=');
        }
    }
    result.into_iter().collect()
}

/// No-op server cert verifier for insecure maintenance mode connections.
/// Equivalent to Go's tls.Config{InsecureSkipVerify: true}.
#[derive(Debug)]
struct AcceptAnyCert;

impl ServerCertVerifier for AcceptAnyCert {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message, cert, dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message, cert, dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}
