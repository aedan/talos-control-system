//! Connection builder for Talos gRPC API with mTLS support

use crate::error::{Error, Result};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use std::sync::Arc;
use std::io::{Read, Write};
use std::pin::Pin;
use std::task::{Context, Poll};
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint, Identity};
use hyper_util::rt::TokioIo;
use hyper_util::rt::is_http2::IsConnectionHttp2;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
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

    /// Build a rustls ClientConfig with AcceptAnyCert verifier, no client auth, and ALPN h2.
    fn build_insecure_rustls_config() -> rustls::ClientConfig {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut config = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("safe defaults")
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAnyCert))
            .with_no_client_auth();
        config.alpn_protocols = vec![b"h2".to_vec()];
        config
    }

    /// Connect to the Talos API
    #[instrument(skip(self))]
    pub async fn connect(self) -> Result<Channel> {
        debug!("Connecting to Talos API at {} (insecure={})", self.endpoint, self.insecure);

        let channel = if self.insecure {
            // Match talosctl WithMaintenanceMode: InsecureSkipVerify, no client cert,
            // custom HTTP/2 window sizes (same as talosctl's makeConnection).
            info!(endpoint = %self.endpoint, "using insecure maintenance TLS (InsecureSkipVerify)");

            let parsed = url::Url::parse(&self.endpoint)
                .map_err(|e| Error::Other(format!("invalid endpoint: {}", e)))?;
            let host = parsed.host_str()
                .ok_or_else(|| Error::Other("no host in endpoint".to_string()))?;

            // Build rustls config: AcceptAnyCert, no client auth (InsecureSkipVerify)
            let endpoint = Endpoint::from_shared(self.endpoint.clone())
                .map_err(|e| Error::Other(format!("invalid endpoint: {}", e)))?;

            // Use custom connector: TCP + rustls with AcceptAnyCert
            let connector = InsecureConnector::new(Self::build_insecure_rustls_config());

            // Match talosctl's window sizes (applied via tower service if available)
            let channel = endpoint
                .connect_with_connector(connector)
                .await
                .map_err(|e| Error::TlsConfig(format!("connection failed: {}", e)))?;

            info!(%host, "connected with InsecureSkipVerify");
            channel
        } else {
            // Standard mTLS path
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

/// Tower service that wraps TCP in rustls with AcceptAnyCert (InsecureSkipVerify).
struct InsecureConnector {
    tls_config: Arc<rustls::ClientConfig>,
}

impl InsecureConnector {
    fn new(tls_config: rustls::ClientConfig) -> Self {
        Self {
            tls_config: Arc::new(tls_config),
        }
    }
}

/// Wrapper around TLS stream that signals HTTP/2 to hyper via `IsConnectionHttp2`.
struct Http2TlsStream(tokio_rustls::client::TlsStream<tokio::net::TcpStream>);

impl Read for Http2TlsStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        Pin::new(&mut self.0).read(buf)
    }
}

impl Write for Http2TlsStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        Pin::new(&mut self.0).write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Pin::new(&mut self.0).flush()
    }
}

impl AsyncRead for Http2TlsStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl AsyncWrite for Http2TlsStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

impl IsConnectionHttp2 for Http2TlsStream {
    fn is_http2(&self) -> bool {
        true
    }
}

impl tower::Service<http::Uri> for InsecureConnector {
    type Response = TokioIo<Http2TlsStream>;
    type Error = std::io::Error;
    type Future = std::pin::Pin<Box<dyn std::future::Future<Output = std::result::Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(
        &mut self,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::result::Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: http::Uri) -> Self::Future {
        let tls_config = self.tls_config.clone();
        let host = req.host().unwrap_or("").to_string();
        let port = req.port_u16().unwrap_or(443);
        Box::pin(async move {
            let addr = format!("{}:{}", host, port);
            let stream = tokio::net::TcpStream::connect(&addr).await?;
            let server_name: ServerName<'static> =
                ServerName::try_from(host).map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid server name"))?;
            let connector = tokio_rustls::TlsConnector::from(tls_config);
            let tls_stream = connector.connect(server_name, stream).await
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::ConnectionAborted, e.to_string()))?;
            Ok(TokioIo::new(Http2TlsStream(tls_stream)))
        })
    }
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
