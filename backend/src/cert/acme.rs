use acme2::AccountBuilder;
use acme2::ChallengeStatus;
use acme2::Csr;
use acme2::DirectoryBuilder;
use acme2::OrderBuilder;
use acme2::AuthorizationStatus;
use acme2::Error as AcmeError;
use dashmap::DashMap;
use base64::{Engine as _, engine::general_purpose};

use crate::cert::CertError;
use crate::config::tls::ChallengeType;

const LETS_ENCRYPT_URL: &str = "https://acme-v02.api.letsencrypt.org/directory";

fn acme_error_to_cert(e: AcmeError) -> CertError {
    CertError::Acme(format!("{:?}", e))
}

fn pem_encode(data: &[u8], label: &str) -> String {
    let encoded = general_purpose::STANDARD.encode(data);
    let mut result = format!("-----BEGIN {}-----\n", label);
    for chunk in encoded.as_bytes().chunks(64) {
        result.push_str(std::str::from_utf8(chunk).unwrap());
        result.push('\n');
    }
    result.push_str(&format!("-----END {}-----\n", label));
    result
}

fn x509_der_to_pem(der: &[u8]) -> String {
    pem_encode(der, "CERTIFICATE")
}

pub async fn obtain_http01_certificate(
    domains: &[String],
    email: &str,
    acme_store: &DashMap<String, String>,
) -> Result<(String, String), CertError> {
    if domains.is_empty() {
        return Err(CertError::Config("No domains specified for certificate".to_string()));
    }

    tracing::info!(domains = ?domains, email, "Starting ACME HTTP-01 certificate issuance");

    // Connect to ACME directory
    let dir = DirectoryBuilder::new(LETS_ENCRYPT_URL.to_string())
        .build()
        .await
        .map_err(acme_error_to_cert)?;
    tracing::info!("Connected to ACME directory");

    // Create/retrieve account
    let mut account_builder = AccountBuilder::new(dir.clone());
    account_builder.contact(vec![format!("mailto:{}", email)]);
    account_builder.terms_of_service_agreed(true);
    let account = account_builder.build().await.map_err(acme_error_to_cert)?;
    tracing::info!(account_id = %account.id, "ACME account created/retrieved");

    // Create order
    let mut order_builder = OrderBuilder::new(account.clone());
    for domain in domains {
        order_builder.add_dns_identifier(domain.clone());
    }
    let order = order_builder.build().await.map_err(acme_error_to_cert)?;
    tracing::info!("ACME order created");

    // Process authorizations and set up HTTP-01 challenges
    let authorizations = order.authorizations().await.map_err(acme_error_to_cert)?;
    tracing::info!(count = authorizations.len(), "Retrieved authorizations");

    for auth in &authorizations {
        let challenge = auth
            .get_challenge("http-01")
            .ok_or_else(|| CertError::Acme("No http-01 challenge available".to_string()))?;

        let token = challenge
            .token
            .clone()
            .ok_or_else(|| CertError::Acme("Challenge missing token".to_string()))?;

        let key_auth = challenge
            .key_authorization()
            .map_err(acme_error_to_cert)?
            .ok_or_else(|| CertError::Acme("Failed to compute key authorization".to_string()))?;

        tracing::info!(
            token,
            domain = %auth.identifier.value,
            "Storing ACME HTTP-01 challenge token"
        );
        acme_store.insert(token, key_auth);
    }

    // Validate challenges and wait for authorization
    for auth in authorizations {
        let challenge = auth
            .get_challenge("http-01")
            .ok_or_else(|| CertError::Acme("No http-01 challenge available".to_string()))?;

        let domain = auth.identifier.value.clone();

        challenge.validate().await.map_err(acme_error_to_cert)?;
        tracing::info!(domain = %domain, "ACME challenge validation requested");

        let challenge = challenge
            .wait_done(std::time::Duration::from_secs(5), 12)
            .await
            .map_err(acme_error_to_cert)?;

        if challenge.status != ChallengeStatus::Valid {
            return Err(CertError::Acme(format!(
                "Challenge failed for {}: {:?}",
                domain, challenge.error
            )));
        }
        tracing::info!(domain = %domain, "ACME challenge validated");

        let auth = auth
            .wait_done(std::time::Duration::from_secs(5), 12)
            .await
            .map_err(acme_error_to_cert)?;

        if auth.status != AuthorizationStatus::Valid {
            return Err(CertError::Acme(format!(
                "Authorization not valid for {}",
                auth.identifier.value
            )));
        }
        tracing::info!(domain = %auth.identifier.value, "Authorization valid");
    }

    // Wait for order to be ready
    let order = order
        .wait_ready(std::time::Duration::from_secs(5), 12)
        .await
        .map_err(acme_error_to_cert)?;
    tracing::info!("Order is ready for finalization");

    // Generate EC P-256 key pair with rcgen (serializable)
    let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
        .map_err(|e| CertError::Acme(format!("Failed to generate EC key pair: {}", e)))?;

    // Build CSR params
    let mut cert_params = rcgen::CertificateParams::default();
    cert_params.distinguished_name = rcgen::DistinguishedName::new();
    for domain in domains {
        cert_params
            .subject_alt_names
            .push(rcgen::SanType::DnsName(
                domain.clone().try_into().map_err(|e: rcgen::Error| {
                    CertError::Acme(format!("Invalid domain for SAN: {}", e))
                })?
            ));
    }

    // Serialize CSR as DER, then wrap into openssl X509Req for Csr::Custom
    let csr = cert_params
        .serialize_request(&key_pair)
        .map_err(|e| CertError::Acme(format!("Failed to generate CSR: {}", e)))?;
    let csr_der = csr.der().as_ref();

    let x509req = openssl::x509::X509Req::from_der(csr_der)
        .map_err(|e| CertError::Acme(format!("Failed to parse CSR into X509Req: {}", e)))?;

    // Finalize order with CSR
    let order = order
        .finalize(Csr::Custom(x509req))
        .await
        .map_err(acme_error_to_cert)?;
    tracing::info!("Order finalized, waiting for certificate");

    // Wait for order completion
    let order = order
        .wait_done(std::time::Duration::from_secs(5), 12)
        .await
        .map_err(acme_error_to_cert)?;
    tracing::info!("Order complete, downloading certificate");

    // Download certificate chain
    let cert_chain = order
        .certificate()
        .await
        .map_err(acme_error_to_cert)?
        .ok_or_else(|| CertError::Acme("No certificate URL available".to_string()))?;

    // Convert X.509 certs to PEM chain
    let cert_pem: String = cert_chain
        .iter()
        .map(|x509| {
            let der = x509.to_der().map_err(|e| {
                CertError::Acme(format!("Failed to serialize X.509 cert: {}", e))
            })?;
            Ok(x509_der_to_pem(&der))
        })
        .collect::<Result<Vec<_>, CertError>>()?
        .join("");

    // Serialize our private key to PEM
    let key_pem = key_pair.serialize_pem();

    acme_store.clear();
    tracing::info!("ACME challenge tokens cleared");

    tracing::info!("Let's Encrypt certificate obtained successfully");

    Ok((cert_pem, key_pem))
}

