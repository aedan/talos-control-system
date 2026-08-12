//! Connection builder for Talos gRPC API with mTLS support

use crate::error::{Error, Result};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use std::sync::Arc;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint, Identity};
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

    /// Disable TLS verification (accept any cert, skip client auth).
    /// Fetches the server's self-signed cert at runtime and trusts it.
    pub fn insecure(mut self) -> Self {
        self.insecure = true;
        self
    }

    /// Fetch the server's TLS cert via raw TCP, return PEM bytes.
    async fn fetch_server_cert(host: &str, port: u16) -> Result<Vec<u8>> {
        let host_owned = host.to_string();
        let addr_str = format!("{}:{}", host_owned, port);
        let stream = tokio::net::TcpStream::connect(&addr_str).await
            .map_err(|e| Error::TlsConfig(format!("connect to {}: {}", addr_str, e)))?;

        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("safe defaults")
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAnyCert))
            .with_no_client_auth();
        let connector = tokio_rustls::TlsConnector::from(Arc::new(config));

        let server_name: ServerName<'static> = ServerName::try_from(host_owned.clone())
            .map_err(|e| Error::TlsConfig(format!("invalid server name: {}", e)))?;
        let tls_stream = connector.connect(server_name, stream).await
            .map_err(|e| Error::TlsConfig(format!("TLS handshake to {}: {}", addr_str, e)))?;

        let cert_der = tls_stream.get_ref().1.peer_certificates()
            .and_then(|certs| certs.first())
            .ok_or_else(|| Error::TlsConfig("server did not present certificate".to_string()))?;

        let pem = format!(
            "-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----",
            base64_encode(cert_der.as_ref())
        );
        Ok(pem.into_bytes())
    }

    /// Connect to the Talos API
    #[instrument(skip(self))]
    pub async fn connect(self) -> Result<Channel> {
        debug!("Connecting to Talos API at {} (insecure={})", self.endpoint, self.insecure);

        let channel = if self.insecure {
            info!(endpoint = %self.endpoint, "using insecure maintenance TLS");
            let parsed = url::Url::parse(&self.endpoint)
                .map_err(|e| Error::Other(format!("invalid endpoint: {}", e)))?;
            let host = parsed.host_str()
                .ok_or_else(|| Error::Other("no host in endpoint".to_string()))?;
            let port = parsed.port().unwrap_or(443);

            let server_ca_pem = Self::fetch_server_cert(host, port).await?;
            info!(%host, port, "trusted server cert");

            let mut tls_config = ClientTlsConfig::new()
                .ca_certificate(Certificate::from_pem(server_ca_pem));
            tls_config = tls_config.domain_name(host);

            Endpoint::from_shared(self.endpoint.clone())
                .map_err(|e| Error::Other(format!("invalid endpoint: {}", e)))?
                .tls_config(tls_config)
                .map_err(|e| Error::TlsConfig(format!("TLS config error: {}", e)))?
                .connect()
                .await
                .map_err(|e| Error::TlsConfig(format!("connection failed: {}", e)))?
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

/// Base64-encode bytes with 64-char line wrapping (standard PEM encoding).
fn base64_encode(data: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let len = data.len();
    let mut out = Vec::with_capacity((len + 2) * 4 / 3);
    for chunk in data.chunks(3) {
        let a = chunk[0] as u32;
        let b = chunk.get(1).copied().unwrap_or(0) as u32;
        let c = chunk.get(2).copied().unwrap_or(0) as u32;
        let triple = (a << 16) | (b << 8) | c;
        out.push(TABLE[((triple >> 18) & 0x3F) as usize]);
        out.push(TABLE[((triple >> 12) & 0x3F) as usize]);
        out.push(TABLE[((triple >> 6) & 0x3F) as usize]);
        out.push(TABLE[(triple & 0x3F) as usize]);
    }
    let total = out.len();
    match len % 3 {
        1 => {
            out[total - 2] = b'=';
            out[total - 1] = b'=';
        }
        2 => {
            out[total - 1] = b'=';
        }
        _ => {}
    }
    let s = String::from_utf8(out).unwrap();
    let mut result = String::with_capacity(s.len() + s.len() / 64);
    for (i, line) in s.as_bytes().chunks(64).enumerate() {
        if i > 0 {
            result.push('\n');
        }
        for &b in line {
            result.push(b as char);
        }
    }
    result
}

/// No-op server cert verifier for fetching the installer's self-signed cert.
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
