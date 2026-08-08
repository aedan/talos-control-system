use rcgen::{CertificateParams, DistinguishedName, KeyPair, SanType};
use crate::cert::CertError;

pub async fn generate_self_signed(domains: &[String]) -> Result<(String, String), CertError> {
    if domains.is_empty() {
        return Err(CertError::Config("At least one domain must be provided".to_string()));
    }

    let mut params = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(rcgen::DnType::CommonName, &domains[0]);
    params.distinguished_name = dn;

    for domain in domains {
        params.subject_alt_names.push(SanType::DnsName(
            domain.clone().try_into().map_err(|_| {
                CertError::Cert(format!("Invalid domain name: {}", domain))
            })?,
        ));
    }

    tracing::info!(domains = ?domains, "Generating self-signed certificate");

    let key_pair = KeyPair::generate().map_err(|e| {
        CertError::Cert(format!("Failed to generate key pair: {}", e))
    })?;

    let cert = params.self_signed(&key_pair).map_err(|e| {
        CertError::Cert(format!("Failed to generate self-signed certificate: {}", e))
    })?;

    let key = key_pair.serialize_pem();
    let cert_pem = cert.pem();

    tracing::info!("Self-signed certificate generated successfully");

    Ok((cert_pem, key))
}
