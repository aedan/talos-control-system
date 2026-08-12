//! Connection builder for Talos gRPC API with mTLS support

use crate::error::{Error, Result};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::CertificateDer;
use std::net::ToSocketAddrs;
use std::sync::Arc;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint, Identity};
use tracing::{debug, instrument, info};

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
    /// Use this for maintenance-mode connections where the installer
    /// serves a self-signed certificate.
    pub fn insecure(mut self) -> Self {
        self.insecure = true;
        self
    }

    /// Fetch the server's TLS certificate by connecting via raw TCP.
    async fn fetch_server_cert(addr: &str, port: u16) -> Result<Vec<u8>> {
        info!(%addr, port, "fetching server TLS cert for insecure connection");
        let addr_str = format!("{}:{}", addr, port);
        let socket = tokio::net::TcpStream::connect(addr_str.as_str())
            .await
            .map_err(|e| Error::TlsConfig(format!("Failed to connect to {}: {}", addr_str, e)))?;

        // Wrap with TLS using a permissive config
        let config = tokio_rustls::TlsConnector::from(Arc::new(
            rustls::ClientConfig::builder()
                .with_safe_defaults()
                .with_custom_certificate_verifier(Arc::new(NoVerifier))
                .with_no_client_auth(),
        ));

        let server_name = rustls::pki_types::ServerName::try_from(addr)
            .map_err(|e| Error::TlsConfig(format!("Invalid server name: {}", e)))?;

        let mut tls_stream = config.connect(server_name, socket)
            .await
            .map_err(|e| Error::TlsConfig(format!("TLS handshake failed: {}", e)))?;

        // Read the server cert from the TLS connection
        let cert = tls_stream
            .peer_certificate()
            .ok_or_else(|| Error::TlsConfig("Server did not present a certificate".to_string()))?
            .clone();

        // Convert DER to PEM using rustls-pki-types
        let pem = cert.to_pem()
            .map_err(|e| Error::TlsConfig(format!("PEM encode failed: {}", e)))?;
        Ok(pem.into_bytes())
    }

    /// Connect to the Talos API
    #[instrument(skip(self))]
    pub async fn connect(self) -> Result<Channel> {
        debug!("Connecting to Talos API at {} (insecure={})", self.endpoint, self.insecure);

        let channel = if self.insecure {
            info!(endpoint=%self.endpoint, "using insecure maintenance TLS");
            // Parse host and port from endpoint URL
            let parsed = url::Url::parse(&self.endpoint)
                .map_err(|e| Error::Other(format!("Invalid endpoint: {}", e)))?;
            let host = parsed.host_str()
                .ok_or_else(|| Error::Other("No host in endpoint".to_string()))?;
            let port = parsed.port().unwrap_or(443);

            // Fetch the server's self-signed cert and trust it
            let server_ca_pem = Self::fetch_server_cert(host, port).await
                .map_err(|e| Error::TlsConfig(format!("Failed to fetch server cert: {}", e)))?;
            let server_ca = Certificate::from_pem(server_ca_pem);

            let mut tls_config = ClientTlsConfig::new()
                .ca_certificate(server_ca);

            tls_config = tls_config.domain_name(host);

            Endpoint::from_shared(self.endpoint.clone())
                .map_err(|e| Error::Other(format!("Invalid endpoint: {}", e)))?
                .tls_config(tls_config)
                .map_err(|e| Error::TlsConfig(format!("Failed to configure TLS: {}", e)))?
                .connect()
                .await
                .map_err(|e| Error::TlsConfig(format!("Failed to connect: {}", e)))?
        } else {
            debug!("Using mTLS connector");
            let ca_cert = self
                .ca_cert
                .ok_or_else(|| Error::MissingConfig("CA certificate".to_string()))?;
            let client_cert = self
                .client_cert
                .ok_or_else(|| Error::MissingConfig("Client certificate".to_string()))?;
            let client_key = self
                .client_key
                .ok_or_else(|| Error::MissingConfig("Client key".to_string()))?;

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
                .map_err(|e| Error::Other(format!("Invalid endpoint: {}", e)))?
                .tls_config(tls_config)
                .map_err(|e| Error::TlsConfig(format!("Failed to set TLS config: {}", e)))?
                .connect()
                .await?
        };

        debug!("Successfully connected to Talos API");
        Ok(channel)
    }
}

/// No-op certificate verifier for fetching the server cert
#[derive(Debug)]
struct NoVerifier;

impl rustls::client::danger::ServerCertVerifier for NoVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> std::result::Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}
