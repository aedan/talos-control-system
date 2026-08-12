//! Connection builder for Talos gRPC API with mTLS support

use std::sync::Arc;

use crate::error::{Error, Result};
use futures_util::future::BoxFuture;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::ClientConfig;
use tokio_rustls::client::TlsStream;
use tokio::net::TcpStream;
use tower_service::Service;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint, Identity};
use tracing::{debug, instrument};

/// Insecure TLS connector that accepts any certificate.
struct InsecureTlsConnector {
    tls_config: tokio_rustls::TlsConnector,
}

impl InsecureTlsConnector {
    fn new() -> std::result::Result<Self, Error> {
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
                    message,
                    cert,
                    dss,
                    &rustls::crypto::ring::default_provider()
                        .signature_verification_algorithms,
                )
            }

            fn verify_tls13_signature(
                &self,
                message: &[u8],
                cert: &CertificateDer<'_>,
                dss: &rustls::DigitallySignedStruct,
            ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
                rustls::crypto::verify_tls13_signature(
                    message,
                    cert,
                    dss,
                    &rustls::crypto::ring::default_provider()
                        .signature_verification_algorithms,
                )
            }

            fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
                rustls::crypto::ring::default_provider()
                    .signature_verification_algorithms
                    .supported_schemes()
            }
        }

        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config: ClientConfig = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("safe default protocol versions should always work")
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAnyCert))
            .with_no_client_auth();

        Ok(Self {
            tls_config: tokio_rustls::TlsConnector::from(Arc::new(config)),
        })
    }
}

impl Service<http::Uri> for InsecureTlsConnector {
    type Response = hyper_util::rt::TokioIo<TlsStream<TcpStream>>;
    type Error = std::io::Error;
    type Future = BoxFuture<'static, std::result::Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut std::task::Context<'_>) -> std::task::Poll<std::result::Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: http::Uri) -> Self::Future {
        let mut tls_config = self.tls_config.clone();
        let host = req.host().unwrap_or("localhost").to_string();
        let port = req.port_u16().unwrap_or(443);
        Box::pin(async move {
            let stream = TcpStream::connect(format!("{}:{}", host, port)).await?;
            let server_name = ServerName::try_from(host.clone())
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
            let tls_stream = tls_config.connect(server_name, stream)
                .await
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
            Ok(hyper_util::rt::TokioIo::new(tls_stream))
        })
    }
}

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

    /// Connect to the Talos API
    #[instrument(skip(self))]
    pub async fn connect(self) -> Result<Channel> {
        debug!("Connecting to Talos API at {}", self.endpoint);

        let channel = if self.insecure {
            let connector = InsecureTlsConnector::new()?;
            Endpoint::from_shared(self.endpoint.clone())
                .map_err(|e| Error::Other(format!("Invalid endpoint: {e}")))?
                .connect_with_connector(connector)
                .await
                .map_err(|e| Error::TlsConfig(format!("Failed to connect: {e}")))?
        } else {
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
                .map_err(|e| Error::Other(format!("Invalid endpoint: {e}")))?
                .tls_config(tls_config)
                .map_err(|e| Error::TlsConfig(format!("Failed to set TLS config: {e}")))?
                .connect()
                .await?
        };

        debug!("Successfully connected to Talos API");
        Ok(channel)
    }
}