pub struct AcmeClient {
    email: String,
    challenge_type: ChallengeType,
}

impl AcmeClient {
    pub fn new(
        email: &str,
        _dns_config: Option<crate::config::tls::DnsProviderConfig>,
        challenge_type: ChallengeType,
    ) -> Result<Self, CertError> {
        Ok(Self {
            email: email.to_string(),
            challenge_type,
        })
    }

    pub async fn obtain_certificate(
        &self,
        domains: &[String],
        acme_store: Option<&DashMap<String, String>>,
    ) -> Result<(String, String), CertError> {
        if domains.is_empty() {
            return Err(CertError::Config("No domains specified for certificate".to_string()));
        }

        match &self.challenge_type {
            ChallengeType::Http01 => {
                let store = acme_store.ok_or_else(|| {
                    CertError::Config("acme_store required for HTTP-01 challenge".to_string())
                })?;
                obtain_http01_certificate(domains, &self.email, store).await
            }
            ChallengeType::Dns01 => {
                crate::cert::self_signed::generate_self_signed(domains).await
            }
        }
    }

    pub async fn renew_certificate(&self, domains: &[String]) -> Result<(String, String), CertError> {
        self.obtain_certificate(domains, None).await
    }

    pub async fn renew_certificate_with_store(
        &self,
        domains: &[String],
        acme_store: &DashMap<String, String>,
    ) -> Result<(String, String), CertError> {
        self.obtain_certificate(domains, Some(acme_store)).await
    }
}
